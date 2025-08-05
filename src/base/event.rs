use crate::{atomic::*, base::{lot::AsyncLotSignature, signal::ValidityMarker}, yielder::Yield};
use core::{
    ops::ControlFlow,
    pin::{Pin, pin},
    task::{Context, Poll},
};

use core::future::Future;


/// Describes an event interface that can be created in
/// a set or unset state and can be waited upon. 
/// 
/// Implementing this wrong in and of itself will not cause
/// any undefined behaviour, but great caution should be taken
/// in which events you rely on for your asynchronous primitives.
/// 
/// For instance, if you are writing a mutex and are depending
/// on an unsound implementation of an [EventSetter], this could
/// cause immediate and catastrophic undefined behaviour.
pub trait EventSetter<'a> {
    /// The future representing a pending acquisition of the event.
    type Waiter: Future<Output = ()> + 'a + Unpin;

    /// Creates a new event in the unset state. This
    /// is the expected mode of operation for most events,
    /// and means that if a task begins waiting, they will
    /// wait until the event is set.
    fn new() -> Self;
    /// Creates an event in the set state. This means that the event
    /// can be immediately acquired.
    fn new_set() -> Self;
    /// Waits on the event. This returns a future which when polled
    /// will wait for the event to be acquired.
    fn wait(&'a self) -> Self::Waiter;
    /// Sets the event. This will wake up any events from the queue if
    /// any are pending. This returns a boolean if we were able to actually
    /// wake up an event or not.
    fn set_one(&self) -> bool;
    /// Sets the event and wakes up all events from the queue if they
    /// are pending with the functor. This functor allows more advanced book-keeping
    /// per event, for instance with counted events.
    fn set_all<F: FnMut()>(&self, functor: F);
    /// Will try to immediately acquire the event along the fast path. This
    /// method is non-blocking and will return true if we could acquire the event,
    /// and false if we cannot. This will clear the set flag.
    fn try_wait(&self) -> bool;
    /// If the event has any pending waiters.
    fn has_waiters(&self) -> bool;
   
}


pub(crate) trait EventState {
    fn new(state: u8) -> Self;
    fn cmpxchng_weak(
        &self,
        current: u8,
        new: u8,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u8, u8>;
    fn store(
        &self,
        value: u8,
        ordering: Ordering
    );
}

pub(crate) struct RawEvent<S, L>
{
    state: S,
    lot: L,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawEventAwaitState {
    /// This is a waiter that was just created, so there
    /// has been no interaction with queues.
    Init,
    /// This is a waiter that has been polled and put in
    /// a queue.
    Waiting,
    /// This is a waiter that has fully completed it's lifecycle,
    /// and has no remaining state.
    Completed,
}

const BACKOFF_MAX: usize = 3;
const SIGNAL_FREE: u8 = 0b00;
const SIGNAL_SET: u8 = 0b10;

pub(crate) struct RawEventAwait<'a, S, L>
where
    L: AsyncLotSignature,
{
    event: &'a RawEvent<S, L>,
    state: RawEventAwaitState,
    backoff: usize,
    yielder: Option<Yield>,
    wait_link: Option<L::ValidityMarker>,
}

impl<'a, S, L> RawEventAwait<'a, S, L>
where
    L: AsyncLotSignature,
    S: EventState,
    Self: Unpin
{
    #[inline]
    fn park_waker(&mut self, cx: &Context<'_>) -> L::ValidityMarker {
        self.state = RawEventAwaitState::Waiting;
        self.event.lot.park(cx)
    }
    #[inline]
    fn backoff(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.backoff += 1;

        if self.backoff <= BACKOFF_MAX {
            // Straight up return.
            core::hint::spin_loop();
            return self.poll(cx);
        } else {
            self.yielder = Some(Yield::default());
            return self.poll(cx);
        }
    }

    #[inline]
    fn try_yield(&mut self, cx: &mut Context<'_>) -> ControlFlow<()> {
        if let Some(mut yielder) = self.yielder.take() {
            match pin!(&mut yielder).poll(cx) {
                Poll::Pending => {
                    self.yielder = Some(yielder);
                    ControlFlow::Continue(())
                }
                Poll::Ready(()) => {
                    // here we proceed.
                    ControlFlow::Break(())
                }
            }
        } else {
            ControlFlow::Break(())
        }
    }
}

impl<'a, S, L> Future for RawEventAwait<'a, S, L>
where 

    S: EventState,
    L: AsyncLotSignature,
    Self: Unpin
{
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.state == RawEventAwaitState::Completed {
            // If we are already completed then this should
            // just return ready again.
            return Poll::Ready(());
        }

        // Here we check if we are yielding, this is an asynchronous version of
        // backoff. If this returns `Continue` then we must poll again.
        if let ControlFlow::Continue(()) = self.try_yield(cx) {
            return Poll::Pending;
        }

        // First we want to check if the signal is set, if so we can just bypass all this checking.
        match self
            .event
            .state
            .cmpxchng_weak(SIGNAL_SET, SIGNAL_FREE, Acquire, Acquire)
        {
            Ok(_) => {
                // The optimistic path has succeeded, this means we came in just as the signal was set (already set).
                // For this to be fair, we can only return here if there are no other threads enqueued. If there are
                // other threads enqueued and we were to return here then the last person to the critical section wins,
                // and that is not fair.

                // The exception is if we were already parked. In this case, we can just unpark the next thread and then return ourselves.
                if self.state == RawEventAwaitState::Waiting {
                    // Release.
                    return release_complete(&mut *self);
                }

                // SOLUTION:
                // Here we unpark a thread, this will tell us if there is another thread or not. If there is,
                // then we will add the current thread into the queue so that this is fair.
                if self.event.lot.unpark_one() {
                    // In this case there was another thread already there, therefore we must park the current thread.
                    self.wait_link = Some(self.park_waker(cx));
                    return Poll::Pending;
                } else {
                    // There was nothing there, thus we are exclusive and
                    // can immediately return. In this case, we never actually parked
                    // and so we don't need to notify our waitlist that a waiter
                    // has awoken.
                    self.state = RawEventAwaitState::Completed;
                    return Poll::Ready(());
                }
            }
            Err(x) => {
                match x {
                    SIGNAL_FREE => {
                        // We need to recognize the case in which we were PREVIOUSLY parked and
                        // are no longer parked. In this case we were awakened on a set, and the
                        // bit was cleared in the meantime.
                        //
                        // This sort of situation can often happen when two `set()` calls are made back
                        // to back. In this case let's say Thread A & B are enqueued. Then the first set() call
                        // wakes `A`, `A` unsets the bit, and then when thread `B` is polled it believes it
                        // was called by mistake. Thus, we return here.
                        if self.state == RawEventAwaitState::Waiting {
                            return release_complete(&mut *self);
                        }

                        // The wait flag was set by someone else, so the
                        // work has been done for us. This is nice, so let's
                        // just park and wait.
                        self.wait_link = Some(self.park_waker(cx));
                        return Poll::Pending;
                    }
                    SIGNAL_SET => {
                        // If this is set, then this means there was
                        // a spurious failure, or another thread came in
                        // and set it at the same time.
                        //
                        // In this case we can backoff and try agian.
                        return self.backoff(cx);
                    }

                    _ => {
                        // SAFETY: Only the first bit is ever altered.
                        unsafe { core::hint::unreachable_unchecked() };
                    }
                }
            }
        }
    }
}

impl<'a, S, L> EventSetter<'a> for RawEvent<S, L>
where 
    S: EventState ,
    L: AsyncLotSignature,
    Self: 'a
{
    type Waiter = RawEventAwait<'a, S, L>;

    #[inline]
    fn new() -> Self {
        Self {
            lot: L::default(),
            state: S::new(SIGNAL_FREE)
        }
    }
    #[inline]
    fn new_set() -> Self {
        Self {
            lot: L::default(),
            state: S::new(SIGNAL_SET)
        }
    }

    #[inline]
    fn set_all<F: FnMut()>(&self, mut functor: F) {
        let mut did_signal = false;
        while self.lot.unpark_one() {
            did_signal = true;
            functor();
        }
        if !did_signal {
            self.state.store(SIGNAL_SET, Release);
        }
    }

    #[inline]
    fn set_one(&self) -> bool {
        if !self.lot.unpark_one() {
            self.state.store(SIGNAL_SET, Release);
            false
        } else {
            true
        }
    }

    #[inline]
    fn try_wait(&self) -> bool {
        !self.has_waiters()
         && self.state.cmpxchng_weak(SIGNAL_SET, SIGNAL_FREE, Acquire, Relaxed)
         .is_ok()
    }

    #[inline]
    fn wait(&'a self) -> Self::Waiter {
        RawEventAwait {
            backoff: 0,
            event: &self,
            state: RawEventAwaitState::Init,
            wait_link: None,
            yielder: None
        }
    }

    #[inline]
    fn has_waiters(&self) -> bool {
        self.lot.has_waiters()
    }

}



#[inline(always)]
fn release_complete<S, L>(waiter: &mut RawEventAwait<'_, S, L>) -> Poll<()>
where 
    L: AsyncLotSignature
{
    waiter.event.lot.notify_wait_release();
    release_partial(waiter)
}

#[inline(always)]
fn release_partial<S, L>(
    waiter: &mut RawEventAwait<'_, S, L>
) -> Poll<()>
where 
    L: AsyncLotSignature
{
    waiter.state = RawEventAwaitState::Completed;
    Poll::Ready(())
}



impl<'a, S, L> Drop for RawEventAwait<'a, S, L>
where 
    L: AsyncLotSignature
{
    fn drop(&mut self) {
        match self.state {
            RawEventAwaitState::Init => {
                // No state modified, no special handling required.
            }
            RawEventAwaitState::Waiting => {
                // We have modified the state and are waiting in
                // the queue.
                if let Some(waiter) = &self.wait_link {
                    waiter.set(false);
                    // self.event.waker.notify_wait_release();
                    // TODO: Unset bit here.
                    self.event.lot.notify_wait_release();
                }
            }
            RawEventAwaitState::Completed => {
                // We once did modify the state but we have completely
                // reversed all these changes.
            }
        }
    }
}


macro_rules! impl_async_event {
    (
        name = $name:ident,
        waitername = $waitername:ident,
        lot = $lot:ident,
        state = $state:ident
    ) => {
        
        
        pub struct $name(crate::base::event::RawEvent<$state, $lot>);

        #[pin_project::pin_project]
        pub struct $waitername<'a>(#[pin] crate::base::event::RawEventAwait<'a, $state, $lot>);
        

        impl<'a> Future for $waitername<'a> {
            type Output = ();

            fn poll(
                self: core::pin::Pin<&mut Self>,
                ctx: &mut core::task::Context<'_>
            ) -> core::task::Poll<()> {
                self
                    .project()
                    .0
                    .poll(ctx)
               
            }
        }

        impl<'a> crate::base::event::EventSetter<'a> for $name {
            type Waiter = $waitername<'a>;

            #[inline]
            fn new() -> Self {
                Self(crate::base::event::RawEvent::new())
            }

            #[inline]
            fn new_set() -> Self {
                Self(crate::base::event::RawEvent::new_set())
            }

            #[inline]
            fn wait(&'a self) -> $waitername<'a> {
                $waitername(crate::base::event::RawEvent::wait(&self.0))
            }

            #[inline]
            fn try_wait(&self) -> bool {
                crate::base::event::RawEvent::try_wait(&self.0)
            }

            #[inline]
            fn set_one(&self) -> bool {
                crate::base::event::RawEvent::set_one(&self.0)
            }

            #[inline]
            fn set_all<F: FnMut()>(&self, functor: F) {
                crate::base::event::RawEvent::set_all(&self.0, functor)
            }

            #[inline]
            fn has_waiters(&self) -> bool {
                crate::base::event::RawEvent::has_waiters(&self.0)
            }

        }

        impl Default for $name {
            fn default() -> Self {
                <Self as crate::EventSetter>::new()
            }

        }
    };
}

pub(crate) use impl_async_event;
use std::{
    ops::ControlFlow,
    pin::{Pin, pin},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

mod atomic;
use crate::{asyncstd::Event, base::{event::EventSetter, lot::AsyncLotSignature}, yielder::Yield};
use pin_project::pinned_drop;


mod yielder;
mod base;
mod asyncstd;
mod nostds;


use crate::atomic::*;


/// A [CountedEvent] wraps a type that implements the [EventSetter] trait. It can tell
/// you how many waiters there are on that very event. The count will only go up once the
/// waiter actually starts waiting on the event. If there is an immediate return on the first
/// time the wait is polled, the count will not go up.
///
/// The count will go up if the first call to poll wait returns [Poll::Pending], indicating that
/// the waiter is actually waiting on notification. Upon notification, the count will
/// be decremented.
///
/// # Cancel Safety
/// On [Drop] the count is decremented if the [CountedAwaiter] had already
/// incremented the count and is not yet in the complete state.
pub struct CountedEvent<E> {
    inner: E,
    counter: AtomicUsize,
}

#[pin_project::pin_project(PinnedDrop)]
#[allow(private_bounds)]
pub struct CountedAwaiter<'a, E, F>
where
    E: EventSetter<'a>,
    F: Future<Output = ()>,
{
    master: &'a CountedEvent<E>,
    #[pin]
    future: F,
    state: CountedAwaiterState,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CountedAwaiterState {
    /// The waiter has just been created and has
    /// not yet incremented the count.
    Init,
    /// The waiter is in the waiting state, therefore
    /// there has been an increase in the count
    Waiting,
    /// The waiter is actually complete, meaning that the
    /// count has been decremented already.
    Complete,
}

#[pinned_drop]
impl<'a, E, F> PinnedDrop for CountedAwaiter<'a, E, F>
where
    E: EventSetter<'a>,
    F: Future<Output = ()>,
{
    fn drop(self: Pin<&mut Self>) {
        match self.state {
            CountedAwaiterState::Init => {
                // If we are initializing then we have not actually
                // modified any of the state, so we are in the clear.
            }
            CountedAwaiterState::Waiting => {
                // If we are in the waiting state then we have incremented
                // the counter, so we need to undo this operation.
                self.master.counter.fetch_sub(1, Ordering::Release);
            }
            CountedAwaiterState::Complete => {
                // If we are complete than there is nothing to
                // do so we do not execute any special drop code.
            }
        }
    }
}

impl<'a, E, F> Future for CountedAwaiter<'a, E, F>
where
    E: EventSetter<'a>,
    F: Future<Output = ()> + Unpin,
{
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // NOTE ON STATE: State is very important mostly for the reason of
        // properly undoing things when stuff gets cancelled (dropped).
        match this.state {
            CountedAwaiterState::Init => {
                // We are currently initializing. In this case we actually need
                // to increment the waiting variables if there is not an immediate return.
                match this.future.poll(cx) {
                    Poll::Pending => {
                        // In this case we have polled our future, and clearly have not immediately returned,
                        // and thus we must increment the count and transition to the waiting
                        // state.
                        this.master.counter.fetch_add(1, Ordering::Release);
                        *this.state = CountedAwaiterState::Waiting;
                        Poll::Pending
                    }
                    Poll::Ready(_) => {
                        // This is a special case of a fast return. This immediately short circuits
                        // the future, and thus there is no need to increment the count.
                        Poll::Ready(())
                    }
                }
            }
            CountedAwaiterState::Waiting => {
                // We need to check if we are ready to complete.
                match this.future.poll(cx) {
                    // If we are still pending, then there
                    // is nothing to be said here and we just return.
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(_) => {
                        // If we are ready then we decrement the count.
                        this.master.counter.fetch_sub(1, Ordering::Release);
                        *this.state = CountedAwaiterState::Complete;
                        Poll::Ready(())
                    }
                }
            }
            CountedAwaiterState::Complete => {
                // The future should never really be polled here but
                // if we do then we are ready.
                Poll::Ready(())
            }
        }
    }
}

#[allow(private_bounds)]
impl<E> CountedEvent<E>
where
    for<'a> E: EventSetter<'a>,
{
    #[allow(private_interfaces)]
    pub fn new(source: E) -> Self {
        Self {
            inner: source,
            counter: AtomicUsize::new(0),
        }
    }

    #[allow(private_interfaces)]
    pub fn wait(&self) -> <Self as EventSetter>::Waiter {
        <Self as EventSetter>::wait(&self)
    }

    #[allow(private_interfaces)]
    pub fn set_one(&self) {
        <Self as EventSetter>::set_one(&self);
    }

    #[allow(private_interfaces)]
    pub fn set_all(&self) {
        <Self as EventSetter>::set_all(&self, || {});
    }

    pub fn count(&self) -> usize {
        self.counter.load(Acquire)
    }
}

impl<'a, E> EventSetter<'a> for CountedEvent<E>
where
    E: EventSetter<'a> + 'a,
{
    type Waiter = CountedAwaiter<'a, E, E::Waiter>;


    fn new() -> Self {
        Self {
            counter: AtomicUsize::default(),
            inner: E::new()
        }
    }

    fn new_set() -> Self {
        Self {
            counter: AtomicUsize::default(),
            inner: E::new_set()
        }
    }

    fn wait(&'a self) -> CountedAwaiter<'a, E, E::Waiter> {
        // self.counter.fetch_add(1, Release);
        CountedAwaiter {
            master: self,
            future: self.inner.wait(),
            state: CountedAwaiterState::Init,
        }
    }

    fn try_wait(&self) -> bool {
        if self.inner.try_wait() {
            // self.counter.fetch_sub(1, Ordering::Release);
            true
        } else {
            false
        }
    }

    fn set_all<F: FnMut()>(&self, mut functor: F) {
        self.inner.set_all(|| {
            functor();
            self.counter.fetch_sub(1, Ordering::Release);
        });
    }
    fn set_one(&self) -> bool {
        if self.inner.set_one() {
            // self.counter.fetch_sub(1, Ordering::Release);
            true
        } else {
            false
        }
    }
    fn has_waiters(&self) -> bool {
        self.inner.has_waiters()
    }
}


#[cfg(test)]
mod tests {
    use std::{
        pin::{pin, Pin}, sync::{atomic::Ordering, Arc}, task::{Context, Wake, Waker}, thread::sleep, time::Duration
    };

    // use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter}

    use crate::{CountedEvent, Event, EventSetter};

    /// Checks if a future will immediately return.
    fn can_poll_immediate(fut: impl Future) -> bool {
        let pinned = core::pin::pin!(fut);
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        pinned.poll(&mut ctx).is_ready()
    }


    /// Performs most of the heavy lifting on the drop
    /// cancel tests, sets up a cancelled task.
    fn setup_drop_cancel_test<'a>(event: &'a impl EventSetter<'a>) {
        {
            // Let's do a basic waker setup, as we are interested
            // in interacting with the future at various set
            // polling points!
            let mut waiter = core::pin::pin!(event.wait());
            let waker = Waker::from(Arc::new(TestWaker {}));
            let mut context = Context::from_waker(&waker);

            // Let it park itself. Now the future is actually properly
            // loaded and we can start trying to cause dangerous things
            // to happen.
            assert!(waiter.as_mut().poll(&mut context).is_pending());

            // The drop should occur here as these variables
            // go out of scope.
        }
    }


    #[test]
    pub fn test_countable_correctness() {
        // TEST: This verifies the correctness of the countable
        // event. To see the exact invariants this is upholding,
        // check the documentation for CountedEvent.

        let event = CountedEvent::new(Event::new());
        assert_eq!(event.count(), 0); // initial count should be zero

        // Create some async context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        // Create a wait event.
        let mut waiting = core::pin::pin!(event.wait());
        // Here the count should STILL be zero since it has not actually been awoken.
        assert_eq!(event.count(), 0);

        // This should poll to pending, and the count goes to one.
        assert!(waiting.as_mut().poll(&mut ctx).is_pending());
        assert_eq!(event.count(), 1);

        // Now we will wake up the event.
        event.set_one();
        assert!(waiting.as_mut().poll(&mut ctx).is_ready());
        assert_eq!(event.count(), 0);
    }

    #[test]
    pub fn test_event_hotpath() {
        // TEST: Checks that the hotpath code works correctly,
        // this makes it so if an event is set and then waited upon,
        // it immediately polls ready.

        // Setup an event.
        let event = Event::new();

        // Setup the context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        // Wakes up one event.
        event.set_one();

        // Should immediately poll to true.
        assert!(core::pin::pin!(event.wait()).poll(&mut ctx).is_ready());
    }

    #[test]
    pub fn event_has_waiter_with_priors() {
        // TEST: Checks if the event can report on if it
        // has waiters properly. This case specifically
        // explores if the bit is reset correctly when priors are involved.
        let event = Event::new();
        assert!(!event.has_waiters());

        // Configure an async context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        // Now we check and verify that it is properly
        // pending.
        let mut waiter1 = core::pin::pin!(event.wait());
        let mut waiter2 = core::pin::pin!(event.wait());
        assert!(waiter1.as_mut().poll(&mut ctx).is_pending());
        assert!(waiter2.as_mut().poll(&mut ctx).is_pending());

        // The event should now have waiters.
        assert!(event.has_waiters());

        // Now we free the event.
        event.set_one();

        // We have not polled it yet, so it should still have waiters.
        assert!(event.has_waiters());

        // The future resolves successfully.
        assert!(waiter1.as_mut().poll(&mut ctx).is_ready());

        // The event should still have waiters, because we have another
        // event that is yet to wakeup.
        assert!(event.has_waiters());
    }

    #[test]
    pub fn event_has_waiter() {
        // TEST: Checks if the event can report on if it
        // has waiters properly.
        let event = Event::new();
        assert!(!event.has_waiters());

        // Configure an async context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        // Now we check and verify that it is properly
        // pending.
        let mut waiter = core::pin::pin!(event.wait());
        assert!(waiter.as_mut().poll(&mut ctx).is_pending());

        // The event should now have waiters.
        assert!(event.has_waiters());

        // Now we free the event.
        event.set_one();

        // We have not polled it yet, so it should still have waiters.
        assert!(event.has_waiters());

        // The future resolves successfully.
        assert!(waiter.as_mut().poll(&mut ctx).is_ready());

        // The event has no more waiters.
        assert!(!event.has_waiters());
    }

 
    #[test]
    pub fn test_event_standard() {
        // TEST: Checks the event under a fairly standard
        // cycle, create, and then wakup.
        let event = Event::new();

        // Setup the context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        // This should be pending.
        let mut waiter = event.wait();
        assert!(core::pin::pin!(&mut waiter).poll(&mut ctx).is_pending());

        // Now we wake up the event.
        event.set_one();

        // Now we should poll again and we'll notice that
        // we are ready immediately.
        assert!(core::pin::pin!(&mut waiter).poll(&mut ctx).is_ready());
    }

    #[test]
    pub fn test_countable_correctness_hotpath() {
        // TEST: This verifies the correctness of the countable
        // event. To see the exact invariants this is upholding,
        // check the documentation for CountedEvent. This specifically
        // checks if the invariant upholds under a hotpath situation.

        let event = CountedEvent::new(Event::new());
        assert_eq!(event.count(), 0); // initial count should be zero

        // Create some async context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        // Create a wait event.
        let mut waiting = core::pin::pin!(event.wait());
        // Here the count should STILL be zero since it has not actually been awoken.
        assert_eq!(event.count(), 0);

        // Set the event, forcing an immediate return.
        event.set_one();

        // This should poll to pending, and the count goes to one.
        assert!(waiting.as_mut().poll(&mut ctx).is_ready());
        assert_eq!(event.count(), 0);
    }

    #[test]
    pub fn test_drop_cancel_countable() {
        // TEST: Countable events are a special case, this
        // checks their cancel safety.

        // Initialize a counted event.
        let event = CountedEvent::new(Event::new());
        assert_eq!(event.count(), 0);

        // Waits an event properly and then cancels it.
        setup_drop_cancel_test(&event);

        // This should return to zero.
        assert_eq!(event.count(), 0);
    }

    #[test]
    pub fn test_drop_cancel_try_wait() {
        // TEST: Checks if the event is actually sound
        // if a waiter gets dropped. This is SUPER important
        // for cancel safety as structures like the Mutex
        // rely on this correctness guarantee.

        let event = Event::new();
        assert!(!event.try_wait());

        setup_drop_cancel_test(&event);

        // Now we set it, which should allow us to grab it again
        // considering we just dropped the old one.
        event.set_one();

        // This should pass as we have no valid waiters.
        assert!(event.try_wait());
    }

    #[test]
    pub fn test_drop_cancel_real_wait() {
        // TEST: Checks if the event is actually sound
        // if a waiter gets dropped. This is SUPER important
        // for cancel safety as structures like the Mutex
        // rely on this correctness guarantee.

        let event = Event::new();
        assert!(!event.try_wait());

        // Setup the test.
        setup_drop_cancel_test(&event);

        // Now we set it, which should allow us to grab it again
        // considering we just dropped the old one.
        event.set_one();

        // This should pass as we have no valid waiters.
        assert!(can_poll_immediate(event.wait()));
    }

    // #[test]
    // pub fn test_event_fast_path() {
    //     // TEST: Checks to see if the event fastpath logic
    //     // is sound.
    // }

    

 
 

    #[test]
    pub fn test_proper_order() {
        use core::pin::pin;
        // TEST: Tests that futures are woken up
        // in the correct order.
        let event = Event::new();


        // Configure the context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);


 
        // Configure waiters and get them into ready position.
        let mut array: [_; 10] = core::array::from_fn(|_| {
            let mut ev = event.wait();
            assert!(pin!(&mut ev).poll(&mut ctx).is_pending());
            ev
        });

        for i in 0..10 {
            event.set_one();
            assert!(pin!(&mut array[i]).poll(&mut ctx).is_ready());
        }

    }

    #[test]
    pub fn test_double_set() {
        // TEST: Tests that a double set correctly wakes up two
        // waiters.
        let event = Event::new();

        // Configure the context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        // Configure waiters and get them into ready position.
        let mut waiter1 = core::pin::pin!(event.wait());
        let mut waiter2 = core::pin::pin!(event.wait());
        assert!(waiter1.as_mut().poll(&mut ctx).is_pending());
        assert!(waiter2.as_mut().poll(&mut ctx).is_pending());

        event.set_one();
        event.set_one();

        // They should both wake up immediately.
        assert!(waiter1.as_mut().poll(&mut ctx).is_ready());
        assert!(waiter2.as_mut().poll(&mut ctx).is_ready());






    }


    struct TestWaker {}

    impl Wake for TestWaker {
        fn wake(self: Arc<Self>) {}
    }

    #[test]
    pub fn no_steal() {
        // TEST: This tests that a try_wait() cannot steal
        // the slot of a waiting task. This sounds like a trivial
        // case but can be problematic if our event does not
        // actually check if the queue is empty before giving
        // up the signal by resetting it.

        let event = Event::new();
        assert!(!event.try_wait());

        // Let's do a basic waker setup, as we are interested
        // in interacting with the future at various set
        // polling points!
        let mut waiter = core::pin::pin!(event.wait());
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut context = Context::from_waker(&waker);

        // Let it park itself. Now the future is actually properly
        // loaded and we can start trying to cause dangerous things
        // to happen.
        assert!(waiter.as_mut().poll(&mut context).is_pending());

        // Let's set the event.
        event.set_one();

        // Let's try to steal the event, even though there
        // is already someone parked. This should fail!
        assert!(!event.try_wait());

        // The original waiter should return ready as designed.
        assert!(waiter.as_mut().poll(&mut context).is_ready());
    }


}

// // pub struct AutoEventAwait<'a> {

// // }

//! # Overview
//! This crate provides executor-agnostic async signalling primitives for building
//! more complex asynchronous datastructures. They are designed to be lightweight and
//! extremely easy to use. The documentation contains many examples on how to apply them
//! to a variety of situations. There also exists local varieties for runtimes that are `!Send`
//! and `!Sync`.
//! 
//! The inspiration from the crate is two-fold. First, the excellent synchronous event notification
//! mechansism [rsevents](https://docs.rs/rsevents/latest/rsevents/). Additionally, Tokio's [Notify](https://docs.rs/tokio/latest/tokio/sync/struct.Notify.html) primitive.
//! The main difference being that this crate is very lightweight and can be brought in without bringing
//! in the entire Tokio runtime.
//! 
//! It can be thought of as a `Semaphore` starting without any permits, although you can 
//! optionally create an event that is already set. Every event mechanism in this crate can
//! be described in terms of the [EventSetter] interface. A task waits for an event via the
//! [wait](EventSetter::wait) method, and then is woken up through one of two calls:
//! 1. [set_one](EventSetter::set_one) wakes up a single event.
//! 2. [set_all](EventSetter::set_all) wakes up all the queued events.
//! There is additionally a synchronous method, [try_wait](EventSetter::try_wait) which tries to
//! acquire the event along the fast path. This is composed of two simple atomic operations and thus is
//! very fast and does not need to be asynchronous.
//! 
//! # Quickstart
//! For quickstart, you are most likely uniquely interested in the most basic event, which is
//! also the most versatile, [Event].
//! 
//! A quick guide to usage for this event is shown below:
//! ```
//! # use pollster::FutureExt as _;
//! # async {
//! use asyncnal::{Event, EventSetter};
//! 
//! // We create a new event in the unset state.
//! let event = Event::new();
//! 
//! // We set the event.
//! event.set_one();
//! 
//! // We will be able to immediately acquire this event.
//! event.wait().await;
//! 
//! # }.block_on()
//! 
//! ```
//! 
//! # Fairness
//! The events are stored in a lock-free concurrent queue provided by [lfqueue](https://docs.rs/lfqueue/latest/lfqueue/),
//! and are thus woken in a FIFO order. This means that if events are queued up in order `A`, `B`, `C`, they will be woken
//! up in that order. For the case of [set_all](EventSetter::set_all), the events are guaranteed to be unloaded in FIFO order
//! but events may make progress at different times depending on the nature of the executor.
//! 
//! 
//! # Cancel Safety
//! The events provided by the crate all fully cancel safe. This is achieved by special
//! drop handling of the futures. Essentially, when a waiter is dropped, we check if we have modified
//! the internal state of the event, and in this case, we roll it back. There is one catch though:
//! 
//! Due to the concurrent nature of the queues, we cannot simply just iterate over it and kick one event
//! out of the queue. Therefore, the event has a way of communicating back to the queue where it signals that
//! said queue entry is actually invalid, causing it to be skipped over. This is not, however, immediately evited
//! from the queue.
//! 
//! # Examples
//! ## Basic
//! The example below shows the creation of an event, along with setting an event,
//! and then immediately acquiring that event.
//! ```
//! # use pollster::FutureExt as _;
//! # async {
//! use asyncnal::{Event, EventSetter};
//! 
//! let event = Event::new();
//! assert!(!event.has_waiters());
//! 
//! // We'll pre-set the event.
//! event.set_one();
//! 
//! // This will immediately return.
//! event.wait().await;
//! # }.block_on()
//! ```
//! ## Asynchronous Mutex
//! This signalling mechanism can be composed to create an asynchronous mutex to
//! avoid system calls.
//! ```
//! # use pollster::FutureExt as _;
//! # async {
//! use asyncnal::*;
//! use core::{cell::UnsafeCell, ops::{Deref, DerefMut}};
//! 
//! struct AsyncMutex<T> {
//!     cell: UnsafeCell<T>,
//!     event: Event
//! }
//! 
//! struct AsyncMutexGuard<'a, T>(&'a AsyncMutex<T>);
//! 
//! impl<T> AsyncMutex<T> {
//!     pub fn new(item: T) -> Self {
//!         Self {
//!             cell: UnsafeCell::new(item),
//!             // Since the mutex should be in an available state initially,
//!             // we should initialize the event in a pre-set state.
//!             event: Event::new_set()
//!         }
//!     }
//!     pub fn try_lock(&self) -> Option<AsyncMutexGuard<'_, T>> {
//!         // Using the try_wait method we can implement a try_lock that
//!         // is synchronous!
//!         if self.event.try_wait() {
//!             Some(AsyncMutexGuard(self))
//!         } else {
//!             // We could not acquire the lock on the fast
//!             // path.
//!             None
//!         }
//!     }
//!     pub async fn lock(&self) -> AsyncMutexGuard<'_, T> {
//!         // Wait for the event to become available.
//!         self.event.wait().await;
//!         AsyncMutexGuard(self)
//!     }
//! }
//! 
//! impl<'a, T> Drop for AsyncMutexGuard<'a, T> {
//!     fn drop(&mut self) {
//!         // Now we need to set the event to indicate it is now available!
//!         self.0.event.set_one();
//!     }
//! }
//! 
//! impl<'a, T> Deref for AsyncMutexGuard<'a, T> {
//!     type Target = T;
//!     fn deref(&self) -> &T {
//!         // SAFETY: The event guarantees we are the only
//!         // ones with this guard.
//!         unsafe { &*self.0.cell.get() }
//!     }
//! }
//! 
//! impl<'a, T> DerefMut for AsyncMutexGuard<'a, T> {
//!     fn deref_mut(&mut self) -> &mut T {
//!         // SAFETY: The event guarantees we are the only
//!         // ones with this guard.
//!         unsafe { &mut *self.0.cell.get() }
//!     }
//! 
//! }
//! 
//! let mutex = AsyncMutex::new(4);
//! assert_eq!(*mutex.lock().await, 4);
//! 
//! // Let's try to double acquire.
//! // We start by acquiring a handle.
//! let handle = mutex.lock().await;
//! // Now the try_lock method should fail.
//! assert!(mutex.try_lock().is_none());
//! drop(handle);
//! // Now that we have released the handle, we
//! // can acquire :)
//! assert!(mutex.try_lock().is_some());
//! 
//! 
//! *mutex.lock().await = 5;
//! 
//! assert_eq!(*mutex.lock().await, 5);
//! 
//! # }.block_on();
//! ```
//! 
//! ## Asynchronous Channels
//! We can use this abstraction to trivially build asynchronous channels for
//! local runtimes.
//! ```
//! # use pollster::FutureExt as _;
//! # async {
//! use std::{collections::VecDeque, cell::RefCell, rc::Rc};
//! use asyncnal::{LocalEvent, EventSetter};
//! 
//! #[derive(Clone)]
//! struct Channel<T> {
//!     queue: Rc<RefCell<VecDeque<T>>>,
//!     event: Rc<LocalEvent>
//! }
//! 
//!
//! 
//! impl<T> Channel<T> {
//!     pub fn new() -> Self {
//!         Self {
//!             queue: Rc::default(),
//!             event: Rc::default()
//!         }
//!     }
//!     pub fn send(&self, item: T) {
//!         self.queue.borrow_mut().push_back(item);
//!         self.event.set_one();
//!     }
//!     fn try_remove(&self) -> Option<T> {
//!         self.queue.borrow_mut().pop_front()
//!     }
//!     pub async fn recv(&self) -> T {
//!         let mut value = self.try_remove();
//!         while value.is_none() {
//!             // Wait for a notificaiton
//!             self.event.wait().await;
//!             value = self.try_remove();
//!         }
//!         value.unwrap()
//!     }
//!     
//! }
//! 
//! let channel = Channel::<usize>::new();
//! channel.send(4);
//! assert_eq!(channel.recv().await, 4);
//! # }.block_on();
//! ```
//! 
#![cfg_attr(not(feature = "std"), no_std)]

mod atomic;


mod yielder;
mod base;

#[cfg(feature = "std")]
mod asyncstd;
mod nostds;

pub use base::event::EventSetter;
pub use yielder::Yield;


#[cfg(feature = "std")]
pub use asyncstd::{Event, EventAwait, CountedAwaiter, CountedEvent, LocalEvent, LocalEventAwait};






#[cfg(all(test, feature = "std"))]
mod tests {
    use core::pin::pin;
    use std::{
        sync::Arc, task::{Context, Wake, Waker}
    };

    use crate::{asyncstd::{CountedEvent, Event}, base::event::EventSetter};

    // use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter}

 
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
    pub fn test_event_hotpath_steal() {
        // TEST: Checks that if the event is set and there is a
        // pending event then it cannot be stolen and bypass the queue.

        // Setup an event.
        let event = Event::new();

        // Setup the context.
        let waker = Waker::from(Arc::new(TestWaker {}));
        let mut ctx = Context::from_waker(&waker);

        let mut waiter = core::pin::pin!(event.wait());

        assert!(waiter.as_mut().poll(&mut ctx).is_pending());

        // Wakes up one event.
        event.set_one();

        // Now this call should fail because the set was immediately (semantically)
        // unset by the waiting event.
        assert!(pin!(event.wait()).poll(&mut ctx).is_pending());

        // Should immediately poll to true.
        assert!(waiter.as_mut().poll(&mut ctx).is_ready());
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

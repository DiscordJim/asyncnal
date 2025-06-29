use std::{
    pin::{pin, Pin},
    sync::atomic::{AtomicU8, Ordering},
    task::{Context, Poll, Waker},
};


mod atomic;

use crate::{async_lot::AsyncLot, yielder::Yield};

mod async_lot;
mod yielder;

const SIGNAL_FREE: u8 = 0b00;
const SIGNAL_WAIT: u8 = 0b01;
const SIGNAL_SET: u8 = 0b10;

use crate::atomic::*;

pub struct AutoEvent {
    inner: RawEvent,
}

struct RawEvent {
    inner: AtomicU8,
    waker: AsyncLot,
}

unsafe impl Send for RawEvent {}
unsafe impl Sync for RawEvent {}

const BACKOFF_MAX: usize = 3;

pub struct RawEventAwait<'a> {
    event: &'a RawEvent,
    is_parked: bool,
    backoff: usize,
    yielder: Option<Yield>
}

impl<'a> RawEventAwait<'a> {
    #[inline]
    fn swap_in(&self, current: u8, new: u8) -> bool {
        self.event
            .inner
            .compare_exchange_weak(current, new, Acquire, Relaxed)
            .is_ok()
    }
    #[inline]
    fn park_waker(&mut self, waker: &Waker) {
        self.is_parked = true;
        self.event.waker.park(waker);
    }
    #[inline]
    fn waker_list(&self) -> &AsyncLot {
        &self.event.waker
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
    fn proper_unpark(&self) {
        let mut new_pattern = SIGNAL_WAIT;
        if !self.event.waker.unpark_one() {
            // We failed to unpark the next one, queue should be empty.
            new_pattern = SIGNAL_FREE;

        }

        // if self.event.inner.compare_exchange_weak(SIGNAL_SET | SIGNAL_WAIT, new_pattern, Ordering::Release, Ordering::Relaxed).is_ok() {
        //     println!("properly reset...");
        // }
    }
}

impl Future for RawEventAwait<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {

        if let Some(mut yielder) = self.yielder.take() {
            match pin!(&mut yielder).poll(cx) {
                Poll::Pending => {
                    self.yielder = Some(yielder);
                },
                Poll::Ready(()) => {
                    // here we proceed.
                }
            }
        }

        // First we want to check if the signal is set, if so we can just bypass all this checking.
        match self
            .event
            .inner
            .compare_exchange_weak(SIGNAL_SET, SIGNAL_FREE, Acquire, Relaxed)
        {
            Ok(_) => {
                println!("OK PATH");
                // what if we do self.wakerlist unpark, and then if it returns true we enqueue it? negating the wait bit so that things
                // that are already parked get priority.
                let res = self.waker_list().unpark_one();
                // Optimistic acquisition failure.
                debug_assert!(!res, "We optimistically acquired the signal but there was another thread in the queue.");
                return Poll::Ready(()); // We can return.
            }
            Err(x) => {
                // If we enter this branch then the signal was not set.
                println!("Tried to swap, read pattern: {:08b}", x);
                match x {
                    SIGNAL_FREE => {
                        // bit pattern of SIGNAL_FREE
                        if self.swap_in(SIGNAL_FREE, SIGNAL_WAIT) {
                            // try to set the bit pattern to waiting.
                            self.park_waker(cx.waker());
                            return Poll::Pending; // return pending as we are waiting
                        } else {
                            // In this case we just retry.
                            return self.backoff(cx);
                        }

                    },
                    SIGNAL_WAIT => {
                        // bit pattern: SIGNAL WAIT
                        self.park_waker(cx.waker());
                        return Poll::Pending;
                    }
                    SIGNAL_SET => {
                        // failed spontaneously, try again.
                        println!("PATH B/SIGNAL_SET");
                        return self.backoff(cx);
                    }
                    0b11 => {
                        // Result: The signal is both set & waiting.
                        if self.is_parked {
                            // If we are already parked then we must be the top of the queue.

                            // ready the next wakeup.
                            self.proper_unpark();

                            return Poll::Ready(());
                        } else {
                            self.park_waker(cx.waker());
                            return Poll::Pending;
                        }
                    }
                    _ => unreachable!()
                }
                
            }
        }
    }
}

// impl Future for RawEventAwait<'_> {
//     type Output = ();
//     fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//         // println!("Getting polled...");
//         // This is a similar optimization that is applied in the AutoResetEvents from the `rsevents` crate, basically if the flag is
//         // set and the signal is not contended then we immediately can poll [Poll::Ready].
//         match self.event.inner.compare_exchange_weak(SIGNAL_SET, SIGNAL_FREE, Ordering::Acquire, Ordering::Relaxed) {
//             Ok(_) => {
//                 // println!("Fast path...");
//                 if self.is_parked {
//                     // println!("Fast path live. Double popping the queue.");
//                     self.event.waker.unpark_one();
//                 }
//                 self.event.waker.unpark_one();
//                 return Poll::Ready(())
//             }
//             Err(current) => {
//                 // The bit pattern is restricted to the first tow.
//                 debug_assert_eq!(current & !0x3, 0);

//                 match current {
//                     SIGNAL_FREE => {
//                         if self.swap_in(SIGNAL_FREE, SIGNAL_WAIT) {
//                             self.is_parked = true;
//                             self.event.waker.park_front(cx.waker());
//                             return Poll::Pending;
//                         }

//                     }
//                     SIGNAL_WAIT => {
//                         /* We let the control flow fall through here, we will be scheduled for wakeup automatically at the end of the match. */
//                     }
//                     0b11 => {
//                         if self.is_parked {
//                             // If we are already parked then the following conditions must be true:
//                             // 1. We have already scheduled ourselves.
//                             // 2. We were called in order.

//                             // This kicks us off the stack.
//                             self.event.waker.unpark_off_stack();

//                             if self.event.waker.len() == 0 {
//                                 // println!("FULLY FREEING THE SIGNAL!!");
//                                 // Since there is nothing let in the queue, the signal is fully free.
//                                 self.event.inner.store(SIGNAL_FREE, Ordering::Release);
//                             } else {
//                                 // Since the pattern is SIGNAL_SET | SIGNAL_WAIT, we want to unset
//                                 // the SET bit, but we do not want new people jumping to the top of the
//                                 // queue so we leave the wait bit set.
//                                 self.event.inner.store(SIGNAL_WAIT, Ordering::Release);
//                             }

//                             self.event.waker.unpark_one();
//                             return Poll::Ready(());
//                         }
//                     }
//                     SIGNAL_SET => {
//                         // if
//                         println!("SIGNAL SET??");
//                     }
//                     x => panic!("Invalid bit pattern: {x:08b}")
//                 }

//                 if !self.is_parked {
//                     self.is_parked = true;
//                     self.event.waker.park(cx.waker());
//                 }

//                 // This is the failure path, basically if the current op did not work
//                 // we fall back into the pending state.
//                 Poll::Pending
//             }
//         }
//     }
// }

impl AutoEvent {
    pub fn new() -> Self {
        Self {
            inner: RawEvent {
                inner: AtomicU8::default(),
                waker: AsyncLot::default(),
            },
        }
    }
    fn has_waiter(&self) -> bool {
        self.inner.inner.load(Acquire) & SIGNAL_WAIT != 0
    }
    pub fn wait(&self) -> RawEventAwait<'_> {
        RawEventAwait {
            event: &self.inner,
            is_parked: false,
            backoff: 0,
            yielder: None
        }
    }
    pub fn set_one(&self) {
        let state = self.inner.inner.load(Ordering::Acquire);
        self.inner.inner.store(state | SIGNAL_SET, Ordering::SeqCst);
        self.inner.waker.unpark_one();
    
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{atomic::Ordering, Arc},
        thread::{self, current, sleep},
        time::Duration,
    };

    // use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
    use rsevents::{AutoResetEvent, Awaitable};
    use tokio::sync::Barrier;

    use crate::{AutoEvent, SIGNAL_FREE, SIGNAL_WAIT};

    // struct Test {
    //     link: LinkedListAtomicLink,
    //     value: usize,
    // }

    // intrusive_adapter!(MyAdapter = Box<Test>: Test { link: LinkedListAtomicLink });

    // #[test]
    // pub fn test_linky() {
    //     // let tra = LinkedList::new(MyAdapter::new());
    //     // tra.pop_back()

    //     panic!("yee");
    // }

    // #[test]
    // pub fn waiter_rsevents() {
    //     use std::sync::Barrier;
    //     use std::thread::sleep;

    //     let signal = Arc::new(AutoResetEvent::new(rsevents::EventState::Unset));
    //     let barrier = Arc::new(Barrier::new(11));

    //     let waiters = 10;
    //     let mut bucket = vec![];

    //     for i in 0..waiters {
    //         bucket.push(thread::spawn({
    //             let signal = signal.clone();
    //             let barrier = barrier.clone();
    //             move || {
    //                 println!("Waiting...");
    //                 barrier.wait();
    //                 signal.wait();

    //                 println!("Wake: {:?}", current().id());
    //             }
    //         }));
    //         // sleep(Duration::from_secs(1)).await;
    //     }

    //     // sleep(Duration::from_secs(1)).await;
    //     // barrier.wait();

    //     // sleep(Duration::from_millis(10));

    //      barrier.wait();

    //     for i in 0..waiters {
    //         // sleep(Duration::from_millis(10)).await;
    //         signal.set();
    //         println!("Signal woken #{}", i);
    //         // signal.wake();
    //     }

    //     for v in bucket {
    //         v.join().unwrap();
    //     }

    //     // signal.wait().await;
    // }

    #[tokio::test]
    pub async fn waiter_async() {
        let signal = Arc::new(AutoEvent::new());
        let barrier = Arc::new(Barrier::new(11));

        let waiters = 10;
        let mut bucket = vec![];

        for i in 0..waiters {
            bucket.push(tokio::spawn({
                let signal = signal.clone();
                let barrier = barrier.clone();
                async move {
                    println!("Task {i} waiting...");
                    barrier.wait().await;

                    signal.wait().await;
                    println!("Wake;... {i}");
                    // signal.wake();
                }
            }));
            // sleep(Duration::from_secs(1)).await;
        }

        // sleep(Duration::from_secs(1)).await;

        barrier.wait().await;
        sleep(Duration::from_millis(10));
        println!("Barrier fully done...");

        for i in 0..10 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            signal.set_one();
        }

        for v in bucket {
            v.await.unwrap();
        }

        // signal.wait().await;
    }


 
    


    #[tokio::test]
    pub async fn proper_wait_to_free_reset() {
        // Checks if the waiting bit is unset correctly.
        let event = Arc::new(AutoEvent::new());

        // should be the free bit.
        assert_eq!(event.inner.inner.load(std::sync::atomic::Ordering::Relaxed), SIGNAL_FREE);


        // spawn a waiter.

        let barrier = Arc::new(Barrier::new(2));

        tokio::spawn({
            let event = event.clone();
            let barrier = barrier.clone();
            async move {
                event.wait().await;
                barrier.wait().await;
            }
        });

        loop {
            // check for it to be active

            if event.inner.inner.load(Ordering::Acquire) & SIGNAL_WAIT != 0 {
                println!("the wait bit has been set...");
                break; // break out of loop!
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        // now we set the event.
        event.set_one();

        // let the other thread see it.
        barrier.wait().await;

        // should be fully freed!
        assert_eq!(event.inner.inner.load(Ordering::Acquire), SIGNAL_FREE);


    }
    

    #[tokio::test]
    pub async fn test_optimistic_acquire() {
        // checks if we can optimistically acquire the event & that it is properly reset.
        let waker = AutoEvent::new();
        
        // release a notification.
        waker.set_one();

        // acquire the vent.
        waker.wait().await;

        // should be properly reset.
        assert_eq!(waker.inner.inner.load(Ordering::SeqCst), SIGNAL_FREE);
    }

}

// // pub struct AutoEventAwait<'a> {

// // }

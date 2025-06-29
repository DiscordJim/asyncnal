use std::{
    pin::Pin,
    sync::atomic::{AtomicU8, Ordering},
    task::{Context, Poll, Waker},
};


mod atomic;

use crate::async_lot::AsyncLot;

mod async_lot;
mod atom_lock;
mod yielder;

const SIGNAL_FREE: u8 = 0x00;
const SIGNAL_WAIT: u8 = 0x01;
const SIGNAL_SET: u8 = 0x02;

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

pub struct RawEventAwait<'a> {
    event: &'a RawEvent,
    is_parked: bool,
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
}

impl Future for RawEventAwait<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // First we want to check if the signal is set, if so we can just bypass all this checking.
        match self
            .event
            .inner
            .compare_exchange_weak(SIGNAL_SET, SIGNAL_FREE, Acquire, Relaxed)
        {
            Ok(_) => {
                // We've gone through the fast path.
                if self.is_parked {
                    // Direct acquisition, we must be at the top of the queue.
                    self.event.waker.unpark_one();
                }
                // Wake up the next person in line (if there is one)
                self.event.waker.unpark_one();
                return Poll::Ready(()); // We can return.
            }
            Err(x) => {
                // If we enter this branch then the signal was not set.
                println!("Tried to swap, read pattern: {:08b}", x);
                match x {
                    0b00 => {
                        // bit pattern of SIGNAL_FREE
                        if self.swap_in(SIGNAL_FREE, SIGNAL_WAIT) {
                            // try to set the bit pattern to waiting.
                            self.park_waker(cx.waker());
                            return Poll::Pending; // return pending as we are waiting
                        } else {
                            // In this case we just retry.
                            return self.poll(cx);
                        }

                    },
                    0b01 => {
                        // bit pattern: SIGNAL WAIT
                        self.park_waker(cx.waker());
                        return Poll::Pending;
                    }
                    0b10 => {
                        // failed spontaneously, try again.
                        return self.poll(cx);
                    }
                    0b11 => {
                        // Result: The signal is both set & waiting.
                        if self.is_parked {
                            // If we are already parked then we must be the top of the queue.

                            // ready the next wakeup.
                            self.event.waker.unpark_one();

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
    pub fn wait(&self) -> RawEventAwait<'_> {
        RawEventAwait {
            event: &self.inner,
            is_parked: false,
        }
    }
    pub fn wake(&self) {
        let state = self.inner.inner.load(Ordering::Acquire);
        self.inner.inner.store(state | SIGNAL_SET, Ordering::SeqCst);
        self.inner.waker.unpark_one();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        thread::{self, current, sleep},
        time::Duration,
    };

    // use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
    use rsevents::{AutoResetEvent, Awaitable};
    use tokio::sync::Barrier;

    use crate::AutoEvent;

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
            signal.wake();
        }

        for v in bucket {
            v.await.unwrap();
        }

        // signal.wait().await;
    }
}

// // pub struct AutoEventAwait<'a> {

// // }

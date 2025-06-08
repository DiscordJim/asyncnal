use std::{
    cell::UnsafeCell,
    collections::LinkedList,
    pin::Pin,
    ptr,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicPtr, AtomicU8, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::sleep,
    time::{Duration, Instant},
};

use tokio::task::id;

use crate::async_lot::AsyncLot;

mod async_lot;
mod atom_lock;
mod linked_list;

const SIGNAL_FLAG_EMPTY: usize = 0x00;
const SIGNAL_FLAG_WAIT: usize = 0x01;
const SIGNAL_FLAG_SET: usize = 0x02;

pub struct AutoEvent {
    inner: RawEvent,
}

struct RawEvent {
    inner: AtomicUsize,
    waker: AsyncLot,
}

unsafe impl Send for RawEvent {}
unsafe impl Sync for RawEvent {}

pub struct RawEventAwait<'a> {
    event: &'a RawEvent,
    is_live: bool,
}

impl<'a> RawEventAwait<'a> {
    fn swap_in(&self, current: usize, new: usize) -> bool {
        self.event.inner.compare_exchange_weak(current, new, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
}

impl Future for RawEventAwait<'_> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let current = self.event.inner.load(Ordering::Relaxed);

        let current_count = current >> 2;

        if self.is_live {
            // We are the live entity, i.e., the entity at the front of the queue about to be
            // activated.
            if self.swap_in(current | SIGNAL_FLAG_SET, subtract_current(current) & !0x3) {
                // Wake up the next.
                self.event.waker.unpark_one();
                return Poll::Ready(());
            } else {
                // We failed, in this case we need to retry.
                self.event.waker.park_front(cx.waker());
                return Poll::Pending;
            }
        } else if current_count != 0 && !self.is_live && current & SIGNAL_FLAG_WAIT == 0 {
            // We are not live and the count is not zero.

            // Clear the last two bits, this allows us to acquire the lock and immediately signal.
            let alledged = ((current >> 2) - 1) << 2;

            if self
                .event
                .inner
                .compare_exchange_weak(current, alledged, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                println!("Fast path (A)");
                self.event.waker.unpark_one();
                return Poll::Ready(());
            } else {
                println!("Deferred path (A)");
            }
        } else if
            // No waiters quite yet.
            current_count == 0
            // There is no active waiter.
            && current & SIGNAL_FLAG_WAIT == 0
            // And we aquire the live status.
            && self.swap_in(current, current | SIGNAL_FLAG_WAIT)
            {
                println!("Path (B)");
                self.is_live = true;
                self.event.waker.park_front(cx.waker());
                return Poll::Pending;

        }

  

        // This is the failing path.
        println!("Going to failure: {:?}", tokio::task::id());
        println!("Current: {:064b}", current);
        self.event.waker.park(cx.waker());
        Poll::Pending
    }
}

#[inline(always)]
fn subtract_current(current: usize) -> usize {
    let original = current & 0x3;
    (((current >> 2) - 1) << 2) | original
}

impl AutoEvent {
    pub fn new() -> Self {
        Self {
            inner: RawEvent {
                inner: AtomicUsize::new(0),
                waker: AsyncLot::default(),
            },
        }
    }
    pub fn wait(&self) -> RawEventAwait<'_> {
        RawEventAwait {
            event: &self.inner,
            is_live: false
        }
    }
    pub fn wake(&self) {
        let state = self.inner.inner.load(Ordering::Acquire);
        // let bits = state & 0x3;
        let count = (((state >> 2) + 1) << 2) | (state & 0x03);
        let new = count | SIGNAL_FLAG_SET;
        println!("New: {:064b}", count);
        self.inner
            .inner
            .store(count | SIGNAL_FLAG_SET, Ordering::SeqCst);

        if count & SIGNAL_FLAG_WAIT != 0 && count & SIGNAL_FLAG_SET == 0 {
            println!("We were signaled, we are going to wake.");
            // Someone is waiting, so we wake them up.
            self.inner.waker.unpark_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread::{self, current, sleep}, time::Duration};

    use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
    use rsevents::{AutoResetEvent, Awaitable};
    use tokio::{sync::Barrier};

    use crate::AutoEvent;

    struct Test {
        link: LinkedListAtomicLink,
        value: usize,
    }

    intrusive_adapter!(MyAdapter = Box<Test>: Test { link: LinkedListAtomicLink });

    #[test]
    pub fn test_linky() {
        // let tra = LinkedList::new(MyAdapter::new());
        // tra.pop_back()

        panic!("yee");
    }

    #[test]
    pub fn waiter_rsevents() {
        use std::sync::Barrier;
        use std::thread::sleep;
    
        let signal = Arc::new(AutoResetEvent::new(rsevents::EventState::Unset));
        let barrier = Arc::new(Barrier::new(11));

        let waiters = 10;
        let mut bucket = vec![];

        for i in 0..waiters {
            bucket.push(thread::spawn({
                let signal = signal.clone();
                let barrier = barrier.clone();
                move || {
                    barrier.wait();
                    signal.wait();

                    println!("Wake: {:?}", current().id());

                }
            }));
            // sleep(Duration::from_secs(1)).await;
        }

        // sleep(Duration::from_secs(1)).await;
        // barrier.wait();

        // sleep(Duration::from_millis(10));

        for i in 0..5 {
            // sleep(Duration::from_millis(10)).await;
            signal.set();
            println!("Signal woken #{}", i);
            // signal.wake();
        }
        barrier.wait();

        for v in bucket {
            v.join().unwrap();
        }

        // signal.wait().await;
    }


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
                    barrier.wait().await;

                    signal.wait().await;
                    println!("Wake;... {i}");
                    // signal.wake();
                }
            }));
            // sleep(Duration::from_secs(1)).await;
        }

        // sleep(Duration::from_secs(1)).await;
        

        sleep(Duration::from_millis(10));

        for i in 0..5 {
            // sleep(Duration::from_millis(10)).await;
            signal.wake();
        }

        barrier.wait().await;

        for v in bucket {
            v.await.unwrap();
        }

        // signal.wait().await;
    }
}

// pub struct AutoEventAwait<'a> {

// }

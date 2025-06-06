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

mod linked_list;

const SIGNAL_FLAG_EMPTY: usize = 0x00;
const SIGNAL_FLAG_WAIT: usize = 0x01;
const SIGNAL_FLAG_SET: usize = 0x02;

pub struct AutoEvent {
    inner: RawEvent,
}

struct RawEvent {
    inner: AtomicUsize,
    waker: Mutex<LinkedList<Waker>>,
}

unsafe impl Send for RawEvent {}
unsafe impl Sync for RawEvent {}

pub struct RawEventAwait<'a> {
    event: &'a RawEvent,
    is_live: bool, // waker: UnsafeCell<Option<Waker>>,
    is_parked: bool,
}

impl<'a> RawEventAwait<'a> {
    fn swap_in(&self, current: u8, new: u8) {}
}

impl Future for RawEventAwait<'_> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let current = self.event.inner.load(Ordering::Relaxed);

        let current_count = current >> 2;


        if current_count != 0 && !self.is_live && current & SIGNAL_FLAG_WAIT == 0 {
            // We are not live and the count is not zero.
        
            // Clear the last two bits, this allows us to acquire the lock and immediately signal.
            let alledged = (((current >> 2) - 1) << 2);

            if self.event.inner.compare_exchange_weak(current, alledged, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                if self.is_parked {
                    // Remove us from the queue
                    self.event.waker.lock().unwrap().pop_front();
                }
                if let Some(next) = self.event.waker.lock().unwrap().front() {
                    next.wake_by_ref();
                }
                // self.event.waker.lock().unwrap().front().unwrap()
                return Poll::Ready(());
            }
        }

        if
        // Check if the wait bit is set.
        current & SIGNAL_FLAG_WAIT == 0
            // Try to acquire the atomic.
            && self.event.inner.compare_exchange_weak(current, current | SIGNAL_FLAG_WAIT, Ordering::Acquire, Ordering::Relaxed).is_ok()
        {
            

             // We were empty and now we are filled.
                self.is_live = true;
                self.event
                    .waker
                    .lock()
                    .unwrap()
                    .push_front(cx.waker().clone());

            
            println!("Entering wait... #{} [{}]", id(), current_count);
            // println!("modded: {:064b}", self.event.inner.load(Ordering::Acquire));
            return Poll::Pending;
        }

        // if self.is_live {
        //     println!("Hello:\t\t{:064b}", current);
        //     println!("Proposed:\t{:064b}", current | SIGNAL_FLAG_SET);
        // }

        if self.is_live
            && (self
                .event
                .inner
                .compare_exchange_weak(
                    current | SIGNAL_FLAG_SET,
                    current & !0x03,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok())
        {
            let handle = self.event.waker.lock().unwrap();
            if !handle.is_empty() {
                println!("List is not empty.");
                handle.front().unwrap().wake_by_ref();
            }
            println!("Exiting signal: #{}", id());
            return Poll::Ready(());
        }

        // if (!self.event.inner.compare_exchange_weak(current, current | 0x01, Ordering::Acquire, Ordering::Relaxed) & 0x01) != 0 {
        //     println!("EMPTY");
        // }

        // This is the failing path.
        println!("Going to failure: {:?}", tokio::task::id());
        println!("Current: {:064b}", current);
        self.event
            .waker
            .lock()
            .unwrap()
            .push_back(cx.waker().clone());
        self.is_parked = true;
        Poll::Pending
    }
}

impl AutoEvent {
    pub fn new() -> Self {
        Self {
            inner: RawEvent {
                inner: AtomicUsize::new(0),
                waker: Mutex::default(),
            },
        }
    }
    pub fn wait(&self) -> RawEventAwait<'_> {
        RawEventAwait {
            event: &self.inner,
            is_live: false,
            is_parked: false
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
            if let Some(inner) = self.inner.waker.lock().unwrap().pop_front() {
                inner.wake_by_ref();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use intrusive_collections::{intrusive_adapter, LinkedList, LinkedListAtomicLink};
    use tokio::{sync::Barrier, time::sleep};

    use crate::AutoEvent;

    struct Test {
        link: LinkedListAtomicLink,
        value: usize
    }

    intrusive_adapter!(MyAdapter = Box<Test>: Test { link: LinkedListAtomicLink });

    #[test]
    pub fn test_linky() {

        // let tra = LinkedList::new(MyAdapter::new());
        // tra.pop_back()

        panic!("yee");
    }

    #[tokio::test]
    pub async fn waiter() {
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
        barrier.wait().await;

        sleep(Duration::from_millis(10)).await;

        for i in 0..10 {
            // sleep(Duration::from_millis(10)).await;
            signal.wake();
        }

        for v in bucket {
            v.await.unwrap();
        }

        // signal.wait().await;
    }
}

// pub struct AutoEventAwait<'a> {

// }

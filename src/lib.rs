use std::{
    cell::UnsafeCell, collections::LinkedList, ptr, sync::{
        atomic::{AtomicPtr, AtomicU8, Ordering}, Mutex, MutexGuard
    }, task::{Poll, Wake, Waker}, thread::sleep, time::{Duration, Instant}
};

const SIGNAL_FLAG_EMPTY: u8 = 0x00;
const SIGNAL_FLAG_WAIT: u8 = 0x01;
const SIGNAL_FLAG_SET: u8 = 0x02;

pub struct AutoEvent {
    inner: RawEvent,
}

struct RawEvent {
    inner: AtomicU8,
    waker: Mutex<LinkedList<Waker>>,
}

unsafe impl Send for RawEvent {}
unsafe impl Sync for RawEvent {}




pub struct RawEventAwait<'a> {
    event: &'a RawEvent,
    is_live: bool, // waker: UnsafeCell<Option<Waker>>
}

impl<'a> RawEventAwait<'a> {
    fn swap_in(&self, current: u8, new: u8) {}
   
}

impl Future for RawEventAwait<'_> {
    type Output = ();
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        

        if self.is_live
            && self
                .event
                .inner
                .compare_exchange_weak(
                    SIGNAL_FLAG_SET,
                    SIGNAL_FLAG_EMPTY,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
        {
            println!("hello");
            // We are executing the critical section.
            let mut handle = self.event.waker.lock().unwrap();
            let _ = handle.pop_front();
            if !handle.is_empty() {
                println!("Hello");
                handle.pop_front().unwrap().wake_by_ref();
            }
            return Poll::Ready(());
        }

        // if self
        //     .event
        //     .inner
        //     .compare_exchange_weak(
        //         SIGNAL_FLAG_SET,
        //         SIGNAL_FLAG_EMPTY,
        //         Ordering::Acquire,
        //         Ordering::Relaxed,
        //     )
        //     .is_ok()
        // {
        //     println!("Success A (#{})", tokio::task::id());
        //     return Poll::Ready(());
        // }

        if self
            .event
            .inner
            .compare_exchange_weak(
                SIGNAL_FLAG_EMPTY,
                SIGNAL_FLAG_WAIT,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            println!("SUCCESS! {}", tokio::task::id());
            self.is_live = true;
            self.event
                .waker
                .lock()
                .unwrap()
                .push_front(cx.waker().clone());
            return Poll::Pending;
        } else {
            self.event
                .waker
                .lock()
                .unwrap()
                .push_back(cx.waker().clone());
            return Poll::Pending;
        }
    }
}

impl AutoEvent {
    pub fn new() -> Self {
        Self {
            inner: RawEvent {
                inner: AtomicU8::new(0),
                waker: Mutex::default(),
            },
        }
    }
    pub fn wait(&self) -> RawEventAwait<'_> {
        RawEventAwait {
            event: &self.inner,
            is_live: false,
        }
    }
    pub fn wake(&self) {
        self.inner.inner.store(SIGNAL_FLAG_SET, Ordering::SeqCst);
        if let Some(inner) = self.inner.waker.lock().unwrap().front() {
            inner.wake_by_ref();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use tokio::{sync::Barrier, time::sleep};

    use crate::AutoEvent;

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

        for i in 0..10 {
            sleep(Duration::from_millis(10)).await;
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

use std::{ops::Deref, sync::atomic::{AtomicU8, Ordering}, task::Poll};

const LL_FREE: u8 = 0x00;
const LL_LOCK: u8 = 0x01;

pub(crate) struct AtomicLock<T> {
    state: AtomicU8,
    inner: T
}

impl<T> AtomicLock<T> {
    pub fn new(inner: T) -> Self {
        Self {
            state: AtomicU8::default(),
            inner
        }
    }
}

/// An asynchronous runtime agnostic yielder, inspired off the `YieldNow` structure
/// featured in the `smol_rs` library: https://docs.rs/futures-lite/2.3.0/src/futures_lite/future.rs.html#219
#[derive(Default)]
pub(crate) struct AsyncYield(bool);


impl Future for AsyncYield {
    type Output = ();
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        if !self.0 {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

pub(crate) struct AtomicLocked<'a, T>(&'a AtomicLock<T>, &'a T);

impl<T> AtomicLock<T> {
    pub fn lock(&self) -> AtomicLocked<'_, T> {
        let mut cycles: usize = 0;
        while self.state.compare_exchange_weak(LL_FREE, LL_LOCK, Ordering::Acquire, Ordering::Relaxed).is_err() {
            cycles += 1;
        }
        println!("Waited {} iterations to get the lock.", cycles);
        AtomicLocked(self, &self.inner)
    }
}

impl<'a, T> Deref for AtomicLocked<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.1
    }
}

impl<'a, T> Drop for AtomicLocked<'a, T> {
    fn drop(&mut self) {
        self.0.state.store(LL_FREE, Ordering::Release);
    }
}


use std::task::Waker;

use lfqueue::UnboundedQueue;

#[derive(Default)]
pub(crate) struct AsyncLot {
    inner: UnboundedQueue<Waker>
}

impl AsyncLot {
    pub fn park(&self, waker: &Waker) {
        self.inner.enqueue(waker.clone());
    }
    pub fn unpark_one(&self) -> bool {
        if let Some(value) = self.try_unpark() {
            value.wake_by_ref();
            true
        } else {
            false
        }
    }
    fn try_unpark(&self) -> Option<Waker> {
        self.inner.dequeue()
    }
}
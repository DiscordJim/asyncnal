use std::{collections::VecDeque, sync::Mutex, task::Waker};

#[derive(Default)]
pub(crate) struct AsyncLot {
    inner: Mutex<VecDeque<Waker>>
}

impl AsyncLot {
    pub fn park_front(&self, waker: &Waker) {
        self.inner.lock().unwrap().push_front(waker.clone());
    }
    pub fn park(&self, waker: &Waker) {
        self.inner.lock().unwrap().push_back(waker.clone());
    }
    pub fn unpark_one(&self) {
        if let Some(inner) = self.inner.lock().unwrap().pop_front() {
            inner.wake_by_ref();
        }
        // self.inner.lock().unwrap().pop_front()
    }
}
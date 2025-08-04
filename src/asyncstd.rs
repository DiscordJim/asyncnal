use std::{cell::{Cell, RefCell}, collections::VecDeque, rc::Rc, sync::{atomic::{AtomicBool, AtomicUsize}, Arc}, task::{Context, Waker}};

use crossbeam_utils::CachePadded;
use lfqueue::UnboundedQueue;

use crate::{base::{event::EventState, lot::{AsyncLotSignature, AsyncLotTemplate}, signal::{TechnicalCounter, TechnicalWaker, ValidityMarker, WakeQueue}}, impl_async_event, impl_async_lot};

use crate::atomic::*;

impl TechnicalWaker for Waker {
    fn wake_by_ref(&self) {
        Waker::wake_by_ref(&self)
    }
}

impl ValidityMarker for Arc<AtomicBool> {
    fn create() -> Self {
        Arc::new(AtomicBool::new(true))
    }
    fn get(&self) -> bool {
        self.load(Acquire)
    }
    fn set(&self, value: bool) {
        self.store(value, Release);
    }
}

impl TechnicalCounter for CachePadded<AtomicUsize> {
    fn decrement(&self) {
        self.fetch_sub(1, Release);
    }
    fn increment(&self) {
        self.fetch_add(1, Release);
    }
    fn get(&self) -> usize {
        self.load(Acquire)
    }
}

impl<T> WakeQueue<T> for UnboundedQueue<T>
where 
    T: Send + Sync
{
    fn dequeue(&self) -> Option<T> {
        UnboundedQueue::dequeue(&self)
    }
    fn enqueue(&self, item: T) {
        UnboundedQueue::enqueue(&self, item);
    }
}





#[derive(Default)]
struct LocalCounter(Cell<usize>);

impl TechnicalCounter for LocalCounter {
    fn decrement(&self) {
        let current = self.0.get();
        self.0.set(current - 1);
    }
    fn get(&self) -> usize {
        self.0.get()
    }
    fn increment(&self) {
        let current = self.0.get();
        self.0.set(current + 1);
    }
}

struct LocalQueue<T>(RefCell<VecDeque<T>>);


impl<T> Default for LocalQueue<T> {
    fn default() -> Self {
        Self(RefCell::default())
    }
}


impl<T> WakeQueue<T> for LocalQueue<T> {
    fn dequeue(&self) -> Option<T> {
        self.0.borrow_mut().pop_front()
    }
    fn enqueue(&self, item: T) {
        self.0.borrow_mut().push_back(item);
    }
}

impl ValidityMarker for Rc<Cell<bool>> {
    fn create() -> Self {
        Rc::new(Cell::new(true))
    }
    fn get(&self) -> bool {
        Cell::get(&self)
    }
    fn set(&self, value: bool) {
        Cell::set(&self, value);
    }
}




impl_async_lot!(
    name = LocalAllocatedAsyncLot,
    waker = Waker,
    validity = Rc<Cell<bool>>,
    counter = LocalCounter,
    queue = LocalQueue
);

impl_async_lot!(
    name = AllocatedAsyncLot,
    waker = Waker,
    validity = Arc<AtomicBool>,
    counter = CachePadded<AtomicUsize>,
    queue = UnboundedQueue
);


impl EventState for AtomicU8 {
    fn new(state: u8) -> Self {
        AtomicU8::new(state)
    }
    fn cmpxchng_weak(
            &self,
            current: u8,
            new: u8,
            success: Ordering,
            failure: Ordering,
        ) -> Result<u8, u8> {
        AtomicU8::compare_exchange_weak(&self, current, new, success, failure)
    }
    fn store(
            &self,
            value: u8,
            ordering: Ordering
        ) {
        AtomicU8::store(&self, value, ordering);
    }
}

impl_async_event!(
    name = Event,
    waitername = EventAwait,
    lot = AllocatedAsyncLot,
    state = AtomicU8
);

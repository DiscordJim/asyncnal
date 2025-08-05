use core::{pin::Pin, task::Poll};
use std::{cell::{Cell, RefCell}, collections::VecDeque, rc::Rc, sync::{Arc}, task::{Context, Waker}};

use crossbeam_utils::CachePadded;
use lfqueue::UnboundedQueue;

use crate::base::{event::{impl_async_event, EventSetter, EventState}, lot::{impl_async_lot, AsyncLotSignature, AsyncLotTemplate}, signal::{TechnicalCounter, ValidityMarker, WakeQueue}};

use crate::atomic::*;


/// A [CountedEvent] wraps a type that implements the [EventSetter] trait. It can tell
/// you how many waiters there are on that very event. The count will only go up once the
/// waiter actually starts waiting on the event. If there is an immediate return on the first
/// time the wait is polled, the count will not go up.
///
/// The count will go up if the first call to poll wait returns [Poll::Pending], indicating that
/// the waiter is actually waiting on notification. Upon notification, the count will
/// be decremented.
///
/// # Cancel Safety
/// On [Drop] the count is decremented if the [CountedAwaiter] had already
/// incremented the count and is not yet in the complete state.
pub struct CountedEvent<E> {
    inner: E,
    counter: AtomicUsize,
}

#[pin_project::pin_project(PinnedDrop)]
#[allow(private_bounds)]
pub struct CountedAwaiter<'a, E, F>
where
    E: EventSetter<'a>,
    F: Future<Output = ()>,
{
    master: &'a CountedEvent<E>,
    #[pin]
    future: F,
    state: CountedAwaiterState,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CountedAwaiterState {
    /// The waiter has just been created and has
    /// not yet incremented the count.
    Init,
    /// The waiter is in the waiting state, therefore
    /// there has been an increase in the count
    Waiting,
    /// The waiter is actually complete, meaning that the
    /// count has been decremented already.
    Complete,
}

#[pin_project::pinned_drop]
impl<'a, E, F> PinnedDrop for CountedAwaiter<'a, E, F>
where
    E: EventSetter<'a>,
    F: Future<Output = ()>,
{
    fn drop(self: Pin<&mut Self>) {
        match self.state {
            CountedAwaiterState::Init => {
                // If we are initializing then we have not actually
                // modified any of the state, so we are in the clear.
            }
            CountedAwaiterState::Waiting => {
                // If we are in the waiting state then we have incremented
                // the counter, so we need to undo this operation.
                self.master.counter.fetch_sub(1, Ordering::Release);
            }
            CountedAwaiterState::Complete => {
                // If we are complete than there is nothing to
                // do so we do not execute any special drop code.
            }
        }
    }
}

impl<'a, E, F> Future for CountedAwaiter<'a, E, F>
where
    E: EventSetter<'a>,
    F: Future<Output = ()> + Unpin,
{
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // NOTE ON STATE: State is very important mostly for the reason of
        // properly undoing things when stuff gets cancelled (dropped).
        match this.state {
            CountedAwaiterState::Init => {
                // We are currently initializing. In this case we actually need
                // to increment the waiting variables if there is not an immediate return.
                match this.future.poll(cx) {
                    Poll::Pending => {
                        // In this case we have polled our future, and clearly have not immediately returned,
                        // and thus we must increment the count and transition to the waiting
                        // state.
                        this.master.counter.fetch_add(1, Ordering::Release);
                        *this.state = CountedAwaiterState::Waiting;
                        Poll::Pending
                    }
                    Poll::Ready(_) => {
                        // This is a special case of a fast return. This immediately short circuits
                        // the future, and thus there is no need to increment the count.
                        Poll::Ready(())
                    }
                }
            }
            CountedAwaiterState::Waiting => {
                // We need to check if we are ready to complete.
                match this.future.poll(cx) {
                    // If we are still pending, then there
                    // is nothing to be said here and we just return.
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(_) => {
                        // If we are ready then we decrement the count.
                        this.master.counter.fetch_sub(1, Ordering::Release);
                        *this.state = CountedAwaiterState::Complete;
                        Poll::Ready(())
                    }
                }
            }
            CountedAwaiterState::Complete => {
                // The future should never really be polled here but
                // if we do then we are ready.
                Poll::Ready(())
            }
        }
    }
}

#[allow(private_bounds)]
impl<E> CountedEvent<E>
where
    for<'a> E: EventSetter<'a>,
{
    #[allow(private_interfaces)]
    pub fn new(source: E) -> Self {
        Self {
            inner: source,
            counter: AtomicUsize::new(0),
        }
    }

    #[allow(private_interfaces)]
    pub fn wait(&self) -> <Self as EventSetter>::Waiter {
        <Self as EventSetter>::wait(&self)
    }

    #[allow(private_interfaces)]
    pub fn set_one(&self) {
        <Self as EventSetter>::set_one(&self);
    }

    #[allow(private_interfaces)]
    pub fn set_all(&self) {
        <Self as EventSetter>::set_all(&self, || {});
    }

    pub fn count(&self) -> usize {
        self.counter.load(Acquire)
    }
}

impl<'a, E> EventSetter<'a> for CountedEvent<E>
where
    E: EventSetter<'a> + 'a,
{
    type Waiter = CountedAwaiter<'a, E, E::Waiter>;


    fn new() -> Self {
        Self {
            counter: AtomicUsize::default(),
            inner: E::new()
        }
    }

    fn new_set() -> Self {
        Self {
            counter: AtomicUsize::default(),
            inner: E::new_set()
        }
    }

    fn wait(&'a self) -> CountedAwaiter<'a, E, E::Waiter> {
        // self.counter.fetch_add(1, Release);
        CountedAwaiter {
            master: self,
            future: self.inner.wait(),
            state: CountedAwaiterState::Init,
        }
    }

    fn try_wait(&self) -> bool {
        if self.inner.try_wait() {
            // self.counter.fetch_sub(1, Ordering::Release);
            true
        } else {
            false
        }
    }

    fn set_all<F: FnMut()>(&self, mut functor: F) {
        self.inner.set_all(|| {
            functor();
            self.counter.fetch_sub(1, Ordering::Release);
        });
    }
    fn set_one(&self) -> bool {
        if self.inner.set_one() {
            // self.counter.fetch_sub(1, Ordering::Release);
            true
        } else {
            false
        }
    }
    fn has_waiters(&self) -> bool {
        self.inner.has_waiters()
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
    /// An event for normal usage. It is [Send] and [Sync] and
    /// can be used within multithreaded environments.
    name = Event,
    waitername = EventAwait,
    lot = AllocatedAsyncLot,
    state = AtomicU8
);

unsafe impl Send for Event {}
unsafe impl Sync for Event {}

struct LocalState(Cell<u8>);

impl EventState for LocalState {
    fn cmpxchng_weak(
            &self,
            current: u8,
            new: u8,
            _: Ordering,
            _: Ordering,
        ) -> Result<u8, u8> {
        let value = self.0.get();
        if value == current {
            self.0.set(new);
            Ok(new)
        } else {
            Err(value)
        }
    }
    fn new(state: u8) -> Self {
        Self(Cell::new(state))
    }
    fn store(
            &self,
            value: u8,
            _: Ordering
        ) {
        self.0.set(value);
    }
}

impl_async_event!(
    /// An event for usage within local runtimess. It is `!Send` and
    /// `!Sync`.
    name = LocalEvent,
    waitername = LocalEventAwait,
    lot = LocalAllocatedAsyncLot,
    state = LocalState
);
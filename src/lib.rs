use std::{
    ops::ControlFlow, pin::{pin, Pin}, sync::atomic::{AtomicU8, AtomicUsize, Ordering}, task::{Context, Poll, Waker}
};


mod atomic;

use pin_project_lite::pin_project;

use crate::{async_lot::AsyncLot, yielder::Yield};

mod async_lot;
mod yielder;

const SIGNAL_FREE: u8 = 0b00;
// const SIGNAL_WAIT: u8 = 0b01;
const SIGNAL_SET: u8 = 0b10;

use crate::atomic::*;

trait EventSetter<'a> {
    type Waiter: Future<Output = ()> + 'a + Unpin;

    fn wait(&'a self) -> Self::Waiter;
    fn set_one(&self) -> bool;
    fn set_all<F: FnMut()>(&self, functor: F);
}

pub struct Event {
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
    fn try_yield(&mut self, cx: &mut Context<'_>) -> ControlFlow<()> {
        if let Some(mut yielder) = self.yielder.take() {
            match pin!(&mut yielder).poll(cx) {
                Poll::Pending => {
                    self.yielder = Some(yielder);
                    ControlFlow::Continue(())
                },
                Poll::Ready(()) => {
                    // here we proceed.
                    ControlFlow::Break(())
                }
            }
        } else {
            ControlFlow::Break(())
        }
    }
  
    // #[inline]
    // fn proper_unpark(&self) {
    //     // let mut new_pattern = SIGNAL_WAIT;
    //     if !self.event.waker.unpark_one() {
    //         // We failed to unpark the next one, queue should be empty.
    //         // new_pattern = SIGNAL_FREE;

    //     }

    //     // if self.event.inner.compare_exchange_weak(SIGNAL_SET | SIGNAL_WAIT, new_pattern, Ordering::Release, Ordering::Relaxed).is_ok() {
    //     //     println!("properly reset...");
    //     // }
    // }
}

impl Future for RawEventAwait<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {

        // Here we check if we are yielding, this is an asynchronous version of
        // backoff. If this returns `Continue` then we must poll again.
        if let ControlFlow::Continue(()) = self.try_yield(cx) {
            return Poll::Pending;
        }

        // First we want to check if the signal is set, if so we can just bypass all this checking.
        match self
            .event
            .inner
            .compare_exchange_weak(SIGNAL_SET, SIGNAL_FREE, Acquire, Relaxed)
        {
            Ok(_) => {
                // The optimistic path has succeeded, this means we came in just as the signal was set (already set).
                // For this to be fair, we can only return here if there are no other threads enqueued. If there are
                // other threads enqueued and we were to return here then the last person to the critical section wins,
                // and that is not fair.

                // The exception is if we were already parked. In this case, we can just unpark the next thread and then return ourselves.
                if self.is_parked {
                    return Poll::Ready(());
                }

                // SOLUTION:
                // Here we unpark a thread, this will tell us if there is another thread or not. If there is,
                // then we will add the current thread into the queue so that this is fair.
                if self.waker_list().unpark_one() {
                    // In this case there was another thread already there, therefore we must park the current thread.
                    self.park_waker(cx.waker());
                    return Poll::Pending;
                } else {
                    // There was nothing there, thus we are exclusive and
                    // can immediately return. 
                    return Poll::Ready(());
                }
            }
            Err(x) => {
                match x {
                    SIGNAL_FREE => {

                        // We need to recognize the case in which we were PREVIOUSLY parked and
                        // are no longer parked. In this case we were awakened on a set, and the
                        // bit was cleared in the meantime.
                        //
                        // This sort of situation can often happen when two `set()` calls are made back
                        // to back. In this case let's say Thread A & B are enqueued. Then the first set() call
                        // wakes `A`, `A` unsets the bit, and then when thread `B` is polled it believes it
                        // was called by mistake. Thus, we return here.
                        if self.is_parked {
                            return Poll::Ready(());
                        }

                        // We can park ourselves.
                        self.park_waker(cx.waker());
                        return Poll::Pending;
                    }
                    SIGNAL_SET => {
                        // If this is set, then this means there was
                        // a spurious failure. In this case we can backoff and try agian.
                        return self.backoff(cx);
                    }
                    _ => {
                        // SAFETY: We only ever mess with the first bit.
                        debug_assert!(false, "Entered the unreachable zone!");
                        unsafe { std::hint::unreachable_unchecked() };
                    }
                }
                
            }
        }
    }
}


impl Event {
    pub fn new() -> Self {
        Self {
            inner: RawEvent {
                inner: AtomicU8::default(),
                waker: AsyncLot::default(),
            },
        }
    }
    pub fn wait(&self) -> RawEventAwait<'_> {
        <Self as EventSetter>::wait(&self)
    }
    pub fn set_all(&self) {
        <Self as EventSetter>::set_all(&self, || {});
    }
    pub fn set_one(&self) {
        <Self as EventSetter>::set_one(&self);
    }
}

impl<'a> EventSetter<'a> for Event
where 
    Self: 'a
{
    type Waiter = RawEventAwait<'a>;

    fn set_all<F: FnMut()>(&self, mut f: F) {
        self.inner.inner.store(SIGNAL_SET, Ordering::Release);
        while self.inner.waker.unpark_one() {
            // unpark them all.
            f();
        }
    }
    fn set_one(&self) -> bool {
        self.inner.inner.store(SIGNAL_SET, Ordering::Release);
        self.inner.waker.unpark_one()
    }
    
    fn wait(&'a self) -> Self::Waiter {
        RawEventAwait {
            event: &self.inner,
            is_parked: false,
            backoff: 0,
            yielder: None
        }
    }
}

pub struct CountedEvent<E> {
    inner: E,
    counter: AtomicUsize
}

pin_project! {
    #[allow(private_bounds)]
    pub struct CountedAwaiter<'a, E, F>
    where
        E: EventSetter<'a>,
        F: Future<Output = ()>
    {
        master: &'a CountedEvent<E>,
        #[pin]
        future: F,
        is_done: bool
        
    }
}


impl<'a, E, F> Future for CountedAwaiter<'a, E, F>
where 
    E: EventSetter<'a>,
    F: Future<Output = ()> + Unpin
{
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("I GOT POLLED!!!");
        let this = self.project();
        if *this.is_done {
            match this.future.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(_) => {
                    this.master.counter.fetch_sub(1, Ordering::Release);
                    return Poll::Ready(());
                }
            }
        } else {
            println!("intering...");
            let result = this.future.poll(cx);
            println!("entering...");
            *this.is_done = true;
            this.master.counter.fetch_add(1, Ordering::Release);
            println!("entering... {}", this.master.counter.load(Ordering::Acquire));
            return result;
        }
    }
}


#[allow(private_bounds)]
impl<E> CountedEvent<E>
where 
    for<'a> E: EventSetter<'a>
{
    #[allow(private_interfaces)]
    pub fn new(source: E) -> Self {
        Self {
            inner: source,
            counter: AtomicUsize::new(0)
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
    E: EventSetter<'a> + 'a
{

    type Waiter = CountedAwaiter<'a, E, E::Waiter>;

    fn wait(&'a self) -> CountedAwaiter<'a, E, E::Waiter> {
        // self.counter.fetch_add(1, Release);
        CountedAwaiter { master: self, future: self.inner.wait(), is_done: false }

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


}


impl Drop for Event {
    fn drop(&mut self) {
        // on drop all the taks are set.
        self.set_all();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{atomic::Ordering, Arc},
        thread::sleep,
        time::Duration,
    };

    // use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
    use tokio::sync::Barrier;

    use crate::{CountedEvent, Event, EventSetter, SIGNAL_FREE};

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
    pub async fn waiter_async_set_one() {
        let signal = Arc::new(Event::new());
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
            // tokio::time::sleep(Duration::from_millis(10)).await;
            signal.set_one();
        }

        for v in bucket {
            v.await.unwrap();
        }

        // signal.wait().await;
    }



    #[tokio::test]
    pub async fn waiter_async_set_all() {
        let signal = Arc::new(Event::new());
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

        signal.set_all();

        for v in bucket {
            v.await.unwrap();
        }

        // signal.wait().await;
    }

 


    async fn wait_loop<F: FnMut() -> bool>(mut functor: F) {

        loop {
            if functor() {
                break;
            }
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    pub async fn counted_waiter_single() {
        let waiter = Arc::new(CountedEvent::new(Event::new()));

        assert_eq!(waiter.count(), 0);


        tokio::spawn({
            let waiter = waiter.clone();
            async move {
                waiter.wait().await;
            }
        });


        // Wait until the count is not zero.
        wait_loop(|| waiter.count() != 0).await;


        // Wake up this thread.
        waiter.set_one();

        // Wait until it is zero again.
        wait_loop(|| waiter.count() == 0).await;




    }
    
    


    // #[tokio::test]
    // pub async fn proper_wait_to_free_reset() {
    //     // Checks if the waiting bit is unset correctly.
    //     let event = Arc::new(AutoEvent::new());

    //     // should be the free bit.
    //     assert_eq!(event.inner.inner.load(std::sync::atomic::Ordering::Relaxed), SIGNAL_FREE);


    //     // spawn a waiter.

    //     let barrier = Arc::new(Barrier::new(2));

    //     tokio::spawn({
    //         let event = event.clone();
    //         let barrier = barrier.clone();
    //         async move {
    //             event.wait().await;
    //             barrier.wait().await;
    //         }
    //     });

    //     loop {
    //         // check for it to be active

    //         if event.inner.inner.load(Ordering::Acquire) & SIGNAL_WAIT != 0 {
    //             println!("the wait bit has been set...");
    //             break; // break out of loop!
    //         }
    //         tokio::time::sleep(Duration::from_millis(1)).await;
    //     }

    //     // now we set the event.
    //     event.set_one();

    //     // let the other thread see it.
    //     barrier.wait().await;

    //     // should be fully freed!
    //     assert_eq!(event.inner.inner.load(Ordering::Acquire), SIGNAL_FREE);


    // }
    

    #[tokio::test]
    pub async fn test_optimistic_acquire() {
        // checks if we can optimistically acquire the event & that it is properly reset.
        let waker = Event::new();
        
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

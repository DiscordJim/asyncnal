use core::{marker::PhantomData, task::Waker};

use crate::base::signal::{TechnicalCounter, ValidityMarker, WakeQueue};


pub(crate) struct AsyncLotTemplate<V, C, Q>
{
    inner: Q,
    counter: C,
    _validity: PhantomData<V>
}

impl<V, C, Q> Default for AsyncLotTemplate<V, C, Q>
where 
    C: TechnicalCounter,
    Q: WakeQueue<(Waker, V)>
{
    fn default() -> Self {
        Self {
            inner: Q::default(),
            counter: C::default(),
            _validity: PhantomData
        }
    }

}


impl<V, C, Q> AsyncLotTemplate<V, C, Q>
where 
    V: ValidityMarker,
    C: TechnicalCounter,
    Q: WakeQueue<(Waker, V)>
{
    #[inline]
    fn park(&self, waker: &Waker) -> V {
        self.counter.increment();
        let notifier = V::create();
        self.inner.enqueue((waker.clone(), notifier.clone()));
        notifier
    }
    #[inline]
    fn unpark_one(&self) -> bool {
        if let Some((waker, signal)) = self.try_unpark() {
            if signal.get() {
                waker.wake_by_ref();
                true
            } else {
                self.unpark_one()
            }
        } else {
            false
        }
    }
    fn notify_wait_release(&self) {
        self.counter.decrement()
    }
    fn has_waiters(&self) -> bool {
        self.counter.get() != 0
    }
    fn try_unpark(&self) -> Option<(Waker, V)> {
        self.inner.dequeue()
    }
}


pub trait AsyncLotSignature: Default {
    type ValidityMarker: ValidityMarker;

    fn new() -> Self;
    fn park(&self, cx: &core::task::Context<'_>) -> Self::ValidityMarker;
    fn notify_wait_release(&self);
    fn has_waiters(&self) -> bool;
    fn unpark_one(&self) -> bool;
}

impl<V, C, Q> AsyncLotSignature for AsyncLotTemplate<V, C, Q>
where 
    V: ValidityMarker,
    C: TechnicalCounter,
    Q: WakeQueue<(Waker, V)>
{

    type ValidityMarker = V;


    #[inline]
    fn new() -> Self {
        Self::default()
    }
    #[inline]
    fn has_waiters(&self) -> bool {
        AsyncLotTemplate::has_waiters(&self)
    }
    #[inline]
    fn notify_wait_release(&self) {
        AsyncLotTemplate::notify_wait_release(&self);
    }
    #[inline]
    fn park(&self, cx: &core::task::Context<'_>) -> Self::ValidityMarker {
        AsyncLotTemplate::park(&self, cx.waker())
    }
    #[inline]
    fn unpark_one(&self) -> bool {
        AsyncLotTemplate::unpark_one(&self)
    }
}


macro_rules! impl_async_lot {
    (
        name = $name:ident,
        waker = $waker:ty,
        validity = $vf:ty,
        counter = $counter:ty,
        queue = $queue:ident
    ) => {

        #[repr(transparent)]
        pub struct $name(AsyncLotTemplate<$vf, $counter, $queue<($waker, $vf)>>);


        impl Default for $name {
            fn default() -> Self {
                Self(AsyncLotTemplate::default())
            }
        }

        impl AsyncLotSignature for $name {
            type ValidityMarker = $vf;

            #[inline]
            fn new() -> Self {
                Self(AsyncLotTemplate::new())
            }
            #[inline]
            fn has_waiters(&self) -> bool {
                AsyncLotTemplate::has_waiters(&self.0)
            }
            #[inline]
            fn park(&self, cx: &core::task::Context<'_>) -> $vf {
                AsyncLotTemplate::park(&self.0, cx)
            }
            #[inline]
            fn unpark_one(&self) -> bool {
                AsyncLotTemplate::unpark_one(&self.0)
            }
            #[inline]
            fn notify_wait_release(&self) {
                AsyncLotTemplate::notify_wait_release(&self.0)
            }
        }
    };
}

pub(crate) use impl_async_lot;
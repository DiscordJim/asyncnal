use core::{
    pin::Pin, task::{Context, Poll}
};


/// An asynchronous Yield. This was largely just 
/// an implementation of Yield from the [smol](https://docs.rs/smol/latest/smol/future/struct.YieldNow.html)
/// crate.
#[derive(Default)]
#[repr(transparent)]
pub struct Yield {
    state: bool,
}

impl Future for Yield {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.state {
            self.state = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

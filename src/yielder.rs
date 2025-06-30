use std::{
    pin::Pin, sync::Mutex, task::{Context, Poll}, time::Duration
};


#[derive(Default)]
#[repr(transparent)]
pub struct Yield {
    state: bool,
}

// TODO: give credit to smol here.
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

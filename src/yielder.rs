use std::{
    pin::Pin, sync::Mutex, task::{Context, Poll}, time::Duration
};

const YIELD_SPINS: usize = 5;

enum BackoffState {
    QuickYield { count: usize },
    Sleep { count: usize }
}

pub(crate) struct SmallAsyncMutex {
    state: BackoffState,
}

pub struct Backoff {
    state: BackoffState,
}

#[derive(Default)]
#[repr(transparent)]
pub struct Yield {
    state: bool,
}

impl Future for BackoffState {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut *self {
            Self::QuickYield { count } => {
                if *count == 0 {
                    println!("Elapsed");
                    *self = Self::Sleep { count: 5 };
                    return self.poll(cx);
                }
                *count -= 1;
                println!("Count: {count}");
                match Pin::new(&mut Yield::default()).poll(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(()) => self.poll(cx),
                }
            }
            Self::Sleep { count } => {
                if *count == 0 {
                    println!("Elapsed");
                    return Poll::Ready(());
                }
                *count -= 1;
                println!("Count (S): {count}");

                let mut pd = tokio::time::sleep(Duration::from_millis(5));
                match std::pin::pin!(pd).poll(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(()) => self.poll(cx),
                }
            }
        }
    }
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

// #[cfg(test)]
// mod tests {
//     use crate::yielder::{Backoff, BackoffState, Yield, YIELD_SPINS};

//     #[tokio::test]
//     pub async fn looper() {
//         let mut bf = Backoff {
//             state: BackoffState::QuickYield { count: YIELD_SPINS },
//         };

//         bf.state.await;

//         // let mut handles = vec![];

//         // for i in 0..10 {
//         //     handles.push(tokio::spawn(async move {
//         //         loop {
//         //             for j in 0..(i + 1) {
//         //                 Yield::default().await;
//         //             }

//         //             println!("{i}");

//         //         }
//         //     }))
//         // }

//         // for handle in handles {
//         //     handle.await.unwrap();
//         // }
//     }
// }

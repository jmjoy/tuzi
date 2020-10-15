use futures::task::AtomicWaker;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context, Poll},
};

pub struct WaitGroup {
    inner: Arc<Inner>,
}

#[derive(Clone)]
pub struct Worker {
    inner: Arc<Inner>,
}

pub struct WaitGroupFuture {
    inner: Weak<Inner>,
}

struct Inner {
    waker: AtomicWaker,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.waker.wake();
    }
}

impl WaitGroup {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                waker: AtomicWaker::new(),
            }),
        }
    }

    pub fn worker(&self) -> Worker {
        Worker {
            inner: self.inner.clone(),
        }
    }

    pub fn wait(self) -> WaitGroupFuture {
        WaitGroupFuture {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl Default for WaitGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl Future for WaitGroupFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.upgrade() {
            Some(inner) => {
                inner.waker.register(cx.waker());
                Poll::Pending
            }
            None => Poll::Ready(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn smoke() {
        let wg = WaitGroup::new();

        for _ in 0..100 {
            let w = wg.worker();
            tokio::spawn(async move {
                drop(w);
            });
        }

        wg.wait().await;
    }
}

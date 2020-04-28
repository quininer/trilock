#[cfg(not(feature = "loom"))]
mod loom {
    pub use std::sync;

    pub mod cell {
        pub struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

        impl<T> UnsafeCell<T> {
            #[inline]
            pub fn new(t: T) -> UnsafeCell<T> {
                UnsafeCell(std::cell::UnsafeCell::new(t))
            }

            #[inline]
            pub fn with<F, R>(&self, f: F) -> R
            where F: FnOnce(*const T) -> R
            {
                f(self.0.get())
            }

            #[inline]
            pub fn with_mut<F, R>(&self, f: F) -> R
            where F: FnOnce(*mut T) -> R
            {
                f(self.0.get())
            }
        }
    }
}

use std::mem;
use std::pin::Pin;
use std::future::Future;
use std::ops::{ Deref, DerefMut };
use std::task::{ Context, Waker, Poll };
use loom::cell::UnsafeCell;
use loom::sync::{ Arc, Mutex };


pub struct TriLock<T> {
    inner: Arc<Inner<T>>,
    mark: usize
}

pub struct Guard<'a, T> {
    inner: &'a Inner<T>
}

pub struct TriLockFut<'a, T> {
    inner: Option<&'a TriLock<T>>
}

struct Inner<T> {
    state: Mutex<Semaphore>,
    value: UnsafeCell<T>
}

struct Semaphore {
    idle: bool,
    list: [Option<Waker>; 3],
}

unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

impl<T> TriLock<T> {
    pub fn new(t: T) -> (TriLock<T>, TriLock<T>, TriLock<T>) {
        let inner = Arc::new(Inner {
            state: Mutex::new(Semaphore {
                idle: true,
                list: [None, None, None]

            }),
            value: UnsafeCell::new(t)
        });

        (
            TriLock { inner: inner.clone(), mark: 0 },
            TriLock { inner: inner.clone(), mark: 1 },
            TriLock { inner, mark: 2 }
        )
    }

    fn poll_inner<'a>(&'a self, cx: &mut Context<'_>) -> Poll<Guard<'a, T>> {
        let mut state = self.inner.state.lock().unwrap();

        match mem::replace(&mut state.idle, false) {
            true => {
                state.list[self.mark].take();
                Poll::Ready(Guard { inner: &*self.inner })
            },
            false => {
                let waker = cx.waker().clone();
                state.list[self.mark] = Some(waker);
                Poll::Pending
            }
        }
    }

    #[inline]
    pub fn poll_lock<'a>(&'a mut self, cx: &mut Context<'_>) -> Poll<Guard<'a, T>> {
        self.poll_inner(cx)
    }

    #[inline]
    pub fn lock(&mut self) -> TriLockFut<'_, T> {
        TriLockFut { inner: Some(self) }
    }
}

impl<T> Drop for TriLock<T> {
    fn drop(&mut self) {
        let mut state = self.inner.state.lock().unwrap();

        // wake again
        if state.list[self.mark].take().is_none() && state.idle {
            for e in &mut state.list {
                if let Some(waker) = e.take() {
                    waker.wake();
                    break
                }
            }
        }
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.value.with(|val| unsafe { &*val })
    }
}

impl<T> DerefMut for Guard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.value.with_mut(|val| unsafe { &mut *val })
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        let mut state = self.inner.state.lock().unwrap();

        state.idle = true;

        for e in &mut state.list {
            if let Some(waker) = e.take() {
                waker.wake();
                break
            }
        }
    }
}

impl<'a, T> Future for TriLockFut<'a, T> {
    type Output = Guard<'a, T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        let lock = match this.inner.take() {
            Some(lock) => lock,
            None => return Poll::Pending
        };

        match lock.poll_inner(cx) {
            Poll::Ready(guard) => Poll::Ready(guard),
            Poll::Pending => {
                this.inner = Some(lock);
                Poll::Pending
            }
        }
    }
}

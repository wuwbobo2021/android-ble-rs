use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task;
use std::time::{Duration, Instant};

use async_broadcast::{InactiveReceiver, Receiver, Sender};
use async_lock::{Mutex, MutexGuard, OnceCell};
use futures_core::Stream;
use futures_lite::{FutureExt, StreamExt};
use futures_timer::Delay;

/// Reusable exclusive register for `ResultWaiter`.
pub struct Excluder<T: Send + Clone> {
    inner: Mutex<Option<LockMark>>,
    last_val: Arc<Mutex<Option<T>>>,
    timeout: Duration,
}

/// Prevents other tasks from doing the same operation before the corresponding
/// "foreign" callback is reiceived, or the timeout value is reached.
struct LockMark {
    id: usize,
    callback_sender: Sender<()>,
    #[allow(unused)]
    sender_keeper: InactiveReceiver<()>,
    tp_timeout: Arc<OnceCell<Instant>>,
}

/// Makes waiting for the result of the "foreign" callback possible.
pub struct ResultWaiter<T: Send + Clone> {
    receiver: Receiver<()>,
    last_val: Weak<Mutex<Option<T>>>,
    tp_timeout: Arc<OnceCell<Instant>>,
    timeout: Duration,
}

impl<T: Send + Clone, E: Send + Clone> Excluder<Result<T, E>> {
    /// Locks the excluder, does the operation that will produce the callback,
    /// then waits for the callback's result.
    #[allow(unused)]
    pub async fn obtain(&self, operation: impl FnOnce() -> Result<(), E>) -> Result<Option<T>, E> {
        let waiter = self.lock().await;
        operation()?;
        if let Some(res) = waiter.wait_unlock().await {
            Ok(Some(res?))
        } else {
            Ok(None)
        }
    }
}

impl<T: Send + Clone> Excluder<T> {
    /// Creates a new unlocked `Excluder`.
    pub fn new(callback_timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(None),
            last_val: Arc::new(Mutex::new(None)),
            timeout: callback_timeout,
        }
    }

    /// Clones and returns the last value returned by the "foreign" callback.
    pub fn last_value(&self) -> Option<T> {
        self.last_val.lock_blocking().clone()
    }

    /// Waits until the excluder is unlocked and locks the excluder.
    ///
    /// Call this *right before* calling a method that will produce a "foreign" callback;
    /// after calling that method, call [ResultWaiter::wait_unlock] in the same task.
    /// Otherwise, the lock will become invalid when the returned `ResultWaiter` is dropped;
    /// even if it is not dropped, another task that tries to lock this excluder will sleep
    /// for the general timeout value and then invalidate this lock with a new lock.
    pub async fn lock(&self) -> ResultWaiter<T> {
        let mut waited_without_tp_timeout = None;
        let mut guard_inner = loop {
            let guard_inner = self.inner.lock().await;
            if let Some(lock_mark) = guard_inner.as_ref() {
                if let Some(prev_id) = waited_without_tp_timeout.as_ref() {
                    if prev_id != &lock_mark.id {
                        let _ = waited_without_tp_timeout.take();
                    }
                }
                let dur_wait = if let Some(tp_timeout) = lock_mark.tp_timeout.get() {
                    if let Some(dur) = tp_timeout.checked_duration_since(Instant::now()) {
                        dur
                    } else {
                        break guard_inner;
                    }
                } else if waited_without_tp_timeout.is_none() {
                    waited_without_tp_timeout.replace(lock_mark.id);
                    self.timeout
                } else {
                    break guard_inner;
                };
                if dur_wait.is_zero() {
                    break guard_inner;
                }
                let mut receiver = lock_mark.callback_sender.new_receiver();
                let fut = receiver.recv().or(async {
                    Delay::new(dur_wait).await;
                    Err(async_broadcast::RecvError::Closed)
                });
                drop(guard_inner);
                let _ = fut.await;
            } else {
                break guard_inner;
            }
        };
        self.unchecked_set_lock(&mut guard_inner)
    }

    /// Locks the excluder if it is previously unlocked.
    pub fn try_lock(&self) -> Option<ResultWaiter<T>> {
        let mut guard_inner = self.inner.lock_blocking();
        if let Some(lock_mark) = guard_inner.as_ref() {
            if let Some(&tp_timeout) = lock_mark.tp_timeout.get() {
                if tp_timeout > Instant::now() {
                    return None;
                }
            } else {
                return None;
            }
        }
        Some(self.unchecked_set_lock(&mut guard_inner))
    }

    fn unchecked_set_lock(
        &self,
        guard_inner: &mut MutexGuard<Option<LockMark>>,
    ) -> ResultWaiter<T> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_LOCK_ID: AtomicUsize = AtomicUsize::new(0);

        let (sender, receiver) = async_broadcast::broadcast(2);
        let tp_timeout = Arc::new(OnceCell::new());
        let mark = LockMark {
            id: NEXT_LOCK_ID.fetch_add(1, Ordering::SeqCst),
            callback_sender: sender,
            sender_keeper: receiver.clone().deactivate(),
            tp_timeout: tp_timeout.clone(),
        };
        guard_inner.replace(mark);

        ResultWaiter {
            receiver,
            last_val: Arc::downgrade(&self.last_val),
            tp_timeout,
            timeout: self.timeout,
        }
    }

    /// Sends the "completed" (unlock) signal from the "foreign" callback.
    pub fn unlock(&self, result: T) {
        // XXX: this may be changed to disallow update of "last value" storage if `self`
        // is not locked by an operation.
        self.last_val.lock_blocking().replace(result);

        let mut guard_inner = self.inner.lock_blocking();
        if let Some(lock_mark) = guard_inner.take() {
            drop(guard_inner);
            let _ = lock_mark.callback_sender.broadcast_blocking(());
        }
    }
}

impl<T: Send + Clone> Default for Excluder<T> {
    fn default() -> Self {
        Self::new(Duration::from_secs(5))
    }
}

impl<T: Send + Clone> Drop for Excluder<T> {
    fn drop(&mut self) {
        // makes sure `ResultWaiter::wait_unlock` return `None`.
        let _ = self.last_val.lock_blocking().take();

        let mut guard_inner = self.inner.lock_blocking();
        if let Some(lock_mark) = guard_inner.take() {
            drop(guard_inner);
            let _ = lock_mark.callback_sender.broadcast_blocking(());
        }
    }
}

impl<T: Send + Clone> ResultWaiter<T> {
    /// Waits until the unlock signal is sent from the "foreign" callback or the timeout
    /// is reached. Returns `None` when timeout or when the corresponding `Excluder` is dropped.
    pub async fn wait_unlock(mut self) -> Option<T> {
        let tp_timeout = Instant::now() + self.timeout;
        let _ = self.tp_timeout.set_blocking(tp_timeout);
        let dur_wait = tp_timeout
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::from_millis(1));
        let res = self
            .receiver
            .recv()
            .or(async {
                Delay::new(dur_wait).await;
                Err(async_broadcast::RecvError::Closed)
            })
            .await;
        res.ok()?;
        let last_val = self.last_val.upgrade()?;
        let val = last_val.lock().await.as_ref().cloned();
        val
    }
}

impl<T: Send + Clone> Drop for ResultWaiter<T> {
    fn drop(&mut self) {
        // If `tp_timeout` is not previously set, it indicates that `wait_unlock` hasn't been called
        // before dropping; in this case, just invalidate the registered lock immediately:
        if self.tp_timeout.set_blocking(Instant::now()).is_ok() {
            let _ = self.receiver.new_sender().broadcast_blocking(());
        }
    }
}

/// Sends notifications from "foreign" callbacks if there is any existing `NotifierReceiver`.
pub struct Notifier<T: Send + Clone> {
    capacity: usize,
    inner: Mutex<Weak<NotifierInner<T>>>,
}

struct NotifierInner<T: Send + Clone> {
    sender: Sender<Option<T>>,
    on_stop: Box<dyn Fn() + Send + Sync + 'static>,
}

pub struct NotifierReceiver<T: Send + Clone> {
    holder: Option<Arc<NotifierInner<T>>>,
    receiver: Receiver<Option<T>>,
}

impl<T: Send + Clone> Notifier<T> {
    /// Creates a new inactive `Notifier`.
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Mutex::new(Weak::new()),
        }
    }

    /// Checks if the notifier is active.
    pub fn is_notifying(&self) -> bool {
        // Don't call it in this module
        self.inner.lock_blocking().strong_count() > 0
    }

    /// Creates a new `NotifierReceiver` for the caller to receive notifications.
    /// - `on_start` is called while locking the notifier if the notifier is not active.
    /// - `on_stop` is what the notifier should do when it is deactivated, but it is not
    ///   replaced if the notifier is already active.
    pub async fn subscribe<E>(
        &self,
        on_start: impl FnOnce() -> Result<(), E>,
        on_stop: impl Fn() + Send + Sync + 'static,
    ) -> Result<NotifierReceiver<T>, E> {
        let mut guard_inner = self.inner.lock().await;
        if let Some(inner) = guard_inner.upgrade() {
            let receiver = inner.sender.new_receiver();
            Ok(NotifierReceiver {
                holder: Some(inner),
                receiver,
            })
        } else {
            on_start()?;
            let (mut sender, receiver) = async_broadcast::broadcast(self.capacity);
            sender.set_overflow(true);
            let new_inner = Arc::new(NotifierInner {
                sender,
                on_stop: Box::new(on_stop),
            });
            *guard_inner = Arc::downgrade(&new_inner);
            Ok(NotifierReceiver {
                holder: Some(new_inner),
                receiver,
            })
        }
    }

    /// Sends a notifcation value from the "foreign" callback.
    pub fn notify(&self, value: T) {
        let inner = self.inner.lock_blocking().upgrade();
        if let Some(inner) = inner {
            let _ = inner.sender.broadcast_blocking(Some(value));
        }
    }
}

impl<T: Send + Clone> futures_core::Stream for NotifierReceiver<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Option<T>> {
        if self.holder.is_none() {
            task::Poll::Ready(None)
        } else if let task::Poll::Ready(result) = std::pin::pin!(&mut self.receiver).poll_next(cx) {
            if let Some(value) = result.flatten() {
                task::Poll::Ready(Some(value))
            } else {
                let _ = self.holder.take();
                task::Poll::Ready(None)
            }
        } else {
            task::Poll::Pending
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.receiver.size_hint()
    }
}

impl<T: Send + Clone> Drop for Notifier<T> {
    fn drop(&mut self) {
        let inner = self.inner.lock_blocking().upgrade();
        if let Some(inner) = inner {
            let _ = inner.sender.broadcast_blocking(None);
        }
    }
}

impl<T: Send + Clone> Drop for NotifierInner<T> {
    fn drop(&mut self) {
        (self.on_stop)()
    }
}

/// Wraps the main stream and also checks an event stream; ends and fuses the main stream when
/// the event stream ends or the checker returns true for a received event item.
pub struct StreamUntil<T, E, S, F>
where
    T: Send + Unpin,
    E: Send,
    S: Stream<Item = E> + Send + Unpin,
    F: Fn(&E) -> bool + Send + Sync + Unpin + 'static,
{
    stream: S,
    event_checker: F,
    ph: PhantomData<T>,
}

impl<T, E, S, F> StreamUntil<T, E, S, F>
where
    T: Send + Unpin,
    E: Send,
    S: Stream<Item = E> + Send + Unpin,
    F: Fn(&E) -> bool + Send + Sync + Unpin + 'static,
{
    /// Creates the `StreamUntil`.
    pub fn create(
        stream: impl Stream<Item = T>,
        event_stream: S,
        event_checker: F,
    ) -> impl Stream<Item = T> {
        stream
            .or(StreamUntil {
                stream: event_stream,
                event_checker,
                ph: PhantomData,
            })
            .fuse()
    }
}

impl<T, E, S, F> futures_core::Stream for StreamUntil<T, E, S, F>
where
    T: Send + Unpin,
    E: Send,
    S: Stream<Item = E> + Send + Unpin,
    F: Fn(&E) -> bool + Send + Sync + Unpin + 'static,
{
    type Item = T;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures_core::task::Poll;
        match self.stream.poll_next(cx) {
            Poll::Ready(Some(event)) if (self.event_checker)(&event) => Poll::Ready(None),
            Poll::Ready(None) => Poll::Ready(None),
            _ => Poll::Pending,
        }
    }
}

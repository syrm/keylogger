/// Wraps a value whose `Drop` impl may block (e.g. closing a device fd that
/// the kernel takes 50-100ms to tear down), and offloads that drop onto
/// Tokio's blocking thread pool instead of running it on the async worker
/// thread, regardless of which exit path (return, panic, ...) triggers it.
pub(crate) struct DeferredDrop<T: Send + 'static>(Option<T>);

impl<T: Send + 'static> DeferredDrop<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Some(value))
    }

    /// Takes the inner value out, bypassing the deferred drop (use this when
    /// you need to move the value onward, e.g. into another owning type).
    fn into_inner(mut self) -> T {
        self.0.take().expect("value already taken")
    }
}

impl<T: Send + 'static> std::ops::Deref for DeferredDrop<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("value already taken")
    }
}

impl<T: Send + 'static> std::ops::DerefMut for DeferredDrop<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("value already taken")
    }
}

impl<T: Send + 'static> Drop for DeferredDrop<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            tokio::task::spawn_blocking(move || drop(value));
        }
    }
}

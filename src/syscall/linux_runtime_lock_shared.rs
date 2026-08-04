pub(crate) struct LinuxRuntimeLock<T> {
    locked: core::sync::atomic::AtomicBool,
    value: core::cell::UnsafeCell<T>,
}

// SAFETY: The atomic lock serializes all access to the contained value.
unsafe impl<T: Send> Sync for LinuxRuntimeLock<T> {}

impl<T> LinuxRuntimeLock<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }

    pub(crate) fn try_lock(&self) -> Option<LinuxRuntimeGuard<'_, T>> {
        self.locked
            .compare_exchange(
                false,
                true,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
            .ok()
            .map(|_| LinuxRuntimeGuard { lock: self })
    }

    pub(crate) fn lock(&self) -> LinuxRuntimeGuard<'_, T> {
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            core::hint::spin_loop();
        }
    }
}

pub(crate) struct LinuxRuntimeGuard<'a, T> {
    lock: &'a LinuxRuntimeLock<T>,
}

impl<T> core::ops::Deref for LinuxRuntimeGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The guard holds the lock for its full lifetime.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> core::ops::DerefMut for LinuxRuntimeGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The guard uniquely owns mutable access while the lock is held.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for LinuxRuntimeGuard<'_, T> {
    fn drop(&mut self) {
        self.lock
            .locked
            .store(false, core::sync::atomic::Ordering::Release);
    }
}

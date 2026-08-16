use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// The owner of a cooperative process cancellation signal.
#[derive(Debug)]
pub struct CancellationSource {
    shared: Arc<AtomicBool>,
}

impl CancellationSource {
    /// Creates a cancellation source in the active state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns a read-only token for one supervised operation.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        CancellationToken {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Requests cancellation. Repeated calls are idempotent.
    pub fn cancel(&self) {
        self.shared.store(true, Ordering::Release);
    }
}

impl Default for CancellationSource {
    fn default() -> Self {
        Self::new()
    }
}

/// A cloneable, read-only cooperative cancellation signal.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    shared: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Returns a token which has not been cancelled.
    #[must_use]
    pub fn new() -> Self {
        CancellationSource::new().token()
    }

    /// Reports whether the owning source requested cancellation.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.shared.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

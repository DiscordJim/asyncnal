
#[cfg(all(not(feature = "portable-atomic")))]
pub use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering::*, AtomicU8, Ordering};

#[cfg(feature = "portable-atomic")]
pub use portable_atomic::{AtomicBool, AtomicUsize, Ordering::*, AtomicU8, Ordering};
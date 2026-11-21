//! The window asks the heavy work to stay a step behind it.
//!
//! A conversion still takes the cores it is given. On Windows and macOS a
//! program started by a thread does not inherit that thread's priority, so
//! the comparison's RawTherapee is lowered on its own when a window asked.

use std::sync::atomic::{AtomicBool, Ordering};

static SHOWING: AtomicBool = AtomicBool::new(false);

/// A window is on screen. Called once, before any conversion.
pub fn window_is_showing() {
    SHOWING.store(true, Ordering::Relaxed);
}

/// Windows and macOS read this when they start RawTherapee. On Linux the
/// program inherits the priority of the thread that starts it.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub(crate) fn yields_to_a_window() -> bool {
    SHOWING.load(Ordering::Relaxed)
}

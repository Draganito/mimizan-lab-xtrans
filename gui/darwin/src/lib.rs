//! Lower the calling thread on macOS, and leave every other thread alone.
//!
//! Niceness there belongs to the whole process, so a lower nice value
//! would slow the window too. The portable priority call does not lower
//! a normal thread. The utility class does: while the window is idle the
//! work still fills the cores, and when the window draws it is served first.

#[cfg(target_os = "macos")]
mod qos {
    use std::os::raw::{c_int, c_uint, c_void};

    /// Long work that must not get in front of the window. The top of
    /// that class is still behind the window's own class.
    const UTILITY: c_uint = 0x11;

    extern "C" {
        fn pthread_set_qos_class_self_np(class: c_uint, relative_priority: c_int) -> c_int;
        fn pthread_get_qos_class_np(
            thread: *mut c_void,
            class: *mut c_uint,
            relative_priority: *mut c_int,
        ) -> c_int;
        fn pthread_self() -> *mut c_void;
    }

    /// This thread only. The window's thread never calls it.
    pub fn step_behind() {
        // The class is the documented utility value, and 0 is the top of
        // its relative range (-15..=0). A failure leaves the thread as it
        // was; the caller has no second way to lower just this thread.
        let _ = unsafe { pthread_set_qos_class_self_np(UTILITY, 0) };
    }

    /// The class of this thread, or 0 when the system will not say.
    pub fn class_of_this_thread() -> u32 {
        let mut class: c_uint = 0;
        let mut relative: c_int = 0;
        // Both pointers address these locals, and the thread is this one.
        let rc = unsafe { pthread_get_qos_class_np(pthread_self(), &mut class, &mut relative) };
        if rc == 0 { class } else { 0 }
    }

    pub const UTILITY_CLASS: u32 = UTILITY;
}

#[cfg(target_os = "macos")]
pub use qos::{class_of_this_thread, step_behind, UTILITY_CLASS};

//! Keep the window able to draw while a conversion takes the processor.
//!
//! The work runs on every core but one, and a step behind the window's
//! own thread. While the window is idle the cores stay full. When it
//! draws, it is served first. On Linux a child program inherits that
//! step. On Windows and macOS it does not, and the comparison lowers
//! RawTherapee itself.

#[cfg(not(target_os = "macos"))]
use thread_priority::{set_current_thread_priority, ThreadPriority, ThreadPriorityValue};

/// Cores the conversion may fill. One stays with the window. A machine
/// with a single core shares it, and the step back is what keeps the
/// window moving.
pub fn cores_for_the_work() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).saturating_sub(1).max(1)
}

/// About ten steps down on Linux (`nice` 9) and below normal on Windows.
/// 23 of 0..=99 lands there on both. macOS uses the utility class instead:
/// this call does not lower a normal thread there.
#[cfg(not(target_os = "macos"))]
fn priority() -> ThreadPriority {
    let step = ThreadPriorityValue::try_from(23u8).expect("23 is inside 0..=99");
    ThreadPriority::Crossplatform(step)
}

/// This thread runs a step behind the window.
pub fn step_behind() {
    #[cfg(target_os = "macos")]
    mimizan_lab_xtrans_darwin::step_behind();
    #[cfg(not(target_os = "macos"))]
    let _ = set_current_thread_priority(priority());
}

/// The pool every conversion uses. Called once, before any work, from
/// the window. The command line never calls it and keeps every core.
pub fn install() {
    mimizan_lab_xtrans::room::window_is_showing();
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(cores_for_the_work())
        .thread_name(|i| format!("work-{i}"))
        .start_handler(|_| step_behind())
        .build_global();
}

#[cfg(test)]
mod tests {
    use super::cores_for_the_work;

    #[test]
    fn one_core_stays_with_the_window() {
        let all = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let work = cores_for_the_work();
        assert!(work >= 1);
        if all > 1 {
            assert_eq!(work, all - 1);
        } else {
            assert_eq!(work, 1);
        }
    }

    /// A child prints the niceness it inherited. The window's thread stays
    /// where it started; the work's thread steps down, and so does a
    /// program it starts. The start is measured, not assumed to be zero:
    /// a runner may already be stepped back.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_work_steps_behind_and_the_window_thread_does_not() {
        fn nice_of_a_child() -> i32 {
            let out = std::process::Command::new("python3")
                .args(["-c", "import os; print(os.nice(0))"])
                .output()
                .expect("python3");
            String::from_utf8_lossy(&out.stdout).trim().parse().expect("a niceness")
        }
        let window = nice_of_a_child();
        let from_the_work = std::thread::spawn(|| {
            super::step_behind();
            nice_of_a_child()
        })
        .join()
        .expect("the work thread");
        assert!(from_the_work > window, "work {from_the_work}, window {window}");
        assert_eq!(nice_of_a_child(), window);
    }

    /// The work takes the utility class. The window's thread keeps the
    /// class it had. Niceness is the wrong ruler here: macOS reports it
    /// shifted, and it would cover the whole process.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_work_steps_behind_and_the_window_thread_does_not() {
        let window = mimizan_lab_xtrans_darwin::class_of_this_thread();
        let from_the_work = std::thread::spawn(|| {
            super::step_behind();
            mimizan_lab_xtrans_darwin::class_of_this_thread()
        })
        .join()
        .expect("the work thread");
        assert_eq!(from_the_work, mimizan_lab_xtrans_darwin::UTILITY_CLASS, "{from_the_work:#x}");
        assert_eq!(mimizan_lab_xtrans_darwin::class_of_this_thread(), window);
    }
}

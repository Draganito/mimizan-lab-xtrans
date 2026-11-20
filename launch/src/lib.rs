//! The programs are compiled for x86-64-v3. On an older CPU their first
//! AVX2 instruction is an illegal-instruction crash, before `main`.
//! This starter is compiled without those instructions. It checks the CPU
//! and only then replaces itself with the real program.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What an older CPU is told, on Linux and on Windows.
pub fn requirement() -> &'static str {
    "\
Mimizan Lab X-Trans cannot start on this CPU.

It needs x86-64-v3: AVX2 and FMA. Intel Core processors have had these \
since Haswell (2013), and AMD processors since Excavator (2015), including \
every Ryzen.

This computer does not have them."
}

/// True when the CPU has every instruction x86-64-v3 adds, and the
/// operating system has enabled them. On any other architecture there is
/// nothing to check: the Apple silicon build is not an x86-64-v3 program.
pub fn supports_cpu() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::is_x86_feature_detected;
        is_x86_feature_detected!("avx")
            && is_x86_feature_detected!("avx2")
            && is_x86_feature_detected!("bmi1")
            && is_x86_feature_detected!("bmi2")
            && is_x86_feature_detected!("cmpxchg16b")
            && is_x86_feature_detected!("f16c")
            && is_x86_feature_detected!("fma")
            && is_x86_feature_detected!("lzcnt")
            && is_x86_feature_detected!("movbe")
            && is_x86_feature_detected!("popcnt")
            && is_x86_feature_detected!("sse3")
            && is_x86_feature_detected!("sse4.1")
            && is_x86_feature_detected!("sse4.2")
            && is_x86_feature_detected!("ssse3")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

/// The real program next to this starter.
///
/// Debian installs the starter in `/usr/bin` and the program in
/// `/usr/lib/mimizan-lab-xtrans`. The Windows archive puts the program in
/// `bin` beside the starter.
pub fn find_program(launcher: &Path, program: &str) -> Option<PathBuf> {
    let dir = launcher.parent()?;
    let file = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    [dir.join("bin").join(&file), dir.join("../lib/mimizan-lab-xtrans").join(&file)]
        .into_iter()
        .find(|path| path.is_file())
}

/// Check the CPU, then replace this process with `program`.
/// `window` is the GUI starter: on Linux a menu launch has no terminal,
/// so the message is also a dialog. On Windows the console of a double
/// click closes with the process, so the command gets a dialog too.
pub fn start(program: &str, window: bool) -> ! {
    if !supports_cpu() {
        tell(requirement(), window);
        std::process::exit(1);
    }
    let launcher = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            tell(&format!("Mimizan Lab X-Trans could not find its own path: {error}"), window);
            std::process::exit(1);
        }
    };
    let Some(real) = find_program(&launcher, program) else {
        tell(
            &format!(
                "Mimizan Lab X-Trans could not find its program ({program}). \
                 The installation is incomplete."
            ),
            window,
        );
        std::process::exit(1);
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    hand_over(&real, &args, window)
}

fn tell(text: &str, window: bool) {
    eprintln!("{text}");
    // Windows opens a console for the command and closes it as the process
    // ends, so a box is the only thing a double click can read. The window
    // on Linux is started with no terminal at all.
    if window || cfg!(target_os = "windows") {
        dialog(text);
    }
}

fn hand_over(real: &Path, args: &[std::ffi::OsString], window: bool) -> ! {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new(real).args(args).exec();
        tell(&format!("Mimizan Lab X-Trans could not start {}: {error}", real.display()), window);
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        match Command::new(real).args(args).status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(error) => {
                tell(&format!("Mimizan Lab X-Trans could not start {}: {error}", real.display()), window);
                std::process::exit(1);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn dialog(text: &str) {
    // Whichever dialog the desktop has. A machine with none still has the
    // line on stderr, for a launch from a terminal.
    let zenity = Command::new("zenity")
        .args(["--error", "--title=Mimizan Lab X-Trans", "--no-wrap", "--text", text])
        .status();
    if matches!(zenity, Ok(status) if status.success()) {
        return;
    }
    let kdialog = Command::new("kdialog").args(["--title", "Mimizan Lab X-Trans", "--error", text]).status();
    if matches!(kdialog, Ok(status) if status.success()) {
        return;
    }
    let _ = Command::new("notify-send").args(["Mimizan Lab X-Trans", text]).status();
}

#[cfg(target_os = "windows")]
fn dialog(text: &str) {
    use std::os::windows::ffi::OsStrExt;
    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }
    let text = wide(text);
    let title = wide("Mimizan Lab X-Trans");
    // SAFETY: a null owner is the desktop, and both strings are NUL-terminated UTF-16.
    // 0x10 is MB_ICONERROR.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x0000_0010);
    }
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn dialog(_text: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_names_the_instructions_and_the_processors() {
        let text = requirement();
        for word in ["AVX2", "FMA", "x86-64-v3", "Haswell", "Excavator", "Ryzen"] {
            assert!(text.contains(word), "{word}");
        }
    }

    #[test]
    fn this_build_machine_can_run_the_program() {
        assert!(supports_cpu(), "this CPU is missing an x86-64-v3 instruction");
    }

    #[test]
    fn the_program_is_found_beside_the_starter_and_in_the_debian_lib_dir() {
        let root = std::env::temp_dir().join(format!("xtrans-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin_dir = root.join("win");
        let lib_dir = root.join("usr/lib/mimizan-lab-xtrans");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(bin_dir.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::create_dir_all(&lib_dir).unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        let win_program = bin_dir.join("bin").join(format!("xtrans2dng{suffix}"));
        let deb_program = lib_dir.join(format!("xtrans2dng-gui{suffix}"));
        std::fs::write(&win_program, b"x").unwrap();
        std::fs::write(&deb_program, b"x").unwrap();

        let found = find_program(&bin_dir.join(format!("xtrans2dng{suffix}")), "xtrans2dng").unwrap();
        assert_eq!(found.canonicalize().unwrap(), win_program.canonicalize().unwrap());
        let found = find_program(&root.join("usr/bin/xtrans2dng-gui"), "xtrans2dng-gui").unwrap();
        assert_eq!(found.canonicalize().unwrap(), deb_program.canonicalize().unwrap());
        assert!(find_program(&root.join("usr/bin/xtrans2dng"), "xtrans2dng").is_none());

        let _ = std::fs::remove_dir_all(&root);
    }
}

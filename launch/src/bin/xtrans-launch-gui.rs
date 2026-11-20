//! The window's starter. Installed as `xtrans2dng-gui`. On Windows it is
//! a windowed program, so a double click opens no console.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    mimizan_lab_xtrans_launch::start("xtrans2dng-gui", true);
}

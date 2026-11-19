//! xtrans2dng in a window: Fuji X-Trans RAFs are taken into a list,
//! marked, and converted to linear DNGs one after the other. The camera's
//! preview is shown until a file's DNG is written, then the DNG.
//!
//! The conversion is the library's, the same the command line runs. The
//! program is GPL-3.0-or-later.
//!
//! On Windows this is a windowed program: a double click opens no
//! console.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

mod app;
mod load;
mod queue;
mod settings;
mod theme;
mod view;
mod work;

const NAME: &str = "Mimizan Lab X-Trans";
/// Name in the title bar, with the version this binary was built from.
const WINDOW_TITLE: &str = concat!("Mimizan Lab X-Trans ", env!("CARGO_PKG_VERSION"));

/// The letter M of Mimizan Lab.
fn app_icon() -> Arc<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/icon-256.png"))
        .expect("embedded application icon")
        .into_rgba8();
    let (width, height) = image.dimensions();
    Arc::new(egui::IconData { rgba: image.into_raw(), width, height })
}

fn main() -> eframe::Result {
    // Files and folders named on the command line are in the list from
    // the start: "Open with" of a file manager.
    let initial: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    // Answered without a window, so a package can be tried where there
    // is no screen.
    if matches!(initial.as_slice(), [only] if only.as_os_str() == "--version") {
        println!("xtrans2dng-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // `XTRANS_GUI_WINDOW=1600x1000` sets the size and leaves the frame
    // off, for a picture the window takes of itself: a window manager
    // shrinks or maximizes a framed window as it sees fit.
    let asked = std::env::var("XTRANS_GUI_WINDOW").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        let size = [w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?];
        size.iter().all(|v| v.is_finite() && (320.0..=16384.0).contains(v)).then_some(size)
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(asked.unwrap_or([1360.0, 860.0]))
        .with_min_inner_size([900.0, 560.0])
        .with_title(WINDOW_TITLE)
        .with_icon(app_icon())
        .with_drag_and_drop(true)
        .with_app_id("mimizan-lab-xtrans");
    if let Some(size) = asked {
        viewport = viewport
            .with_decorations(false)
            .with_resizable(false)
            .with_min_inner_size(size)
            .with_max_inner_size(size);
    }
    #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // Windows: DirectX 12 and nothing else, so the window does not look
    // for an OpenGL or Vulkan driver that a virtual machine does not have.
    #[cfg(target_os = "windows")]
    {
        options.renderer = eframe::Renderer::Wgpu;
        let mut wgpu = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        wgpu.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        options.wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(wgpu);
    }
    eframe::run_native(NAME, options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial)))))
}

//! On Windows, bake the letter M into xtrans2dng-gui.exe so Explorer shows
//! it. The window's own icon is set when it opens, from the same drawing.

fn main() {
    let icon = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .join("assets/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_icon(&icon);
}

#[cfg(target_os = "windows")]
fn embed_icon(icon: &std::path::Path) {
    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon.to_str().expect("icon path is unicode"))
        .set("ProductName", "Mimizan Lab X-Trans")
        .set("FileDescription", "Mimizan Lab X-Trans");
    res.compile().expect("compile Windows icon resource");
}

#[cfg(not(target_os = "windows"))]
fn embed_icon(_icon: &std::path::Path) {}

//! An optional pair of pictures: this program's, and RawTherapee's 3-pass
//! of the same RAF, developed with one profile. Nothing here runs unless
//! it is asked for. A DNG already beside the RAF is used and not changed.
//! Otherwise a temporary one is written, and the picture it was made from
//! is dropped before RawTherapee starts.
//!
//! RawTherapee has to be installed. The profile pins the look: camera
//! white balance, no tone curve, no contrast, no sharpening, no noise
//! reduction, no highlight recovery, sRGB. The one thing that differs is
//! the demosaic. RawTherapee uses Markesteijn 3-pass; false colour
//! suppression is the only number that can be set, from 0 to 5. The DNG
//! is already three channels, so RawTherapee does not demosaic it.
//! Both pictures are written as 16-bit TIFF, without loss.

use crate::convert::{self, Settings};
use crate::dng;
use crate::file;
use image::{ImageBuffer, Rgb};

/// 16-bit RGB. The crate keeps this name private, so it is named here.
type Rgb16 = ImageBuffer<Rgb<u16>, Vec<u16>>;
use rawler::decoders::{Decoder, RawDecodeParams};
use rawler::rawsource::RawSource;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Rows in one strip of the TIFF. A window reads only the strips it shows.
const ROWS_PER_STRIP: u32 = 32;

/// Pixels RawTherapee leaves off each edge of an X-Trans mosaic. The
/// profile sets this, and the crop of its picture uses the same number.
const BORDER: usize = 7;

/// What RawTherapee 5.11 writes for Markesteijn's three passes.
const THREE_PASS: &str = "3-pass (best)";

/// The two 16-bit TIFFs [`make`] writes in the folder beside the RAF.
pub const OURS: &str = "mimizan-lab-xtrans.tif";
pub const THEIRS: &str = "rawtherapee.tif";

/// False colour suppression for the 3-pass only. Off is none, whatever
/// the stored steps are. On uses the steps, from 0 to 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suppression {
    pub on: bool,
    pub steps: u8,
}

impl Suppression {
    /// The value the profile gets.
    pub fn steps(self) -> u8 {
        if self.on {
            self.steps.min(5)
        } else {
            0
        }
    }
}

/// The profile both pictures are developed with. `steps` is the false
/// colour suppression, already decided by [`Suppression::steps`].
pub fn profile(steps: u8) -> String {
    let steps = steps.min(5);
    format!(
        "\
[Exposure]
Auto=false
Clip=0
Compensation=0
Brightness=0
Contrast=0
Saturation=0
Black=0
HighlightCompr=0
HighlightComprThreshold=0
ShadowCompr=0
HistogramMatching=false
Curve=0;
Curve2=0;

[HLRecovery]
Enabled=false

[Sharpening]
Enabled=false

[SharpenEdge]
Enabled=false

[SharpenMicro]
Enabled=false

[Impulse Denoising]
Enabled=false

[Directional Pyramid Denoising]
Enabled=false

[Defringing]
Enabled=false

[Dehaze]
Enabled=false

[Shadows & Highlights]
Enabled=false

[Vibrance]
Enabled=false

[ToneEqualizer]
Enabled=false

[PostDemosaicSharpening]
Enabled=false

[White Balance]
Enabled=true
Setting=Camera

[Color Management]
InputProfile=(cameraICC)
ToneCurve=false
ApplyLookTable=false
ApplyBaselineExposureOffset=false
ApplyHueSatMap=false
WorkingProfile=ProPhoto
OutputProfile=RTv4_sRGB
OutputProfileIntent=Relative

[Crop]
Enabled=false

[Resize]
Enabled=false

[Distortion]
Amount=0

[Rotation]
Degree=0

[LensProfile]
LcMode=none

[Vignetting Correction]
Amount=0

[RAW]
CA=false
HotPixelFilter=false
DeadPixelFilter=false

[RAW Bayer]
CcSteps={steps}

[RAW X-Trans]
Method={THREE_PASS}
Border={BORDER}
CcSteps={steps}
"
    )
}

/// Where RawTherapee's command is, or the one that was chosen.
pub fn find(chosen: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = chosen.filter(|p| p.is_file()) {
        return Some(path.to_path_buf());
    }
    on_path(cli_name()).or_else(well_known).or_else(flatpak_cli)
}

fn cli_name() -> &'static str {
    if cfg!(windows) {
        "rawtherapee-cli.exe"
    } else {
        "rawtherapee-cli"
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file())
}

fn well_known() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let mut found = Vec::new();
    if cfg!(windows) {
        for var in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Some(root) = std::env::var_os(var) {
                found.extend(rawtherapee_dirs(&PathBuf::from(root)));
            }
        }
    } else if cfg!(target_os = "macos") {
        found.push(PathBuf::from("/Applications/RawTherapee.app/Contents/MacOS/rawtherapee-cli"));
        found.push(PathBuf::from("/opt/homebrew/bin/rawtherapee-cli"));
        found.push(PathBuf::from("/usr/local/bin/rawtherapee-cli"));
        if let Some(home) = &home {
            found.push(home.join("Applications/RawTherapee.app/Contents/MacOS/rawtherapee-cli"));
        }
    } else {
        found.push(PathBuf::from("/usr/bin/rawtherapee-cli"));
        found.push(PathBuf::from("/usr/local/bin/rawtherapee-cli"));
    }
    found.into_iter().find(|p| p.is_file())
}

/// `C:\Program Files\RawTherapee*\rawtherapee-cli.exe`.
fn rawtherapee_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(read) = fs::read_dir(root) else { return out };
    for entry in read.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.to_ascii_lowercase().starts_with("rawtherapee") {
            out.push(entry.path().join(cli_name()));
        }
    }
    out
}

/// A Flatpak build has no file on the path. The command is then a small
/// script, and [`find`] returns that.
fn flatpak_cli() -> Option<PathBuf> {
    if cfg!(windows) || !flatpak_has_rawtherapee() {
        return None;
    }
    let dir = std::env::temp_dir().join("mimizan-lab-xtrans-compare");
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join("rawtherapee-flatpak");
    let script = "#!/bin/sh\nexec flatpak run --command=rawtherapee-cli com.rawtherapee.RawTherapee \"$@\"\n";
    fs::write(&path, script).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&path).ok()?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&path, perm).ok()?;
    }
    Some(path)
}

fn flatpak_has_rawtherapee() -> bool {
    Command::new("flatpak")
        .args(["info", "com.rawtherapee.RawTherapee"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Raw pixel of RawTherapee's top left.
///
/// Its picture is either the file's crop or the whole mosaic, each with
/// [`BORDER`] pixels taken off every edge or without that. Failing that,
/// it is the rectangle LibRaw gives RawTherapee for this sensor, again
/// with the border taken off. Anything else is not laid over the DNG.
pub fn rt_origin(
    raw: (usize, usize),
    crop: (usize, usize, usize, usize),
    border: usize,
    out: (usize, usize),
) -> Option<(usize, usize)> {
    let inset = |x: usize, y: usize, w: usize, h: usize| {
        let (w, h) = (w.saturating_sub(border.saturating_mul(2)), h.saturating_sub(border.saturating_mul(2)));
        (x + border, y + border, w, h)
    };
    let (cw, ch) = (crop.2, crop.3);
    let mut candidates = [
        Some(inset(crop.0, crop.1, cw, ch)),
        Some(inset(0, 0, raw.0, raw.1)),
        Some((crop.0, crop.1, cw, ch)),
        Some((0, 0, raw.0, raw.1)),
        None,
        None,
    ];
    // RawTherapee 5.11 demosaics what LibRaw hands it, then drops `border`
    // pixels on every side. That rectangle is not the file's own crop: an
    // X-T1 comes out 4920 x 3282 from a 4992 x 3296 mosaic.
    if let Some((x, y, w, h)) = fuji_libraw(raw) {
        candidates[4] = Some(inset(x, y, w, h));
        candidates[5] = Some((x, y, w, h));
    }
    candidates.into_iter().flatten().find(|(_, _, w, h)| (*w, *h) == out).map(|(x, y, _, _)| (x, y))
}

/// Where LibRaw 0.21 places the picture it gives RawTherapee, for the Fuji
/// sensors it treats by their full width. `(left, top)` in raw pixels.
///
/// The even margin is `(full - picture) / 4 * 2`. A few widths then move
/// the left edge, and the 24-megapixel bodies are set outright. The height
/// RawTherapee keeps can be two pixels under LibRaw's; those two are the
/// bottom, so the top is taken from the taller picture.
fn fuji_libraw(raw: (usize, usize)) -> Option<(usize, usize, usize, usize)> {
    let (full_w, full_h) = raw;
    let placed = |width: usize, height: usize, left: usize, top: usize| {
        (left + width <= full_w && top + height <= full_h).then_some((left, top, width, height))
    };
    let even = |full: usize, picture: usize| (full.saturating_sub(picture) >> 2) << 1;
    match full_w {
        // X-T1, X-E2, X-T10 and the same sensor. LibRaw forces the left edge
        // to 4 and keeps the height of the full mosaic.
        4992 => placed(4934, full_h, 4, 0),
        // X-T3, X-T4, X-Pro3, the 3:2 frame.
        6384 => placed(6246, 4170, 0, 6),
        // X-T5, X-H2.
        7872 => {
            let width = 7752;
            let height = 5184;
            placed(width, height, even(full_w, width), even(full_h, height))
        }
        // X-E3, X-T20, X-H1, the compressed 24-megapixel frame. LibRaw's
        // picture is 6032 x 4028; RawTherapee keeps 6032 x 4026 and the two
        // pixels are the bottom. The top was computed from the taller one.
        6048 => {
            let top = even(full_h, 4028);
            placed(6032, 4026, 0, top)
        }
        _ => None,
    }
}

/// The rectangle both cover, in raw pixels: x, y, width, height.
fn overlap(
    a: (usize, usize, usize, usize),
    b: (usize, usize, usize, usize),
) -> Option<(usize, usize, usize, usize)> {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    let x = ax.max(bx);
    let y = ay.max(by);
    let right = (ax + aw).min(bx + bw);
    let bottom = (ay + ah).min(by + bh);
    (right > x && bottom > y).then_some((x, y, right - x, bottom - y))
}

/// Where the frame sits inside a picture whose top left is `origin` in
/// raw pixels: left, top, width, height.
pub fn frame_in(
    origin: (usize, usize),
    frame: (usize, usize, usize, usize),
    out: (usize, usize),
) -> Option<(u32, u32, u32, u32)> {
    let (x, y, w, h) = frame;
    let left = x.checked_sub(origin.0)?;
    let top = y.checked_sub(origin.1)?;
    if left + w > out.0 || top + h > out.1 || w == 0 || h == 0 {
        return None;
    }
    Some((left as u32, top as u32, w as u32, h as u32))
}

/// What [`make`] leaves beside the RAF.
pub struct Made {
    pub folder: PathBuf,
    /// RawTherapee's version, when it said one.
    pub version: String,
    pub pixels: (usize, usize),
    pub steps: u8,
}

/// Develop `raf` and RawTherapee's 3-pass of it, and write the pair in a
/// folder beside the RAF. `program` is `rawtherapee-cli`.
pub fn make(raf: &Path, program: &Path, steps: u8, tell: &dyn Fn(&str)) -> Result<Made, String> {
    let steps = steps.min(5);
    let stem = raf.file_stem().ok_or("the RAF has no name")?;
    let parent = raf.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let folder = parent.join(format!("{} compare", stem.to_string_lossy()));
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;

    let work = std::env::temp_dir().join(format!("mimizan-lab-xtrans-compare-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let outcome = make_in(raf, program, steps, &folder, &work, tell);
    let _ = fs::remove_dir_all(&work);
    outcome
}

/// Where the picture sits in the raw file.
struct Geo {
    /// Left, top, width and height of the developed frame.
    frame_at: (usize, usize, usize, usize),
    raw: (usize, usize),
    crop: (usize, usize, usize, usize),
    /// Exif orientation. 1 is the file's own pixels, already upright.
    orientation: u16,
}

fn geo_of(img: &rawler::RawImage, orientation: u16) -> Result<Geo, String> {
    let frame = convert::frame_of(img)?;
    let crop = img.crop_area.map_or((0, 0, img.width, img.height), |r| (r.p.x, r.p.y, r.d.w, r.d.h));
    Ok(Geo {
        frame_at: (frame.x, frame.y, frame.width, frame.height),
        raw: (img.width, img.height),
        crop,
        orientation,
    })
}

/// The Exif orientation. The picture itself reports none: rawler leaves
/// that field at 1, and the tag travels in the metadata RawTherapee reads.
fn camera_orientation(decoder: &dyn Decoder, source: &RawSource) -> u16 {
    let params = RawDecodeParams::default();
    decoder
        .raw_metadata(source, &params)
        .ok()
        .and_then(|meta| meta.exif.orientation)
        .filter(|tag| (1..=8).contains(tag))
        .unwrap_or(1)
}

/// The DNG beside the RAF, when that file is there.
fn existing_dng(raf: &Path) -> Option<PathBuf> {
    let dng = file::beside(raf);
    dng.is_file().then_some(dng)
}

/// Put the DNG RawTherapee will develop at `dng_path`.
///
/// The decoded picture and the separated channels live only inside this
/// function. They are gone when it returns, and RawTherapee has not been
/// started yet.
fn comparison_dng(raf: &Path, dng_path: &Path, tell: &dyn Fn(&str)) -> Result<Geo, String> {
    let source = RawSource::new(raf).map_err(|e| format!("cannot be read: {e}"))?;
    let decoder =
        rawler::get_decoder(&source).map_err(|e| format!("not a raw file this program reads: {e}"))?;
    let orientation = camera_orientation(decoder.as_ref(), &source);
    if let Some(dng) = existing_dng(raf) {
        tell("using the DNG beside the RAF");
        // The head is enough: the frame is decided before any photosite is read.
        let img = decoder
            .raw_image(&source, &RawDecodeParams::default(), true)
            .map_err(|e| format!("cannot be decoded: {e}"))?;
        let geo = geo_of(&img, orientation)?;
        drop(img);
        drop(decoder);
        drop(source);
        link_or_copy(&dng, dng_path)?;
        return Ok(geo);
    }
    tell("reading the RAF");
    let img = decoder
        .raw_image(&source, &RawDecodeParams::default(), false)
        .map_err(|e| format!("cannot be decoded: {e}"))?;
    let geo = geo_of(&img, orientation)?;
    tell("separating luminance and colour");
    let dev = convert::develop_watched(&img, &Settings { keep_highlights: true }, tell, &|_| {})?;
    tell("writing the comparison DNG");
    let file = File::create(dng_path).map_err(|e| format!("{}: {e}", dng_path.display()))?;
    dng::write(&mut BufWriter::new(file), &source, decoder.as_ref(), img, dev, true, file::SOFTWARE)?;
    Ok(geo)
}

fn make_in(
    raf: &Path,
    program: &Path,
    steps: u8,
    folder: &Path,
    work: &Path,
    tell: &dyn Fn(&str),
) -> Result<Made, String> {
    let dng_path = work.join("picture.dng");
    let geo = comparison_dng(raf, &dng_path, tell)?;
    let frame_at = geo.frame_at;
    let raw = geo.raw;
    let crop = geo.crop;
    let orientation = geo.orientation;

    let pp3 = work.join("profile.pp3");
    fs::write(&pp3, profile(steps)).map_err(|e| format!("{}: {e}", pp3.display()))?;
    fs::copy(&pp3, folder.join("profile.pp3")).map_err(|e| format!("{}: {e}", folder.display()))?;

    // Away from a sidecar that may sit beside the RAF.
    let raf_copy = work.join("picture.raf");
    link_or_copy(raf, &raf_copy)?;

    let version = version_of(program);
    tell("RawTherapee, the RAF");
    let raf_tif = work.join("rawtherapee.tif");
    develop(program, &pp3, &raf_copy, &raf_tif)?;
    tell("RawTherapee, the DNG");
    let dng_tif = work.join("xtrans.tif");
    develop(program, &pp3, &dng_path, &dng_tif)?;

    tell("writing the pictures");
    let mut xtrans = read_rgb16(&dng_tif)?;
    let mut therapee = read_rgb16(&raf_tif)?;
    let got = (xtrans.width() as usize, xtrans.height() as usize);
    let frame = (frame_at.2, frame_at.3);
    // RawTherapee turns both pictures upright. The crop is measured in the
    // file's own pixels, so that turn comes off, the shared pixels are cut,
    // and the same turn goes back on. The window then matches the camera.
    let turned = rawtherapee_turned(frame, got, orientation);
    if turned {
        xtrans = turn_back(&xtrans, orientation);
        therapee = turn_back(&therapee, orientation);
    }
    if (xtrans.width() as usize, xtrans.height() as usize) != frame {
        return Err(format!(
            "RawTherapee's picture of the DNG is {} x {}, and the DNG is {} x {}",
            got.0, got.1, frame.0, frame.1
        ));
    }
    let out = (therapee.width() as usize, therapee.height() as usize);
    // A known crop lines the pixels up. Anything else is centred: a border
    // RawTherapee drops is not a reason to keep the window closed.
    let origin = rt_origin(raw, crop, BORDER, out);
    let (dx, dy, left, top, w, h, centred) = placed(frame_at, origin, out);
    if w == 0 || h == 0 {
        return Err("the two pictures could not be cut to the same pixels".into());
    }
    let mut xtrans = image::imageops::crop_imm(&xtrans, dx, dy, w, h).to_image();
    let mut therapee = image::imageops::crop_imm(&therapee, left, top, w, h).to_image();
    if therapee.dimensions() != xtrans.dimensions() {
        return Err("the two pictures came out different sizes".into());
    }
    if turned {
        xtrans = turn_upright(&xtrans, orientation);
        therapee = turn_upright(&therapee, orientation);
    }
    let pixels = (xtrans.width() as usize, xtrans.height() as usize);

    write_tiff(&folder.join(OURS), xtrans)?;
    write_tiff(&folder.join(THEIRS), therapee)?;
    let note = note(&version, steps, pixels, origin.filter(|_| !centred), turned);
    fs::write(folder.join("how.txt"), note).map_err(|e| format!("{}: {e}", folder.display()))?;

    Ok(Made { folder: folder.to_path_buf(), version, pixels, steps })
}

/// True when RawTherapee wrote the DNG turned upright.
///
/// A quarter turn swaps the sides. A half turn keeps them, and is taken
/// back anyway so the crop stays on the file's own pixels. A mirror is
/// not a turn: RawTherapee leaves those on the DNG.
fn rawtherapee_turned(frame: (usize, usize), got: (usize, usize), orientation: u16) -> bool {
    match orientation {
        6 | 8 => got == (frame.1, frame.0),
        3 => got == frame,
        _ => false,
    }
}

/// Take off the turn RawTherapee put on, back to the file's own pixels.
fn turn_back(image: &Rgb16, orientation: u16) -> Rgb16 {
    match orientation {
        3 => image::imageops::rotate180(image),
        6 => image::imageops::rotate270(image),
        8 => image::imageops::rotate90(image),
        _ => image.clone(),
    }
}

/// The camera's turn, put back on a crop that was cut in the file's pixels.
fn turn_upright(image: &Rgb16, orientation: u16) -> Rgb16 {
    match orientation {
        3 => image::imageops::rotate180(image),
        6 => image::imageops::rotate90(image),
        8 => image::imageops::rotate270(image),
        _ => image.clone(),
    }
}

/// Cuts of the two pictures: left and top in the DNG picture, left and top
/// in RawTherapee's picture, then the shared width and height. The last
/// value is true when the two were centred because no shared origin is known.
fn placed(
    frame_at: (usize, usize, usize, usize),
    origin: Option<(usize, usize)>,
    rt: (usize, usize),
) -> (u32, u32, u32, u32, u32, u32, bool) {
    if let Some(origin) = origin {
        if let Some(shared) = overlap(frame_at, (origin.0, origin.1, rt.0, rt.1)) {
            if let (Some((left, top, w, h)), Some((dx, dy, _, _))) = (
                frame_in(origin, shared, rt),
                frame_in((frame_at.0, frame_at.1), shared, (frame_at.2, frame_at.3)),
            ) {
                return (dx, dy, left, top, w, h, false);
            }
        }
    }
    let w = frame_at.2.min(rt.0);
    let h = frame_at.3.min(rt.1);
    let dx = (frame_at.2 - w) / 2;
    let dy = (frame_at.3 - h) / 2;
    let left = (rt.0 - w) / 2;
    let top = (rt.1 - h) / 2;
    (dx as u32, dy as u32, left as u32, top as u32, w as u32, h as u32, true)
}

fn link_or_copy(from: &Path, to: &Path) -> Result<(), String> {
    if fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).map(|_| ()).map_err(|e| format!("{}: {e}", to.display()))
}

fn develop(program: &Path, pp3: &Path, input: &Path, output: &Path) -> Result<(), String> {
    // -c is last. -t -b16 is 16-bit TIFF, uncompressed, so nothing is lost.
    let mut cmd = Command::new(program);
    cmd.args(["-q", "-o"]).arg(output).arg("-p").arg(pp3).args(["-t", "-b16", "-Y", "-c"]).arg(input);
    // Windows does not hand a thread's priority to the program it starts.
    // Below normal is one step under the window. Idle, RawTherapee still
    // takes the cores. On Linux the thread that starts it already stepped
    // back, and the program inherits that.
    #[cfg(windows)]
    if crate::room::yields_to_a_window() {
        use std::os::windows::process::CommandExt;
        const BELOW_NORMAL: u32 = 0x0000_4000;
        let _ = cmd.creation_flags(BELOW_NORMAL);
    }
    let status = cmd.status().map_err(|e| format!("RawTherapee could not be started: {e}"))?;
    if !status.success() {
        return Err(format!("RawTherapee stopped ({status})"));
    }
    if !output.is_file() {
        return Err("RawTherapee wrote no picture".into());
    }
    Ok(())
}

fn version_of(program: &Path) -> String {
    let output = Command::new(program).output();
    let text = output.map(|o| {
        let mut both = o.stdout;
        both.extend(o.stderr);
        String::from_utf8_lossy(&both).into_owned()
    });
    text.ok().and_then(|t| parse_version(&t)).unwrap_or_else(|| "RawTherapee".into())
}

fn parse_version(text: &str) -> Option<String> {
    let rest = text.split("version ").nth(1)?;
    let version: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    (!version.is_empty()).then_some(version)
}

fn read_rgb16(path: &Path) -> Result<Rgb16, String> {
    let mut decoder = tiff_decoder(path)?;
    let (w, h) = decoder.dimensions().map_err(|e| format!("{}: {e}", path.display()))?;
    let samples = match decoder.read_image().map_err(|e| format!("{}: {e}", path.display()))? {
        tiff::decoder::DecodingResult::U16(samples) => samples,
        _ => return Err(format!("{}: not 16-bit", path.display())),
    };
    let need = w as usize * h as usize * 3;
    if samples.len() < need {
        return Err(format!("{}: the picture is short", path.display()));
    }
    let mut samples = samples;
    samples.truncate(need);
    Rgb16::from_raw(w, h, samples).ok_or_else(|| format!("{}: the picture could not be read", path.display()))
}

/// 16-bit RGB, uncompressed, in strips of [`ROWS_PER_STRIP`]. The same
/// writer for both, so the files match. A strip is the unit a window reads.
fn write_tiff(path: &Path, image: Rgb16) -> Result<(), String> {
    write_tiff_rows(path, &image, ROWS_PER_STRIP)
}

fn write_tiff_rows(path: &Path, image: &Rgb16, rows: u32) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = tiff::encoder::TiffEncoder::new(BufWriter::new(file))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = encoder
        .new_image::<tiff::encoder::colortype::RGB16>(image.width(), image.height())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    out.rows_per_strip(rows.max(1)).map_err(|e| format!("{}: {e}", path.display()))?;
    out.write_data(image.as_raw()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Width and height of a 16-bit RGB TIFF, from its header.
pub fn tiff_size(path: &Path) -> Result<(usize, usize), String> {
    let mut decoder = tiff_decoder(path)?;
    let (w, h) = decoder.dimensions().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((w as usize, h as usize))
}

/// The high byte of a window of a 16-bit RGB TIFF, sampled to `out`.
///
/// `origin` is the window's top left and `span` its width and height, in
/// pixels of the file. `out` is how many pixels to return, width then
/// height. Only the strips the window touches are read. The bytes are
/// red, green, blue, row by row.
pub fn screen_window(
    path: &Path,
    origin: (u32, u32),
    span: (u32, u32),
    out: (usize, usize),
) -> Result<Vec<u8>, String> {
    if span.0 == 0 || span.1 == 0 || out.0 == 0 || out.1 == 0 || out.0 > 8192 || out.1 > 8192 {
        return Err(format!("{}: the view is empty", path.display()));
    }
    let mut decoder = tiff_decoder(path)?;
    let (width, height) = decoder.dimensions().map_err(|e| format!("{}: {e}", path.display()))?;
    if origin.0.saturating_add(span.0) > width || origin.1.saturating_add(span.1) > height {
        return Err(format!("{}: the view lies outside the picture", path.display()));
    }
    let (_, rows) = decoder.chunk_dimensions();
    if decoder.get_chunk_type() != tiff::decoder::ChunkType::Strip || rows == 0 {
        return Err(format!("{}: the picture is not stored in strips", path.display()));
    }
    let mut pixels = vec![0_u8; out.0 * out.1 * 3];
    let mut held: Option<(u32, Vec<u16>, usize, usize)> = None;
    for j in 0..out.1 {
        let sy = origin.1 + at(j, out.1, span.1);
        let strip = sy / rows;
        if held.as_ref().is_none_or(|(index, ..)| *index != strip) {
            let (dw, dh) = decoder.chunk_data_dimensions(strip);
            let samples = match decoder.read_chunk(strip).map_err(|e| format!("{}: {e}", path.display()))? {
                tiff::decoder::DecodingResult::U16(samples) => samples,
                _ => return Err(format!("{}: not 16-bit", path.display())),
            };
            if samples.len() < dw as usize * dh as usize * 3 {
                return Err(format!("{}: a strip is short", path.display()));
            }
            held = Some((strip, samples, dw as usize, dh as usize));
        }
        let (_, samples, dw, dh) = held.as_ref().expect("the strip was just read");
        let row = (sy % rows) as usize;
        if row >= *dh {
            return Err(format!("{}: a strip is short", path.display()));
        }
        for i in 0..out.0 {
            let sx = (origin.0 + at(i, out.0, span.0)) as usize;
            let from = (row * *dw + sx) * 3;
            if sx >= *dw || from + 2 >= samples.len() {
                return Err(format!("{}: a strip is short", path.display()));
            }
            let into = (j * out.0 + i) * 3;
            pixels[into] = (samples[from] >> 8) as u8;
            pixels[into + 1] = (samples[from + 1] >> 8) as u8;
            pixels[into + 2] = (samples[from + 2] >> 8) as u8;
        }
    }
    Ok(pixels)
}

/// The source pixel at the centre of output pixel `index`.
fn at(index: usize, out: usize, span: u32) -> u32 {
    let span = u64::from(span);
    let out = out.max(1) as u64;
    let n = (index as u64 * span * 2 + span) / (out * 2);
    u32::try_from(n.min(span - 1)).unwrap_or(0)
}

fn tiff_decoder(path: &Path) -> Result<tiff::decoder::Decoder<BufReader<File>>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = tiff::decoder::Decoder::new(BufReader::new(file))
        .map_err(|e| format!("{}: {e}", path.display()))?
        .with_limits(tiff::decoder::Limits::unlimited());
    match decoder.colortype().map_err(|e| format!("{}: {e}", path.display()))? {
        tiff::ColorType::RGB(16) => Ok(decoder),
        _ => Err(format!("{}: not a 16-bit RGB TIFF", path.display())),
    }
}

fn note(
    version: &str,
    steps: u8,
    size: (usize, usize),
    origin: Option<(usize, usize)>,
    upright: bool,
) -> String {
    let suppression = if steps == 0 { "off (0)".to_owned() } else { steps.to_string() };
    let (w, h) = size;
    let place = match origin {
        Some(origin) => format!(
            "{OURS} is this program. {THEIRS} is RawTherapee's 3-pass, cropped to the shared pixels,\n\
             {w} x {h}. RawTherapee's picture began at raw pixel {},{}.",
            origin.0, origin.1
        ),
        None => format!(
            "{OURS} is this program. {THEIRS} is RawTherapee's 3-pass. The two crops do not share\n\
             a recorded origin, so both pictures were centred. The part written is {w} x {h}."
        ),
    };
    let held =
        if upright { "Both pictures were turned upright, the way the camera held them.\n\n" } else { "" };
    format!(
        "\
Both pictures are 16-bit TIFF, uncompressed. They were developed by RawTherapee {version}
with the one profile in profile.pp3. Camera white balance, no tone curve, contrast 0,
no sharpening, no noise reduction, no highlight recovery, sRGB. Highlights were left
as the sensor recorded them, on both.

The only difference is the demosaic. RawTherapee used Markesteijn 3-pass with false
colour suppression {suppression}. Mimizan Lab X-Trans had already separated the mosaic;
RawTherapee did not demosaic that picture again.

{place}

{held}{}.
",
        file::SOFTWARE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_one_look_and_only_the_suppression_changes() {
        let off = profile(0);
        let on = profile(3);
        assert!(off.contains("Method=3-pass (best)"));
        assert!(off.contains("Border=7"));
        assert!(off.contains("ShadowCompr=0"));
        assert!(off.contains("Contrast=0"));
        assert!(off.contains("Setting=Camera"));
        assert!(off.contains("Curve=0;"));
        assert!(off.contains("[Sharpening]\nEnabled=false"));
        assert!(off.contains("CcSteps=0"));
        assert!(on.contains("CcSteps=3"));
        assert!(!on.contains("CcSteps=0"));
        let without = |text: &str| text.replace("CcSteps=0", "").replace("CcSteps=3", "");
        assert_eq!(without(&off), without(&on));
    }

    #[test]
    fn off_means_no_suppression_whatever_the_stored_steps() {
        assert_eq!(Suppression { on: false, steps: 4 }.steps(), 0);
        assert_eq!(Suppression { on: true, steps: 9 }.steps(), 5);
        assert_eq!(Suppression { on: true, steps: 0 }.steps(), 0);
    }

    #[test]
    fn the_origin_is_the_crop_inset_by_the_border() {
        // An X-E3: RawTherapee's crop is (0, 5, 6032, 4026), and with
        // seven pixels off each edge the picture is 6018 x 4012.
        let origin = rt_origin((6160, 4032), (0, 5, 6032, 4026), 7, (6018, 4012));
        assert_eq!(origin, Some((7, 12)));
        // An X-T1. LibRaw's picture is 4934 x 3296 at (4, 0); RawTherapee
        // drops seven pixels and the picture is 4920 x 3282 at (11, 7).
        assert_eq!(rt_origin((4992, 3296), (24, 16, 4896, 3264), 7, (4920, 3282)), Some((11, 7)));
        assert_eq!(rt_origin((6384, 4182), (0, 0, 1, 1), 7, (6232, 4156)), Some((7, 13)));
        assert_eq!(rt_origin((7872, 5196), (0, 0, 1, 1), 7, (7738, 5170)), Some((67, 13)));
        assert_eq!(rt_origin((6048, 4038), (0, 0, 1, 1), 7, (6018, 4012)), Some((7, 11)));
        let frame = frame_in((7, 12), (8, 14, 5998, 3997), (6018, 4012));
        assert_eq!(frame, Some((1, 2, 5998, 3997)));
        assert!(rt_origin((100, 80), (0, 0, 100, 80), 7, (10, 10)).is_none());
        assert!(frame_in((7, 12), (0, 0, 10, 10), (6018, 4012)).is_none());
    }

    /// `XTRANS_SAMPLES` is a directory of RAFs. The test checks that each
    /// named file's frame sits inside the picture RawTherapee 5.11 writes.
    #[test]
    fn the_named_samples_lie_inside_rawtherapees_picture() {
        let Ok(dir) = std::env::var("XTRANS_SAMPLES") else { return };
        let dir = PathBuf::from(dir);
        let samples = [
            ("Fujifilm - X-T1 - 14bit 14bit uncompressed (3_2).RAF", (4920, 3282)),
            ("Fujifilm - X-T3 - 14bit 14bit uncompressed (3_2).RAF", (6232, 4156)),
            ("Fujifilm - X-T4 - 14bit 14bit uncompressed (3_2).RAF", (6232, 4156)),
            ("Fujifilm - X-T5 - 14bit 14bit uncompressed (3_2).RAF", (7738, 5170)),
            ("DSCF3861.RAF", (6018, 4012)),
        ];
        for (name, out) in samples {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            let source = rawler::rawsource::RawSource::new(&path).unwrap();
            let decoder = rawler::get_decoder(&source).unwrap();
            let img =
                decoder.raw_image(&source, &rawler::decoders::RawDecodeParams::default(), true).unwrap();
            let frame = convert::frame_of(&img).unwrap();
            let raw = (img.width, img.height);
            let crop = img.crop_area.map_or((0, 0, img.width, img.height), |r| (r.p.x, r.p.y, r.d.w, r.d.h));
            let at = (frame.x, frame.y, frame.width, frame.height);
            let origin = rt_origin(raw, crop, BORDER, out)
                .unwrap_or_else(|| panic!("{name}: no origin, raw {raw:?}, crop {crop:?}, frame {at:?}"));
            let shared = overlap(at, (origin.0, origin.1, out.0, out.1))
                .unwrap_or_else(|| panic!("{name}: frame {at:?} misses {out:?} at {origin:?}"));
            assert!(
                at.2 - shared.2 <= 64 && at.3 - shared.3 <= 64,
                "{name}: shared {shared:?}, frame {at:?}"
            );
        }
    }

    #[test]
    fn the_exif_orientation_is_read_from_the_metadata() {
        let Ok(path) = std::env::var("XTRANS_ONE") else { return };
        let source = RawSource::new(Path::new(&path)).unwrap();
        let decoder = rawler::get_decoder(&source).unwrap();
        let img = decoder.raw_image(&source, &RawDecodeParams::default(), true).unwrap();
        let tag = camera_orientation(decoder.as_ref(), &source);
        assert_eq!(img.orientation.to_u16(), 1, "the picture field stays at 1");
        assert!((1..=8).contains(&tag), "{tag}");
        let frame = convert::frame_of(&img).unwrap();
        let turned = rawtherapee_turned((frame.width, frame.height), (frame.height, frame.width), tag);
        assert_eq!(turned, matches!(tag, 6 | 8));
    }

    #[test]
    fn a_turned_picture_comes_back_to_the_same_pixels() {
        // 7726 x 5149 held upright is what RawTherapee wrote as 5149 x 7726.
        assert!(rawtherapee_turned((7726, 5149), (5149, 7726), 6));
        assert!(rawtherapee_turned((7726, 5149), (5149, 7726), 8));
        assert!(rawtherapee_turned((7726, 5149), (7726, 5149), 3));
        assert!(!rawtherapee_turned((7726, 5149), (5149, 7726), 1));
        assert!(!rawtherapee_turned((7726, 5149), (7726, 5149), 6));

        let mut sensor = Rgb16::new(2, 3);
        for (i, (x, y)) in [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)].into_iter().enumerate() {
            let v = (i as u16 + 1) << 8;
            sensor.put_pixel(x, y, Rgb([v, v, v]));
        }
        for orientation in [3_u16, 6, 8] {
            let upright = turn_upright(&sensor, orientation);
            let back = turn_back(&upright, orientation);
            assert_eq!(back.dimensions(), sensor.dimensions(), "{orientation}");
            assert_eq!(back.as_raw(), sensor.as_raw(), "{orientation}");
            let again = turn_upright(&back, orientation);
            assert_eq!(again.as_raw(), upright.as_raw(), "{orientation}");
        }
    }

    #[test]
    fn pictures_that_do_not_line_up_are_centred() {
        // An X-Pro1: RawTherapee writes 4938 x 3274 from a 5120 x 3288 mosaic,
        // which has no recorded origin. The smaller picture is kept whole.
        let (dx, dy, left, top, w, h, centred) = placed((0, 0, 4896, 3264), None, (4938, 3274));
        assert!(centred);
        assert_eq!((w, h), (4896, 3264));
        assert_eq!((dx, dy), (0, 0));
        assert_eq!((left, top), (21, 5));
        // A known origin still lines the pixels up instead of centring.
        let lined = placed((8, 14, 100, 80), Some((7, 12)), (200, 150));
        assert_eq!(lined, (0, 0, 1, 2, 100, 80, false));
    }

    #[test]
    fn a_dng_beside_the_raf_is_the_one_used() {
        let dir = std::env::temp_dir().join(format!("xtrans-dng-beside-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let raf = dir.join("a.RAF");
        fs::write(&raf, b"x").unwrap();
        assert!(existing_dng(&raf).is_none());
        let dng = dir.join("a.dng");
        fs::write(&dng, b"dng").unwrap();
        assert_eq!(existing_dng(&raf).as_deref(), Some(dng.as_path()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_window_is_read_from_the_strips_it_lies_on() {
        let path = std::env::temp_dir().join(format!("xtrans-strips-{}.tif", std::process::id()));
        let (w, h) = (6_u32, 10_u32);
        let mut image = Rgb16::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x as u16) << 8) | (y as u16);
                image.put_pixel(x, y, Rgb([v, v, v]));
            }
        }
        write_tiff_rows(&path, &image, 4).unwrap();
        assert_eq!(tiff_size(&path).unwrap(), (6, 10));
        let whole = read_rgb16(&path).unwrap();
        assert_eq!(whole.get_pixel(2, 5)[0], (2_u16 << 8) | 5);
        // Rows 3..=7 cross three strips of four rows.
        let got = screen_window(&path, (1, 3), (2, 5), (2, 5)).unwrap();
        assert_eq!(got.len(), 2 * 5 * 3);
        for row in 0..5 {
            for col in 0..2 {
                let i = (row * 2 + col) * 3;
                assert_eq!(got[i], (1 + col) as u8, "row {row}");
                assert_eq!(&got[i..i + 3], &[got[i]; 3]);
            }
        }
        // Centres of a 2 by 2 view of the first 4 by 4 pixels are x = 1 and 3.
        let small = screen_window(&path, (0, 0), (4, 4), (2, 2)).unwrap();
        assert_eq!(small[0], 1);
        assert_eq!(small[3], 3);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_version_line_is_read() {
        assert_eq!(parse_version("RawTherapee, version 5.11, command line.\n").as_deref(), Some("5.11"));
        assert!(parse_version("no version here").is_none());
    }
}

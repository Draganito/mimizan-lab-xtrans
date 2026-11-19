//! Reading for the window, off its thread: what each file says about
//! itself, its small picture for the list, and the two large pictures of
//! the file that is looked at, the camera's preview and the DNG. One
//! thread does it all, the large pictures first: they are what is waited
//! for.

use crate::queue::Id;
use crate::view::{self, Shown};
use egui::ColorImage;
use image::ImageDecoder;
use mimizan_lab_xtrans::convert;
use rawler::decoders::RawDecodeParams;
use rawler::rawimage::RawPhotometricInterpretation;
use rawler::rawsource::RawSource;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

/// Longest side of the small picture in the list, pixels.
const THUMB: u32 = 192;

/// What a RAF says about itself.
#[derive(Clone, Debug)]
pub struct Info {
    pub make: String,
    pub model: String,
    /// "ISO 160 · 1/250 s · f/5.6 · 23 mm", as far as the file tells.
    pub exposure: String,
    pub iso: Option<u32>,
    /// Exposure correction in stops, if it is not none.
    pub correction: Option<f32>,
    pub lens: Option<String>,
    pub taken: Option<String>,
    /// Width and height of the mosaic.
    pub sensor: (usize, usize),
    pub bits: usize,
    /// The mosaic's tile, as rawler names it.
    pub pattern: String,
    /// Width and height of the picture a DNG will hold, or why there
    /// will be none.
    pub picture: Result<(usize, usize), String>,
    /// What red and blue are multiplied by to make grey grey, from the
    /// camera.
    pub balance: Option<(f32, f32)>,
    /// Width and height of the camera's preview in the file, upright.
    pub preview: Option<(usize, usize)>,
}

/// One of the two large pictures.
#[derive(Clone, Debug)]
pub enum Large {
    /// The camera's JPEG in the RAF.
    Preview(PathBuf),
    /// The DNG at this path.
    Developed(PathBuf),
}

/// What the thread sends back.
pub enum Loaded {
    Info { id: Id, info: Result<Box<Info>, String>, thumb: Option<ColorImage> },
    Preview { turn: u64, image: Result<ColorImage, String> },
    Developed { turn: u64, shown: Result<Shown, String> },
}

#[derive(Default)]
struct Wanted {
    /// Which file is looked at: counts up with every change, so what
    /// comes back for an earlier one is known to be late.
    turn: u64,
    large: VecDeque<Large>,
    infos: VecDeque<(Id, PathBuf)>,
}

struct Shared {
    wanted: Mutex<Wanted>,
    wake: Condvar,
    /// The longest side the graphics card takes for a picture.
    max_side: AtomicUsize,
}

pub struct Loader {
    shared: Arc<Shared>,
    pub loaded: Receiver<Loaded>,
}

enum Task {
    Large(u64, Large),
    Info(Id, PathBuf),
}

impl Loader {
    pub fn spawn(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared {
            wanted: Mutex::default(),
            wake: Condvar::new(),
            max_side: AtomicUsize::new(2048),
        });
        let (send, loaded) = mpsc::channel();
        let theirs = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("load".into())
            .spawn(move || run(&theirs, &send, &ctx))
            .expect("the loading thread starts");
        Self { shared, loaded }
    }

    fn wanted(&self) -> std::sync::MutexGuard<'_, Wanted> {
        self.shared.wanted.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Read what `raf` says about itself, after those asked for before.
    pub fn info(&self, id: Id, raf: PathBuf) {
        self.wanted().infos.push_back((id, raf));
        self.shared.wake.notify_one();
    }

    /// Another file is looked at: load `large` for it, in this order, and
    /// drop what was wanted for the one before. Gives the turn the
    /// answers will carry.
    pub fn look_at(&self, large: Vec<Large>) -> u64 {
        let mut wanted = self.wanted();
        wanted.turn += 1;
        wanted.large = large.into();
        self.shared.wake.notify_one();
        wanted.turn
    }

    /// One more large picture for the file that is looked at.
    pub fn also(&self, large: Large) {
        self.wanted().large.push_back(large);
        self.shared.wake.notify_one();
    }

    pub fn set_max_side(&self, side: usize) {
        self.shared.max_side.store(side, Ordering::Relaxed);
    }
}

fn run(shared: &Shared, send: &Sender<Loaded>, ctx: &egui::Context) {
    // How light the preview of the file looked at is: its DNG is shown
    // as light.
    let mut lightness: Option<(u64, f32)> = None;
    loop {
        let task = {
            let mut wanted = shared.wanted.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if let Some(large) = wanted.large.pop_front() {
                    break Task::Large(wanted.turn, large);
                }
                if let Some((id, raf)) = wanted.infos.pop_front() {
                    break Task::Info(id, raf);
                }
                wanted = shared.wake.wait(wanted).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let max_side = shared.max_side.load(Ordering::Relaxed);
        let loaded = match task {
            Task::Info(id, raf) => {
                let picture = preview(&raf).ok();
                let size = picture.as_ref().map(|p| (p.width() as usize, p.height() as usize));
                let info = info(&raf).map(|mut info| {
                    info.preview = size;
                    Box::new(info)
                });
                let thumb = picture.map(|p| colour_image(&p.thumbnail(THUMB, THUMB)));
                Loaded::Info { id, info, thumb }
            }
            Task::Large(turn, Large::Preview(raf)) => {
                let image = preview(&raf).map(|p| colour_image(&p));
                if let Ok(image) = &image {
                    lightness = Some((turn, view::lightness_of(image)));
                }
                Loaded::Preview { turn, image: image.map(|i| view::shrink(i, max_side)) }
            }
            Task::Large(turn, Large::Developed(dng)) => {
                let like = lightness.filter(|(of, _)| *of == turn).map(|(_, l)| l);
                Loaded::Developed { turn, shown: developed(&dng, like, max_side) }
            }
        };
        if send.send(loaded).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn colour_image(picture: &image::DynamicImage) -> ColorImage {
    let rgb = picture.to_rgb8();
    ColorImage::from_rgb([rgb.width() as usize, rgb.height() as usize], rgb.as_raw())
}

/// The JPEG the camera put into `raf`, as it lies in the file. A RAF
/// begins with a head of fixed places: the JPEG's offset and length are
/// at bytes 84 and 88, big-endian.
fn embedded_jpeg(raf: &Path) -> Result<Vec<u8>, String> {
    let fail = |e: std::io::Error| format!("cannot be read: {e}");
    let mut file = File::open(raf).map_err(fail)?;
    let mut head = [0u8; 92];
    // A file too short for the head is no RAF either.
    match file.read_exact(&mut head) {
        Err(e) if e.kind() != std::io::ErrorKind::UnexpectedEof => return Err(fail(e)),
        Ok(()) if head.starts_with(b"FUJIFILMCCD-RAW") => {}
        _ => return Err("not a RAF".into()),
    }
    let at = |i: usize| u64::from(u32::from_be_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]));
    let (offset, length) = (at(84), at(88));
    if length == 0 || offset + length > file.metadata().map_err(fail)?.len() {
        return Err("the file holds no preview".into());
    }
    let mut jpeg = vec![0u8; length as usize];
    file.seek(SeekFrom::Start(offset)).map_err(fail)?;
    file.read_exact(&mut jpeg).map_err(fail)?;
    Ok(jpeg)
}

fn jpeg_decoder(jpeg: Vec<u8>) -> Result<impl ImageDecoder, String> {
    image::ImageReader::with_format(Cursor::new(jpeg), image::ImageFormat::Jpeg)
        .into_decoder()
        .map_err(|e| format!("the preview cannot be read: {e}"))
}

/// The camera's preview in `raf`, upright.
fn preview(raf: &Path) -> Result<image::DynamicImage, String> {
    let mut decoder = jpeg_decoder(embedded_jpeg(raf)?)?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture =
        image::DynamicImage::from_decoder(decoder).map_err(|e| format!("the preview cannot be read: {e}"))?;
    picture.apply_orientation(orientation);
    Ok(picture)
}

/// The DNG at `path` for the screen.
fn developed(path: &Path, lightness: Option<f32>, max_side: usize) -> Result<Shown, String> {
    let source = RawSource::new(path).map_err(|e| format!("the DNG cannot be read: {e}"))?;
    let decoder = rawler::get_decoder(&source).map_err(|e| format!("the DNG cannot be read: {e}"))?;
    let img = decoder
        .raw_image(&source, &RawDecodeParams::default(), false)
        .map_err(|e| format!("the DNG cannot be decoded: {e}"))?;
    view::develop(&img, lightness, max_side)
}

/// A shutter time as it is said: 1/250 s, 0.5 s, 30 s.
fn time(seconds: f32) -> String {
    if seconds < 0.4 {
        format!("1/{:.0} s", 1.0 / seconds)
    } else if seconds < 10.0 && (seconds - seconds.round()).abs() > 0.05 {
        format!("{seconds:.1} s")
    } else {
        format!("{seconds:.0} s")
    }
}

/// A number with one decimal, and none where it is 0: 5.6, 8, 23.
fn short(v: f32) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_owned)
}

/// What `raf` says about itself: its head is read, not its mosaic.
fn info(raf: &Path) -> Result<Info, String> {
    let source = RawSource::new(raf).map_err(|e| format!("cannot be read: {e}"))?;
    let decoder =
        rawler::get_decoder(&source).map_err(|e| format!("not a raw file this program reads: {e}"))?;
    let params = RawDecodeParams::default();
    let img = decoder.raw_image(&source, &params, true).map_err(|e| format!("cannot be decoded: {e}"))?;
    let meta = decoder.raw_metadata(&source, &params).ok();
    let exif = meta.as_ref().map(|m| &m.exif);

    let positive = |r: Option<rawler::formats::tiff::Rational>| {
        r.filter(|r| r.d != 0).map(|r| r.n as f32 / r.d as f32).filter(|v| v.is_finite() && *v > 0.0)
    };
    let iso = exif
        .and_then(|e| e.iso_speed_ratings.map(u32::from).or(e.iso_speed).or(e.recommended_exposure_index))
        .filter(|v| *v > 0);
    let parts = [
        iso.map(|v| format!("ISO {v}")),
        positive(exif.and_then(|e| e.exposure_time)).map(time),
        positive(exif.and_then(|e| e.fnumber)).map(|v| format!("f/{}", short(v))),
        positive(exif.and_then(|e| e.focal_length)).map(|v| format!("{} mm", short(v))),
    ];
    let correction =
        exif.and_then(|e| e.exposure_bias).filter(|r| r.d != 0 && r.n != 0).map(|r| r.n as f32 / r.d as f32);
    let text = |s: Option<&String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let lens = meta
        .as_ref()
        .and_then(|m| m.lens.as_ref())
        .and_then(|l| text(Some(&l.lens_name)))
        .or_else(|| text(exif.and_then(|e| e.lens_model.as_ref())));
    // "2020:05:12 14:33:10" reads better with the date as a date.
    let taken = text(exif.and_then(|e| e.date_time_original.as_ref())).map(|t| match t.split_once(' ') {
        Some((date, clock)) => format!("{} {clock}", date.replace(':', "-")),
        None => t,
    });
    let pattern = match &img.photometric {
        RawPhotometricInterpretation::Cfa(config) => config.cfa.name.clone(),
        _ => String::new(),
    };
    let wb = img.wb_coeffs;
    let balance = (wb[..3].iter().all(|v| v.is_finite() && *v > 0.0)).then(|| (wb[0] / wb[1], wb[2] / wb[1]));
    Ok(Info {
        make: img.clean_make.clone(),
        model: img.clean_model.clone(),
        exposure: parts.into_iter().flatten().collect::<Vec<_>>().join(" · "),
        iso,
        correction,
        lens,
        taken,
        sensor: (img.width, img.height),
        // What the sensor's values reach, not what they are stored in.
        bits: img.whitelevel.0.iter().max().map_or(img.bps, |w| (u32::BITS - w.leading_zeros()) as usize),
        pattern,
        picture: convert::frame_of(&img).map(|f| (f.width, f.height)),
        balance,
        preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{embedded_jpeg, short, time};
    use crate::view;
    use egui::{Color32, ColorImage};
    use std::fs;

    #[test]
    fn exposures_read_as_they_are_said() {
        assert_eq!(time(1.0 / 250.0), "1/250 s");
        assert_eq!(time(1.0 / 3.0), "1/3 s");
        assert_eq!(time(0.5), "0.5 s");
        assert_eq!(time(2.0), "2 s");
        assert_eq!(time(2.5), "2.5 s");
        assert_eq!(time(30.0), "30 s");
        assert_eq!(
            (short(5.6), short(8.0), short(23.0), short(1.4)),
            ("5.6".into(), "8".into(), "23".into(), "1.4".into())
        );
    }

    #[test]
    fn the_preview_is_found_by_the_head_of_the_raf() {
        let path = std::env::temp_dir().join(format!("xtrans-gui-head-{}.raf", std::process::id()));
        let mut raf = vec![0u8; 200];
        raf[..15].copy_from_slice(b"FUJIFILMCCD-RAW");
        raf[84..88].copy_from_slice(&150u32.to_be_bytes());
        raf[88..92].copy_from_slice(&4u32.to_be_bytes());
        raf[150..154].copy_from_slice(b"JPEG");
        fs::write(&path, &raf).unwrap();
        assert_eq!(embedded_jpeg(&path).unwrap(), b"JPEG");

        // A preview that would run past the end, none at all, and no RAF.
        raf[88..92].copy_from_slice(&51u32.to_be_bytes());
        fs::write(&path, &raf).unwrap();
        assert!(embedded_jpeg(&path).is_err());
        raf[88..92].copy_from_slice(&0u32.to_be_bytes());
        fs::write(&path, &raf).unwrap();
        assert!(embedded_jpeg(&path).is_err());
        fs::write(&path, vec![0u8; 200]).unwrap();
        assert_eq!(embedded_jpeg(&path).unwrap_err(), "not a RAF");
        fs::write(&path, b"FUJIFILMCCD-RAW").unwrap();
        assert_eq!(embedded_jpeg(&path).unwrap_err(), "not a RAF");
        let _ = fs::remove_file(path);
    }

    /// The DNG is turned by this program, the preview by the `image`
    /// crate: both must mean the same by an orientation.
    #[test]
    fn a_dng_is_turned_as_the_preview_is() {
        let pixels: Vec<Color32> = (0..6).map(|v| Color32::from_gray(v * 10)).collect();
        let raw: Vec<u8> = pixels.iter().flat_map(|p| [p.r(), p.g(), p.b()]).collect();
        for orientation in 1..=8u8 {
            let mut theirs =
                image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(3, 2, raw.clone()).unwrap());
            theirs.apply_orientation(image::metadata::Orientation::from_exif(orientation).unwrap());
            let ours = view::turn(ColorImage::new([3, 2], pixels.clone()), u16::from(orientation));
            assert_eq!(ours.size, [theirs.width() as usize, theirs.height() as usize], "{orientation}");
            let theirs: Vec<u8> = theirs.to_rgb8().pixels().map(|p| p.0[0]).collect();
            assert_eq!(ours.pixels.iter().map(|p| p.r()).collect::<Vec<_>>(), theirs, "{orientation}");
        }
    }
}

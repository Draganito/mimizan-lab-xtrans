//! The Kodak bench, the same way Mimizan Lab measures its plate.
//!
//! Twenty-four 8-bit sRGB pictures are linearised, optionally blurred or
//! enlarged, then sampled through the X-Trans tile and quantised to 16 bit.
//! That one mosaic goes to this program and, as a DNG, to RawTherapee.
//! The truth is the linear picture. CPSNR and the PSNR of each channel are
//! on sRGB-encoded values, 16 px of the edge left out.
//!
//! The balance is 1, 1, 1: the picture is already the colour, and a grey
//! world would be a second processing step. The separation runs under the
//! sRGB curve and comes back, which is the way a camera file is developed.
//! Nothing is treated as burnt out: a Kodak white is the picture's white,
//! not a photosite that reached the sensor's ceiling.
//!
//! RawTherapee 5.11 is asked for Markesteijn's 1-pass (medium), the 3-pass,
//! and the 3-pass with false-colour suppression 3. Those are the X-Trans
//! methods it has. A same-colour average is the floor, computed here.

use mimizan_lab_xtrans::convert::{self, frame};
use mimizan_lab_xtrans::filter::{gaussian_kernel, lowpass};
use mimizan_lab_xtrans::plane::{Plane, Rgb};
use mimizan_lab_xtrans::xtrans::{self, Color, Curve, Prior, Refine, Selection, XParams, XTile, FUJI};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const BORDER: usize = 16;
const XYZ_TO_SRGB: [f64; 9] = [3.2406, -1.5372, -0.4986, -0.9689, 1.8758, 0.0415, 0.0557, -0.2040, 1.0570];

struct Args {
    images: PathBuf,
    out: PathBuf,
    gate: bool,
    shift: bool,
    only: Option<String>,
    condition: Option<String>,
}

fn args() -> Args {
    let mut a = Args {
        images: PathBuf::from("testdata/kodak"),
        out: PathBuf::from("out/kodak"),
        gate: false,
        shift: false,
        only: None,
        condition: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--images" => a.images = PathBuf::from(it.next().expect("--images needs a directory")),
            "--out" => a.out = PathBuf::from(it.next().expect("--out needs a directory")),
            "--only" => a.only = Some(it.next().expect("--only needs a stem")),
            "--condition" => a.condition = Some(it.next().expect("--condition needs pixel, lens or x2")),
            "--gate" => a.gate = true,
            "--shift" => a.shift = true,
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

fn main() {
    let a = args();
    fs::create_dir_all(&a.out).unwrap();
    let tile = XTile::from_name(FUJI).expect("the Fuji tile");
    let gate = gate_of(&a.out, &tile);
    println!("{gate}");
    if gate.contains("FAIL") {
        std::process::exit(1);
    }
    if a.gate {
        return;
    }
    if a.shift {
        shift_check(&a, &tile);
        return;
    }

    let mut names: Vec<String> = fs::read_dir(&a.images)
        .unwrap_or_else(|e| panic!("{}: {e}", a.images.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    if let Some(only) = &a.only {
        names.retain(|n| n == only);
    }
    let conditions = match a.condition.as_deref() {
        None => vec![Condition::Pixel, Condition::Lens, Condition::Twice],
        Some("pixel") => vec![Condition::Pixel],
        Some("lens") => vec![Condition::Lens],
        Some("x2") => vec![Condition::Twice],
        Some(other) => panic!("condition is pixel, lens or x2, not {other}"),
    };

    let mut report = String::new();
    for c in conditions {
        let rows = run_condition(&a, &tile, &names, c);
        report.push_str(&table(c, &rows));
        report.push('\n');
        println!("{}", table(c, &rows));
    }
    fs::write(a.out.join("report.md"), &report).unwrap();
}

#[derive(Clone, Copy)]
enum Condition {
    Pixel,
    Lens,
    Twice,
}

impl Condition {
    fn name(self) -> &'static str {
        match self {
            Condition::Pixel => "pixel-sharp",
            Condition::Lens => "behind a lens (σ 0.75)",
            Condition::Twice => "twice the size",
        }
    }
    fn dir(self) -> &'static str {
        match self {
            Condition::Pixel => "pixel",
            Condition::Lens => "lens",
            Condition::Twice => "x2",
        }
    }
}

#[derive(Clone)]
struct Scores {
    name: String,
    cpsnr: f64,
    r: f64,
    g: f64,
    b: f64,
    leads: usize,
}

fn run_condition(a: &Args, tile: &XTile, names: &[String], c: Condition) -> Vec<Scores> {
    let dir = a.out.join(c.dir());
    fs::create_dir_all(&dir).unwrap();
    let methods = ["floor", "1-pass (medium)", "3-pass", "3-pass, suppression 3", "Mimizan Lab X-Trans"];
    let mut per: Vec<Vec<Colour>> = vec![Vec::new(); methods.len()];
    for name in names {
        eprint!("{name} {} … ", c.dir());
        let path = a.images.join(format!("{name}.png"));
        let mut truth = load_srgb(&path);
        if matches!(c, Condition::Twice) {
            truth = resize2(&truth);
        }
        if matches!(c, Condition::Lens) {
            truth = blur(&truth, 0.75);
        }
        let (w, h) = (truth.width(), truth.height());
        let samples = mosaic(&truth, tile);
        let placed = frame(tile, (0, 0, w, h)).expect("a frame the tile mirrors on");
        let ours = separate(&samples, w, &placed);
        let dng = dir.join(format!("{name}.dng"));
        write_dng(&dng, w, h, &samples);
        let rt = [
            ("1-pass", "1-pass (medium)", 0),
            ("3-pass", "3-pass (best)", 0),
            ("3-pass-s3", "3-pass (best)", 3),
        ];
        let mut colours = Vec::new();
        colours.push(colour_of(&floor(&samples, w, h, tile, &placed), &crop_truth(&truth, &placed)));
        for (stem, method, steps) in rt {
            let tif = dir.join(format!("{name}.{stem}.tif"));
            develop_rt(&dng, &tif, method, steps);
            let got = read_rt(&tif);
            let aligned = align_rt(&got, (w, h), &placed);
            colours.push(colour_of(&aligned, &crop_truth(&truth, &placed)));
        }
        colours.push(colour_of(&ours, &crop_truth(&truth, &placed)));
        let line: Vec<String> = colours.iter().map(|q| format!("{:.2}", q.cpsnr)).collect();
        eprintln!("{}", line.join("  "));
        for (i, q) in colours.into_iter().enumerate() {
            per[i].push(q);
        }
    }
    let n = names.len();
    let mut leads = vec![0usize; methods.len()];
    for i in 0..n {
        let mut best = 0;
        for k in 1..methods.len() {
            if per[k][i].cpsnr > per[best][i].cpsnr {
                best = k;
            }
        }
        leads[best] += 1;
    }
    methods
        .iter()
        .enumerate()
        .map(|(k, name)| Scores {
            name: (*name).to_string(),
            cpsnr: mean(per[k].iter().map(|q| q.cpsnr)),
            r: mean(per[k].iter().map(|q| q.r)),
            g: mean(per[k].iter().map(|q| q.g)),
            b: mean(per[k].iter().map(|q| q.b)),
            leads: leads[k],
        })
        .collect()
}

fn table(c: Condition, rows: &[Scores]) -> String {
    let n = rows.iter().map(|r| r.leads).sum::<usize>();
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| b.cpsnr.partial_cmp(&a.cpsnr).unwrap_or(std::cmp::Ordering::Equal));
    let mut s = format!(
        "# {name}\n\nCPSNR and per-channel PSNR in dB, sRGB-encoded 0..255, border {BORDER} px, {n} pictures.\n\n| Method | CPSNR | R | G | B | leads |\n|---|---|---|---|---|---|\n",
        name = c.name()
    );
    for r in rows {
        s.push_str(&format!(
            "| {} | {:.2} | {:.2} | {:.2} | {:.2} | {}/{n} |\n",
            r.name, r.cpsnr, r.r, r.g, r.b, r.leads
        ));
    }
    s
}

fn mean(v: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = v.collect();
    v.iter().sum::<f64>() / v.len() as f64
}

// ---------------------------------------------------------------- pictures

fn srgb_decode(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_encode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn load_srgb(path: &Path) -> Rgb {
    let img = image::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).to_rgb8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let lut: Vec<f64> = (0..256).map(|c| srgb_decode(c as f64 / 255.0)).collect();
    let mut rgb = Rgb::new(w, h);
    for (i, px) in img.pixels().enumerate() {
        let y = i / w;
        let x = i % w;
        rgb.set(x, y, lut[px[0] as usize], lut[px[1] as usize], lut[px[2] as usize]);
    }
    rgb
}

fn blur(src: &Rgb, sigma: f64) -> Rgb {
    let k = gaussian_kernel(sigma);
    Rgb { r: lowpass(&src.r, &k), g: lowpass(&src.g, &k), b: lowpass(&src.b, &k) }
}

/// Lanczos-3, twice as wide and tall. The centre of a destination pixel and
/// the support of 3 match the desktop bench's resize, rows before columns
/// when the two intermediate sizes are equal.
fn resize2(src: &Rgb) -> Rgb {
    Rgb { r: lanczos2(&src.r), g: lanczos2(&src.g), b: lanczos2(&src.b) }
}

fn lanczos3(x: f64) -> f64 {
    let x = x.abs();
    if x < 1e-12 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let p = std::f64::consts::PI * x;
        3.0 * p.sin() * (p / 3.0).sin() / (p * p)
    }
}

fn lanczos2(src: &Plane) -> Plane {
    let (dw, dh) = (src.width * 2, src.height * 2);
    let wide = resize_axis(src, dw, true);
    resize_axis(&wide, dh, false)
}

fn resize_axis(src: &Plane, dst_len: usize, rows: bool) -> Plane {
    let src_len = if rows { src.width } else { src.height };
    let scale = src_len as f64 / dst_len as f64;
    let (w, h) = if rows { (dst_len, src.height) } else { (src.width, dst_len) };
    let mut out = Plane::zeros(w, h);
    for i in 0..dst_len {
        let centre = (i as f64 + 0.5) * scale - 0.5;
        let lo = (centre - 3.0).floor() as isize;
        let hi = (centre + 3.0).ceil() as isize;
        let mut weights = Vec::new();
        let mut sum = 0.0;
        for j in lo..=hi {
            let v = lanczos3(j as f64 - centre);
            weights.push(v);
            sum += v;
        }
        for v in &mut weights {
            *v /= sum;
        }
        if rows {
            for y in 0..src.height {
                let mut acc = 0.0;
                for (k, j) in (lo..=hi).enumerate() {
                    let jc = j.clamp(0, src_len as isize - 1) as usize;
                    acc += weights[k] * src.at(jc, y);
                }
                out.set(i, y, acc);
            }
        } else {
            for x in 0..src.width {
                let mut acc = 0.0;
                for (k, j) in (lo..=hi).enumerate() {
                    let jc = j.clamp(0, src_len as isize - 1) as usize;
                    acc += weights[k] * src.at(x, jc);
                }
                out.set(x, i, acc);
            }
        }
    }
    out
}

fn mosaic(rgb: &Rgb, tile: &XTile) -> Vec<u16> {
    let (w, h) = (rgb.width(), rgb.height());
    let mut s = vec![0u16; w * h];
    for y in 0..h {
        for x in 0..w {
            let v = match tile.color_at(x as isize, y as isize) {
                Color::R => rgb.r.at(x, y),
                Color::G => rgb.g.at(x, y),
                Color::B => rgb.b.at(x, y),
            };
            s[y * w + x] = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
        }
    }
    s
}

/// The shipping separation on the mirroring frame: curve, the prior the
/// mosaic asks for, the bank, the edge step, then back to linear light.
fn separate(samples: &[u16], width: usize, placed: &mimizan_lab_xtrans::convert::Frame) -> Rgb {
    let mut m = Plane::zeros(placed.width, placed.height);
    for y in 0..placed.height {
        for x in 0..placed.width {
            let v = samples[(placed.y + y) * width + placed.x + x];
            m.set(x, y, f64::from(v) / 65535.0);
        }
    }
    let tone = Curve::Srgb;
    let m = tone.mosaic(&m);
    let p = XParams::default();
    let start = Prior { lens: 0.3, ..Prior::default() };
    let prior = xtrans::fit_prior(&m, &placed.tile, &p, start, 8, [true; 6]);
    let sep = xtrans::separate_bank(&m, &placed.tile, &p, &convert::bank(prior), &Selection::default());
    let sep = xtrans::refine(&m, &placed.tile, &sep, &Refine::default());
    sep.mapped(|v| tone.back(v)).rgb()
}

/// Each missing colour is the inverse-distance mean of the photosites of
/// that colour within four pixels. A photosite of the colour is kept.
fn floor(samples: &[u16], w: usize, h: usize, tile: &XTile, placed: &mimizan_lab_xtrans::convert::Frame) -> Rgb {
    let mut rgb = Rgb::new(placed.width, placed.height);
    for y in 0..placed.height {
        for x in 0..placed.width {
            let (xx, yy) = (placed.x + x, placed.y + y);
            let ch = |c: Color| {
                if tile.color_at(xx as isize, yy as isize) == c {
                    return f64::from(samples[yy * w + xx]) / 65535.0;
                }
                let mut num = 0.0;
                let mut den = 0.0;
                for dy in -4..=4 {
                    for dx in -4..=4 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let (x2, y2) = (xx as isize + dx, yy as isize + dy);
                        if x2 < 0 || y2 < 0 || x2 >= w as isize || y2 >= h as isize {
                            continue;
                        }
                        if tile.color_at(x2, y2) != c {
                            continue;
                        }
                        let weight = 1.0 / (dx * dx + dy * dy) as f64;
                        num += weight * f64::from(samples[y2 as usize * w + x2 as usize]) / 65535.0;
                        den += weight;
                    }
                }
                num / den
            };
            rgb.set(x, y, ch(Color::R), ch(Color::G), ch(Color::B));
        }
    }
    rgb
}

fn crop_truth(truth: &Rgb, placed: &mimizan_lab_xtrans::convert::Frame) -> Rgb {
    let mut rgb = Rgb::new(placed.width, placed.height);
    for y in 0..placed.height {
        for x in 0..placed.width {
            rgb.set(
                x,
                y,
                truth.r.at(placed.x + x, placed.y + y),
                truth.g.at(placed.x + x, placed.y + y),
                truth.b.at(placed.x + x, placed.y + y),
            );
        }
    }
    rgb
}

/// RawTherapee's picture, cropped to the frame. A size that is not the
/// mosaic and not the frame is refused: the pixels would not be the same ones.
fn align_rt(got: &Rgb, full: (usize, usize), placed: &mimizan_lab_xtrans::convert::Frame) -> Rgb {
    if got.width() == placed.width && got.height() == placed.height {
        return got.clone();
    }
    if got.width() == full.0 && got.height() == full.1 {
        return crop_truth(got, placed);
    }
    panic!(
        "RawTherapee wrote {} x {}, the mosaic is {} x {} and the frame {} x {}",
        got.width(),
        got.height(),
        full.0,
        full.1,
        placed.width,
        placed.height
    );
}

// ---------------------------------------------------------------- metric

#[derive(Clone, Copy)]
struct Colour {
    cpsnr: f64,
    r: f64,
    g: f64,
    b: f64,
}

/// CPSNR of `est` laid on `truth` with the top-left of `est` at `(dx, dy)`.
fn shifted(est: &Rgb, truth: &Rgb, dx: isize, dy: isize) -> f64 {
    let (w, h) = (truth.width(), truth.height());
    let (x0, x1) = (BORDER, w - BORDER);
    let (y0, y1) = (BORDER, h - BORDER);
    let mut se = 0.0;
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (xs, ys) = (x as isize - dx, y as isize - dy);
            if xs < 0 || ys < 0 || xs >= est.width() as isize || ys >= est.height() as isize {
                continue;
            }
            for (e, t) in [(&est.r, &truth.r), (&est.g, &truth.g), (&est.b, &truth.b)] {
                let d = srgb_encode(e.at(xs as usize, ys as usize)) * 255.0 - srgb_encode(t.at(x, y)) * 255.0;
                se += d * d;
            }
            n += 3.0;
        }
    }
    10.0 * (255.0 * 255.0 / (se / n)).log10()
}

fn shift_check(a: &Args, tile: &XTile) {
    let path = a.images.join("kodim01.png");
    let truth = load_srgb(&path);
    let (w, h) = (truth.width(), truth.height());
    let placed = frame(tile, (0, 0, w, h)).unwrap();
    let tif = a.out.join("pixel").join("kodim01.3-pass.tif");
    let got = align_rt(&read_rt(&tif), (w, h), &placed);
    let truth = crop_truth(&truth, &placed);
    let mut best = (f64::NEG_INFINITY, 0, 0);
    for dy in -2..=2 {
        for dx in -2..=2 {
            let q = shifted(&got, &truth, dx, dy);
            println!("{dx:+} {dy:+}  {q:.2}");
            if q > best.0 {
                best = (q, dx, dy);
            }
        }
    }
    println!("best {:+} {:+}  {:.2}", best.1, best.2, best.0);
}

fn colour_of(est: &Rgb, truth: &Rgb) -> Colour {
    assert_eq!((est.width(), est.height()), (truth.width(), truth.height()));
    let (w, h) = (est.width(), est.height());
    // The 16 px are of the picture the frame sits in. The frame itself
    // already loses a few pixels at the edge, so the inset is what remains
    // of those 16 inside the frame. A frame that starts at 0 uses all 16.
    let inset = BORDER;
    let (x0, x1) = (inset.min(w / 4), w.saturating_sub(inset));
    let (y0, y1) = (inset.min(h / 4), h.saturating_sub(inset));
    let n = ((x1 - x0) * (y1 - y0)) as f64;
    let mse = |e: &Plane, t: &Plane| {
        let mut s = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let d = srgb_encode(e.at(x, y)) * 255.0 - srgb_encode(t.at(x, y)) * 255.0;
                s += d * d;
            }
        }
        s / n
    };
    let (mr, mg, mb) = (mse(&est.r, &truth.r), mse(&est.g, &truth.g), mse(&est.b, &truth.b));
    let psnr = |m: f64| {
        if m > 0.0 {
            10.0 * (255.0 * 255.0 / m).log10()
        } else {
            f64::INFINITY
        }
    };
    Colour { cpsnr: psnr((mr + mg + mb) / 3.0), r: psnr(mr), g: psnr(mg), b: psnr(mb) }
}

// ---------------------------------------------------------------- RawTherapee

fn develop_rt(dng: &Path, tif: &Path, method: &str, steps: u32) {
    if tif.is_file() {
        return;
    }
    let pp3 = tif.with_extension("pp3");
    fs::write(&pp3, profile(method, steps)).unwrap();
    let status = Command::new("rawtherapee-cli")
        .arg("-o")
        .arg(tif)
        .args(["-t", "-b16", "-Y", "-q", "-p"])
        .arg(&pp3)
        .arg("-c")
        .arg(dng)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("rawtherapee-cli");
    if !status.success() || !tif.is_file() {
        panic!("RawTherapee failed on {}", dng.display());
    }
}

fn profile(method: &str, steps: u32) -> String {
    format!(
        "[Version]\nAppVersion=5.11\nVersion=351\n\n\
[Exposure]\nAuto=false\nClip=0\nCompensation=0\nBrightness=0\nContrast=0\nSaturation=0\nBlack=0\n\
HighlightCompr=0\nHighlightComprThreshold=0\nShadowCompr=0\nHistogramMatching=false\n\
CurveFromHistogramMatching=false\nClampOOG=true\nCurveMode=Standard\nCurveMode2=Standard\nCurve=0\nCurve2=0\n\n\
[HLRecovery]\nEnabled=false\nMethod=Blend\n\n\
[White Balance]\nEnabled=true\nSetting=Camera\nTemperature=6504\nGreen=1\nEqual=1\n\n\
[Color Management]\nInputProfile=(none)\nToneCurve=false\nApplyLookTable=false\nApplyBaselineExposureOffset=false\n\
ApplyHueSatMap=false\nDCPIlluminant=0\nWorkingProfile=sRGB\nOutputProfile=RTv4_sRGB\nOutputProfileIntent=Relative\nOutputBPC=true\n\n\
[Sharpening]\nEnabled=false\n\n[SharpenEdge]\nEnabled=false\n\n[SharpenMicro]\nEnabled=false\n\n\
[Impulse Denoising]\nEnabled=false\n\n[Defringing]\nEnabled=false\n\n[Directional Pyramid Denoising]\nEnabled=false\n\n\
[RAW]\nCA=false\nHotPixelFilter=false\nDeadPixelFilter=false\nPreExposure=1\n\n\
[RAW Bayer]\nMethod=fast\nBorder=0\nCcSteps=0\n\n\
[RAW X-Trans]\nMethod={method}\nBorder=0\nCcSteps={steps}\n"
    )
}

fn read_rt(path: &Path) -> Rgb {
    let img = image::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).to_rgb16();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut rgb = Rgb::new(w, h);
    for (i, px) in img.pixels().enumerate() {
        let y = i / w;
        let x = i % w;
        let f = |c: u16| srgb_decode(f64::from(c) / 65535.0);
        rgb.set(x, y, f(px[0]), f(px[1]), f(px[2]));
    }
    rgb
}

// ---------------------------------------------------------------- the gate

fn gate_of(out: &Path, tile: &XTile) -> String {
    let (w, h) = (192, 192);
    let mut truth = Rgb::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let u = x as f64 / w as f64;
            let v = y as f64 / h as f64;
            truth.set(
                x,
                y,
                0.25 + 0.15 * (std::f64::consts::TAU * u).sin(),
                0.40 + 0.10 * (std::f64::consts::TAU * v).cos(),
                0.35 + 0.08 * (std::f64::consts::TAU * (u + v)).sin(),
            );
        }
    }
    let samples = mosaic(&truth, tile);
    let placed = frame(tile, (0, 0, w, h)).unwrap();
    let ours = separate(&samples, w, &placed);
    let dng = out.join("gate.dng");
    write_dng(&dng, w, h, &samples);
    let mut lines = Vec::new();
    let mut fail = false;
    let q = colour_of(&ours, &crop_truth(&truth, &placed));
    lines.push(format!("Mimizan Lab X-Trans  {:.2}", q.cpsnr));
    fail |= q.cpsnr < 59.0;
    for (stem, method, steps) in [("1", "1-pass (medium)", 0), ("3", "3-pass (best)", 0), ("s", "3-pass (best)", 3)] {
        let tif = out.join(format!("gate.{stem}.tif"));
        let _ = fs::remove_file(&tif);
        develop_rt(&dng, &tif, method, steps);
        let got = read_rt(&tif);
        let q = colour_of(&align_rt(&got, (w, h), &placed), &crop_truth(&truth, &placed));
        lines.push(format!("{method} steps {steps}  {:.2}  ({} x {})", q.cpsnr, got.width(), got.height()));
        fail |= q.cpsnr < 59.0;
    }
    let head = if fail { "gate FAIL" } else { "gate ok" };
    format!("{head}\n{}", lines.join("\n"))
}

// ---------------------------------------------------------------- DNG

const T_BYTE: u16 = 1;
const T_ASCII: u16 = 2;
const T_SHORT: u16 = 3;
const T_LONG: u16 = 4;
const T_RATIONAL: u16 = 5;
const T_SRATIONAL: u16 = 10;

struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    data: Vec<u8>,
}

fn ascii(tag: u16, s: &str) -> Entry {
    let mut data = s.as_bytes().to_vec();
    data.push(0);
    Entry { tag, typ: T_ASCII, count: data.len() as u32, data }
}
fn shorts(tag: u16, v: &[u16]) -> Entry {
    Entry { tag, typ: T_SHORT, count: v.len() as u32, data: v.iter().flat_map(|x| x.to_le_bytes()).collect() }
}
fn longs(tag: u16, v: &[u32]) -> Entry {
    Entry { tag, typ: T_LONG, count: v.len() as u32, data: v.iter().flat_map(|x| x.to_le_bytes()).collect() }
}
fn bytes(tag: u16, v: &[u8]) -> Entry {
    Entry { tag, typ: T_BYTE, count: v.len() as u32, data: v.to_vec() }
}
fn rationals(tag: u16, v: &[(u32, u32)]) -> Entry {
    let data = v.iter().flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat()).collect();
    Entry { tag, typ: T_RATIONAL, count: v.len() as u32, data }
}
fn srationals(tag: u16, v: &[f64]) -> Entry {
    let data = v
        .iter()
        .flat_map(|x| {
            let n = (x * 10_000.0).round() as i32;
            [n.to_le_bytes(), 10_000i32.to_le_bytes()].concat()
        })
        .collect();
    Entry { tag, typ: T_SRATIONAL, count: v.len() as u32, data }
}

fn write_dng(path: &Path, width: usize, height: usize, samples: &[u16]) {
    let cfa: Vec<u8> = FUJI
        .bytes()
        .map(|c| match c {
            b'R' => 0,
            b'G' => 1,
            b'B' => 2,
            _ => unreachable!(),
        })
        .collect();
    let strip_bytes = (samples.len() * 2) as u32;
    let mut entries = vec![
        longs(254, &[0]),
        longs(256, &[width as u32]),
        longs(257, &[height as u32]),
        shorts(258, &[16]),
        shorts(259, &[1]),
        shorts(262, &[32803]),
        ascii(271, "Fujifilm"),
        ascii(272, "X-Trans"),
        longs(273, &[0]),
        shorts(274, &[1]),
        shorts(277, &[1]),
        longs(278, &[height as u32]),
        longs(279, &[strip_bytes]),
        shorts(284, &[1]),
        ascii(305, "xtrans-kodak"),
        shorts(33421, &[6, 6]),
        bytes(33422, &cfa),
        bytes(50706, &[1, 4, 0, 0]),
        bytes(50707, &[1, 1, 0, 0]),
        ascii(50708, "X-Trans"),
        shorts(50711, &[1]),
        longs(50714, &[0]),
        longs(50717, &[65535]),
        srationals(50721, &XYZ_TO_SRGB),
        rationals(50728, &[(1, 1), (1, 1), (1, 1)]),
        shorts(50778, &[21]),
    ];
    entries.sort_by_key(|e| e.tag);
    let ifd_len = 2 + 12 * entries.len() as u32 + 4;
    let mut extra = 8 + ifd_len;
    let mut offsets = Vec::new();
    for e in &entries {
        if e.data.len() > 4 {
            offsets.push(Some(extra));
            extra += e.data.len() as u32 + (e.data.len() as u32 & 1);
        } else {
            offsets.push(None);
        }
    }
    let strip = extra;
    if let Some(e) = entries.iter_mut().find(|e| e.tag == 273) {
        e.data = strip.to_le_bytes().to_vec();
    }
    let mut w = BufWriter::new(fs::File::create(path).unwrap());
    w.write_all(&[b'I', b'I', 42, 0]).unwrap();
    w.write_all(&8u32.to_le_bytes()).unwrap();
    w.write_all(&(entries.len() as u16).to_le_bytes()).unwrap();
    for (e, off) in entries.iter().zip(&offsets) {
        w.write_all(&e.tag.to_le_bytes()).unwrap();
        w.write_all(&e.typ.to_le_bytes()).unwrap();
        w.write_all(&e.count.to_le_bytes()).unwrap();
        match off {
            Some(o) => w.write_all(&o.to_le_bytes()).unwrap(),
            None => {
                let mut v = [0u8; 4];
                v[..e.data.len()].copy_from_slice(&e.data);
                w.write_all(&v).unwrap();
            }
        }
    }
    w.write_all(&0u32.to_le_bytes()).unwrap();
    for (e, off) in entries.iter().zip(&offsets) {
        if off.is_some() {
            w.write_all(&e.data).unwrap();
            if e.data.len() & 1 == 1 {
                w.write_all(&[0]).unwrap();
            }
        }
    }
    for s in samples {
        w.write_all(&s.to_le_bytes()).unwrap();
    }
    w.flush().unwrap();
}

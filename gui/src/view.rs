//! A linear DNG on the screen. The DNG holds the camera's red, green and
//! blue, not balanced and without a tone curve; to be looked at it is
//! balanced, taken to sRGB by the file's own matrix, made as light as the
//! camera's preview and bent into the screen's range. Nothing here is
//! written anywhere: it is the look of the file, not a development.

use egui::{Color32, ColorImage};
use rawler::imgop::xyz::Illuminant;
use rawler::rawimage::{RawImage, RawImageData, RawPhotometricInterpretation};
use rayon::prelude::*;

/// How light the picture is made when there is no preview to go by: the
/// mean of an ordinary photograph, of 1, as the screen's values.
const USUAL_LIGHTNESS: f32 = 0.42;
/// The table from linear light to the screen reaches this far above the
/// sensor's white: the matrix lifts saturated colours over it.
const TABLE_REACH: f32 = 2.0;

/// The grey that stays where it is when the contrast is raised, and by
/// which power of the light around it. A linear picture shown straight
/// looks flat beside the camera's own.
const MIDDLE_GREY: f32 = 0.18;
const CONTRAST: f32 = 1.35;

/// The picture as it is shown.
pub struct Shown {
    pub image: ColorImage,
    /// Width and height of the picture in the file, before it was turned
    /// upright or made smaller for the graphics card.
    pub pixels: (usize, usize),
    /// What the linear light was multiplied by.
    pub gain: f32,
}

/// XYZ from linear sRGB, D65.
const XYZ_FROM_SRGB: [[f32; 3]; 3] = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175],
    [0.019_333_9, 0.119_192, 0.950_304_1],
];

fn times(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut c = [[0.0; 3]; 3];
    for (i, row) in c.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    c
}

fn inverse(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let cof = |i: usize, j: usize| {
        let (a, b) = ((i + 1) % 3, (i + 2) % 3);
        let (c, d) = ((j + 1) % 3, (j + 2) % 3);
        m[a][c] * m[b][d] - m[a][d] * m[b][c]
    };
    let det: f32 = (0..3).map(|j| m[0][j] * cof(0, j)).sum();
    if !det.is_finite() || det.abs() < 1e-9 {
        return None;
    }
    let mut inv = [[0.0; 3]; 3];
    for (i, row) in inv.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = cof(j, i) / det;
        }
    }
    Some(inv)
}

/// Linear sRGB from the balanced camera channels, given the file's matrix
/// from XYZ to the camera. Each row of the way there is scaled to sum 1,
/// so what the balance made grey stays grey.
fn srgb_from_camera(camera_from_xyz: &[f32]) -> Option<[[f32; 3]; 3]> {
    let m = camera_from_xyz;
    if m.len() < 9 {
        return None;
    }
    let mut there = times([[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]], XYZ_FROM_SRGB);
    for row in &mut there {
        let sum: f32 = row.iter().sum();
        if !sum.is_finite() || sum.abs() < 1e-6 {
            return None;
        }
        row.iter_mut().for_each(|v| *v /= sum);
    }
    inverse(there)
}

/// The screen's value for linear light, both of 1.
fn encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Bend light that reaches up to `white` into 0 to 1: nearly straight in
/// the shadows where `white` is large, and `white` lands on 1 whatever
/// it is, also for a picture that is made darker.
fn shoulder(v: f32, white: f32) -> f32 {
    v * (1.0 + v / (white * white)) / (1.0 + v)
}

/// The screen's value for linear light `v` under `gain`: more contrast
/// around a middle grey, as a camera gives its own pictures, then the
/// shoulder, so that the sensor's white is the screen's.
fn on_screen(v: f32, gain: f32) -> f32 {
    let steeper = |x: f32| MIDDLE_GREY * (x / MIDDLE_GREY).powf(CONTRAST);
    encode(shoulder(steeper(v.max(0.0) * gain), steeper(gain)))
}

/// Mean of the screen's values, as lightness, for `samples` of linear
/// sRGB under `gain`.
fn lightness(samples: &[[f32; 3]], gain: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples
        .iter()
        .map(|s| {
            0.2126 * on_screen(s[0], gain) + 0.7152 * on_screen(s[1], gain) + 0.0722 * on_screen(s[2], gain)
        })
        .sum();
    sum / samples.len() as f32
}

/// The gain under which `samples` are as light as `target`.
fn gain_for(samples: &[[f32; 3]], target: f32) -> f32 {
    let (mut low, mut high) = (0.125_f32, 256.0_f32);
    for _ in 0..28 {
        let mid = (low * high).sqrt();
        if lightness(samples, mid) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low * high).sqrt()
}

/// Red, green and blue of every pixel, and what is needed to show them.
struct Linear<'a> {
    data: &'a [u16],
    /// Width of `data` in pixels.
    stride: usize,
    /// The part that is the picture: `x, y, width, height`.
    crop: (usize, usize, usize, usize),
    black: [f32; 3],
    white: [f32; 3],
    /// What makes grey grey; green is 1.
    balance: [f32; 3],
    srgb_from_camera: [[f32; 3]; 3],
}

impl<'a> Linear<'a> {
    fn of(img: &'a RawImage) -> Result<Self, String> {
        if !matches!(img.photometric, RawPhotometricInterpretation::LinearRaw) || img.cpp != 3 {
            return Err("the DNG does not hold red, green and blue for every pixel".into());
        }
        let RawImageData::Integer(data) = &img.data else {
            return Err("the DNG is not in whole numbers".into());
        };
        if data.len() != 3 * img.width * img.height {
            return Err("the DNG does not hold as many values as it says".into());
        }
        let crop = img
            .crop_area
            .or(img.active_area)
            .map_or((0, 0, img.width, img.height), |r| (r.p.x, r.p.y, r.d.w, r.d.h));
        if crop.2 == 0 || crop.3 == 0 || crop.0 + crop.2 > img.width || crop.1 + crop.3 > img.height {
            return Err("the DNG's crop lies outside its picture".into());
        }
        let levels = &img.blacklevel.levels;
        let black = [0, 1, 2].map(|c| levels.get(c % levels.len().max(1)).map_or(0.0, |l| l.as_f32()));
        let whites = &img.whitelevel.0;
        let white = [0, 1, 2].map(|c| whites.get(c % whites.len().max(1)).map_or(65535.0, |w| *w as f32));
        if (0..3).any(|c| white[c] <= black[c]) {
            return Err("the DNG's black and white are no range".into());
        }
        let wb = [img.wb_coeffs[0], img.wb_coeffs[1], img.wb_coeffs[2]];
        let balance = if wb.iter().all(|v| v.is_finite() && *v > 0.0) {
            [wb[0] / wb[1], 1.0, wb[2] / wb[1]]
        } else {
            [1.0; 3]
        };
        // The matrix for daylight if the file has one, else whichever it has.
        let matrix = img.color_matrix.get(&Illuminant::D65).or_else(|| img.color_matrix.values().next());
        let srgb_from_camera = matrix.and_then(|m| srgb_from_camera(m)).unwrap_or([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ]);
        Ok(Self { data, stride: img.width, crop, black, white, balance, srgb_from_camera })
    }

    /// Linear sRGB of the pixel at `x, y` of the crop: balanced, cut at
    /// the sensor's white so that what is burnt out is neutral.
    fn srgb(&self, x: usize, y: usize) -> [f32; 3] {
        let at = 3 * ((self.crop.1 + y) * self.stride + self.crop.0 + x);
        let c: [f32; 3] = std::array::from_fn(|k| {
            let v = (f32::from(self.data[at + k]) - self.black[k]) / (self.white[k] - self.black[k]);
            (v * self.balance[k]).min(1.0)
        });
        self.srgb_from_camera.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2])
    }

    /// The picture for the screen, as light as `target` (the mean of the
    /// screen's values, of 1), and the gain that made it so.
    fn render(&self, target: f32) -> (ColorImage, f32) {
        let (_, _, w, h) = self.crop;
        // About a hundred thousand pixels tell how light the picture is.
        let step = (((w * h) as f64 / 1e5).sqrt() as usize).max(1);
        let samples: Vec<[f32; 3]> =
            (0..h).step_by(step).flat_map(|y| (0..w).step_by(step).map(move |x| self.srgb(x, y))).collect();
        let gain = gain_for(&samples, target.clamp(0.05, 0.9));
        let scale = 65535.0 / TABLE_REACH;
        let table: Vec<u8> =
            (0..=65535).map(|i| (255.0 * on_screen(i as f32 / scale, gain)).round() as u8).collect();
        let mut pixels = vec![Color32::BLACK; w * h];
        pixels.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, pixel) in row.iter_mut().enumerate() {
                let [r, g, b] = self.srgb(x, y).map(|v| table[(v * scale).clamp(0.0, 65535.0) as usize]);
                *pixel = Color32::from_rgb(r, g, b);
            }
        });
        (ColorImage::new([w, h], pixels), gain)
    }
}

/// The DNG `img` for the screen: as light as `lightness` (of the camera's
/// preview) if there is one, upright, and no side longer than `max_side`.
pub fn develop(img: &RawImage, lightness: Option<f32>, max_side: usize) -> Result<Shown, String> {
    let linear = Linear::of(img)?;
    let (image, gain) = linear.render(lightness.unwrap_or(USUAL_LIGHTNESS));
    let pixels = (linear.crop.2, linear.crop.3);
    Ok(Shown { image: shrink(turn(image, img.orientation.to_u16()), max_side), pixels, gain })
}

/// Turn `image` upright by the orientation an Exif tag gives: 1 is
/// upright already, 6 and 8 are the two ways a camera is held on end.
pub fn turn(image: ColorImage, orientation: u16) -> ColorImage {
    if !(2..=8).contains(&orientation) {
        return image;
    }
    let [w, h] = image.size;
    let swap = orientation >= 5;
    let (tw, th) = if swap { (h, w) } else { (w, h) };
    let mut pixels = vec![Color32::BLACK; tw * th];
    pixels.par_chunks_mut(tw).enumerate().for_each(|(ty, row)| {
        for (tx, pixel) in row.iter_mut().enumerate() {
            // Where the pixel at `tx, ty` of the upright picture lies in the file.
            let (x, y) = match orientation {
                2 => (w - 1 - tx, ty),
                3 => (w - 1 - tx, h - 1 - ty),
                4 => (tx, h - 1 - ty),
                5 => (ty, tx),
                6 => (ty, h - 1 - tx),
                7 => (w - 1 - ty, h - 1 - tx),
                _ => (w - 1 - ty, tx),
            };
            *pixel = image.pixels[y * w + x];
        }
    });
    ColorImage::new([tw, th], pixels)
}

/// `image` with no side longer than `max_side`: every `k` by `k` pixels
/// become their mean, for the smallest whole `k` that does it.
pub fn shrink(image: ColorImage, max_side: usize) -> ColorImage {
    let [w, h] = image.size;
    let k = w.max(h).div_ceil(max_side.max(1));
    if k <= 1 {
        return image;
    }
    let (tw, th) = ((w / k).max(1), (h / k).max(1));
    let mut pixels = vec![Color32::BLACK; tw * th];
    pixels.par_chunks_mut(tw).enumerate().for_each(|(ty, row)| {
        for (tx, pixel) in row.iter_mut().enumerate() {
            let mut sum = [0u32; 3];
            let mut n = 0;
            for y in (ty * k..(ty + 1) * k).take_while(|y| *y < h) {
                for p in image.pixels[y * w..].iter().skip(tx * k).take(k.min(w - tx * k)) {
                    sum[0] += u32::from(p.r());
                    sum[1] += u32::from(p.g());
                    sum[2] += u32::from(p.b());
                    n += 1;
                }
            }
            let [r, g, b] = sum.map(|s| ((s + n / 2) / n.max(1)) as u8);
            *pixel = Color32::from_rgb(r, g, b);
        }
    });
    ColorImage::new([tw, th], pixels)
}

/// The mean of the screen's values of `image`, as lightness, of 1.
pub fn lightness_of(image: &ColorImage) -> f32 {
    if image.pixels.is_empty() {
        return USUAL_LIGHTNESS;
    }
    let sum: f64 = image
        .pixels
        .iter()
        .map(|p| 0.2126 * f64::from(p.r()) + 0.7152 * f64::from(p.g()) + 0.0722 * f64::from(p.b()))
        .sum();
    (sum / (255.0 * image.pixels.len() as f64)) as f32
}

#[cfg(test)]
mod tests {
    use super::{
        gain_for, inverse, lightness, lightness_of, on_screen, shrink, srgb_from_camera, times, turn, Linear,
    };
    use egui::{Color32, ColorImage};

    const UNIT: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    fn grey(v: u8) -> Color32 {
        Color32::from_gray(v)
    }

    #[test]
    fn grey_stays_grey_through_the_matrix() {
        // A camera matrix of the usual kind (XYZ to camera, Fuji X-T3).
        let m = [1.3426, -0.6334, -0.1177, -0.4244, 1.2136, 0.2371, -0.0580, 0.1303, 0.5980];
        let back = srgb_from_camera(&m).unwrap();
        for row in back {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-4, "{row:?}");
        }
        let there = inverse(back).unwrap();
        let round = times(there, back);
        for (i, row) in round.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - UNIT[i][j]).abs() < 1e-4);
            }
        }
        assert!(srgb_from_camera(&m[..6]).is_none());
        assert!(srgb_from_camera(&[0.0; 9]).is_none());
    }

    #[test]
    fn the_screen_runs_from_black_to_white_and_never_back() {
        for gain in [0.3, 1.0, 2.5, 16.0] {
            assert_eq!(on_screen(0.0, gain), 0.0);
            assert!((on_screen(1.0, gain) - 1.0).abs() < 1e-5, "white is not white at {gain}");
            let mut last = -1.0;
            for i in 0..=1000 {
                let v = on_screen(i as f32 / 1000.0, gain);
                assert!(v >= last && (0.0..=1.0).contains(&v));
                last = v;
            }
        }
    }

    #[test]
    fn the_gain_makes_the_picture_as_light_as_asked() {
        let samples: Vec<[f32; 3]> =
            (0..500).map(|i| [0.0004 * i as f32, 0.0003 * i as f32, 0.0002 * i as f32]).collect();
        for target in [0.2, 0.42, 0.6] {
            let gain = gain_for(&samples, target);
            assert!((lightness(&samples, gain) - target).abs() < 1e-3, "{target} at gain {gain}");
        }
    }

    /// Four pixels: grey, pure red of the camera, burnt out, and black.
    fn four() -> Vec<u16> {
        vec![8000, 16000, 10000, 20000, 0, 0, 65535, 65535, 65535, 0, 0, 0]
    }

    #[test]
    fn a_dng_is_balanced_cut_at_white_and_cropped() {
        let data = four();
        let linear = Linear {
            data: &data,
            stride: 4,
            crop: (0, 0, 4, 1),
            black: [0.0; 3],
            white: [65535.0; 3],
            balance: [2.0, 1.0, 1.6],
            srgb_from_camera: UNIT,
        };
        // 8000 x 2, 16000 and 10000 x 1.6 are one grey.
        let grey = linear.srgb(0, 0);
        assert!((grey[0] - grey[1]).abs() < 1e-6 && (grey[2] - grey[1]).abs() < 1e-6);
        // Burnt out is white, not the colour of the balance.
        assert_eq!(linear.srgb(2, 0), [1.0; 3]);
        let (image, gain) = linear.render(0.4);
        assert_eq!(image.size, [4, 1]);
        assert!((lightness_of(&image) - 0.4).abs() < 0.02, "{} at gain {gain}", lightness_of(&image));
        let p = image.pixels[0];
        assert!(p.r() == p.g() && p.g() == p.b() && p.r() > 0);
        assert!(image.pixels[1].r() > image.pixels[1].g());
        assert_eq!((image.pixels[2], image.pixels[3]), (Color32::WHITE, Color32::BLACK));

        let part = Linear { crop: (1, 0, 2, 1), ..linear };
        assert_eq!(part.srgb(1, 0), [1.0; 3]);
        assert_eq!(part.render(0.4).0.size, [2, 1]);
    }

    #[test]
    fn every_orientation_puts_the_corners_where_exif_says() {
        // Three wide, two high: 0 1 2 over 3 4 5.
        let image = || ColorImage::new([3, 2], (0..6).map(|v| grey(v * 10)).collect());
        let values = |i: ColorImage| i.pixels.iter().map(|p| p.r() / 10).collect::<Vec<_>>();
        let expected: [(u16, [usize; 2], [u8; 6]); 9] = [
            (0, [3, 2], [0, 1, 2, 3, 4, 5]),
            (1, [3, 2], [0, 1, 2, 3, 4, 5]),
            (2, [3, 2], [2, 1, 0, 5, 4, 3]),
            (3, [3, 2], [5, 4, 3, 2, 1, 0]),
            (4, [3, 2], [3, 4, 5, 0, 1, 2]),
            (5, [2, 3], [0, 3, 1, 4, 2, 5]),
            (6, [2, 3], [3, 0, 4, 1, 5, 2]),
            (7, [2, 3], [5, 2, 4, 1, 3, 0]),
            (8, [2, 3], [2, 5, 1, 4, 0, 3]),
        ];
        for (orientation, size, want) in expected {
            let turned = turn(image(), orientation);
            assert_eq!(turned.size, size, "orientation {orientation}");
            assert_eq!(values(turned), want, "orientation {orientation}");
        }
    }

    #[test]
    fn a_picture_too_large_for_the_card_is_made_smaller_by_whole_steps() {
        let image = ColorImage::new([5, 4], (0..20).map(|v| grey(v * 10)).collect());
        assert_eq!(shrink(image.clone(), 5).size, [5, 4]);
        let half = shrink(image.clone(), 4);
        assert_eq!(half.size, [2, 2]);
        // 0 10 50 60 make 30.
        assert_eq!(half.pixels[0], grey(30));
        assert_eq!(shrink(image, 1).size, [1, 1]);
    }
}

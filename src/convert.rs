//! From the decoded RAF to the three channels the DNG holds.

use crate::plane::Plane;
use crate::xtrans::{self, Curve, Prior, Refine, Selection, XParams, XTile};
use rawler::rawimage::RawPhotometricInterpretation;
use rawler::{RawImage, RawImageData};
use rayon::prelude::*;
use std::time::Instant;

/// What the user can choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Leave the mosaic as recorded where a channel is burnt out. By
    /// default the balanced mosaic is cut at the level the first channel
    /// burns out at: a burnt area is then flat and neutral, and no colour
    /// of the balance spreads from it into what lies around it. Kept, the
    /// channels that still hold something above that level stay for the
    /// raw developer's highlight recovery.
    pub keep_highlights: bool,
}

/// The part of the file that is developed, and the tile at its corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub tile: XTile,
}

/// The largest frame inside `crop` (`x, y, width, height`) whose first and
/// last column and row are symmetry lines of the tile, so that mirroring
/// the edges continues the mosaic. At most seven columns and seven rows
/// of the crop are left out.
pub fn frame(sensor: &XTile, crop: (usize, usize, usize, usize)) -> Option<Frame> {
    let (x0, y0, cw, ch) = crop;
    if cw < 24 || ch < 24 {
        return None;
    }
    for y in y0..y0 + 6 {
        for x in x0..x0 + 6 {
            let (width, height) = ((x0 + cw - x - 1) / 3 * 3 + 1, (y0 + ch - y - 1) / 3 * 3 + 1);
            let tile = sensor.shifted(x as isize, y as isize);
            if tile.mirrors(width, height) {
                return Some(Frame { x, y, width, height, tile });
            }
        }
    }
    None
}

/// A full photosite in the picture; no light is 0.
///
/// The file's own black level is not carried over, though a DNG has a tag
/// for it and the noise around black would keep both its signs: darktable
/// (5.0) does not read that tag for a picture of three channels and shows
/// such a file grey and magenta. RawTherapee and LibRaw do read it.
pub const WHITE: u16 = u16::MAX;

/// What [`develop`] gives.
#[derive(Clone, Debug)]
pub struct Developed {
    pub frame: Frame,
    /// Red, green and blue of every pixel, row by row: the camera's own
    /// channels, not balanced, from 0 at black to [`WHITE`].
    pub data: Vec<u16>,
    /// Values of `data` the noise had put below black, cut to 0.
    pub below: usize,
    /// What red, green and blue are multiplied by to make grey grey
    /// (green is 1).
    pub balance: [f64; 3],
    /// Whether the balance is the camera's; if not, it is the one that
    /// makes the frame's mean grey.
    pub balance_from_file: bool,
    /// The prior the mosaic was most probable under.
    pub prior: Prior,
    /// Photosites of the frame at the file's white level.
    pub burnt: usize,
    /// Sums of the magnitudes of luminance and both chrominances over the
    /// frame, to tell whether two builds give the same picture.
    pub sums: [f64; 3],
    /// Seconds for the search of the prior, the first and the second step.
    pub seconds: [f64; 3],
}

/// The priors the first step weighs against each other: the one found,
/// the same with a sixteenth, a quarter, four and sixteen times the
/// colour, eight with a direction (`cos¹⁶`) at four times the colour, and
/// eight more with a direction for lines near the sensor's limit.
///
/// Under the prior found, which is the frame's mean, such lines leave the
/// dots of the tile in the luminance: they stand far above the frame's
/// mean and are sharper than it, and red, green and blue do not hold the
/// same contrast in them, so the mosaic holds colour as fine as the lines,
/// which the mean takes for noise. What is not taken for colour stays in
/// the luminance. Along one direction the tile holds all three colours in
/// every row and column, so the lines can be told.
///
/// Their priors have a luminance that falls with `f¹` only, a sixteenth of
/// the noise and half the lens. Their colour is not the one found but a
/// sixteenth of the luminance at 0.25 c/px, the same at every frequency:
/// a fortieth of the luminance at 0.1 c/px, an eighth at the limit.
///
/// What was tried and measured on two frames of one chart, at ISO 160 and
/// ISO 3200 (unrest along the lines of the finest wedge; scatter of the
/// colour in a coarser wedge, a star and small print, all grey):
/// - Sixteen times the colour found, falling with `f¹`: the dots go at
///   ISO 160, but the colour found grows with the frame's noise, and at
///   ISO 3200 the noise comes through as colour along every line (scatter
///   in the coarser wedge 2.6 to 4.8).
/// - The colour as above with the luminance of the prior found: the small
///   print takes these priors too and gets a quarter more colour scatter.
///   With `f¹` they are taken where the pattern is fine and nowhere else.
/// - As it is: unrest 1.94 to 0.72 beside 0.74 on the blank paper at
///   ISO 160 and 5.05 to 3.15 beside 3.01 at ISO 3200; the scatter within
///   2 % of what it was at ISO 160 and below it at ISO 3200.
pub fn bank(prior: Prior) -> Vec<Prior> {
    let colour = |f: f64| Prior { chroma: [prior.chroma[0] * f, prior.chroma[1] * f], ..prior };
    let towards = |q: Prior| {
        (0..8).map(move |j| Prior { oriented: Some((std::f64::consts::PI * j as f64 / 8.0, 8.0)), ..q })
    };
    let lines = Prior {
        alpha: prior.alpha.min(1.0),
        alpha_c: 0.0,
        chroma: [0.0625; 2],
        noise: prior.noise / 16.0,
        lens: prior.lens / 2.0,
        ..prior
    };
    let mut priors = vec![prior];
    priors.extend([0.0625, 0.25, 4.0, 16.0].map(colour));
    priors.extend(towards(colour(4.0)));
    priors.extend(towards(lines));
    priors
}

/// The picture of an X-Trans mosaic. `say` is told what is being done.
pub fn develop(img: &RawImage, settings: &Settings, say: &dyn Fn(&str)) -> Result<Developed, String> {
    let RawPhotometricInterpretation::Cfa(config) = &img.photometric else {
        return Err("the file holds no mosaic".into());
    };
    let sensor = (config.cfa.width == 6 && config.cfa.height == 6)
        .then(|| XTile::from_name(&config.cfa.name))
        .flatten()
        .ok_or_else(|| format!("not an X-Trans mosaic (its tile is {})", config.cfa.name))?;
    let RawImageData::Integer(raw) = &img.data else {
        return Err("the mosaic is not in whole numbers".into());
    };
    if img.cpp != 1 || raw.len() != img.width * img.height {
        return Err("the mosaic does not have one value per photosite".into());
    }
    let white = f64::from(*img.whitelevel.0.first().ok_or("the file gives no white level")?);
    // The black level may differ from photosite to photosite of its tile.
    let levels = &img.blacklevel;
    if levels.cpp != 1 || levels.levels.len() != levels.width * levels.height || levels.levels.is_empty() {
        return Err("the file's black levels are not per photosite".into());
    }
    let blacks: Vec<f64> = levels.levels.iter().map(|l| f64::from(l.as_f32())).collect();
    let black_at = |x: usize, y: usize| blacks[(y % levels.height) * levels.width + x % levels.width];
    let black = blacks.iter().sum::<f64>() / blacks.len() as f64;
    if !(white.is_finite() && black.is_finite() && white > black && black >= 0.0 && white <= 65535.0) {
        return Err(format!("black {black} and white {white} are no range"));
    }
    let crop = img
        .crop_area
        .or(img.active_area)
        .map_or((0, 0, img.width, img.height), |r| (r.p.x, r.p.y, r.d.w, r.d.h));
    if crop.0 + crop.2 > img.width || crop.1 + crop.3 > img.height {
        return Err("the file's crop lies outside its mosaic".into());
    }
    let frame = frame(&sensor, crop).ok_or("no frame the tile mirrors on")?;
    let Frame { x: fx, y: fy, width: w, height: h, tile } = frame;

    // The photosites, 0 at black and 1 at white.
    let mut m = Plane::zeros(w, h);
    m.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let line = &raw[(fy + y) * img.width + fx..][..w];
        for (x, (v, r)) in row.iter_mut().zip(line).enumerate() {
            *v = (f64::from(*r) - black_at(fx + x, fy + y)) / (white - black);
        }
    });
    let burnt: usize = (0..h)
        .into_par_iter()
        .map(|y| {
            raw[(fy + y) * img.width + fx..][..w].iter().filter(|v| f64::from(**v) >= white - 1.0).count()
        })
        .sum();

    // The balance: the camera's, or the one that makes the frame grey.
    let from_file = [img.wb_coeffs[0], img.wb_coeffs[1], img.wb_coeffs[2]].map(f64::from);
    let balance_from_file = from_file.iter().all(|v| v.is_finite() && *v > 0.0);
    let balance = if balance_from_file {
        [from_file[0] / from_file[1], 1.0, from_file[2] / from_file[1]]
    } else {
        let mut sums = [(0.0, 0.0); 3];
        for y in 0..h {
            for (x, v) in m.row(y).iter().enumerate() {
                let c = tile.color_at(x as isize, y as isize) as usize;
                sums[c] = (sums[c].0 + v, sums[c].1 + 1.0);
            }
        }
        let mean = sums.map(|(s, n)| (s / n).max(1e-6));
        [mean[1] / mean[0], 1.0, mean[1] / mean[2]]
    };
    m.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, v) in row.iter_mut().enumerate() {
            *v *= balance[tile.color_at(x as isize, y as isize) as usize];
        }
    });
    // A photosite at white measures nothing but "at least this much".
    // Cut at the lowest of the three balanced ceilings, a burnt area is
    // flat and neutral instead of the balance's colour.
    let ceiling = balance.iter().copied().fold(f64::INFINITY, f64::min);
    let cut =
        |m: &Plane| Plane::from_vec(m.width, m.height, m.data.par_iter().map(|v| v.min(ceiling)).collect());

    // The mosaic of a picture under a tone curve is the mosaic under that
    // curve: separate there, and come back channel by channel.
    let tone = Curve::Srgb;
    let p = XParams::default();
    // The prior is that of the cut mosaic also where the highlights are
    // kept: a burnt area has the balance's colour and a hard edge, and
    // says nothing about the picture around it.
    say("searching the prior");
    let t = Instant::now();
    let start = Prior { lens: 0.3, ..Prior::default() };
    let (m, prior) = if settings.keep_highlights {
        let prior = xtrans::fit_prior(&tone.mosaic(&cut(&m)), &tile, &p, start, 8, [true; 6]);
        (tone.mosaic(&m), prior)
    } else {
        let m = tone.mosaic(&cut(&m));
        let prior = xtrans::fit_prior(&m, &tile, &p, start, 8, [true; 6]);
        (m, prior)
    };
    let search = t.elapsed().as_secs_f64();

    say("separating luminance and colour");
    let t = Instant::now();
    let sep = xtrans::separate_bank(&m, &tile, &p, &bank(prior), &Selection::default());
    let first = t.elapsed().as_secs_f64();

    say("sharpening the colour at its edges");
    let t = Instant::now();
    let sep = xtrans::refine(&m, &tile, &sep, &Refine::default());
    let second = t.elapsed().as_secs_f64();
    drop(m);
    let sep = sep.mapped(|v| tone.back(v));
    let sum = |pl: &Plane| pl.data.par_iter().map(|v| v.abs()).sum::<f64>();
    let sums = [sum(&sep.lum), sum(&sep.c1), sum(&sep.c2)];

    // Back to the camera's own channels, black at 0 and white at the top
    // of sixteen bits: over four steps to each of a 14-bit file's, so
    // nothing is rounded more coarsely than the photosites were. What the
    // noise puts below black is cut to it (see `WHITE`).
    let rgb = sep.rgb();
    drop(sep);
    let mut data = vec![0u16; w * h * 3];
    let below: usize = data
        .par_chunks_mut(3 * w)
        .enumerate()
        .map(|(y, row)| {
            let lines = [rgb.r.row(y), rgb.g.row(y), rgb.b.row(y)];
            let mut below = 0;
            for (x, px) in row.chunks_mut(3).enumerate() {
                for (c, v) in px.iter_mut().enumerate() {
                    let level = (lines[c][x] / balance[c] * f64::from(WHITE)).round();
                    below += usize::from(level < 0.0);
                    *v = level.clamp(0.0, f64::from(WHITE)) as u16;
                }
            }
            below
        })
        .sum();
    Ok(Developed {
        frame,
        data,
        below,
        balance,
        balance_from_file,
        prior,
        burnt,
        sums,
        seconds: [search, first, second],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xtrans::FUJI;

    /// The frame lies inside the crop, loses at most five columns and
    /// rows of it, and mirrors the tile at all four edges, wherever the
    /// crop begins and whatever its size.
    #[test]
    fn the_frame_mirrors_the_tile_inside_any_crop() {
        let sensor = XTile::from_name(FUJI).unwrap();
        for (x0, y0) in [(0, 0), (1, 5), (4, 2), (8, 14), (3, 3)] {
            for (cw, ch) in [(60, 48), (61, 47), (65, 50), (6240, 4160)] {
                let f = frame(&sensor, (x0, y0, cw, ch)).expect("a frame");
                assert!(f.x >= x0 && f.y >= y0, "{f:?}");
                assert!(f.x + f.width <= x0 + cw && f.y + f.height <= y0 + ch, "{f:?}");
                assert!(cw - f.width <= 7 && ch - f.height <= 7, "{f:?} of {cw}x{ch}");
                assert_eq!(f.tile, sensor.shifted(f.x as isize, f.y as isize));
                assert!(f.tile.mirrors(f.width, f.height), "{f:?}");
            }
        }
        assert!(frame(&sensor, (0, 0, 20, 400)).is_none());
    }

    #[test]
    fn the_bank_holds_twenty_one_priors_around_the_one_found() {
        let found = Prior { chroma: [0.02, 0.04], noise: 0.016, lens: 0.5, ..Prior::default() };
        let priors = bank(found);
        assert_eq!(priors.len(), 21);
        assert_eq!(priors[0], found);
        assert!(priors[1..5].iter().all(|q| q.oriented.is_none()));
        assert!(priors[5..13].iter().all(|q| q.oriented.is_some() && q.chroma == [0.08, 0.16]));
        assert!(priors[..13].iter().all(|q| q.noise == found.noise && q.lens == found.lens));
        // The lines: a flatter luminance, their own colour, less noise, a
        // sharper lens.
        for (q, plain) in priors[13..].iter().zip(&priors[5..13]) {
            assert_eq!(q.oriented, plain.oriented);
            assert_eq!((q.chroma, q.alpha_c), ([0.0625; 2], 0.0));
            assert_eq!((q.noise, q.lens), (0.001, 0.25));
            assert_eq!((q.alpha, q.f0), (1.0, found.f0));
        }
    }
}

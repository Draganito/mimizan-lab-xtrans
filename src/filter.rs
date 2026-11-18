//! Gaussian kernels and separable convolution with mirrored borders.

use crate::plane::Plane;
use rayon::prelude::*;

/// Normalised Gaussian of standard deviation `sigma` pixels, radius
/// `ceil(3σ)`, at least 1. A sigma that is not a positive finite number
/// gives the identity tap.
pub fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    if sigma.is_nan() || sigma <= 0.0 || sigma.is_infinite() {
        return vec![1.0];
    }
    let r = (3.0 * sigma).ceil().max(1.0) as usize;
    let k: Vec<f64> = (0..=2 * r)
        .map(|i| {
            let d = i as f64 - r as f64;
            (-d * d / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let s: f64 = k.iter().sum();
    k.into_iter().map(|v| v / s).collect()
}

/// reflect-101 index: ... 2 1 | 0 1 2 ... n-2 n-1 | n-2 n-3 ...
/// A length of one (or zero) always gives 0.
#[inline]
pub fn reflect(i: isize, n: usize) -> usize {
    let n = n as isize;
    if n <= 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut j = i.rem_euclid(period);
    if j >= n {
        j = period - j;
    }
    j as usize
}

/// Outputs computed together in registers: the taps stream past a fixed block
/// of accumulators (independent lanes vectorise; a per-pixel dot product does
/// not, since f64 sums are never reassociated). Every output still sums its
/// taps in index order t = 0..n, so results are deterministic and independent
/// of the thread count.
const LANES: usize = 16;
/// Output rows per band in the column pass: the band plus its tap halo of
/// source rows, one x-strip wide, stays in L2.
const Y_BAND: usize = 32;
const X_STRIP: usize = 512;

#[inline(always)]
fn fma_lanes(acc: &mut [f64; LANES], kv: f64, s: &[f64; LANES]) {
    for j in 0..LANES {
        acc[j] += kv * s[j];
    }
}

/// Convolve rows with `k` (odd length), reflect-101 borders, output has the
/// same size. `carrier(x, y)` multiplies each source sample first.
pub fn convolve_rows_mod_into(
    src: &Plane,
    k: &[f64],
    carrier: impl Fn(usize, usize) -> f64 + Sync,
    out: &mut Plane,
) {
    let w = src.width;
    let half = k.len() / 2;
    assert!(out.width == w && out.height == src.height, "convolve_rows: size mismatch");
    out.data.par_chunks_mut(w).enumerate().for_each_init(
        || vec![0.0f64; w + 2 * half + LANES],
        |pad, (y, orow)| {
            let row = src.row(y);
            for (i, p) in pad.iter_mut().enumerate().take(w + 2 * half) {
                let xr = reflect(i as isize - half as isize, w);
                *p = carrier(xr, y) * row[xr];
            }
            let mut x0 = 0;
            while x0 < w {
                let n = (w - x0).min(LANES);
                let mut acc = [0.0f64; LANES];
                for (t, &kv) in k.iter().enumerate() {
                    let s: &[f64; LANES] = pad[x0 + t..x0 + t + LANES].try_into().expect("pad slack");
                    fma_lanes(&mut acc, kv, s);
                }
                orow[x0..x0 + n].copy_from_slice(&acc[..n]);
                x0 += LANES;
            }
        },
    );
}

/// [`convolve_rows_mod_into`] into a new plane.
pub fn convolve_rows_mod(src: &Plane, k: &[f64], carrier: impl Fn(usize, usize) -> f64 + Sync) -> Plane {
    let mut out = Plane::zeros(src.width, src.height);
    convolve_rows_mod_into(src, k, carrier, &mut out);
    out
}

/// Row convolution without a carrier.
pub fn convolve_rows(src: &Plane, k: &[f64]) -> Plane {
    convolve_rows_mod(src, k, |_, _| 1.0)
}

/// Convolve columns with `k` (odd length), reflect-101 borders. `row_sign(sy)`
/// multiplies source row `sy`. Same tap order per output as the row pass.
pub fn convolve_cols_signed_into(
    src: &Plane,
    k: &[f64],
    row_sign: impl Fn(usize) -> f64 + Sync,
    out: &mut Plane,
) {
    let (w, h) = (src.width, src.height);
    let half = k.len() as isize / 2;
    let taps = k.len();
    assert!(out.width == w && out.height == h, "convolve_cols: size mismatch");
    out.data.par_chunks_mut(w * Y_BAND).enumerate().for_each_init(
        || (Vec::<usize>::new(), Vec::<f64>::new()),
        |(sy_tab, kk), (band, oband)| {
            let y0 = band * Y_BAND;
            let rows = oband.len() / w;
            // Source row and signed tap per (dy, t), once per band.
            sy_tab.clear();
            kk.clear();
            for dy in 0..rows {
                for (t, &kv) in k.iter().enumerate() {
                    let sy = reflect((y0 + dy) as isize + t as isize - half, h);
                    sy_tab.push(sy);
                    kk.push(kv * row_sign(sy));
                }
            }
            for x0 in (0..w).step_by(X_STRIP) {
                let x1 = (x0 + X_STRIP).min(w);
                for dy in 0..rows {
                    let orow = &mut oband[dy * w..(dy + 1) * w];
                    let tab = &sy_tab[dy * taps..(dy + 1) * taps];
                    let kt = &kk[dy * taps..(dy + 1) * taps];
                    let mut xb = x0;
                    while xb < x1 {
                        let n = (x1 - xb).min(LANES);
                        let mut acc = [0.0f64; LANES];
                        if n == LANES {
                            for (&sy, &kv) in tab.iter().zip(kt) {
                                let s: &[f64; LANES] =
                                    src.row(sy)[xb..xb + LANES].try_into().expect("full block");
                                fma_lanes(&mut acc, kv, s);
                            }
                        } else {
                            for (&sy, &kv) in tab.iter().zip(kt) {
                                let s = &src.row(sy)[xb..xb + n];
                                for j in 0..n {
                                    acc[j] += kv * s[j];
                                }
                            }
                        }
                        orow[xb..xb + n].copy_from_slice(&acc[..n]);
                        xb += LANES;
                    }
                }
            }
        },
    );
}

/// [`convolve_cols_signed_into`] into a new plane.
pub fn convolve_cols_signed(src: &Plane, k: &[f64], row_sign: impl Fn(usize) -> f64 + Sync) -> Plane {
    let mut out = Plane::zeros(src.width, src.height);
    convolve_cols_signed_into(src, k, row_sign, &mut out);
    out
}

/// Column convolution without a row sign.
pub fn convolve_cols(src: &Plane, k: &[f64]) -> Plane {
    convolve_cols_signed(src, k, |_| 1.0)
}

/// Separable low-pass: rows then columns.
pub fn lowpass(src: &Plane, k: &[f64]) -> Plane {
    convolve_cols(&convolve_rows(src, k), k)
}

/// Naive 2-D reference for tests (same kernel both axes, reflect-101).
pub fn lowpass_reference(src: &Plane, k: &[f64]) -> Plane {
    let (w, h) = (src.width, src.height);
    let half = (k.len() / 2) as isize;
    let mut out = Plane::zeros(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (ty, &ky) in k.iter().enumerate() {
                let sy = reflect(y as isize + ty as isize - half, h);
                for (tx, &kx) in k.iter().enumerate() {
                    let sx = reflect(x as isize + tx as isize - half, w);
                    acc += ky * kx * src.at(sx, sy);
                }
            }
            out.set(x, y, acc);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflect_101() {
        assert_eq!(reflect(-1, 5), 1);
        assert_eq!(reflect(-2, 5), 2);
        assert_eq!(reflect(5, 5), 3);
        assert_eq!(reflect(6, 5), 2);
        assert_eq!(reflect(2, 5), 2);
    }

    #[test]
    fn a_gaussian_kernel_sums_to_one() {
        for sigma in [0.5, 1.0, 2.0, 6.0] {
            let k = gaussian_kernel(sigma);
            assert_eq!(k.len(), 2 * (3.0 * sigma).ceil() as usize + 1);
            assert!((k.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }
        assert_eq!(gaussian_kernel(0.0), vec![1.0]);
        assert_eq!(gaussian_kernel(f64::NAN), vec![1.0]);
    }

    #[test]
    fn separable_matches_reference() {
        let (w, h) = (23, 17);
        let mut p = Plane::zeros(w, h);
        for y in 0..h {
            for x in 0..w {
                p.set(x, y, ((x * 7 + y * 13) % 11) as f64 / 11.0 + if (x + y) % 2 == 0 { 0.3 } else { 0.0 });
            }
        }
        let k = gaussian_kernel(2.5);
        let a = lowpass(&p, &k);
        let b = lowpass_reference(&p, &k);
        for (x, y) in a.data.iter().zip(&b.data) {
            assert!((x - y).abs() < 1e-12);
        }
    }
}

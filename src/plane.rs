//! Row-major f64 image planes.

/// A single-channel image, row-major, `f64` per sample.
///
/// The fields are public so that the hot loops can run over `data` directly;
/// the invariant `data.len() == width * height` is kept by every constructor
/// and must be kept by anyone who replaces `data`.
#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    /// Samples, row `y` at `data[y * width..(y + 1) * width]`.
    pub data: Vec<f64>,
}

impl Plane {
    /// A plane of zeros. Panics only if `width * height` overflows `usize`.
    pub fn zeros(width: usize, height: usize) -> Self {
        let n = width.checked_mul(height).expect("plane dimensions overflow usize");
        Self { width, height, data: vec![0.0; n] }
    }

    /// Wrap a buffer of exactly `width * height` samples. Panics on a
    /// mismatch.
    pub fn from_vec(width: usize, height: usize, data: Vec<f64>) -> Self {
        let n = width.checked_mul(height).expect("plane dimensions overflow usize");
        assert!(data.len() == n, "plane is {width}×{height} ({n} samples) but the buffer has {}", data.len());
        Self { width, height, data }
    }

    /// Sample at `(x, y)`; bounds-checked like the slice it reads.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f64 {
        self.data[y * self.width + x]
    }

    /// Store `v` at `(x, y)`; bounds-checked like the slice it writes.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: f64) {
        self.data[y * self.width + x] = v;
    }

    /// Row `y` as a slice of `width` samples.
    pub fn row(&self, y: usize) -> &[f64] {
        &self.data[y * self.width..(y + 1) * self.width]
    }

    /// Row `y` as a mutable slice of `width` samples.
    pub fn row_mut(&mut self, y: usize) -> &mut [f64] {
        &mut self.data[y * self.width..(y + 1) * self.width]
    }
}

/// Three linear planes of the same size: a picture in red, green and blue.
#[derive(Clone, Debug)]
pub struct Rgb {
    pub r: Plane,
    pub g: Plane,
    pub b: Plane,
}

impl Rgb {
    /// Black picture of `w × h`.
    pub fn new(w: usize, h: usize) -> Self {
        Self { r: Plane::zeros(w, h), g: Plane::zeros(w, h), b: Plane::zeros(w, h) }
    }
    pub fn width(&self) -> usize {
        self.r.width
    }
    pub fn height(&self) -> usize {
        self.r.height
    }
    /// Store one pixel.
    pub fn set(&mut self, x: usize, y: usize, r: f64, g: f64, b: f64) {
        self.r.set(x, y, r);
        self.g.set(x, y, g);
        self.b.set(x, y, b);
    }
}

#[cfg(test)]
mod tests {
    use super::Plane;

    #[test]
    #[should_panic(expected = "has 1")]
    fn a_short_buffer_is_refused() {
        let _ = Plane::from_vec(2, 2, vec![1.0]);
    }
}

//! One RAF to one DNG: the whole way, as the command line and a window
//! both go it.

use crate::convert::{self, Settings};
use crate::dng;
use crate::xtrans::Prior;
use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// What the DNG says it was written by.
pub const SOFTWARE: &str = concat!("xtrans2dng ", env!("CARGO_PKG_VERSION"), " (Mimizan Lab X-Trans)");

/// How one file is converted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Leave burnt-out areas as recorded ([`Settings::keep_highlights`]).
    pub keep_highlights: bool,
    /// Write the picture uncompressed.
    pub uncompressed: bool,
    /// Write over a DNG that is already there.
    pub force: bool,
}

/// What [`convert`] tells while it works, in this order.
#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
    /// The RAF is read and its mosaic decoded.
    Decoded { make: &'a str, model: &'a str, width: usize, height: usize },
    /// What is being done now, in the words of [`convert::develop`].
    Doing(&'a str),
    /// The share of the separation that is done; it is most of the time.
    Separated(f64),
    /// The picture is there and the DNG is being written.
    Writing,
}

/// What became of one file.
#[derive(Clone, Debug)]
pub struct Report {
    pub make: String,
    pub model: String,
    /// Width and height of the mosaic.
    pub photosites: (usize, usize),
    /// Width and height of the picture in the DNG.
    pub pixels: (usize, usize),
    /// The prior the mosaic was most probable under.
    pub prior: Prior,
    /// Whether the balance is the camera's ([`convert::Developed`]).
    pub balance_from_file: bool,
    /// Photosites of the frame at the file's white level.
    pub burnt: usize,
    /// Values the noise had put below black, cut to 0.
    pub below: usize,
    /// Sums of the magnitudes of luminance and both chrominances.
    pub sums: [f64; 3],
    /// Seconds for the search of the prior, the first and the second step.
    pub seconds: [f64; 3],
    /// Seconds for the whole file, reading and writing included.
    pub total: f64,
}

impl Report {
    /// Share of the frame's photosites that are burnt out, of 1.
    pub fn burnt_share(&self) -> f64 {
        self.burnt as f64 / (self.pixels.0 * self.pixels.1) as f64
    }

    /// Share of the picture's values that lay below black, of 1.
    pub fn below_share(&self) -> f64 {
        self.below as f64 / (3 * self.pixels.0 * self.pixels.1) as f64
    }
}

/// The DNG of `raf` beside it.
pub fn beside(raf: &Path) -> PathBuf {
    raf.with_extension("dng")
}

/// Convert `raf` to the DNG `to`. The DNG is written under another name
/// and renamed when it is whole, so a DNG that is there is finished.
pub fn convert(raf: &Path, to: &Path, options: &Options, tell: &dyn Fn(Event)) -> Result<Report, String> {
    if to.exists() && !options.force {
        return Err(format!("{} is there already (--force writes over it)", to.display()));
    }
    if fs::canonicalize(to).ok().is_some_and(|t| fs::canonicalize(raf).ok() == Some(t)) {
        return Err("the DNG would be written over the RAF".into());
    }
    let started = Instant::now();
    let source = RawSource::new(raf).map_err(|e| format!("cannot be read: {e}"))?;
    let decoder =
        rawler::get_decoder(&source).map_err(|e| format!("not a raw file this program reads: {e}"))?;
    let img = decoder
        .raw_image(&source, &RawDecodeParams::default(), false)
        .map_err(|e| format!("cannot be decoded: {e}"))?;
    let (make, model, photosites) =
        (img.clean_make.clone(), img.clean_model.clone(), (img.width, img.height));
    tell(Event::Decoded { make: &make, model: &model, width: img.width, height: img.height });

    let settings = Settings { keep_highlights: options.keep_highlights };
    let dev = convert::develop_watched(&img, &settings, &|what| tell(Event::Doing(what)), &|share| {
        tell(Event::Separated(share));
    })?;
    let report = Report {
        make,
        model,
        photosites,
        pixels: (dev.frame.width, dev.frame.height),
        prior: dev.prior,
        balance_from_file: dev.balance_from_file,
        burnt: dev.burnt,
        below: dev.below,
        sums: dev.sums,
        seconds: dev.seconds,
        total: 0.0,
    };

    tell(Event::Writing);
    // Under another name until it is whole: a DNG that is there is done.
    let part = PathBuf::from(format!("{}.part", to.display()));
    let written = File::create(&part).map_err(|e| format!("{}: {e}", part.display())).and_then(|file| {
        let mut out = BufWriter::new(file);
        dng::write(&mut out, &source, decoder.as_ref(), img, dev, !options.uncompressed, SOFTWARE)?;
        let file = out.into_inner().map_err(|e| format!("{}: {e}", part.display()))?;
        file.sync_all().map_err(|e| format!("{}: {e}", part.display()))
    });
    if let Err(e) = written.and_then(|()| fs::rename(&part, to).map_err(|e| format!("{}: {e}", to.display())))
    {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    Ok(Report { total: started.elapsed().as_secs_f64(), ..report })
}

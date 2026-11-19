//! `xtrans2dng`: Fuji X-Trans RAF in, linear DNG out.

use clap::Parser;
use mimizan_lab_xtrans::file::{self, Event, Options};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Fuji X-Trans RAF to linear DNG.
///
/// The mosaic is not interpolated: luminance and colour are separated by
/// the frequencies the X-Trans tile carries them on, under the prior the
/// mosaic itself makes most probable. The DNG holds red, green and blue
/// for every pixel in the camera's own colours, not balanced and without
/// a tone curve; a raw developer opens it like the RAF.
///
/// A 26-megapixel file takes about three quarters of a minute on eight
/// cores and 8 GB of memory.
#[derive(Parser, Debug)]
#[command(name = "xtrans2dng", version)]
struct Cli {
    /// The RAF files; each gives a DNG of its name beside it.
    #[arg(required = true, value_name = "RAF")]
    files: Vec<PathBuf>,

    /// Where the DNG goes: a directory, or with one RAF a file name.
    #[arg(short, long, value_name = "PATH")]
    out: Option<PathBuf>,

    /// Write over a DNG that is already there.
    #[arg(short, long)]
    force: bool,

    /// Leave burnt-out areas as recorded, for the raw developer's
    /// highlight recovery. By default they are cut to flat and neutral.
    #[arg(long)]
    keep_highlights: bool,

    /// Write the picture uncompressed (half as large again).
    #[arg(long)]
    uncompressed: bool,

    /// Say nothing but errors.
    #[arg(short, long)]
    quiet: bool,
}

/// Where the DNG of `raf` goes.
fn target(raf: &Path, out: Option<&Path>, many: bool) -> Result<PathBuf, String> {
    let name =
        || raf.file_stem().map(|s| PathBuf::from(s).with_extension("dng")).ok_or("the RAF has no name");
    match out {
        None => Ok(raf.with_extension("dng")),
        Some(dir) if dir.is_dir() => Ok(dir.join(name()?)),
        Some(file) if !many => Ok(file.to_path_buf()),
        Some(dir) => Err(format!("{} is no directory, and there is more than one RAF", dir.display())),
    }
}

fn convert_one(raf: &Path, to: &Path, cli: &Cli) -> Result<(), String> {
    let say = |s: &str| {
        if !cli.quiet {
            eprintln!("  {s}");
        }
    };
    let options =
        Options { keep_highlights: cli.keep_highlights, uncompressed: cli.uncompressed, force: cli.force };
    let r = file::convert(raf, to, &options, &|event| match event {
        Event::Decoded { make, model, width, height } => {
            say(&format!("{make} {model}, {width} x {height} photosites"));
        }
        Event::Doing(what) => say(what),
        Event::Separated(_) | Event::Writing => {}
    })?;
    let (q, t) = (r.prior, r.seconds);
    say(&format!(
        "prior: luminance falls with f^{:.2}, colour with f^{:.2}; colour {:.4} and {:.4}, noise {:.1e}, lens {:.2} px",
        q.alpha, q.alpha_c, q.chroma[0], q.chroma[1], q.noise, q.lens
    ));
    if !r.balance_from_file {
        say("the file gives no white balance: balanced to a grey mean");
    }
    if r.burnt > 0 {
        let what = if cli.keep_highlights { "kept as recorded" } else { "cut to neutral" };
        say(&format!("{:.3} % of the photosites are burnt out, {what}", 100.0 * r.burnt_share()));
    }
    if r.below > 0 {
        say(&format!(
            "{:.3} % of the values lay below black in the noise, cut to black",
            100.0 * r.below_share()
        ));
    }
    say(&format!("sums: lum {:.9e} c1 {:.9e} c2 {:.9e}", r.sums[0], r.sums[1], r.sums[2]));
    say(&format!(
        "{} x {} pixels to {} in {:.0} s (prior {:.1}, separation {:.1}, colour edges {:.1})",
        r.pixels.0,
        r.pixels.1,
        to.display(),
        r.total,
        t[0],
        t[1],
        t[2]
    ));
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut failed = 0;
    for raf in &cli.files {
        if !cli.quiet {
            eprintln!("{}", raf.display());
        }
        let done =
            target(raf, cli.out.as_deref(), cli.files.len() > 1).and_then(|to| convert_one(raf, &to, &cli));
        if let Err(e) = done {
            eprintln!("{}: {e}", raf.display());
            failed += 1;
        }
    }
    if failed > 0 {
        eprintln!("{failed} of {} not converted", cli.files.len());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

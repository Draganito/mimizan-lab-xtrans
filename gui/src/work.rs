//! The conversions, off the window's thread: one file at a time, because
//! one picture takes several GB of memory and every core there is.

use crate::queue::Id;
use mimizan_lab_xtrans::convert;
use mimizan_lab_xtrans::file::{self, Event, Options, Report};
use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use sysinfo::System;

/// Memory one photosite of the frame takes at the peak of a conversion,
/// bytes. Measured: 7.2 to 7.8 GB for 26 million photosites, 12 GB for
/// 40 million.
const BYTES_PER_PHOTOSITE: u64 = 300;

/// The memory a picture of this many photosites takes, bytes.
pub fn needed(photosites: u64) -> u64 {
    photosites * BYTES_PER_PHOTOSITE
}

/// What is free now, bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub free: u64,
    pub swap: u64,
}

impl Memory {
    pub fn read(system: &mut System) -> Self {
        system.refresh_memory();
        Self { free: system.available_memory(), swap: system.free_swap() }
    }

    /// Whether a picture that takes `need` may be begun. A program that
    /// is given no memory ends on the spot, and one that swaps several GB
    /// holds the whole computer up, so it is asked before.
    pub fn takes(self, need: u64, use_swap: bool) -> Result<(), String> {
        let room = if use_swap { self.free + self.swap } else { self.free };
        if need <= room {
            return Ok(());
        }
        let more =
            if !use_swap && need <= self.free + self.swap { " (the swap would hold the rest)" } else { "" };
        Err(format!("needs about {} of memory, {} are free{more}", gb(need), gb(self.free)))
    }
}

/// Bytes as GB with one decimal.
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

pub struct Job {
    pub id: Id,
    pub raf: PathBuf,
    pub to: PathBuf,
    pub options: Options,
    pub use_swap: bool,
}

/// What the thread tells of the job it has.
pub enum Told {
    /// How far it is, of 1, and what is being done if that has changed.
    Step {
        id: Id,
        words: Option<String>,
        share: f32,
    },
    Done {
        id: Id,
        to: PathBuf,
        result: Result<Box<Report>, String>,
    },
}

/// Where a file is, of 1, when `event` comes, and what is done from
/// there on. The shares are those of the time each step takes.
pub fn step(event: &Event) -> (Option<String>, f32) {
    match event {
        Event::Decoded { .. } => (None, 0.03),
        Event::Doing(what) => {
            let share = match *what {
                "searching the prior" => 0.03,
                "separating luminance and colour" => 0.13,
                _ => 0.85,
            };
            (Some((*what).to_owned()), share)
        }
        Event::Separated(share) => (None, 0.13 + 0.72 * share.clamp(0.0, 1.0) as f32),
        Event::Writing => (Some("writing the DNG".to_owned()), 0.96),
    }
}

/// How many photosites of `raf` are developed; its head tells.
fn photosites(raf: &Path) -> Option<u64> {
    let source = RawSource::new(raf).ok()?;
    let decoder = rawler::get_decoder(&source).ok()?;
    let img = decoder.raw_image(&source, &RawDecodeParams::default(), true).ok()?;
    let frame = convert::frame_of(&img).ok()?;
    Some((frame.width * frame.height) as u64)
}

fn convert(job: &Job, system: &mut System, tell: &dyn Fn(Told)) -> Result<Box<Report>, String> {
    // A file whose head cannot be read is left to the conversion to
    // say what is wrong with it.
    if let Some(sites) = photosites(&job.raf) {
        Memory::read(system).takes(needed(sites), job.use_swap)?;
    }
    if let Some(folder) = job.to.parent().filter(|f| !f.as_os_str().is_empty() && !f.is_dir()) {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    file::convert(&job.raf, &job.to, &job.options, &|event| {
        let (words, share) = step(&event);
        tell(Told::Step { id: job.id, words, share });
    })
    .map(Box::new)
    .map_err(|e| e.replace(" (--force writes over it)", ""))
}

pub struct Worker {
    jobs: Sender<Job>,
    pub told: Receiver<Told>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (jobs, taken) = mpsc::channel::<Job>();
        let (tell, told) = mpsc::channel();
        std::thread::Builder::new()
            .name("convert".into())
            .spawn(move || {
                let mut system = System::new();
                for job in taken {
                    let say = |told: Told| {
                        let _ = tell.send(told);
                        ctx.request_repaint();
                    };
                    let result = catch_unwind(AssertUnwindSafe(|| convert(&job, &mut system, &say)))
                        .unwrap_or_else(|_| Err("the program failed on this file".into()));
                    say(Told::Done { id: job.id, to: job.to, result });
                }
            })
            .expect("the converting thread starts");
        Self { jobs, told }
    }

    /// Give the thread its next file. The window gives one when the one
    /// before is done.
    pub fn begin(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

#[cfg(test)]
mod tests {
    use super::{gb, needed, step, Memory};
    use mimizan_lab_xtrans::file::Event;

    #[test]
    fn a_picture_is_begun_only_if_the_memory_holds_it() {
        // An X-T4 and an X-T5, on a computer with 10.5 GB free.
        let (xt4, xt5) = (needed(6231 * 4155), needed(7726 * 5146));
        assert_eq!((gb(xt4), gb(xt5)), ("7.8 GB".to_owned(), "11.9 GB".to_owned()));
        let memory = Memory { free: 10_500_000_000, swap: 15_000_000_000 };
        assert_eq!(memory.takes(xt4, false), Ok(()));
        assert_eq!(
            memory.takes(xt5, false).unwrap_err(),
            "needs about 11.9 GB of memory, 10.5 GB are free (the swap would hold the rest)"
        );
        assert_eq!(memory.takes(xt5, true), Ok(()));
        let small = Memory { free: 3_000_000_000, swap: 1_000_000_000 };
        assert_eq!(small.takes(xt4, true).unwrap_err(), "needs about 7.8 GB of memory, 3.0 GB are free");
    }

    #[test]
    fn a_file_only_moves_forward() {
        let events = [
            Event::Decoded { make: "Fujifilm", model: "X-T4", width: 6246, height: 4170 },
            Event::Doing("searching the prior"),
            Event::Doing("separating luminance and colour"),
            Event::Separated(0.0),
            Event::Separated(0.5),
            Event::Separated(1.0),
            Event::Doing("sharpening the colour at its edges"),
            Event::Writing,
        ];
        let shares: Vec<f32> = events.iter().map(|e| step(e).1).collect();
        assert!(shares.windows(2).all(|w| w[0] <= w[1]), "{shares:?}");
        assert!(shares[0] > 0.0 && *shares.last().unwrap() < 1.0);
        assert_eq!(step(&events[4]).0, None);
        assert_eq!(step(&events[7]).0.as_deref(), Some("writing the DNG"));
    }
}

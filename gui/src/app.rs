//! The window: the list of files at the left, the picture in the middle,
//! what the file says and where its DNG goes at the right.

use crate::load::{Info, Large, Loaded, Loader};
use crate::queue::{Entry, Id, Queue, Read, Status};
use crate::settings::{self, Settings};
use crate::theme::{self, Chip};
use crate::work::{self, Job, Memory, Told, Worker};
use egui::{
    pos2, vec2, Color32, Key, Modifiers, Rect, RichText, Sense, TextureHandle, TextureOptions, Ui, Vec2,
};
use egui_file_dialog::{DialogState, FileDialog, OpeningMode};
use mimizan_lab_xtrans::file::Report;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::System;

/// Height of a file's row in the list, points.
const ROW: f32 = 58.0;
/// The room a row keeps for the small picture.
const THUMB: Vec2 = vec2(66.0, 44.0);
/// Side of the box a file is marked in.
const MARK: f32 = 16.0;
/// Around the picture when it is seen whole.
const MARGIN: f32 = 16.0;
/// The picture is enlarged up to this many screen pixels for one of its own.
const MAX_ZOOM: f32 = 8.0;

/// A large picture is shown pixel for pixel when enlarged, and smoothly
/// when it is smaller than it is.
const LARGE: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

struct Picture {
    texture: TextureHandle,
    /// Width and height, pixels.
    size: Vec2,
}

impl Picture {
    fn new(ctx: &egui::Context, name: &str, image: egui::ColorImage) -> Self {
        let size = vec2(image.size[0] as f32, image.size[1] as f32);
        Self { texture: ctx.load_texture(name, image, LARGE), size }
    }
}

struct Developed {
    picture: Picture,
    /// Width and height of the picture in the DNG.
    pixels: (usize, usize),
    gain: f32,
}

/// The file that is looked at, and how.
struct Stage {
    id: Option<Id>,
    /// The loader's count of the files looked at: what comes back with
    /// another is late.
    turn: u64,
    preview: Option<Result<Picture, String>>,
    developed: Option<Result<Developed, String>>,
    /// The DNG is being read for the screen.
    developing: bool,
    /// The DNG is wanted, not the preview, as soon as it is there.
    want_dng: bool,
    /// Whether the DNG was what the last frame showed.
    shows_dng: bool,
    dng_bytes: Option<u64>,
    /// The picture is seen whole.
    fit: bool,
    /// Width of the whole picture on the screen, points. Preview and DNG
    /// are drawn this wide, whatever their pixels, so one lies on the
    /// other.
    width: f32,
    /// The point of the picture at the middle of the room, of 1.
    centre: Vec2,
    /// As of the last frame: the room the picture lies in, the width at
    /// which a pixel of the picture is one of the screen, and the scale
    /// shown.
    room: Rect,
    full_width: f32,
    scale: f32,
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            id: None,
            turn: 0,
            preview: None,
            developed: None,
            developing: false,
            want_dng: false,
            shows_dng: false,
            dng_bytes: None,
            fit: true,
            width: 0.0,
            centre: vec2(0.5, 0.5),
            room: Rect::ZERO,
            full_width: 0.0,
            scale: 0.0,
        }
    }
}

/// The files being converted, one after the other.
struct Run {
    order: VecDeque<Id>,
    total: usize,
    done: usize,
    failed: usize,
    /// The file the thread has, and the DNG it writes.
    working: Option<(Id, PathBuf)>,
    /// No file is begun after the one in work.
    stop: bool,
    since: Instant,
}

/// What the dialog is open for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Files,
    Folder,
    Output,
}

/// When the window takes a picture of itself (`XTRANS_GUI_SHOT`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum When {
    /// The files are read and the one looked at is shown.
    Idle,
    /// A file is half converted.
    Working,
    /// The run is over and the DNG is shown.
    Done,
}

/// The window photographs itself and closes: how it looks is checked
/// without a hand on it.
struct Shot {
    to: PathBuf,
    when: When,
    /// Enlarge to this many per cent first.
    zoom: Option<f32>,
    /// Show the preview though the DNG is there.
    preview: bool,
    /// Open this first: the dialog for files, or the question at closing.
    open: Option<String>,
    begun: bool,
    /// Frames since all was ready; the picture is asked for after a few.
    ready: u32,
}

impl Shot {
    fn from_env() -> Option<Self> {
        let to = PathBuf::from(std::env::var_os("XTRANS_GUI_SHOT")?);
        let when = match std::env::var("XTRANS_GUI_SHOT_WHEN").as_deref() {
            Ok("working") => When::Working,
            Ok("done") => When::Done,
            _ => When::Idle,
        };
        let zoom = std::env::var("XTRANS_GUI_SHOT_ZOOM").ok().and_then(|z| z.parse().ok());
        let preview = std::env::var_os("XTRANS_GUI_SHOT_PREVIEW").is_some();
        let open = std::env::var("XTRANS_GUI_SHOT_OPEN").ok();
        Some(Self { to, when, zoom, preview, open, begun: false, ready: 0 })
    }
}

pub struct App {
    queue: Queue,
    current: Option<Id>,
    /// Where a range of marks begins: the file last clicked.
    anchor: Option<Id>,
    /// Bring the current file into view in the list.
    reveal: bool,
    /// Show each file as it is finished, until another is chosen by hand.
    follow: bool,
    settings: Settings,
    saved: Settings,
    /// The settings are written to the disk when they change.
    keep: bool,
    loader: Loader,
    worker: Worker,
    run: Option<Run>,
    /// What last happened, said at the right of the toolbar.
    note: Option<String>,
    stage: Stage,
    dialog: FileDialog,
    picking: Option<Pick>,
    system: System,
    memory: Memory,
    memory_read: Instant,
    quit_asked: bool,
    quit_allowed: bool,
    shot: Option<Shot>,
}

/// "25.9 MP".
fn megapixels((w, h): (usize, usize)) -> String {
    format!("{:.1} MP", (w * h) as f64 / 1e6)
}

/// "6231 × 4155".
fn by((w, h): (usize, usize)) -> String {
    format!("{w} × {h}")
}

/// "56.3 MB".
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

/// "44 s", "2 min 10 s".
fn duration(seconds: f64) -> String {
    let s = seconds.round() as u64;
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {} s", s / 60, s % 60)
    }
}

/// "1 file", "3 files".
fn files(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

/// The file being written before it has its name.
fn part_of(to: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", to.display()))
}

/// What a file's row says in its last line, and in which colour.
fn status_line(entry: &Entry) -> (String, Color32) {
    match (&entry.status, &entry.info) {
        (Status::Working { words, share, .. }, _) => {
            (format!("{:.0} % · {words}", 100.0 * share), theme::AMBER)
        }
        (Status::Queued, _) => ("waits its turn".into(), theme::INK_QUIET),
        (Status::Done(report), _) => (format!("converted in {}", duration(report.total)), theme::GOOD),
        (Status::Failed(why), _) => (why.clone(), theme::BAD),
        (Status::Waiting, Read::Failed(why)) => (why.clone(), theme::BAD),
        (Status::Waiting, Read::Ready(info)) if info.picture.is_err() => {
            (info.picture.clone().err().unwrap_or_default(), theme::BAD)
        }
        (Status::Waiting, _) if entry.dng.is_some() => ("DNG is there".into(), theme::INK_QUIET),
        (Status::Waiting, _) => ("not converted".into(), theme::INK_SMALL),
    }
}

/// What was clicked in a file's row.
#[derive(Clone, Copy)]
enum Click {
    Row,
    Mark,
}

impl App {
    /// Take up the settings of the last time and the files named at the
    /// start.
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Vec<PathBuf>) -> Self {
        let shot = Shot::from_env();
        // A window that photographs itself neither reads nor leaves
        // settings; it writes where `XTRANS_GUI_SHOT_OUT` says.
        let settings = if shot.is_some() {
            let folder = std::env::var_os("XTRANS_GUI_SHOT_OUT").map(PathBuf::from);
            Settings { into_folder: folder.is_some(), folder, ..Default::default() }
        } else {
            Settings::load()
        };
        let keep = shot.is_none();
        Self::with(&cc.egui_ctx, initial, settings, keep, shot)
    }

    /// Start the two threads. The settings are written when they change
    /// if `keep` says so.
    fn with(
        ctx: &egui::Context,
        initial: Vec<PathBuf>,
        settings: Settings,
        keep: bool,
        shot: Option<Shot>,
    ) -> Self {
        theme::apply(ctx);
        let mut dialog = FileDialog::new()
            .add_file_filter_extensions("Fuji RAF", vec!["raf", "RAF", "Raf"])
            .default_file_filter("Fuji RAF");
        dialog.config_mut().opening_mode = OpeningMode::AlwaysInitialDir;
        let mut system = System::new();
        let memory = Memory::read(&mut system);
        let mut app = Self {
            queue: Queue::default(),
            current: None,
            anchor: None,
            reveal: false,
            follow: false,
            saved: settings.clone(),
            settings,
            keep,
            loader: Loader::spawn(ctx.clone()),
            worker: Worker::spawn(ctx.clone()),
            run: None,
            note: None,
            stage: Stage::default(),
            dialog,
            picking: None,
            system,
            memory,
            memory_read: Instant::now(),
            quit_asked: false,
            quit_allowed: false,
            shot,
        };
        if !initial.is_empty() {
            app.add(&initial);
        }
        app
    }

    fn entry(&self) -> Option<&Entry> {
        self.current.and_then(|id| self.queue.get(id))
    }

    /// Take files and folders into the list, and say what became of them.
    fn add(&mut self, paths: &[PathBuf]) {
        let settings = self.settings.clone();
        let added = self.queue.add(paths, &|raf| settings.target(raf));
        for id in &added.ids {
            if let Some(entry) = self.queue.get(*id) {
                self.loader.info(*id, entry.raf.clone());
            }
        }
        let mut said = vec![format!("{} added", files(added.ids.len()))];
        if added.known > 0 {
            said.push(format!("{} in the list already", added.known));
        }
        if added.other > 0 {
            said.push(format!("{} no RAF", added.other));
        }
        self.note = Some(said.join(" · "));
        // The next dialog opens where these came from.
        if let Some(first) = paths.first().and_then(|p| std::path::absolute(p).ok()) {
            let dir = if first.is_dir() { Some(first) } else { first.parent().map(Path::to_path_buf) };
            self.settings.last_dir = dir.or(self.settings.last_dir.take());
        }
        if self.current.is_none() {
            self.look_at(added.ids.first().copied());
        }
    }

    /// Look at another file: its preview, and its DNG if it has one.
    fn look_at(&mut self, id: Option<Id>) {
        self.current = id;
        // The view stays when the same file is looked at afresh.
        let view = (self.stage.id == id).then_some((self.stage.fit, self.stage.width, self.stage.centre));
        self.stage = Stage { id, ..Stage::default() };
        if let Some((fit, width, centre)) = view {
            (self.stage.fit, self.stage.width, self.stage.centre) = (fit, width, centre);
        }
        let Some(entry) = self.entry() else { return };
        let mut large = vec![Large::Preview(entry.raf.clone())];
        let dng = entry.dng.clone();
        if let Some(dng) = &dng {
            large.push(Large::Developed(dng.clone()));
        }
        self.stage.turn = self.loader.look_at(large);
        self.stage.developing = dng.is_some();
        self.stage.want_dng = dng.is_some();
        self.stage.dng_bytes = dng.and_then(|d| std::fs::metadata(d).ok()).map(|m| m.len());
    }

    /// The file looked at has a new DNG: read it for the screen and show
    /// it in the preview's place, which stays until it is there.
    fn show_dng(&mut self) {
        let Some(dng) = self.entry().and_then(|e| e.dng.clone()) else { return };
        self.stage.developed = None;
        self.stage.developing = true;
        self.stage.want_dng = true;
        self.stage.dng_bytes = std::fs::metadata(&dng).ok().map(|m| m.len());
        self.loader.also(Large::Developed(dng));
    }

    /// A file was chosen by hand.
    fn choose(&mut self, id: Id) {
        self.follow = false;
        self.anchor = Some(id);
        if self.current != Some(id) {
            self.look_at(Some(id));
        }
    }

    /// Take what the two threads have sent.
    fn collect(&mut self, ctx: &egui::Context) {
        while let Ok(loaded) = self.loader.loaded.try_recv() {
            match loaded {
                Loaded::Info { id, info, thumb } => {
                    let Some(entry) = self.queue.get_mut(id) else { continue };
                    entry.thumb =
                        thumb.map(|t| ctx.load_texture(format!("thumb-{id}"), t, TextureOptions::LINEAR));
                    entry.info = match info {
                        Ok(info) => {
                            // What cannot become a DNG is not marked for one.
                            entry.marked &= info.picture.is_ok();
                            Read::Ready(info)
                        }
                        Err(why) => {
                            entry.marked = false;
                            Read::Failed(why)
                        }
                    };
                }
                Loaded::Preview { turn, image } if turn == self.stage.turn => {
                    self.stage.preview = Some(image.map(|i| Picture::new(ctx, "preview", i)));
                }
                Loaded::Developed { turn, shown } if turn == self.stage.turn => {
                    self.stage.developing = false;
                    self.stage.developed = Some(shown.map(|s| Developed {
                        picture: Picture::new(ctx, "developed", s.image),
                        pixels: s.pixels,
                        gain: s.gain,
                    }));
                }
                Loaded::Preview { .. } | Loaded::Developed { .. } => {}
            }
        }
        while let Ok(told) = self.worker.told.try_recv() {
            match told {
                Told::Step { id, words: new, share: now } => {
                    if let Some(Status::Working { words, share, .. }) =
                        self.queue.get_mut(id).map(|e| &mut e.status)
                    {
                        *share = now;
                        if let Some(new) = new {
                            *words = new;
                        }
                    }
                }
                Told::Done { id, to, result } => self.finished(id, to, result),
            }
        }
    }

    /// The thread is through with a file.
    fn finished(&mut self, id: Id, to: PathBuf, result: Result<Box<Report>, String>) {
        let ok = result.is_ok();
        if let Some(entry) = self.queue.get_mut(id) {
            match result {
                Ok(report) => {
                    entry.status = Status::Done(report);
                    entry.dng = Some(to);
                    entry.marked = false;
                }
                Err(why) => entry.status = Status::Failed(why),
            }
        }
        if let Some(run) = &mut self.run {
            run.working = None;
            if ok {
                run.done += 1;
            } else {
                run.failed += 1;
            }
        }
        // The DNG takes the preview's place: of the file looked at, or,
        // while the run is followed, of the file just finished.
        if ok && self.current == Some(id) {
            self.show_dng();
        } else if ok && self.follow {
            self.reveal = true;
            self.look_at(Some(id));
        }
        self.next();
    }

    /// Begin a run with the marked files.
    fn convert(&mut self) {
        if self.run.is_some() {
            return;
        }
        let force = self.settings.force;
        let order: VecDeque<Id> =
            self.queue.entries.iter().filter(|e| e.convertible(force)).map(|e| e.id).collect();
        if order.is_empty() {
            return;
        }
        for id in &order {
            if let Some(entry) = self.queue.get_mut(*id) {
                entry.status = Status::Queued;
            }
        }
        self.note = None;
        self.follow = true;
        self.reveal = true;
        let first = order[0];
        self.run = Some(Run {
            total: order.len(),
            order,
            done: 0,
            failed: 0,
            working: None,
            stop: false,
            since: Instant::now(),
        });
        if self.current != Some(first) {
            self.look_at(Some(first));
        }
        self.next();
    }

    /// Give the thread the next file of the run, or end the run.
    fn next(&mut self) {
        loop {
            let Some(run) = &mut self.run else { return };
            if run.working.is_some() {
                return;
            }
            let id = if run.stop { None } else { run.order.pop_front() };
            let Some(id) = id else { break };
            let Some(entry) = self.queue.get(id) else { continue };
            let (raf, to) = (entry.raf.clone(), self.settings.target(&entry.raf));
            // Two RAFs of one name from two folders go to one DNG in a
            // common folder: the second would be written over the first.
            let taken = self.queue.entries.iter().any(|e| {
                e.id != id && matches!(e.status, Status::Done(_)) && e.dng.as_deref() == Some(to.as_path())
            });
            let Some(entry) = self.queue.get_mut(id) else { continue };
            if taken {
                entry.status = Status::Failed("another file of the list was written to this DNG".into());
                run.failed += 1;
                continue;
            }
            entry.status =
                Status::Working { words: "reading the RAF".into(), share: 0.0, since: Instant::now() };
            run.working = Some((id, to.clone()));
            let (options, use_swap) = (self.settings.options(), self.settings.use_swap);
            self.worker.begin(Job { id, raf, to, options, use_swap });
            return;
        }
        // The run is over: what was not begun waits again.
        let Some(run) = self.run.take() else { return };
        for id in run.order {
            if let Some(entry) = self.queue.get_mut(id) {
                entry.status = Status::Waiting;
            }
        }
        let mut said = vec![format!("{} converted", run.done)];
        if run.failed > 0 {
            said.push(format!("{} not", run.failed));
        }
        said.push(duration(run.since.elapsed().as_secs_f64()));
        self.note = Some(said.join(" · "));
    }

    /// The output was changed: the DNGs are looked for where they would
    /// be now, and the settings are kept.
    fn follow_settings(&mut self) {
        if self.settings == self.saved {
            return;
        }
        let moved = (self.settings.into_folder, &self.settings.folder)
            != (self.saved.into_folder, &self.saved.folder);
        self.saved = self.settings.clone();
        if self.keep {
            self.settings.save();
        }
        if moved {
            let settings = self.settings.clone();
            self.queue.look_for_dngs(&|raf| settings.target(raf));
            self.look_at(self.current);
        }
    }

    fn open_dialog(&mut self, pick: Pick) {
        let from = match pick {
            Pick::Output => self.settings.folder.clone().or_else(|| self.settings.last_dir.clone()),
            Pick::Files | Pick::Folder => self.settings.last_dir.clone(),
        };
        if let Some(dir) = from.filter(|d| d.is_dir()).or_else(settings::home) {
            self.dialog.config_mut().initial_directory = dir;
        }
        match pick {
            Pick::Files => self.dialog.pick_multiple(),
            Pick::Folder | Pick::Output => self.dialog.pick_directory(),
        }
        self.picking = Some(pick);
    }

    fn handle_dialog(&mut self, ctx: &egui::Context) {
        let Some(pick) = self.picking else { return };
        self.dialog.update(ctx);
        match pick {
            Pick::Files => {
                if let Some(paths) = self.dialog.take_picked_multiple() {
                    self.add(&paths);
                }
            }
            Pick::Folder => {
                if let Some(folder) = self.dialog.take_picked() {
                    self.add(&[folder]);
                }
            }
            Pick::Output => {
                if let Some(folder) = self.dialog.take_picked() {
                    self.settings.folder = Some(folder);
                    self.settings.into_folder = true;
                }
            }
        }
        if !matches!(self.dialog.state(), DialogState::Open) {
            self.picking = None;
        }
    }

    /// Step through the list by `by` files.
    fn step(&mut self, by: isize) {
        let n = self.queue.entries.len();
        if n == 0 {
            return;
        }
        let at = self.current.and_then(|id| self.queue.index(id));
        let to = match at {
            Some(at) => at.saturating_add_signed(by).min(n - 1),
            None => 0,
        };
        let id = self.queue.entries[to].id;
        self.choose(id);
        self.reveal = true;
    }

    /// Take the current file out of the list, and look at its neighbour.
    fn remove_current(&mut self) {
        let Some((id, at)) = self.current.and_then(|id| Some((id, self.queue.index(id)?))) else { return };
        if self.queue.remove_where(|e| e.id == id) == 0 {
            return;
        }
        let next = self.queue.entries.get(at).or(self.queue.entries.last()).map(|e| e.id);
        self.look_at(next);
        self.reveal = true;
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if self.picking.is_some() || self.quit_asked || ctx.egui_wants_keyboard_input() {
            return;
        }
        let (all, open) = ctx.input_mut(|i| {
            (i.consume_key(Modifiers::COMMAND, Key::A), i.consume_key(Modifiers::COMMAND, Key::O))
        });
        if all {
            self.queue.mark_where(|_| true);
        }
        if open {
            self.open_dialog(Pick::Files);
        }
        let plain = |key: Key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key));
        if plain(Key::ArrowUp) {
            self.step(-1);
        }
        if plain(Key::ArrowDown) {
            self.step(1);
        }
        if plain(Key::Home) {
            self.step(isize::MIN);
        }
        if plain(Key::End) {
            self.step(isize::MAX);
        }
        if plain(Key::Space) {
            if let Some(entry) = self.current.and_then(|id| self.queue.get_mut(id)) {
                entry.marked = !entry.marked;
            }
        }
        if plain(Key::D) {
            self.stage.want_dng = !self.stage.shows_dng;
        }
        if plain(Key::F) {
            self.stage.fit = true;
        }
        if plain(Key::Num1) {
            self.full_size();
        }
        // A Mac's keyboard has the backward delete alone.
        if plain(Key::Delete) || plain(Key::Backspace) {
            self.remove_current();
        }
    }

    /// The keys of a shortcut as this system names them: Ctrl+O, and
    /// Cmd+O on a Mac.
    fn shortcut(ui: &Ui, key: Key) -> String {
        ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, key))
    }

    /// One pixel of the picture on one of the screen.
    fn full_size(&mut self) {
        if self.stage.full_width > 0.0 {
            self.stage.fit = false;
            self.stage.width = self.stage.full_width;
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (open, all) = (Self::shortcut(ui, Key::O), Self::shortcut(ui, Key::A));
            if ui.add(Chip::action("Add files…").key(&open)).clicked() {
                self.open_dialog(Pick::Files);
            }
            if ui.add(Chip::action("Add folder…")).clicked() {
                self.open_dialog(Pick::Folder);
            }
            theme::sep(ui);
            theme::caption(ui, "Mark");
            let some = !self.queue.entries.is_empty();
            if ui.add_enabled(some, Chip::action("All").key(&all)).clicked() {
                self.queue.mark_where(|_| true);
            }
            if ui.add_enabled(some, Chip::action("None")).clicked() {
                self.queue.mark_where(|_| false);
            }
            let without = ui
                .add_enabled(some, Chip::action("Without DNG"))
                .on_hover_text("Mark the files that have no DNG yet, and no others");
            if without.clicked() {
                self.queue.mark_where(|e| e.dng.is_none());
            }
            theme::sep(ui);
            let marked = self.queue.marked();
            let remove = ui
                .add_enabled(marked > 0, Chip::action("Remove marked"))
                .on_hover_text("Take the marked files out of the list. Nothing on the disk is touched");
            if remove.clicked() {
                let current = self.current;
                self.queue.remove_where(|e| e.marked);
                if current.is_some_and(|id| self.queue.get(id).is_none()) {
                    self.look_at(self.queue.entries.first().map(|e| e.id));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                let with_dng = self.queue.entries.iter().filter(|e| e.dng.is_some()).count();
                let counts =
                    format!("{} · {marked} marked · {with_dng} with DNG", files(self.queue.entries.len()));
                let said = self.note.clone().filter(|_| some).unwrap_or(counts);
                ui.add(egui::Label::new(RichText::new(said).color(theme::INK_QUIET)).truncate());
            });
        });
    }

    fn list(&mut self, ui: &mut Ui) {
        let mut clicked: Option<(Id, Click, Modifiers)> = None;
        let reveal = std::mem::take(&mut self.reveal);
        let (n, marked) = (self.queue.entries.len(), self.queue.marked());
        theme::glass().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Files").strong().color(theme::INK_LIT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if n > 0 {
                        ui.label(theme::quiet(format!("{marked} of {n} marked")));
                    }
                });
            });
            ui.separator();
            if n == 0 {
                ui.label(theme::quiet("No files yet. Add RAFs, or drop them on the window."));
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for entry in &self.queue.entries {
                    let current = self.current == Some(entry.id);
                    let (response, click) = file_row(ui, entry, current);
                    if reveal && current {
                        response.scroll_to_me(None);
                    }
                    if let Some(click) = click {
                        clicked = Some((entry.id, click, ui.input(|i| i.modifiers)));
                    }
                }
            });
        });
        let Some((id, click, modifiers)) = clicked else { return };
        let was = self.queue.get(id).is_some_and(|e| e.marked);
        match (click, self.anchor) {
            // With Shift, from the file clicked before to this one.
            (Click::Mark, Some(from)) if modifiers.shift => self.queue.mark_range(from, id, !was),
            (Click::Row, Some(from)) if modifiers.shift => self.queue.mark_range(from, id, true),
            (Click::Mark, _) => {
                if let Some(entry) = self.queue.get_mut(id) {
                    entry.marked = !was;
                }
            }
            (Click::Row, _) if modifiers.command => {
                if let Some(entry) = self.queue.get_mut(id) {
                    entry.marked = !was;
                }
            }
            (Click::Row, _) => {}
        }
        match click {
            Click::Row if !modifiers.shift && !modifiers.command => self.choose(id),
            _ => self.anchor = Some(id),
        }
    }

    fn file_panel(&self, ui: &mut Ui) {
        theme::panel(
            ui,
            "File",
            |_| {},
            |ui| {
                let Some(entry) = self.entry() else {
                    ui.label(theme::quiet("No file chosen."));
                    return;
                };
                theme::rows(ui, |ui| {
                    theme::row(ui, "name", theme::value(&entry.name))
                        .on_hover_text(entry.raf.display().to_string());
                    if let Some(folder) = entry.raf.parent().filter(|f| !f.as_os_str().is_empty()) {
                        let folder = folder.display().to_string();
                        theme::row(ui, "folder", theme::value(&folder)).on_hover_text(&folder);
                    }
                    theme::row(ui, "size", theme::value(megabytes(entry.bytes)));
                    match &entry.info {
                        Read::Pending => {
                            theme::row(ui, "camera", theme::quiet("reading…"));
                        }
                        Read::Failed(why) => {
                            theme::row(ui, "camera", RichText::new(why).color(theme::BAD)).on_hover_text(why);
                        }
                        Read::Ready(info) => info_rows(ui, info),
                    }
                });
            },
        );
    }

    fn dng_panel(&self, ui: &mut Ui) {
        theme::panel(
            ui,
            "DNG",
            |_| {},
            |ui| {
                let Some(entry) = self.entry() else {
                    ui.label(theme::quiet("No file chosen."));
                    return;
                };
                let to = entry.dng.clone().unwrap_or_else(|| self.settings.target(&entry.raf));
                let name = to.file_name().unwrap_or_default().to_string_lossy().into_owned();
                theme::rows(ui, |ui| {
                    theme::row(ui, "file", theme::value(name)).on_hover_text(to.display().to_string());
                    let (state, colour) = match &entry.status {
                        Status::Done(_) => ("written just now".to_owned(), theme::GOOD),
                        Status::Working { words, .. } => (words.clone(), theme::AMBER),
                        Status::Queued => ("waits its turn".into(), theme::INK),
                        Status::Failed(why) => (why.clone(), theme::BAD),
                        Status::Waiting if entry.dng.is_some() => ("there from before".into(), theme::INK),
                        Status::Waiting => ("not written yet".into(), theme::INK_SMALL),
                    };
                    theme::row(ui, "state", RichText::new(&state).color(colour)).on_hover_text(&state);
                    if let (Some(bytes), true) = (self.stage.dng_bytes, entry.dng.is_some()) {
                        theme::row(ui, "size", theme::value(megabytes(bytes)));
                    }
                    let shown = self.stage.developed.as_ref().and_then(|d| d.as_ref().ok());
                    if let Status::Done(report) = &entry.status {
                        report_rows(ui, report, self.settings.keep_highlights);
                    } else if let Some(shown) = shown.filter(|_| entry.dng.is_some()) {
                        theme::row(
                            ui,
                            "picture",
                            theme::value(format!("{} · {}", by(shown.pixels), megapixels(shown.pixels))),
                        );
                    }
                    if let Some(shown) = shown.filter(|_| entry.dng.is_some()) {
                        theme::row(
                            ui,
                            "on screen",
                            theme::value(format!("light × {:.2}, as the preview", shown.gain)),
                        )
                        .on_hover_text(
                            "The DNG holds linear light without a tone curve. To be looked at here it is \
                             balanced, taken to sRGB by its own matrix, made as light as the camera's \
                             preview and given a mild contrast. The file itself is not touched by this.",
                        );
                    }
                });
            },
        );
    }

    fn output_panel(&mut self, ui: &mut Ui) {
        theme::panel(
            ui,
            "Output",
            |_| {},
            |ui| {
                let s = &mut self.settings;
                ui.radio_value(&mut s.into_folder, false, "Beside the RAF");
                let mut choose = false;
                ui.horizontal(|ui| {
                    if ui.radio(s.into_folder, "In a folder").clicked() {
                        s.into_folder = s.folder.is_some();
                        choose = s.folder.is_none();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        choose |= ui.add(Chip::action("Choose…").height(20.0)).clicked();
                    });
                });
                if let Some(folder) = &s.folder {
                    let folder = folder.display().to_string();
                    ui.add(egui::Label::new(theme::quiet(&folder)).truncate()).on_hover_text(&folder);
                }
                ui.add_space(4.0);
                ui.checkbox(&mut s.force, "Write over a DNG that is there");
                ui.checkbox(&mut s.keep_highlights, "Keep burnt-out areas as recorded").on_hover_text(
                    "A burnt-out area is cut to neutral white, as a raw developer shows it. With this it is \
                 left as the sensor recorded it, for a developer that reconstructs highlights itself.",
                );
                ui.checkbox(&mut s.uncompressed, "Uncompressed").on_hover_text(
                    "The DNG is compressed without loss. Without compression it is about twice as large.",
                );
                let rest = if cfg!(target_os = "windows") {
                    "with this it is begun all the same and Windows enlarges its page file"
                } else if work::SWAP_GROWS {
                    "with this it is begun all the same and macOS swaps the rest"
                } else {
                    "with this it is begun as long as the swap holds the rest"
                };
                // Windows gives no more than its page file holds, and a
                // program that is given no memory ends on the spot.
                let fixed = if cfg!(target_os = "windows") {
                    " With a page file of a set size, or none, the program can end without a word \
                     when the memory runs out."
                } else {
                    ""
                };
                ui.checkbox(&mut s.use_swap, "Use the swap when memory is short").on_hover_text(format!(
                    "A picture takes about 0.3 GB of memory for every megapixel. A file that needs more \
                     than is free is left out; {rest}, which is slow and holds up other programs.{fixed}"
                ));
                if choose {
                    self.open_dialog(Pick::Output);
                }
            },
        );
    }

    /// The one button, and how far the run is.
    fn go_panel(&mut self, ui: &mut Ui) {
        let mut convert = false;
        let mut stop = false;
        theme::glass().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width();
            if let Some(run) = &self.run {
                let working = run.working.as_ref().and_then(|(id, _)| self.queue.get(*id));
                let at = run.done + run.failed + 1;
                let name = working.map_or("", |e| e.name.as_str());
                ui.add(
                    egui::Label::new(theme::value(format!(
                        "{} of {} · {name}",
                        at.min(run.total),
                        run.total
                    )))
                    .truncate(),
                );
                let (words, share, since) = match working.map(|e| &e.status) {
                    Some(Status::Working { words, share, since }) => (words.as_str(), *share, Some(*since)),
                    _ => ("", 0.0, None),
                };
                let seconds = since.map_or(0.0, |s| s.elapsed().as_secs_f64());
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{words} · {}", duration(seconds))).color(theme::AMBER),
                    )
                    .truncate(),
                );
                ui.add_space(2.0);
                theme::progress(ui, share);
                ui.add_space(6.0);
                if run.stop {
                    ui.label(theme::quiet("stops after this file"));
                } else {
                    stop = ui
                        .add(Chip::toggle("Stop after this file", true).wide(width).height(30.0))
                        .on_hover_text("The file in work is finished; the files after it stay marked")
                        .clicked();
                }
                // The seconds count on while nothing else moves.
                ui.ctx().request_repaint_after(Duration::from_millis(500));
            } else {
                let force = self.settings.force;
                let takes: Vec<&Entry> = self.queue.entries.iter().filter(|e| e.convertible(force)).collect();
                let held = self.queue.entries.iter().filter(|e| e.marked && !e.convertible(true)).count();
                let there = self.queue.marked() - takes.len() - held;
                // The memory the largest of them takes.
                let need = takes
                    .iter()
                    .filter_map(|e| match &e.info {
                        Read::Ready(info) => {
                            info.picture.as_ref().ok().map(|(w, h)| work::needed((w * h) as u64))
                        }
                        _ => None,
                    })
                    .max();
                if let Some(need) = need {
                    let fits = self.memory.takes(need, false).is_ok();
                    let colour = match (fits, self.memory.takes(need, self.settings.use_swap).is_ok()) {
                        (true, _) => theme::INK_SMALL,
                        (false, true) => theme::AMBER,
                        (false, false) => theme::BAD,
                    };
                    let text = format!(
                        "the largest takes about {} · {} free",
                        work::gb(need),
                        work::gb(self.memory.free)
                    );
                    ui.add(egui::Label::new(RichText::new(text).color(colour)).truncate()).on_hover_text(
                        "One file is converted at a time, on every core. A file that needs more memory \
                         than is free when its turn comes is left out and says so.",
                    );
                }
                if there > 0 {
                    ui.add(
                        egui::Label::new(theme::quiet(format!(
                            "{} with a DNG: not written over",
                            files(there)
                        )))
                        .truncate(),
                    );
                }
                ui.add_space(4.0);
                let label = match takes.len() {
                    0 => "Convert".to_owned(),
                    n => format!("Convert {}", files(n)),
                };
                convert = ui
                    .add_enabled(!takes.is_empty(), Chip::keep(&label).wide(width).height(30.0))
                    .on_disabled_hover_text("Mark the files that are to be converted")
                    .clicked();
            }
        });
        if convert {
            self.convert();
        }
        if let (true, Some(run)) = (stop, &mut self.run) {
            run.stop = true;
        }
    }

    /// What is set and pressed stands at the bottom and is always there;
    /// what is only read is above it and scrolls when the window is low.
    fn side(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("go")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin { top: 8, ..Default::default() }))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                self.output_panel(ui);
                self.go_panel(ui);
            });
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            self.file_panel(ui);
            self.dng_panel(ui);
        });
    }

    /// The row under the picture: which of the two is shown, and how large.
    fn view_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_height(theme::ROW_H);
            let has_dng = self.entry().is_some_and(|e| e.dng.is_some());
            let some = self.stage.id.is_some();
            let preview = ui
                .add_enabled(some, Chip::toggle("Preview", some && !self.stage.shows_dng))
                .on_hover_text("The JPEG the camera put into the RAF");
            if preview.clicked() {
                self.stage.want_dng = false;
            }
            let dng = ui
                .add_enabled(has_dng, Chip::toggle("DNG", self.stage.shows_dng).key("D"))
                .on_hover_text("The DNG as it was written, made light enough to be looked at")
                .on_disabled_hover_text("This file has no DNG yet");
            if dng.clicked() {
                self.stage.want_dng = true;
            }
            theme::sep(ui);
            if ui.add_enabled(some, Chip::toggle("Fit", some && self.stage.fit).key("F")).clicked() {
                self.stage.fit = true;
            }
            let full = some && !self.stage.fit && (self.stage.width - self.stage.full_width).abs() < 0.5;
            if ui.add_enabled(some, Chip::toggle("100 %", full).key("1")).clicked() {
                self.full_size();
            }
            theme::sep(ui);
            let failed = self.stage.developed.as_ref().and_then(|d| d.as_ref().err());
            if let Some(why) = failed.filter(|_| has_dng) {
                ui.add(egui::Label::new(RichText::new(why).color(theme::BAD)).truncate());
            } else if self.stage.developing && has_dng {
                ui.label(RichText::new("reading the DNG for the screen…").color(theme::AMBER));
            } else if self.stage.scale > 0.0 {
                ui.label(theme::quiet(format!("{:.0} %", 100.0 * self.stage.scale)));
            }
        });
    }

    fn stage(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("view")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::STAGE).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(ui, |ui| self.view_bar(ui));
        let room = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(room, Sense::click_and_drag());
        let painter = ui.painter_at(room);
        // In lines when the room is narrower than the words, not cut.
        let say = |text: &str, colour: Color32| {
            let font = egui::FontId::proportional(theme::TEXT);
            let width = (room.width() - 32.0).max(0.0);
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, colour, width);
            job.halign = egui::Align::Center;
            let words = painter.layout_job(job);
            let at = egui::pos2(room.center().x, room.center().y - words.size().y / 2.0);
            painter.galley(at, words, colour);
        };
        if ui.input(|i| !i.raw.hovered_files.is_empty()) {
            painter.rect_stroke(
                room.shrink(10.0),
                10.0,
                egui::Stroke::new(1.5, theme::AMBER),
                egui::StrokeKind::Inside,
            );
        }

        let stage = &mut self.stage;
        stage.room = room;
        let developed = stage.developed.as_ref().and_then(|d| d.as_ref().ok()).map(|d| &d.picture);
        let preview = stage.preview.as_ref().and_then(|p| p.as_ref().ok());
        let (picture, dng) = match (stage.want_dng, developed, preview) {
            (true, Some(d), _) | (_, Some(d), None) => (d, true),
            (_, _, Some(p)) => (p, false),
            (_, None, None) => {
                stage.shows_dng = false;
                stage.scale = 0.0;
                match (&stage.preview, stage.id) {
                    (_, None) if self.queue.entries.is_empty() => {
                        say(
                            "Drop Fuji X-Trans RAFs or folders here, or add them with the buttons above.",
                            theme::INK_SMALL,
                        );
                    }
                    (_, None) => {}
                    (Some(Err(why)), _) => say(why, theme::BAD),
                    _ => say("reading the preview…", theme::INK_SMALL),
                }
                return;
            }
        };
        let (texture, size) = (picture.texture.id(), picture.size);
        stage.shows_dng = dng;

        let aspect = size.y / size.x;
        let fit_width =
            (room.width() - 2.0 * MARGIN).min((room.height() - 2.0 * MARGIN) / aspect).max(MARGIN);
        let per_point = ui.ctx().pixels_per_point();
        stage.full_width = size.x / per_point;
        let widest = (stage.full_width * MAX_ZOOM).max(fit_width);
        if stage.fit || stage.width <= fit_width {
            (stage.fit, stage.width, stage.centre) = (true, fit_width, vec2(0.5, 0.5));
        }
        stage.width = stage.width.min(widest);

        // Enlarge so that what is under the pointer stays under it.
        let zoom_at = |stage: &mut Stage, pointer: egui::Pos2, width: f32| {
            let width = width.clamp(fit_width, widest);
            if width <= fit_width * 1.001 {
                (stage.fit, stage.width, stage.centre) = (true, fit_width, vec2(0.5, 0.5));
                return;
            }
            let before = vec2(stage.width, stage.width * aspect);
            let under = (pointer - (room.center() - stage.centre * before)) / before;
            let after = vec2(width, width * aspect);
            stage.centre = (room.center() - (pointer - under * after)) / after;
            (stage.fit, stage.width) = (false, width);
        };
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll / 300.0).exp() * pinch;
            if (factor - 1.0).abs() > 1e-4 {
                zoom_at(stage, pointer, stage.width * factor);
            }
            if response.double_clicked() {
                let to = if stage.fit { stage.full_width } else { fit_width };
                zoom_at(stage, pointer, to);
            }
        }
        let on_screen = vec2(stage.width, stage.width * aspect);
        if response.dragged() && !stage.fit {
            stage.centre -= response.drag_delta() / on_screen;
        }
        // The picture covers the room where it is larger than the room,
        // and lies in its middle where it is smaller.
        let half = room.size() / on_screen / 2.0;
        for axis in 0..2 {
            stage.centre[axis] =
                if half[axis] >= 0.5 { 0.5 } else { stage.centre[axis].clamp(half[axis], 1.0 - half[axis]) };
        }
        stage.scale = stage.width / stage.full_width;
        let rect = Rect::from_min_size(room.center() - stage.centre * on_screen, on_screen);
        painter.image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        if response.dragged() && !stage.fit {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }

    /// The window is not closed over a file in work without a word.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.run.is_some() && !self.quit_allowed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_asked = true;
        }
    }

    /// Close now: the file in work is left unfinished, and what there is
    /// of it is taken away.
    fn quit(&mut self, ctx: &egui::Context) {
        if let Some((_, to)) = self.run.as_ref().and_then(|r| r.working.as_ref()) {
            let _ = std::fs::remove_file(part_of(to));
        }
        self.quit_allowed = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn quit_window(&mut self, ctx: &egui::Context) {
        if !self.quit_asked {
            return;
        }
        if self.run.is_none() {
            self.quit_asked = false;
            return;
        }
        let mut answer = None;
        egui::Modal::new(egui::Id::new("quit")).frame(theme::glass().fill(theme::BAR_FILL)).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new("A file is being converted").strong().color(theme::INK_LIT));
            ui.separator();
            ui.label(theme::value(
                "Closing now leaves it unfinished. The DNGs written before it are whole, and the files \
                 after it are not begun.",
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(Chip::toggle("Keep converting", true)).clicked() {
                    answer = Some(false);
                }
                if ui.add(Chip::action("Close now")).clicked() {
                    answer = Some(true);
                }
            });
        });
        match answer {
            Some(true) => self.quit(ctx),
            Some(false) => self.quit_asked = false,
            None => {}
        }
    }

    /// The window photographs itself when it is as `XTRANS_GUI_SHOT_WHEN`
    /// says, and closes.
    fn take_shot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &self.shot else { return };
        let (to, when, begun, ready) = (shot.to.clone(), shot.when, shot.begun, shot.ready);
        ctx.request_repaint();
        // A window started without files has nothing to wait for: its
        // picture shows that it opens and draws.
        let empty = self.queue.entries.is_empty();
        let read = (empty && when == When::Idle)
            || !empty
                && self.queue.entries.iter().all(|e| !matches!(e.info, Read::Pending))
                && self.stage.preview.is_some()
                && !self.stage.developing;
        if when != When::Idle && !begun {
            if read {
                self.convert();
                if let Some(shot) = &mut self.shot {
                    shot.begun = true;
                }
            }
            return;
        }
        let share = self.queue.entries.iter().find_map(|e| match e.status {
            Status::Working { share, .. } => Some(share),
            _ => None,
        });
        let now = match when {
            When::Idle => read,
            When::Working => share.is_some_and(|s| s >= 0.5),
            When::Done => self.run.is_none() && read,
        };
        if !now && ready == 0 {
            return;
        }
        if ready == 0 {
            let asked = self.shot.as_ref().map(|s| (s.preview, s.zoom, s.open.clone()));
            let (preview, zoom, open) = asked.unwrap_or_default();
            if preview {
                self.stage.want_dng = false;
            }
            if let Some(zoom) = zoom {
                self.stage.fit = false;
                self.stage.width = self.stage.full_width * zoom / 100.0;
            }
            match open.as_deref() {
                Some("files") => self.open_dialog(Pick::Files),
                Some("quit") => self.quit_asked = true,
                _ => {}
            }
        }
        if let Some(shot) = &mut self.shot {
            shot.ready += 1;
        }
        // A few frames for the pictures to be on the card.
        if ready == 4 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let taken = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(taken) = taken {
            let [w, h] = taken.size;
            // A JPEG has no alpha.
            let saved = image::RgbaImage::from_raw(w as u32, h as u32, taken.as_raw().to_vec())
                .ok_or_else(|| "no picture".to_owned())
                .and_then(|picture| {
                    image::DynamicImage::ImageRgba8(picture).into_rgb8().save(&to).map_err(|e| e.to_string())
                });
            if let Err(why) = saved {
                eprintln!("{}: {why}", to.display());
            }
            self.shot = None;
            self.quit(ctx);
        }
    }
}

/// What the file says about itself, row by row.
fn info_rows(ui: &mut Ui, info: &Info) {
    theme::row(ui, "camera", theme::value(format!("{} {}", info.make, info.model)));
    if info.exposure.is_empty() {
        theme::row(ui, "exposure", theme::quiet("not in the file"));
    } else {
        theme::row(ui, "exposure", RichText::new(&info.exposure).strong().color(theme::INK_LIT))
            .on_hover_text(&info.exposure);
    }
    if let Some(correction) = info.correction {
        theme::row(ui, "correction", theme::value(format!("{correction:+.2} EV")));
    }
    if let Some(lens) = &info.lens {
        theme::row(ui, "lens", theme::value(lens)).on_hover_text(lens);
    }
    if let Some(taken) = &info.taken {
        theme::row(ui, "taken", theme::value(taken));
    }
    theme::row(ui, "sensor", theme::value(format!("{} @ {} bit", by(info.sensor), info.bits)));
    let x_trans = info.picture.is_ok();
    let pattern = if x_trans { "X-Trans".to_owned() } else { format!("{} (not X-Trans)", info.pattern) };
    theme::row(ui, "pattern", RichText::new(pattern).color(if x_trans { theme::INK } else { theme::BAD }))
        .on_hover_text(format!("The mosaic's tile: {}", info.pattern));
    match &info.picture {
        Ok(picture) => {
            theme::row(ui, "picture", theme::value(format!("{} · {}", by(*picture), megapixels(*picture))))
                .on_hover_text("What the DNG will hold: the file's crop, less up to seven columns and rows");
        }
        Err(why) => {
            theme::row(ui, "picture", RichText::new(why).color(theme::BAD)).on_hover_text(why);
        }
    }
    if let Some(preview) = info.preview {
        theme::row(ui, "preview", theme::value(format!("JPEG {}", by(preview))));
    }
    match info.balance {
        Some((red, blue)) => {
            theme::row(ui, "balance", theme::value(format!("R {red:.3}  B {blue:.3} (camera)")))
        }
        None => theme::row(ui, "balance", theme::quiet("not in the file")),
    };
}

/// What the conversion found, row by row.
fn report_rows(ui: &mut Ui, report: &Report, kept: bool) {
    theme::row(ui, "picture", theme::value(format!("{} · {}", by(report.pixels), megapixels(report.pixels))));
    let prior = report.prior;
    theme::row(
        ui,
        "prior",
        theme::value(format!("luminance f^{:.2} · colour f^{:.2}", prior.alpha, prior.alpha_c)),
    )
    .on_hover_text(format!(
        "What the mosaic itself made most probable: luminance falls with f^{:.2}, colour with f^{:.2}; \
         colour {:.4} and {:.4}, noise {:.1e}, lens {:.2} px",
        prior.alpha, prior.alpha_c, prior.chroma[0], prior.chroma[1], prior.noise, prior.lens
    ));
    theme::row(ui, "noise", theme::value(format!("{:.1e} · lens {:.2} px", prior.noise, prior.lens)));
    let share = match 100.0 * report.burnt_share() {
        share if share < 0.001 => "under 0.001 %".to_owned(),
        share => format!("{share:.3} %"),
    };
    let burnt = match (report.burnt, kept) {
        (0, _) => "none".to_owned(),
        (_, true) => format!("{share} · kept as recorded"),
        (_, false) => format!("{share} · cut to neutral"),
    };
    theme::row(ui, "burnt out", theme::value(burnt));
    if !report.balance_from_file {
        theme::row(ui, "balance", RichText::new("to a grey mean").color(theme::AMBER))
            .on_hover_text("The file gives no white balance: the picture is balanced to a grey mean");
    }
    let [prior, separation, edges] = report.seconds;
    theme::row(ui, "time", theme::value(duration(report.total))).on_hover_text(format!(
        "prior {prior:.1} s · separation {separation:.1} s · colour edges {edges:.1} s"
    ));
}

/// One file of the list. Gives what was clicked: the box the file is
/// marked in, or the rest of the row.
fn file_row(ui: &mut Ui, entry: &Entry, current: bool) -> (egui::Response, Option<Click>) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let mark =
        Rect::from_center_size(pos2(rect.left() + 6.0 + MARK / 2.0, rect.center().y), Vec2::splat(MARK));
    let mark_response = ui
        .interact(mark.expand(5.0), response.id.with("mark"), Sense::click())
        .on_hover_text("Marked files are converted. Shift marks from the file clicked before to this one");
    let click = if mark_response.clicked() {
        Some(Click::Mark)
    } else if response.clicked() {
        Some(Click::Row)
    } else {
        None
    };
    if !ui.is_rect_visible(rect) {
        return (response, click);
    }
    let painter = ui.painter();
    if current {
        painter.rect_filled(rect, theme::RADIUS, theme::SHOWN);
    } else if response.hovered() || mark_response.hovered() {
        painter.rect_filled(rect, theme::RADIUS, theme::HOVER);
    }
    theme::paint_mark(ui, mark, entry.marked, mark_response.hovered());

    // The small picture, whole, in the middle of its room.
    let room = Rect::from_min_size(pos2(mark.right() + 8.0, rect.center().y - THUMB.y / 2.0), THUMB);
    match &entry.thumb {
        Some(thumb) => {
            let size = thumb.size_vec2();
            let fitted = size * (THUMB.x / size.x).min(THUMB.y / size.y);
            painter.image(
                thumb.id(),
                Rect::from_center_size(room.center(), fitted),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(room, 2.0, Color32::from_gray(24));
        }
    }

    let left = room.right() + 8.0;
    let width = rect.right() - 6.0 - left;
    let name =
        theme::elided(ui, &entry.name, theme::TEXT, if current { theme::INK_LIT } else { theme::INK }, width);
    let about = match &entry.info {
        Read::Ready(info) => {
            let parts = [
                Some(info.model.clone()),
                info.iso.map(|v| format!("ISO {v}")),
                info.picture.as_ref().ok().map(|p| megapixels(*p)),
            ];
            parts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
        }
        Read::Pending => "reading…".into(),
        Read::Failed(_) => megabytes(entry.bytes),
    };
    let about = theme::elided(ui, &about, theme::SMALL, theme::INK_SMALL, width);
    let (status, colour) = status_line(entry);
    let status = theme::elided(ui, &status, theme::SMALL, colour, width);
    let top = rect.top() + 6.0;
    painter.galley(pos2(left, top), name, Color32::PLACEHOLDER);
    painter.galley(pos2(left, top + 17.0), about, Color32::PLACEHOLDER);
    painter.galley(pos2(left, top + 31.0), status, Color32::PLACEHOLDER);

    // How far the file in work is, along the foot of its row.
    if let Status::Working { share, .. } = entry.status {
        let foot = Rect::from_min_max(
            pos2(left, rect.bottom() - 4.0),
            pos2(rect.right() - 6.0, rect.bottom() - 2.0),
        );
        painter.rect_filled(foot, 1.0, theme::SHOWN_HOVER);
        let mut done = foot;
        done.set_width(foot.width() * share.clamp(0.0, 1.0));
        painter.rect_filled(done, 1.0, theme::AMBER);
    }
    let (words, _) = status_line(entry);
    (response.on_hover_text(format!("{}\n{words}", entry.raf.display())), click)
}

impl App {
    /// Take what the threads have sent, and what else goes on without
    /// the window being drawn.
    fn tick(&mut self, ctx: &egui::Context) {
        self.loader.set_max_side(ctx.input(|i| i.max_texture_side));
        self.collect(ctx);
        if self.memory_read.elapsed() > Duration::from_secs(2) {
            self.memory = Memory::read(&mut self.system);
            self.memory_read = Instant::now();
        }
        self.follow_settings();
        self.guard_close(ctx);
    }

    /// Draw the window.
    fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            self.add(&dropped);
        }
        self.handle_dialog(&ctx);
        self.keys(&ctx);

        let bar = egui::Frame::new().fill(theme::BAR_FILL).inner_margin(egui::Margin::symmetric(8, 5));
        egui::Panel::top("toolbar").frame(bar).show(ui, |ui| self.toolbar(ui));
        let side = egui::Frame::new().fill(theme::STAGE).inner_margin(10.0);
        egui::Panel::left("list")
            .resizable(true)
            .show_separator_line(false)
            .default_size(310.0)
            .size_range(250.0..=520.0)
            .frame(side)
            .show(ui, |ui| self.list(ui));
        egui::Panel::right("file")
            .resizable(true)
            .show_separator_line(false)
            .default_size(320.0)
            .size_range(280.0..=460.0)
            .frame(side)
            .show(ui, |ui| self.side(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::STAGE))
            .show(ui, |ui| self.stage(ui));
        self.quit_window(&ctx);
        self.take_shot(&ctx);
    }
}

impl eframe::App for App {
    /// eframe calls `logic` while the window is covered too, so a run
    /// goes on from file to file.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);
    }

    /// A window that photographs itself is not under the pointer: where
    /// the mouse happens to rest, the picture would carry that button's
    /// hover.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.shot.is_some() {
            raw_input.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::PointerMoved(_)
                        | egui::Event::MouseMoved(_)
                        | egui::Event::PointerButton { .. }
                )
            });
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::{by, duration, file_row, files, megabytes, megapixels, part_of, App, Click, Picture};
    use crate::queue::{Queue, Read, Status};
    use crate::settings::Settings;
    use egui::{pos2, vec2, Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect};
    use mimizan_lab_xtrans::file::beside;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    #[test]
    fn numbers_are_said_shortly() {
        assert_eq!(megapixels((6231, 4155)), "25.9 MP");
        assert_eq!(by((6231, 4155)), "6231 × 4155");
        assert_eq!(megabytes(56_300_000), "56.3 MB");
        assert_eq!((duration(44.4), duration(130.0)), ("44 s".to_owned(), "2 min 10 s".to_owned()));
        assert_eq!(
            (files(1), files(3), files(0)),
            ("1 file".to_owned(), "3 files".to_owned(), "0 files".to_owned())
        );
        assert_eq!(part_of(Path::new("/a/b.dng")), Path::new("/a/b.dng.part"));
    }

    /// A folder of its own with files that are named like RAFs and are none.
    fn folder(test: &str, names: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtrans-gui-app-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in names {
            fs::write(dir.join(name), b"not a RAF").unwrap();
        }
        dir
    }

    /// A window without a screen: frames are drawn into nothing.
    struct Window {
        ctx: egui::Context,
        app: App,
        time: f64,
    }

    impl Window {
        fn new(initial: Vec<PathBuf>) -> Self {
            let ctx = egui::Context::default();
            let app = App::with(&ctx, initial, Settings::default(), false, None);
            Self { ctx, app, time: 0.0 }
        }

        fn frame_with(&mut self, modifiers: Modifiers, mut events: Vec<Event>) {
            self.time += 0.02;
            events.insert(0, Event::ModifiersChanged(modifiers));
            let screen_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1360.0, 860.0)));
            let input = egui::RawInput { events, time: Some(self.time), screen_rect, ..Default::default() };
            let app = &mut self.app;
            let _ = self.ctx.run_ui(input, |ui| {
                app.tick(ui.ctx());
                app.show(ui);
            });
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.frame_with(Modifiers::NONE, events);
        }

        fn key_with(&mut self, modifiers: Modifiers, key: Key) {
            let press = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
            self.frame_with(modifiers, vec![press(true)]);
            self.frame_with(modifiers, vec![press(false)]);
        }

        fn key(&mut self, key: Key) {
            self.key_with(Modifiers::NONE, key);
        }

        /// Draw frames until `done` holds; the threads need their time.
        fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
            let since = Instant::now();
            while !done(&self.app) {
                assert!(since.elapsed() < Duration::from_secs(20), "waited in vain for {what}");
                std::thread::sleep(Duration::from_millis(5));
                self.frame(vec![]);
            }
        }

        fn names(&self) -> Vec<&str> {
            self.app.queue.entries.iter().map(|e| e.name.as_str()).collect()
        }

        fn marks(&self) -> Vec<bool> {
            self.app.queue.entries.iter().map(|e| e.marked).collect()
        }

        fn current(&self) -> &str {
            self.app.entry().map_or("", |e| e.name.as_str())
        }
    }

    #[test]
    fn the_list_is_walked_marked_and_thinned_from_the_keyboard() {
        let dir = folder("keys", &["1.raf", "2.raf", "3.raf", "4.raf"]);
        let mut w = Window::new(vec![dir.clone()]);
        assert_eq!(w.names(), ["1.raf", "2.raf", "3.raf", "4.raf"]);
        assert_eq!((w.current(), w.marks()), ("1.raf", vec![true; 4]));
        // What cannot be read cannot be converted, and is not marked for it.
        w.until("the files to be read", |app| {
            app.queue.entries.iter().all(|e| matches!(e.info, Read::Failed(_)))
        });
        assert_eq!(w.marks(), [false; 4]);
        assert!(matches!(&w.app.stage.preview, Some(Err(why)) if why == "not a RAF"));

        w.key(Key::ArrowDown);
        w.key(Key::ArrowDown);
        assert_eq!(w.current(), "3.raf");
        w.key(Key::Space);
        assert_eq!(w.marks(), [false, false, true, false]);
        w.key(Key::Home);
        w.key(Key::ArrowUp);
        assert_eq!(w.current(), "1.raf");
        w.key(Key::End);
        assert_eq!(w.current(), "4.raf");
        w.key_with(Modifiers::COMMAND | Modifiers::CTRL, Key::A);
        assert_eq!(w.marks(), [true; 4]);
        // Out of the list: the file after it is looked at, at the end the one before.
        w.key(Key::Delete);
        assert_eq!((w.names(), w.current()), (vec!["1.raf", "2.raf", "3.raf"], "3.raf"));
        w.key(Key::Home);
        w.key(Key::Delete);
        assert_eq!((w.names(), w.current()), (vec!["2.raf", "3.raf"], "2.raf"));
        assert!(dir.join("1.raf").is_file(), "taken out of the list, not off the disk");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_run_goes_through_its_files_and_says_what_went_wrong() {
        let dir = folder("run", &["1.raf", "2.raf", "3.raf"]);
        let mut w = Window::new(vec![dir.clone()]);
        w.until("the files to be read", |app| {
            app.queue.entries.iter().all(|e| !matches!(e.info, Read::Pending))
        });
        w.key_with(Modifiers::COMMAND | Modifiers::CTRL, Key::A);
        w.app.queue.entries[1].marked = false;
        w.app.convert();
        assert!(w.app.run.is_some());
        w.until("the run to end", |app| app.run.is_none());
        let failed: Vec<bool> =
            w.app.queue.entries.iter().map(|e| matches!(e.status, Status::Failed(_))).collect();
        assert_eq!(failed, [true, false, true]);
        assert!(matches!(w.app.queue.entries[1].status, Status::Waiting));
        // What failed stays marked, to be tried again; nothing was written.
        assert_eq!(w.marks(), [true, false, true]);
        assert!(
            w.app.note.as_deref().is_some_and(|n| n.starts_with("0 converted · 2 not")),
            "{:?}",
            w.app.note
        );
        assert!(!dir.join("1.dng").exists() && !dir.join("1.dng.part").exists());
        let _ = fs::remove_dir_all(dir);
    }

    /// The point of the picture, of 1, that lies under `pointer`.
    fn under(app: &App, pointer: Pos2, aspect: f32) -> egui::Vec2 {
        let stage = &app.stage;
        let on_screen = vec2(stage.width, stage.width * aspect);
        stage.centre + (pointer - stage.room.center()) / on_screen
    }

    #[test]
    fn the_picture_is_enlarged_under_the_pointer_and_never_leaves_the_room() {
        let mut w = Window::new(vec![]);
        let image = egui::ColorImage::new([3000, 2000], vec![Color32::GRAY; 6_000_000]);
        w.app.stage.id = Some(1);
        w.app.stage.preview = Some(Ok(Picture::new(&w.ctx, "test", image)));
        w.frame(vec![]);
        let room = w.app.stage.room;
        assert!(room.width() > 500.0 && room.height() > 500.0, "{room:?}");
        // Seen whole: inside the room, a margin around it.
        let fitted = w.app.stage.width;
        assert!(w.app.stage.fit && fitted < room.width() && fitted * 2.0 / 3.0 < room.height());
        assert!((w.app.stage.scale - fitted / 3000.0).abs() < 1e-6);

        // One pixel on one, by the key.
        w.key(Key::Num1);
        assert!(!w.app.stage.fit && (w.app.stage.width - 3000.0).abs() < 0.5);
        assert!((w.app.stage.scale - 1.0).abs() < 1e-3);
        w.key(Key::F);
        assert!(w.app.stage.fit && (w.app.stage.width - fitted).abs() < 0.5);

        // The wheel enlarges about the pointer.
        let pointer = room.center() + vec2(120.0, -80.0);
        let wheel = |y: f32| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        w.frame(vec![Event::PointerMoved(pointer)]);
        let before = under(&w.app, pointer, 2.0 / 3.0);
        // Far enough that the picture is larger than the room both ways:
        // where it is smaller it lies in the middle, whatever the pointer.
        w.frame(vec![wheel(600.0)]);
        for _ in 0..40 {
            w.frame(vec![]);
        }
        let stage = &w.app.stage;
        assert!(!stage.fit && stage.width * 2.0 / 3.0 > 1.5 * room.height(), "{}", stage.width);
        let after = under(&w.app, pointer, 2.0 / 3.0);
        assert!((after - before).length() < 2e-3, "{before:?} became {after:?}");

        // Dragged far away, the picture still covers the room.
        let button = |pressed| Event::PointerButton {
            pos: pointer,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        w.key(Key::Num1);
        w.frame(vec![button(true)]);
        for step in 1..=20 {
            w.frame(vec![Event::PointerMoved(pointer + vec2(400.0, 300.0) * step as f32)]);
        }
        w.frame(vec![Event::PointerButton {
            pos: pointer + vec2(8000.0, 6000.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        let stage = &w.app.stage;
        let left = stage.room.center().x - stage.centre.x * stage.width;
        let top = stage.room.center().y - stage.centre.y * stage.width * 2.0 / 3.0;
        assert!(
            (left - room.left()).abs() < 0.5 && (top - room.top()).abs() < 0.5,
            "{left} {top} in {room:?}"
        );

        // Back out with the wheel: whole again.
        w.frame(vec![Event::PointerMoved(pointer)]);
        w.frame(vec![wheel(-3000.0)]);
        for _ in 0..40 {
            w.frame(vec![]);
        }
        assert!(w.app.stage.fit && (w.app.stage.width - fitted).abs() < 0.5);
    }

    #[test]
    fn a_click_on_the_box_marks_and_a_click_beside_it_chooses() {
        let dir = folder("row", &["1.raf"]);
        let mut queue = Queue::default();
        queue.add(std::slice::from_ref(&dir), &|p| beside(p));
        let entry = &queue.entries[0];
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut press = |at: Option<Pos2>| -> Option<Click> {
            let mut clicked = None;
            let button = |pressed| Event::PointerButton {
                pos: at.unwrap_or_default(),
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let steps = match at {
                Some(at) => vec![vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]],
                None => vec![vec![]],
            };
            for events in steps {
                time += 0.02;
                let screen_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)));
                let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
                let _ = ctx.run_ui(input, |ui| {
                    let (response, click) = file_row(ui, entry, false);
                    assert!((response.rect.height() - super::ROW).abs() < 0.5);
                    clicked = clicked.or(click);
                });
            }
            clicked
        };
        assert!(press(None).is_none());
        // The box is at the left of the row, in the middle of its height.
        assert!(matches!(press(Some(pos2(8.0 + 6.0 + 8.0, 8.0 + 29.0))), Some(Click::Mark)));
        assert!(matches!(press(Some(pos2(200.0, 8.0 + 12.0))), Some(Click::Row)));
        assert!(press(Some(pos2(200.0, 250.0))).is_none());
        let _ = fs::remove_dir_all(dir);
    }
}

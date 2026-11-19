//! The list of files: what is in it, what is marked, and how far each
//! file is.

use crate::load::Info;
use mimizan_lab_xtrans::file::Report;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The name of a file in the list; it stays when the list changes.
pub type Id = u64;

#[derive(Clone, Debug)]
pub enum Status {
    /// Nothing was done with it since it came into the list.
    Waiting,
    /// In the run, not yet begun.
    Queued,
    Working {
        words: String,
        /// Of 1.
        share: f32,
        since: Instant,
    },
    Done(Box<Report>),
    Failed(String),
}

/// What the file says about itself, once it is read.
pub enum Read {
    Pending,
    Ready(Box<Info>),
    Failed(String),
}

pub struct Entry {
    pub id: Id,
    pub raf: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub marked: bool,
    pub info: Read,
    pub thumb: Option<egui::TextureHandle>,
    pub status: Status,
    /// The DNG of this file, if it is there.
    pub dng: Option<PathBuf>,
}

impl Entry {
    /// In a run: not to be taken out of the list or begun again.
    pub fn busy(&self) -> bool {
        matches!(self.status, Status::Queued | Status::Working { .. })
    }

    /// Whether a run takes it: marked, and without a DNG unless DNGs are
    /// written over.
    pub fn convertible(&self, force: bool) -> bool {
        self.marked && !self.busy() && (force || self.dng.is_none())
    }
}

/// What became of the paths given to [`Queue::add`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Added {
    pub ids: Vec<Id>,
    /// In the list already.
    pub known: usize,
    /// Not RAFs.
    pub other: usize,
}

#[derive(Default)]
pub struct Queue {
    pub entries: Vec<Entry>,
    next: Id,
}

fn is_raf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("raf"))
}

/// The same file under whatever name it was given.
fn same(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

impl Queue {
    /// Take files, and the RAFs of folders (not of the folders in them),
    /// into the list. A file comes in marked unless its DNG, at
    /// `target`, is there already.
    pub fn add(&mut self, paths: &[PathBuf], target: &dyn Fn(&Path) -> PathBuf) -> Added {
        let mut added = Added::default();
        let mut files = Vec::new();
        for path in paths {
            if path.is_dir() {
                let mut inside: Vec<PathBuf> = fs::read_dir(path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| is_raf(p) && p.is_file())
                    .collect();
                inside.sort();
                files.extend(inside);
            } else if is_raf(path) && path.is_file() {
                files.push(path.clone());
            } else {
                added.other += 1;
            }
        }
        let mut known: HashSet<PathBuf> = self.entries.iter().map(|e| same(&e.raf)).collect();
        for raf in files {
            if !known.insert(same(&raf)) {
                added.known += 1;
                continue;
            }
            // By its whole path: the panel says where the file lies.
            let raf = std::path::absolute(&raf).unwrap_or(raf);
            let dng = Some(target(&raf)).filter(|t| t.is_file());
            self.next += 1;
            added.ids.push(self.next);
            self.entries.push(Entry {
                id: self.next,
                name: raf.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                bytes: fs::metadata(&raf).map_or(0, |m| m.len()),
                marked: dng.is_none(),
                info: Read::Pending,
                thumb: None,
                status: Status::Waiting,
                dng,
                raf,
            });
        }
        added
    }

    pub fn get(&self, id: Id) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn index(&self, id: Id) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    pub fn marked(&self) -> usize {
        self.entries.iter().filter(|e| e.marked).count()
    }

    /// Mark the files `which` holds for, and no others.
    pub fn mark_where(&mut self, which: impl Fn(&Entry) -> bool) {
        for e in &mut self.entries {
            e.marked = which(e);
        }
    }

    /// Mark, or unmark, every file from `a` to `b`, both included, in
    /// whichever order they stand.
    pub fn mark_range(&mut self, a: Id, b: Id, on: bool) {
        let (Some(a), Some(b)) = (self.index(a), self.index(b)) else { return };
        for e in &mut self.entries[a.min(b)..=a.max(b)] {
            e.marked = on;
        }
    }

    /// Take the files `which` holds for out of the list, except those a
    /// run has. Nothing on the disk is touched.
    pub fn remove_where(&mut self, which: impl Fn(&Entry) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.busy() || !which(e));
        before - self.entries.len()
    }

    /// Look again where each file's DNG would be. A file that was
    /// converted keeps the DNG it was converted to.
    pub fn look_for_dngs(&mut self, target: &dyn Fn(&Path) -> PathBuf) {
        for e in &mut self.entries {
            if e.busy() || matches!(e.status, Status::Done(_)) && e.dng.as_ref().is_some_and(|d| d.is_file())
            {
                continue;
            }
            if matches!(e.status, Status::Done(_)) {
                e.status = Status::Waiting;
            }
            e.dng = Some(target(&e.raf)).filter(|t| t.is_file());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Queue, Status};
    use mimizan_lab_xtrans::file::beside;
    use std::fs;
    use std::path::PathBuf;

    /// A folder of its own, with `names` in it as empty files.
    fn folder(test: &str, names: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtrans-gui-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in names {
            fs::write(dir.join(name), b"").unwrap();
        }
        dir
    }

    #[test]
    fn a_folder_gives_its_rafs_once_and_in_order() {
        let dir = folder("add", &["b.RAF", "a.raf", "a.dng", "c.jpg", "d.Raf"]);
        fs::create_dir(dir.join("inner.raf")).unwrap();
        let mut q = Queue::default();
        let added = q.add(std::slice::from_ref(&dir), &|p| beside(p));
        let names: Vec<&str> = q.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.raf", "b.RAF", "d.Raf"]);
        assert_eq!((added.ids.len(), added.known, added.other), (3, 0, 0));
        // The file whose DNG is there comes in unmarked.
        assert_eq!(q.entries.iter().map(|e| e.marked).collect::<Vec<_>>(), [false, true, true]);
        assert!(q.entries[0].dng.is_some() && q.entries[1].dng.is_none());

        // Again, by another name for the same file, and with what is no RAF.
        let again = [dir.join("./b.RAF"), dir.join("c.jpg"), dir.join("gone.raf")];
        let added = q.add(&again, &|p| beside(p));
        assert_eq!((added.ids.len(), added.known, added.other), (0, 1, 2));
        assert_eq!(q.entries.len(), 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn marks_go_over_a_range_and_a_run_keeps_its_files() {
        let dir = folder("mark", &["1.raf", "2.raf", "3.raf", "4.raf"]);
        let mut q = Queue::default();
        let ids = q.add(std::slice::from_ref(&dir), &|p| beside(p)).ids;
        q.mark_where(|_| false);
        q.mark_range(ids[2], ids[1], true);
        assert_eq!(q.entries.iter().map(|e| e.marked).collect::<Vec<_>>(), [false, true, true, false]);
        assert_eq!(q.marked(), 2);
        q.mark_range(ids[0], ids[3], true);
        q.mark_range(ids[3], ids[3], false);
        assert_eq!(q.marked(), 3);

        q.get_mut(ids[1]).unwrap().status = Status::Queued;
        assert!(!q.get(ids[1]).unwrap().convertible(true));
        assert!(q.get(ids[0]).unwrap().convertible(false));
        assert_eq!(q.remove_where(|e| e.marked), 2);
        assert_eq!(q.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [ids[1], ids[3]]);
        assert_eq!(q.index(ids[3]), Some(1));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dng_that_is_there_holds_a_file_back_unless_it_is_written_over() {
        let dir = folder("there", &["1.raf", "1.dng", "2.raf"]);
        let mut q = Queue::default();
        q.add(std::slice::from_ref(&dir), &|p| beside(p));
        q.mark_where(|_| true);
        assert!(!q.entries[0].convertible(false) && q.entries[0].convertible(true));
        assert!(q.entries[1].convertible(false));
        // Elsewhere there is none.
        let other = dir.join("out");
        q.look_for_dngs(&|p| other.join(p.file_name().unwrap()));
        assert!(q.entries[0].dng.is_none() && q.entries[0].convertible(false));
        let _ = fs::remove_dir_all(dir);
    }
}

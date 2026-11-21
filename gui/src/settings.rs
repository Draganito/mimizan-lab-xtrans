//! What is kept from one start to the next: where the DNGs go and how
//! they are written. A few lines of `key=value` in the user's
//! configuration folder.

use mimizan_lab_xtrans::file::{self, Options};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Write the DNGs into [`Settings::folder`], not beside their RAFs.
    pub into_folder: bool,
    pub folder: Option<PathBuf>,
    pub force: bool,
    pub keep_highlights: bool,
    pub uncompressed: bool,
    /// Begin a file that needs more memory than is free, as long as the
    /// swap holds the rest.
    pub use_swap: bool,
    /// Where the last files were added from.
    pub last_dir: Option<PathBuf>,
    /// False colour suppression on RawTherapee's 3-pass. Off leaves it at
    /// none; the steps stay, from 0 to 5, for the next time it is on.
    pub suppression_on: bool,
    pub suppression: u8,
    /// `rawtherapee-cli`, when it was chosen by hand. Empty is looked up.
    pub rawtherapee: Option<PathBuf>,
}

impl Settings {
    /// The DNG `raf` is written to.
    pub fn target(&self, raf: &Path) -> PathBuf {
        match (&self.folder, self.into_folder) {
            (Some(folder), true) => folder.join(raf.with_extension("dng").file_name().unwrap_or_default()),
            _ => file::beside(raf),
        }
    }

    pub fn options(&self) -> Options {
        Options { keep_highlights: self.keep_highlights, uncompressed: self.uncompressed, force: self.force }
    }

    fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let on = value == "1";
            let path = (!value.is_empty()).then(|| PathBuf::from(value));
            match key {
                "into_folder" => s.into_folder = on,
                "folder" => s.folder = path,
                "force" => s.force = on,
                "keep_highlights" => s.keep_highlights = on,
                "uncompressed" => s.uncompressed = on,
                "use_swap" => s.use_swap = on,
                "last_dir" => s.last_dir = path,
                "suppression_on" => s.suppression_on = on,
                "suppression" => s.suppression = value.parse().unwrap_or(0).min(5),
                "rawtherapee" => s.rawtherapee = path.filter(|p| p.is_file() && is_rawtherapee_cli(p)),
                _ => {}
            }
        }
        s.into_folder &= s.folder.is_some();
        s
    }

    fn text(&self) -> String {
        // A path that is not text, or runs over a line, is not kept.
        let path = |p: &Option<PathBuf>| {
            p.as_deref().and_then(Path::to_str).filter(|s| !s.contains('\n')).unwrap_or_default().to_owned()
        };
        let on = |b: bool| u8::from(b);
        format!(
            "into_folder={}\nfolder={}\nforce={}\nkeep_highlights={}\nuncompressed={}\nuse_swap={}\nlast_dir={}\nsuppression_on={}\nsuppression={}\nrawtherapee={}\n",
            on(self.into_folder),
            path(&self.folder),
            on(self.force),
            on(self.keep_highlights),
            on(self.uncompressed),
            on(self.use_swap),
            path(&self.last_dir),
            on(self.suppression_on),
            self.suppression.min(5),
            path(&self.rawtherapee),
        )
    }

    pub fn load() -> Self {
        place().and_then(|p| fs::read_to_string(p).ok()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    /// Kept if it can be; a folder that cannot be written is no reason
    /// to stop.
    pub fn save(&self) {
        let Some(place) = place() else { return };
        if let Some(dir) = place.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(place, self.text());
    }
}

/// The file the settings are kept in.
fn place() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let dir = if cfg!(target_os = "windows") {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
    };
    Some(dir?.join("mimizan-lab-xtrans").join("window.conf"))
}

/// The file is the RawTherapee command, not a photograph that happens
/// to have been picked in the same dialog.
pub fn is_rawtherapee_cli(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()).map(|name| name.to_ascii_lowercase()).as_deref(),
        Some("rawtherapee-cli") | Some("rawtherapee-cli.exe")
    )
}

/// The user's own folder, where a dialog opens when nothing was opened
/// before.
pub fn home() -> Option<PathBuf> {
    let name = if cfg!(target_os = "windows") { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{fs, Settings};
    use std::path::{Path, PathBuf};

    #[test]
    fn what_is_written_is_read_back() {
        let s = Settings {
            into_folder: true,
            folder: Some(PathBuf::from("/pictures/dng = out")),
            force: true,
            keep_highlights: false,
            uncompressed: true,
            use_swap: true,
            last_dir: Some(PathBuf::from("/pictures/raf")),
            suppression_on: true,
            suppression: 3,
            rawtherapee: None,
        };
        assert_eq!(Settings::parse(&s.text()), s);
        assert_eq!(Settings::parse(&Settings::default().text()), Settings::default());
        assert_eq!(
            Settings::parse("nonsense\nforce=1\nwhat=ever\n"),
            Settings { force: true, ..Default::default() }
        );
    }

    #[test]
    fn the_dng_goes_beside_the_raf_unless_a_folder_is_chosen() {
        let raf = Path::new("/a/b/DSCF0001.RAF");
        let mut s = Settings::default();
        assert_eq!(s.target(raf), Path::new("/a/b/DSCF0001.dng"));
        s.folder = Some(PathBuf::from("/out"));
        assert_eq!(s.target(raf), Path::new("/a/b/DSCF0001.dng"));
        s.into_folder = true;
        assert_eq!(s.target(raf), Path::new("/out/DSCF0001.dng"));
        // A folder that was never chosen is not one to write into.
        assert!(!Settings::parse("into_folder=1\nfolder=\n").into_folder);
    }

    #[test]
    fn a_photograph_is_not_kept_as_the_program() {
        let dir = std::env::temp_dir().join(format!("xtrans-cli-name-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let raf = dir.join("photo.RAF");
        let cli = dir.join("rawtherapee-cli");
        fs::write(&raf, b"x").unwrap();
        fs::write(&cli, b"x").unwrap();
        assert!(Settings::parse(&format!("rawtherapee={}\n", raf.display())).rawtherapee.is_none());
        assert_eq!(
            Settings::parse(&format!("rawtherapee={}\n", cli.display())).rawtherapee.as_deref(),
            Some(cli.as_path())
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

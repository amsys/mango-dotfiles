//! Watches $XDG_RUNTIME_DIR/mango-powermode — a bare mode word (`full` |
//! `battery` | `eco`), no trailing newline, written in place by
//! mango/scripts/powermode.sh:446 (`printf '%s' "$m" > "$MODE_FILE"`, a
//! truncating write, not an atomic rename — so watching the file's own inode
//! via IN_CLOSE_WRITE is correct; the inode never changes under us).
//!
//! T1 only tracks and logs the mode — the period table it will drive has no
//! consumers until a later stage's cpu/mem collectors exist.

use crate::sys::FileWatch;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Full,
    Battery,
    Eco,
}

impl Mode {
    fn parse(s: &str) -> Self {
        match s.trim() {
            "battery" => Mode::Battery,
            "eco" => Mode::Eco,
            _ => Mode::Full, // absent file, or anything unrecognized, defaults full
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::Battery => "battery",
            Mode::Eco => "eco",
        }
    }
}

pub struct PowermodeWatch {
    path: PathBuf,
    watch: FileWatch,
    pub mode: Mode,
}

impl AsRawFd for PowermodeWatch {
    fn as_raw_fd(&self) -> RawFd {
        self.watch.as_raw_fd()
    }
}

impl PowermodeWatch {
    pub fn new() -> std::io::Result<Self> {
        let path = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join("mango-powermode");
        let watch = FileWatch::new(path.to_str().expect("path is valid UTF-8"))?;
        let mode = Self::read(&path);
        Ok(Self { path, watch, mode })
    }

    fn read(path: &PathBuf) -> Mode {
        Mode::parse(&std::fs::read_to_string(path).unwrap_or_default())
    }

    /// Call after the fd is readable. Returns Some(new_mode) only on an
    /// actual change, so the caller knows whether to log/act.
    pub fn on_event(&mut self) -> Option<Mode> {
        self.watch.drain();
        let mode = Self::read(&self.path);
        if mode != self.mode {
            self.mode = mode;
            Some(mode)
        } else {
            None
        }
    }
}

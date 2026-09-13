//! Watches SC's `Game.log` for a *new* file: the game starts a fresh log at
//! every start (the old one is moved away or overwritten), and its device
//! enumeration lands in the first seconds of it. Changes inside a running log
//! are of no interest — only a replaced file carries a new device order.
//!
//! A metadata poll, not an OS watcher: a watch on the path breaks at the very
//! moment of interest (the file is replaced, not modified), a directory watch
//! reports every line the game writes, and NTFS hands a file recreated under
//! the same name within 15 s its predecessor's creation time (name tunneling),
//! so the creation time alone cannot be trusted either. A log only grows, so a
//! shorter file is a new one.
//!
//! Once a new file is seen it is read incrementally ([`Tail`]: only what the
//! game appended since the last read) for the whole [`SETTLE`] window: the
//! joystick lines land one by one, so the first one found is not the
//! enumeration yet.
//!
//! The pure state machine lives here (testable without a file system); the
//! polling thread that drives it is `lib.rs::spawn_game_log_watch`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Poll interval of the watch thread.
pub const POLL: Duration = Duration::from_secs(2);
/// How long a new file is re-read for its enumeration: the game writes the
/// device lines during engine init, well within this.
pub const SETTLE: Duration = Duration::from_secs(90);

/// What identifies one `Game.log` file on disk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileStamp {
    /// The OS creation time where the file system has one.
    pub created: Option<SystemTime>,
    pub len: u64,
}

/// The file's current stamp, `None` when it does not exist (or is unreadable).
pub fn stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp { created: meta.created().ok(), len: meta.len() })
}

/// Whether the file seen now is another one than the file seen last:
/// appeared, vanished, another creation time, or shorter than before.
pub fn is_new_file(last: Option<FileStamp>, now: Option<FileStamp>) -> bool {
    match (last, now) {
        (None, None) => false,
        (None, Some(_)) | (Some(_), None) => true,
        (Some(a), Some(b)) => a.created != b.created || b.len < a.len,
    }
}

/// What the watch thread does after a poll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Same file as before, or nothing to settle: read nothing.
    Idle,
    /// Another path (the first poll, or another environment): read the whole
    /// file once, whatever it holds — no window, nothing is expected to
    /// turn up.
    Adopt,
    /// A new file is settling: read it. `new_file` marks the poll that saw
    /// the replacement (start the tail over); `settled` the last read of the
    /// window — an outcome without the enumeration is final then, before
    /// that it is still expected to turn up.
    Read { settled: bool, new_file: bool },
}

/// The watch's state between polls.
#[derive(Debug, Default)]
pub struct Watch {
    path: PathBuf,
    last: Option<FileStamp>,
    settle_until: Option<Instant>,
}

impl Watch {
    /// Feed one poll: the path being watched (a changed path — the first
    /// poll, another environment — is adopted: read once, no window), the
    /// file's current stamp and the time.
    pub fn poll(&mut self, path: &Path, stamp: Option<FileStamp>, now: Instant) -> Step {
        if path != self.path {
            self.path = path.to_path_buf();
            self.last = stamp;
            self.settle_until = None;
            return Step::Adopt;
        }
        let new_file = is_new_file(self.last, stamp);
        if new_file {
            self.settle_until = Some(now + SETTLE);
        }
        self.last = stamp;
        match self.settle_until {
            None => Step::Idle,
            Some(until) if now >= until => {
                self.settle_until = None;
                Step::Read { settled: true, new_file }
            }
            Some(_) => Step::Read { settled: false, new_file },
        }
    }
}

/// The text of the file being settled, read incrementally: each [`Tail::read`]
/// appends only what the game wrote since the last one, so a growing log is
/// never read twice. Lossy UTF-8 (the device lines are ASCII; a multi-byte
/// character cut at a read boundary garbles only itself).
#[derive(Debug, Default)]
pub struct Tail {
    pos: u64,
    pub text: String,
}

impl Tail {
    /// Forget the file: the next read starts from its beginning.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Append what the file gained since the last read. A file shorter than
    /// the last position was replaced meanwhile: start over.
    pub fn read(&mut self, path: &Path) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        if len < self.pos {
            self.reset();
        }
        if len == self.pos {
            return Ok(());
        }
        file.seek(SeekFrom::Start(self.pos))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        self.text.push_str(&String::from_utf8_lossy(&bytes));
        self.pos += bytes.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(created: u64, len: u64) -> Option<FileStamp> {
        Some(FileStamp { created: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(created)), len })
    }

    fn watch_at(path: &Path, stamp: Option<FileStamp>, t0: Instant) -> Watch {
        let mut w = Watch::default();
        assert_eq!(w.poll(path, stamp, t0), Step::Adopt, "the first poll adopts the file");
        w
    }

    const READ: Step = Step::Read { settled: false, new_file: false };
    const NEW: Step = Step::Read { settled: false, new_file: true };
    const LAST: Step = Step::Read { settled: true, new_file: false };

    #[test]
    fn same_file_growing_is_idle() {
        let p = Path::new("Game.log");
        let t0 = Instant::now();
        let mut w = watch_at(p, st(1, 100), t0);
        assert_eq!(w.poll(p, st(1, 100), t0 + POLL), Step::Idle);
        assert_eq!(w.poll(p, st(1, 5000), t0 + 2 * POLL), Step::Idle);
    }

    #[test]
    fn shorter_file_settles_until_the_window_ends() {
        let p = Path::new("Game.log");
        let t0 = Instant::now();
        let mut w = watch_at(p, st(1, 100_000), t0);
        // Same creation time (NTFS tunneling), a shorter file: a new log.
        assert_eq!(w.poll(p, st(1, 300), t0 + POLL), NEW);
        // Growing within the window: still settling, not a new file again.
        assert_eq!(w.poll(p, st(1, 900), t0 + 2 * POLL), READ);
        assert_eq!(w.poll(p, st(1, 9000), t0 + POLL + SETTLE), LAST);
        assert_eq!(w.poll(p, st(1, 9001), t0 + 2 * POLL + SETTLE), Step::Idle);
    }

    #[test]
    fn other_creation_time_is_a_new_file() {
        let p = Path::new("Game.log");
        let t0 = Instant::now();
        let mut w = watch_at(p, st(1, 100), t0);
        assert_eq!(w.poll(p, st(2, 100), t0 + POLL), NEW);
    }

    #[test]
    fn vanishing_and_appearing_are_new_files() {
        let p = Path::new("Game.log");
        let t0 = Instant::now();
        let mut w = watch_at(p, st(1, 100), t0);
        assert_eq!(w.poll(p, None, t0 + POLL), NEW);
        assert_eq!(w.poll(p, None, t0 + 2 * POLL), READ);
        // Recreated within the window: the window restarts.
        assert_eq!(w.poll(p, st(5, 10), t0 + 3 * POLL), NEW);
        assert_eq!(w.poll(p, st(5, 50), t0 + 3 * POLL + SETTLE), LAST);
        assert_eq!(w.poll(p, st(5, 60), t0 + 4 * POLL + SETTLE), Step::Idle);
    }

    #[test]
    fn no_file_at_all_is_idle() {
        let p = Path::new("Game.log");
        let t0 = Instant::now();
        let mut w = watch_at(p, None, t0);
        assert_eq!(w.poll(p, None, t0 + POLL), Step::Idle);
    }

    #[test]
    fn another_path_is_adopted_with_one_read() {
        let a = Path::new("LIVE/Game.log");
        let b = Path::new("PTU/Game.log");
        let t0 = Instant::now();
        let mut w = watch_at(a, st(1, 100_000), t0);
        assert_eq!(w.poll(b, st(7, 10), t0 + POLL), Step::Adopt);
        assert_eq!(w.poll(b, st(7, 20), t0 + 2 * POLL), Step::Idle);
        assert_eq!(w.poll(b, st(7, 5), t0 + 3 * POLL), NEW);
        // A settling window is dropped with the old path.
        assert_eq!(w.poll(a, st(1, 100_000), t0 + 4 * POLL), Step::Adopt);
        assert_eq!(w.poll(a, st(1, 100_001), t0 + 5 * POLL), Step::Idle);
        // A missing file is adopted too: the error is the outcome.
        assert_eq!(w.poll(b, None, t0 + 6 * POLL), Step::Adopt);
        assert_eq!(w.poll(b, None, t0 + 7 * POLL), Step::Idle);
    }

    #[test]
    fn stamp_of_a_missing_file_is_none() {
        assert_eq!(stamp(Path::new("definitely/not/here/Game.log")), None);
    }

    #[test]
    fn tail_reads_only_what_was_appended() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("bindsight-tail-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(&path, "<t> first line\n").unwrap();
        let mut tail = Tail::default();
        tail.read(&path).unwrap();
        assert_eq!(tail.text, "<t> first line\n");

        // Nothing new: nothing read, nothing doubled.
        tail.read(&path).unwrap();
        assert_eq!(tail.text, "<t> first line\n");

        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"<t> - Connected joystick0: X {G}\n").unwrap();
        drop(f);
        tail.read(&path).unwrap();
        assert_eq!(tail.text, "<t> first line\n<t> - Connected joystick0: X {G}\n");

        // Replaced by a shorter file: starts over.
        std::fs::write(&path, "<t> new\n").unwrap();
        tail.read(&path).unwrap();
        assert_eq!(tail.text, "<t> new\n");

        std::fs::remove_file(&path).unwrap();
        assert!(tail.read(&path).is_err());
    }

    /// The game logs its joysticks one line at a time (200 ms apart on a
    /// two-stick setup): a read landing between them sees one joystick, the
    /// next read the second — the outcome must keep being re-derived from
    /// the whole tail while the window is open.
    #[test]
    fn joystick_lines_arriving_in_separate_reads_add_up() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("bindsight-tail-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(
            &path,
            "<2026-09-13T22:59:18.272Z> - Connected joystick0:  VKBsim Gladiator EVO  R    {0200231D-0000-0000-0000-504944564944}\n",
        )
        .unwrap();
        let mut tail = Tail::default();
        tail.read(&path).unwrap();
        let one = crate::gamelog::parse(&tail.text).unwrap();
        assert_eq!(one.joysticks.len(), 1);

        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"<2026-09-13T22:59:18.479Z> - Connected joystick1:  VKBsim Gladiator EVO  L    {0201231D-0000-0000-0000-504944564944}\n")
            .unwrap();
        drop(f);
        tail.read(&path).unwrap();
        let two = crate::gamelog::parse(&tail.text).unwrap();
        assert_eq!(two.joysticks.len(), 2);
        assert_eq!(two.joysticks[1].instance, 2);
        assert_ne!(one, two);
        std::fs::remove_file(&path).unwrap();
    }
}

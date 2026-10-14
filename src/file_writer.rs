//! Durable file writes, off the UI thread.
//!
//! [`crate::util::write_with`] replaces a file whole and waits until the
//! new file and its move have reached the disk. Under heavy I/O that wait
//! lasts seconds, and the UI thread must never make it: the window would
//! stop drawing and every control, Pause included, would wait with it. The
//! UI thread encodes what it saves (cheap) and hands the bytes to the
//! [`FileWriter`], which writes them on a thread of its own.
//!
//! Writes to one path keep their order. A newer write to a path whose
//! previous write has not started yet replaces it, so only the newest
//! contents go to the disk. [`FileWriter::flush`] waits, for a bounded
//! time, until everything handed over is written: before the app exits and
//! before the system suspends.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

pub use crate::util::Access;

/// How long exit waits for pending writes.
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(4);

type WriteFn = dyn Fn(&Path, &[u8], Access) -> std::io::Result<()> + Send + Sync;
type OnFailure = Box<dyn FnOnce() + Send>;

struct Job {
    path: PathBuf,
    contents: Vec<u8>,
    access: Access,
    on_failure: Option<OnFailure>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    writing: bool,
    closed: bool,
}

impl State {
    fn pending(&self) -> usize {
        self.queue.len() + usize::from(self.writing)
    }
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    write: Box<WriteFn>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn run(&self, job: Job) {
        if let Err(error) = (self.write)(&job.path, &job.contents, job.access) {
            log::warn!("could not write {}: {error}", job.path.display());
            if let Some(on_failure) = job.on_failure {
                on_failure();
            }
        }
    }
}

/// Writes files whole and durably on a background thread.
pub struct FileWriter {
    shared: Arc<Shared>,
    /// False when the thread could not start; writes then happen in place.
    threaded: bool,
}

impl Default for FileWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWriter {
    pub fn new() -> Self {
        Self::with_write(|path, contents, access| {
            crate::util::write_with(path, access, |file| {
                std::io::Write::write_all(file, contents)
            })
        })
    }

    /// A writer that writes through `write`, for tests that need to hold a
    /// write back or watch what is written.
    pub fn with_write(
        write: impl Fn(&Path, &[u8], Access) -> std::io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            write: Box::new(write),
        });
        let worker = Arc::clone(&shared);
        let threaded = std::thread::Builder::new()
            .name("fastsonic-files".into())
            .spawn(move || work(&worker))
            .map_err(|error| log::error!("could not start the file writer: {error}"))
            .is_ok();
        Self { shared, threaded }
    }

    /// Replaces `path` with `contents`, later and off this thread.
    pub fn write(&self, path: &Path, contents: impl Into<Vec<u8>>, access: Access) {
        self.submit(path, contents.into(), access, None);
    }

    /// As [`Self::write`], calling `on_failure` on the writer's thread if
    /// the write fails. A write replaced by a newer one never calls it.
    pub fn write_or(
        &self,
        path: &Path,
        contents: impl Into<Vec<u8>>,
        access: Access,
        on_failure: impl FnOnce() + Send + 'static,
    ) {
        self.submit(path, contents.into(), access, Some(Box::new(on_failure)));
    }

    fn submit(
        &self,
        path: &Path,
        contents: Vec<u8>,
        access: Access,
        on_failure: Option<OnFailure>,
    ) {
        let job = Job {
            path: path.to_owned(),
            contents,
            access,
            on_failure,
        };
        if !self.threaded {
            self.shared.run(job);
            return;
        }
        let mut state = self.shared.lock();
        // A write already started stays as it is; the newer contents follow it.
        if let Some(waiting) = state
            .queue
            .iter_mut()
            .find(|waiting| waiting.path == job.path)
        {
            *waiting = job;
        } else {
            state.queue.push_back(job);
        }
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Waits until every write handed over so far is done, or `timeout`
    /// passes. Returns whether everything was written; a timeout is logged.
    pub fn flush(&self, timeout: Duration) -> bool {
        let state = self.shared.lock();
        let (state, waited) = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |state| state.pending() > 0)
            .unwrap_or_else(|p| p.into_inner());
        if waited.timed_out() {
            log::warn!(
                "file writes still waiting for the disk after {} ms: {}",
                timeout.as_millis(),
                state.pending()
            );
        }
        !waited.timed_out()
    }
}

impl Drop for FileWriter {
    /// The thread finishes what it holds and stops; callers that need the
    /// writes done flush first.
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.changed.notify_all();
    }
}

fn work(shared: &Shared) {
    let mut state = shared.lock();
    loop {
        if let Some(job) = state.queue.pop_front() {
            state.writing = true;
            drop(state);
            shared.run(job);
            state = shared.lock();
            state.writing = false;
            shared.changed.notify_all();
        } else if state.closed {
            return;
        } else {
            state = shared
                .changed
                .wait(state)
                .unwrap_or_else(|p| p.into_inner());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    /// What a gated writer wrote, in order.
    type Written = mpsc::Receiver<(PathBuf, Vec<u8>)>;

    /// A writer whose writes wait for `release`, recording what each wrote.
    fn gated() -> (FileWriter, mpsc::Sender<()>, Written) {
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (written, writes) = mpsc::channel();
        let writer = FileWriter::with_write(move |path, contents, _| {
            gate.lock().unwrap().recv().unwrap();
            written.send((path.to_owned(), contents.to_vec())).unwrap();
            Ok(())
        });
        (writer, release, writes)
    }

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!(
            "fastsonic-file-writer-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    /// A save hands its bytes over and returns while the disk is still busy.
    #[test]
    fn a_write_does_not_wait_for_the_disk() {
        let (writer, release, writes) = gated();
        let started = Instant::now();
        writer.write(Path::new("settings.json"), b"{}".to_vec(), Access::Default);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(writes.try_recv().is_err(), "nothing written yet");
        release.send(()).unwrap();
        assert_eq!(
            writes.recv_timeout(Duration::from_secs(5)).unwrap(),
            ("settings.json".into(), b"{}".to_vec())
        );
    }

    /// While one write waits for the disk, later saves of the same file
    /// collapse into the newest; other files keep their own writes.
    #[test]
    fn waiting_writes_keep_only_the_newest_contents() {
        let (writer, release, writes) = gated();
        let settings = Path::new("settings.json");
        writer.write(settings, b"1".to_vec(), Access::Default);
        // Let the first write start, so it is held at the gate.
        while !writer.shared.lock().writing {
            std::thread::yield_now();
        }
        writer.write(settings, b"2".to_vec(), Access::Default);
        writer.write(Path::new("session.json"), b"s".to_vec(), Access::Default);
        writer.write(settings, b"3".to_vec(), Access::Default);
        for _ in 0..3 {
            release.send(()).unwrap();
        }
        assert!(writer.flush(Duration::from_secs(5)));
        let written: Vec<_> = writes.try_iter().collect();
        assert_eq!(
            written,
            [
                ("settings.json".into(), b"1".to_vec()),
                ("settings.json".into(), b"3".to_vec()),
                ("session.json".into(), b"s".to_vec()),
            ]
        );
    }

    /// Flush returns once pending writes are done, and gives up after its
    /// timeout when the disk does not answer.
    #[test]
    fn flush_waits_for_pending_writes() {
        let (writer, release, writes) = gated();
        writer.write(Path::new("history.json"), b"[]".to_vec(), Access::Private);
        assert!(!writer.flush(Duration::from_millis(50)), "still held");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            release.send(()).unwrap();
        });
        assert!(writer.flush(Duration::from_secs(5)));
        assert_eq!(writes.try_iter().count(), 1);
        releaser.join().unwrap();
        assert!(writer.flush(Duration::ZERO), "nothing pending");
    }

    /// The real writer replaces the file whole.
    #[test]
    fn the_default_writer_replaces_the_file() {
        let dir = scratch();
        let path = dir.join("nested").join("settings.json");
        let writer = FileWriter::new();
        writer.write(&path, "old", Access::Default);
        writer.write(&path, "new", Access::Default);
        assert!(writer.flush(Duration::from_secs(10)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(crate::util::temporary_files(path.parent().unwrap()).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A failed write reports back, so its owner can write again later.
    #[test]
    fn a_failed_write_reports_back() {
        let writer = FileWriter::with_write(|_, _, _| Err(std::io::Error::other("disk full")));
        let (failed, failures) = mpsc::channel();
        writer.write_or(
            Path::new("history.json"),
            "[]",
            Access::Private,
            move || {
                failed.send(()).unwrap();
            },
        );
        assert!(writer.flush(Duration::from_secs(5)));
        assert!(failures.try_recv().is_ok());
    }
}

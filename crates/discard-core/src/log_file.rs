//! Rotating log files. Clear and writes share a lock.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tracing_subscriber::fmt::MakeWriter;
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
pub const KEEP_ROTATED: usize = 4;
pub const ACTIVE_NAME: &str = "discard.log";
static FILE_LOCK: Mutex<()> = Mutex::new(());
fn rotated(dir: &Path, n: usize) -> PathBuf {
    dir.join(format!("{ACTIVE_NAME}.{n}"))
}
pub struct RotatingFile {
    dir: PathBuf,
    file: File,
    max_bytes: u64,
    keep: usize,
}
impl RotatingFile {
    pub fn open(dir: &Path, max_bytes: u64, keep: usize) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(ACTIVE_NAME))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            file,
            max_bytes,
            keep,
        })
    }
    fn rotate(&mut self) -> io::Result<()> {
        if self.keep == 0 {
            self.file.set_len(0)?;
            return Ok(());
        }
        let oldest = rotated(&self.dir, self.keep);
        if oldest.exists() {
            fs::remove_file(oldest)?;
        }
        for n in (1..self.keep).rev() {
            let from = rotated(&self.dir, n);
            if from.exists() {
                fs::rename(from, rotated(&self.dir, n + 1))?;
            }
        }
        fs::rename(self.dir.join(ACTIVE_NAME), rotated(&self.dir, 1))?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(ACTIVE_NAME))?;
        Ok(())
    }
}
impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _guard = FILE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if self.file.metadata()?.len().saturating_add(buf.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
pub fn log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(usize, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            if name == ACTIVE_NAME {
                Some((0, e.path()))
            } else {
                name.strip_prefix("discard.log.")?
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n > 0)
                    .map(|n| (n, e.path()))
            }
        })
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    files.into_iter().map(|(_, p)| p).collect()
}
pub fn clear(dir: &Path) -> io::Result<usize> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut count = 0;
    for path in log_files(dir) {
        if path.file_name().is_some_and(|n| n == ACTIVE_NAME) {
            OpenOptions::new().write(true).open(path)?.set_len(0)?;
        } else {
            fs::remove_file(path)?;
        }
        count += 1;
    }
    Ok(count)
}
#[derive(Clone)]
pub struct SharedFile(Arc<Mutex<RotatingFile>>);
impl SharedFile {
    pub fn new(file: RotatingFile) -> Self {
        Self(Arc::new(Mutex::new(file)))
    }
}
impl Write for SharedFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).flush()
    }
}
impl<'a> MakeWriter<'a> for SharedFile {
    type Writer = SharedFile;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn appends_lines_to_the_active_file() {
        let d = tempfile::tempdir().unwrap();
        let mut f = RotatingFile::open(d.path(), 100, 2).unwrap();
        writeln!(f, "first").unwrap();
        drop(f);
        let mut f = RotatingFile::open(d.path(), 100, 2).unwrap();
        writeln!(f, "second").unwrap();
        assert_eq!(
            std::fs::read_to_string(d.path().join(ACTIVE_NAME)).unwrap(),
            "first\nsecond\n"
        );
    }
    #[test]
    fn rotates_before_a_write_that_would_pass_the_limit_and_never_splits_a_line() {
        let d = tempfile::tempdir().unwrap();
        let mut f = RotatingFile::open(d.path(), 100, 2).unwrap();
        for c in ['a', 'b', 'c'] {
            f.write_all(format!("{}\n", c.to_string().repeat(39)).as_bytes())
                .unwrap();
        }
        assert_eq!(
            std::fs::read(d.path().join("discard.log.1")).unwrap().len(),
            80
        );
        assert_eq!(std::fs::read(d.path().join(ACTIVE_NAME)).unwrap().len(), 40);
    }
    #[test]
    fn keeps_only_the_newest_rotated_files() {
        let d = tempfile::tempdir().unwrap();
        let mut f = RotatingFile::open(d.path(), 3, 2).unwrap();
        for c in ['a', 'b', 'c', 'd', 'e'] {
            f.write_all(format!("{c}\n").as_bytes()).unwrap();
        }
        assert!(!d.path().join("discard.log.3").exists());
        assert_eq!(
            std::fs::read_to_string(d.path().join("discard.log.1")).unwrap(),
            "d\n"
        );
    }
    #[test]
    fn log_files_lists_oldest_first() {
        let d = tempfile::tempdir().unwrap();
        let mut f = RotatingFile::open(d.path(), 2, 2).unwrap();
        for c in ['a', 'b', 'c'] {
            writeln!(f, "{c}").unwrap();
        }
        assert_eq!(
            log_files(d.path()),
            vec![
                d.path().join("discard.log.2"),
                d.path().join("discard.log.1"),
                d.path().join(ACTIVE_NAME)
            ]
        );
    }
    #[test]
    fn clear_truncates_the_active_file_and_deletes_the_rotated_ones_and_later_writes_land_at_the_start()
     {
        let d = tempfile::tempdir().unwrap();
        let mut f = RotatingFile::open(d.path(), 3, 2).unwrap();
        f.write_all(b"a\n").unwrap();
        f.write_all(b"b\n").unwrap();
        assert_eq!(clear(d.path()).unwrap(), 2);
        f.write_all(b"c\n").unwrap();
        assert_eq!(std::fs::read(d.path().join(ACTIVE_NAME)).unwrap(), b"c\n");
    }
    #[test]
    fn clear_on_a_missing_folder_is_zero() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(clear(&d.path().join("missing")).unwrap(), 0);
    }
    #[test]
    fn open_creates_the_directory() {
        let d = tempfile::tempdir().unwrap();
        RotatingFile::open(&d.path().join("new"), 100, 2).unwrap();
        assert!(d.path().join("new/discard.log").exists());
    }
    #[test]
    fn open_in_a_path_under_a_file_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("file");
        std::fs::write(&path, "x").unwrap();
        assert!(RotatingFile::open(&path.join("bad"), 100, 2).is_err());
    }
    #[test]
    fn shared_file_clones_write_to_one_file_in_order() {
        let d = tempfile::tempdir().unwrap();
        let f = SharedFile::new(RotatingFile::open(d.path(), 100, 2).unwrap());
        let mut a = f.clone();
        let mut b = f.clone();
        std::thread::spawn(move || a.write_all(b"a\n").unwrap())
            .join()
            .unwrap();
        std::thread::spawn(move || b.write_all(b"b\n").unwrap())
            .join()
            .unwrap();
        assert_eq!(
            std::fs::read(d.path().join(ACTIVE_NAME)).unwrap(),
            b"a\nb\n"
        );
    }
    #[test]
    fn the_file_gets_the_redacted_line_and_never_the_secret() {
        let d = tempfile::tempdir().unwrap();
        let f = SharedFile::new(RotatingFile::open(d.path(), 10000, 2).unwrap());
        let sub = tracing_subscriber::fmt()
            .json()
            .with_writer(crate::logging::RedactingMakeWriter(f))
            .finish();
        tracing::subscriber::with_default(sub, || {
            tracing::info!("api_key=sk-live-secret-value-12345")
        });
        let text = std::fs::read_to_string(d.path().join(ACTIVE_NAME)).unwrap();
        assert!(text.contains("[REDACTED]"));
        assert!(!text.contains("sk-live-secret"));
    }
}

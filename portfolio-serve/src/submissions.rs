use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub(super) struct Submissions {
    data: PathBuf,
}

impl Submissions {
    pub(super) fn open(data: &Path, preprovisioned: bool) -> io::Result<Self> {
        if !preprovisioned {
            fs::create_dir_all(data)?;
        }
        let archive = Self { data: data.into() };
        let _lock = archive.exclusive_lock(!preprovisioned)?;
        let file = archive.open_file("submissions.jsonl", !preprovisioned, true)?;
        file.sync_all()?;
        #[cfg(unix)]
        if !preprovisioned {
            File::open(data)?.sync_all()?;
        }
        Ok(archive)
    }

    fn open_file(&self, name: &str, create: bool, append: bool) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(create)
            .create(create)
            .append(append);
        #[cfg(unix)]
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = options.open(self.data.join(name))?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid submission store",
            ));
        }
        Ok(file)
    }

    fn exclusive_lock(&self, create: bool) -> io::Result<File> {
        // The broker and writer use the same stable flock inode, including
        // across atomic replacements of the JSONL file.
        let lock = self.open_file("submissions.lock", create, false)?;
        lock.lock()?;
        Ok(lock)
    }

    pub(super) fn append(&self, line: &[u8]) -> io::Result<()> {
        let _lock = self.exclusive_lock(false)?;
        // Open only after obtaining the lock. A cached descriptor could still
        // address an unlinked archive after a private deletion.
        let mut file = self.open_file("submissions.jsonl", false, true)?;
        if file.metadata()?.len() != 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "incomplete submission tail",
                ));
            }
        }
        file.write_all(line)?;
        file.sync_data()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn concurrent_append_follows_an_atomic_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let archive = Submissions::open(temp.path(), false).unwrap();
        archive.append(b"old\n").unwrap();
        let lock = File::open(temp.path().join("submissions.lock")).unwrap();
        lock.lock().unwrap();
        let (started, starting) = mpsc::channel();
        let (finished, result) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                started.send(()).unwrap();
                finished.send(archive.append(b"new\n")).unwrap();
            });
            starting.recv().unwrap();
            assert!(matches!(
                result.recv_timeout(Duration::from_millis(60)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            let next = temp.path().join("replacement");
            fs::write(&next, b"kept\n").unwrap();
            fs::rename(next, temp.path().join("submissions.jsonl")).unwrap();
            drop(lock);
            result
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
        });
        assert_eq!(
            fs::read(temp.path().join("submissions.jsonl")).unwrap(),
            b"kept\nnew\n"
        );
    }

    #[test]
    fn missing_store_or_partial_tail_is_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        assert!(Submissions::open(temp.path(), true).is_err());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        let archive = Submissions::open(temp.path(), false).unwrap();
        let file = temp.path().join("submissions.jsonl");
        fs::write(&file, b"complete\npartial").unwrap();
        assert!(archive.append(b"later\n").is_err());
        assert_eq!(fs::read(&file).unwrap(), b"complete\npartial");
        fs::remove_file(&file).unwrap();
        assert!(archive.append(b"later\n").is_err());
        assert!(!file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cannot_redirect_appends() {
        let temp = tempfile::tempdir().unwrap();
        let archive = Submissions::open(temp.path(), false).unwrap();
        let data_path = temp.path().join("submissions.jsonl");
        let other = temp.path().join("other");
        fs::rename(&data_path, &other).unwrap();
        std::os::unix::fs::symlink(&other, data_path).unwrap();
        assert!(archive.append(b"new\n").is_err());
        assert!(fs::read(other).unwrap().is_empty());
    }
}

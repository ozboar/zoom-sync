//! Process lock to prevent multiple daemon instances

use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

use directories::ProjectDirs;

/// Guard that holds the lock file open. Lock is released when dropped.
pub struct Lock {
    _file: File,
    path: PathBuf,
}

impl Lock {
    /// Try to acquire an exclusive lock for the daemon.
    /// Returns an error if another instance is already running.
    pub fn acquire() -> io::Result<Self> {
        let path = Self::path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not determine lock file path",
            )
        })?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        // Try to acquire exclusive lock (non-blocking)
        if file.try_lock().is_err() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "another instance of zoom-sync is already running",
            ));
        }

        // Write PID to lock file for debugging
        use std::io::Write;
        let mut file = file;
        writeln!(file, "{}", std::process::id())?;

        Ok(Self { _file: file, path })
    }

    /// Get the lock file path
    fn path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "zoom-sync").map(|dirs| dirs.config_dir().join("zoom-sync.lock"))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Lock is automatically released when file is closed
        // Optionally remove the lock file
        let _ = std::fs::remove_file(&self.path);
    }
}

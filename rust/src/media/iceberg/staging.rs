//! One commit's files on their way to the table, as a transaction.
//!
//! A commit writes data files, a manifest and a manifest list, and only then
//! the metadata document that names them. Over a remote table each of those
//! files is an upload, and what this module settles is what an upload costs
//! and what a failure leaves behind: every file goes out as one upload -
//! multipart above the store's threshold - after being encoded into a local
//! staging file, so at most one file is in memory at a time rather than the
//! whole commit; and every file the commit published is recorded, so a commit
//! that fails removes them again and leaves no file the metadata does not
//! name, on the store or on the local disk.

use std::path::PathBuf;
use std::sync::Mutex;

use super::options::WriteStaging;
use crate::holder::Holder;
use crate::holder::local::Folder;
use crate::{IOBase, MediaType, Result};

/// The files one commit writes, staged locally and rolled back together.
///
/// Shared by every writer thread of a commit by reference: the published
/// files are recorded behind a lock, and the staging directory is the
/// commit's own, never another commit's, so two commits into one table stage
/// apart even when they run at once.
pub(super) struct Staging {
    /// The directory this commit's files are written into first; `None`
    /// writes them straight to the table.
    directory: Option<PathBuf>,
    /// Every file this commit published to the table, removed again when
    /// the commit fails.
    published: Mutex<Vec<Holder>>,
    /// Whether the commit succeeded, which is what keeps the published files.
    committed: bool,
}

impl Staging {
    /// Begin the staging of one commit.
    ///
    /// `staging` is the resolved option; `None` takes the root's own default,
    /// the platform temporary folder when `remote` and off otherwise. The
    /// directory is named by the snapshot and a fresh UUID, so a concurrent
    /// commit into the same table never sees this one's files. Nothing is
    /// created until the first file is staged.
    ///
    /// # Errors
    ///
    /// Returns an error when the staging folder is not a platform path.
    pub(super) fn begin(
        staging: Option<&WriteStaging>,
        remote: bool,
        snapshot_id: i64,
    ) -> Result<Self> {
        let base = match staging {
            Some(WriteStaging::Off) => None,
            Some(WriteStaging::Folder(url)) => Some(url.clone().into_path()?),
            None if remote => Some(Folder::temporary()?.path()?),
            None => None,
        };
        let directory = base.map(|base| {
            base.join(format!(
                "yggdryl-iceberg-{snapshot_id}-{}",
                super::metadata::uuid()
            ))
        });
        Ok(Self {
            directory,
            published: Mutex::new(Vec::new()),
            committed: false,
        })
    }

    /// The directory this commit stages into, when it stages at all.
    #[cfg(test)]
    pub(super) fn directory(&self) -> Option<&std::path::Path> {
        self.directory.as_deref()
    }

    /// Write one file of the commit and answer what `write` answered beside
    /// the file's size.
    ///
    /// Staged, the file is written to the local directory, read back whole
    /// and uploaded as one write, and the local copy is removed before the
    /// upload starts, so the disk holds each writer thread's current file
    /// and nothing else. Unstaged, `write` runs against the table's own
    /// handle. Either way the published file is recorded for the rollback.
    ///
    /// # Errors
    ///
    /// Returns the write's own failure, or the read-back or upload failure.
    pub(super) fn publish<T>(
        &self,
        root: &dyn IOBase,
        relative: &str,
        media_type: &MediaType,
        write: impl FnOnce(&mut Holder) -> Result<T>,
    ) -> Result<(T, u64)> {
        let mut target = leaf(root.child_by_path(relative)?)?;
        target.set_media_type(media_type.clone());
        let Some(directory) = &self.directory else {
            let answer = match write(&mut target).and_then(|answer| {
                target.flush()?;
                Ok(answer)
            }) {
                Ok(answer) => answer,
                Err(error) => return Err(unpublished(target, error)),
            };
            let size = target.size();
            self.record(target)?;
            return Ok((answer, size));
        };
        let mut staged = Holder::file(directory.join(relative))?;
        staged.set_media_type(media_type.clone());
        let answer = write(&mut staged)?;
        staged.flush()?;
        let size = staged.size();
        let bytes = staged.read_all_bytes()?;
        staged.remove(false)?;
        drop(staged);
        if let Err(error) = target.write_all_bytes(&bytes) {
            return Err(unpublished(target, error));
        }
        self.record(target)?;
        Ok((answer, size))
    }

    /// Remember one published file for the rollback.
    fn record(&self, published: Holder) -> Result<()> {
        self.published
            .lock()
            .map_err(|_| poisoned())?
            .push(published);
        Ok(())
    }

    /// End the staging of a commit that succeeded: the published files are
    /// the table's now, and only the staging directory goes.
    pub(super) fn finish(mut self) {
        self.committed = true;
        if let Ok(mut published) = self.published.lock() {
            published.clear();
        }
    }
}

impl Drop for Staging {
    /// Roll back what a failed commit published, and remove the directory.
    ///
    /// A failure here cannot be reported; a file the rollback could not
    /// remove is the orphan a failed commit always could leave, and the
    /// local directory is under the temporary folder.
    fn drop(&mut self) {
        if !self.committed {
            if let Ok(mut published) = self.published.lock() {
                for handle in published.iter_mut().rev() {
                    let _ = handle.remove(false);
                }
                published.clear();
            }
        }
        if let Some(directory) = &self.directory {
            match std::fs::remove_dir_all(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    log::warn!(
                        "the iceberg staging directory {} could not be removed: {error}",
                        directory.display()
                    );
                }
            }
        }
    }
}

/// Let go of a file the commit could not publish, and report why.
///
/// A handle whose upload failed still holds what it staged, and a leaf
/// publishes what it holds when it is dropped: removed first, it publishes
/// nothing, and whatever part of it reached the store goes with it.
fn unpublished(mut target: Holder, error: crate::Error) -> crate::Error {
    let _ = target.remove(false);
    error
}

/// Take a location the metadata names as the file it is.
///
/// A table names files and nothing else - a metadata document, a manifest
/// list, a manifest, a data file - so a handle whose role is undecided needs
/// no listing to settle it: the metadata already answered the question a
/// listing would ask. Every other handle is what it already was.
pub(super) fn leaf(holder: Holder) -> Result<Holder> {
    match holder {
        #[cfg(feature = "object")]
        Holder::ObjectPath(path) => Ok(Holder::ObjectFile(path.as_file()?)),
        other => Ok(other),
    }
}

/// Take a location the table lays out as the container it is.
///
/// The metadata directory is a directory by the table's own layout, so an
/// undecided handle is resolved to its container role without the listing
/// that would ask the store what the layout already says.
pub(super) fn container(holder: Holder) -> Result<Holder> {
    match holder {
        #[cfg(feature = "object")]
        Holder::ObjectPath(path) => Ok(Holder::ObjectFolder(path.as_directory()?)),
        other => Ok(other),
    }
}

/// Tell a file handle the size the manifest recorded for it.
///
/// A manifest states every data file's length, exactly as a listing states
/// every entry's, so a handle built from one starts out knowing it: a scan
/// that asks a file its size before reading it pays no request for the
/// answer. A handle with no such memory is returned as it was.
pub(super) fn sized(holder: Holder, size: u64) -> Holder {
    match holder {
        #[cfg(feature = "object")]
        Holder::ObjectFile(file) => Holder::ObjectFile(file.with_known_size(size)),
        other => other,
    }
}

/// Report a poisoned record lock without panicking a caller.
fn poisoned() -> crate::Error {
    crate::Error::Io(std::io::Error::other(
        "the iceberg staging record was poisoned by a panicking writer",
    ))
}

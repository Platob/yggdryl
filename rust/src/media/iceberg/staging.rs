//! One commit's files on their way to the table, as a transaction.
//!
//! A commit writes data files, a manifest and a manifest list, and only then
//! the metadata document that names them. Over a remote table each of those
//! files is an upload, and what this module settles is what an upload costs
//! and what a failure leaves behind: every file goes out as one upload -
//! multipart above the store's threshold - streamed from a local staging
//! file one part at a time, so memory holds one part of one file per writer
//! thread rather than the whole commit; and every file the commit published
//! is recorded until the versioned metadata document is durable, so a commit
//! that fails before that point removes them again and leaves no file the
//! metadata does not name, on the store or on the local disk. The document
//! is the point of no return: once it is out, the files are the table's,
//! whatever a later step reports.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Every file this commit published to the table that nothing durable
    /// names yet, by its path under the table, removed again when the commit
    /// fails before its document is out.
    published: Mutex<Vec<(String, Holder)>>,
    /// Whether the versioned document went out, which is what keeps the
    /// published files.
    committed: AtomicBool,
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
            committed: AtomicBool::new(false),
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
    /// Staged, the file is written to the local directory and streamed from
    /// there to the table as one upload - a store handle takes it one
    /// multipart part at a time, so memory holds one part of it, and any
    /// other handle takes it whole - and the local copy is removed once the
    /// upload ends, so the disk holds each writer thread's current file and
    /// nothing else. Unstaged, `write` runs against the table's own handle.
    /// Either way the published file is recorded for the rollback.
    ///
    /// # Errors
    ///
    /// Returns the write's own failure, or the upload failure.
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
            self.record(relative, target)?;
            return Ok((answer, size));
        };
        let path = directory.join(relative);
        let mut staged = Holder::file(&path)?;
        staged.set_media_type(media_type.clone());
        let answer = write(&mut staged)?;
        staged.flush()?;
        let size = staged.size();
        drop(staged);
        let uploaded = upload(&mut target, &path, size);
        // The staged copy has served its purpose however the upload ended.
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "the iceberg staging file {} could not be removed: {error}",
                    path.display()
                );
            }
        }
        if let Err(error) = uploaded {
            return Err(unpublished(target, error));
        }
        self.record(relative, target)?;
        Ok((answer, size))
    }

    /// Remember one published file for the rollback.
    fn record(&self, relative: &str, published: Holder) -> Result<()> {
        self.published
            .lock()
            .map_err(|_| poisoned())?
            .push((relative.to_owned(), published));
        Ok(())
    }

    /// Take back one file this commit published and nothing will ever name.
    ///
    /// An attempt beaten on write publishes its manifest list again for the
    /// snapshot it rebases onto, so the list of the attempt it replaces is
    /// removed here - one `DELETE` - rather than kept as the orphan a
    /// successful commit would otherwise leave. A removal the store refuses
    /// is logged rather than reported, because the orphan is a cost and not
    /// a wrong table; the file leaves the record either way.
    ///
    /// # Errors
    ///
    /// Returns an error only when the record lock was poisoned.
    pub(super) fn withdraw(&self, relative: &str) -> Result<()> {
        let withdrawn = {
            let mut published = self.published.lock().map_err(|_| poisoned())?;
            published
                .iter()
                .position(|(path, _)| path == relative)
                .map(|index| published.remove(index))
        };
        if let Some((path, mut handle)) = withdrawn {
            if let Err(error) = handle.remove(false) {
                log::warn!(
                    "the file {path} of a beaten iceberg commit attempt could not be removed: {error}"
                );
            }
        }
        Ok(())
    }

    /// Mark the commit's document durable: the published files are the
    /// table's now, and only the staging directory goes when this drops.
    ///
    /// Called the moment the versioned metadata document is out, before the
    /// hint is written, because a fresh handle resolves the version to that
    /// document whatever the hint write reports afterwards.
    pub(super) fn commit(&self) {
        self.committed.store(true, Ordering::Release);
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
        if !self.committed.load(Ordering::Acquire) {
            if let Ok(mut published) = self.published.lock() {
                for (_, handle) in published.iter_mut().rev() {
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

/// Send a staged file to the table as one upload.
///
/// A store handle streams it up: one read below the multipart threshold,
/// one part-sized buffer at a time above it, so an object of any length
/// costs one part of memory. Any other target - a local or a memory-backed
/// one, where staging is off unless asked for - takes the file whole.
fn upload(target: &mut Holder, path: &Path, size: u64) -> Result<()> {
    match target {
        #[cfg(feature = "object")]
        Holder::ObjectFile(file) => {
            let mut source = std::fs::File::open(path)?;
            file.upload_from(&mut source, size)
        }
        other => {
            let bytes = std::fs::read(path)?;
            other.write_all_bytes(&bytes)
        }
    }
}

/// Let go of a file the commit could not publish, and report why.
///
/// A store handle whose upload was refused holds no object: a refused `PUT`
/// stores nothing and an abandoned multipart upload is aborted, so what the
/// handle staged is dropped and no `DELETE` goes out for a key that was
/// never written. Any other handle may have landed part of the file, and a
/// leaf publishes what it holds when it is dropped, so it is removed first.
fn unpublished(target: Holder, error: crate::Error) -> crate::Error {
    match target {
        #[cfg(feature = "object")]
        Holder::ObjectFile(file) => {
            let _ = file.discard();
        }
        mut other => {
            let _ = other.remove(false);
        }
    }
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
/// answer. A recorded length of zero is not believed - a real file's reader
/// would take it for an empty one and answer no rows without an error - so
/// the handle is returned as it was and the file answers for its own length,
/// which is one request more and the truth. A handle with no such memory is
/// returned as it was.
pub(super) fn sized(holder: Holder, size: u64) -> Holder {
    match holder {
        #[cfg(feature = "object")]
        Holder::ObjectFile(file) if size > 0 => Holder::ObjectFile(file.with_known_size(size)),
        other => other,
    }
}

/// Report a poisoned record lock without panicking a caller.
fn poisoned() -> crate::Error {
    crate::Error::Io(std::io::Error::other(
        "the iceberg staging record was poisoned by a panicking writer",
    ))
}

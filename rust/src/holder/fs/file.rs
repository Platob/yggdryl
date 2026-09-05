//! A byte leaf over one bound Arrow filesystem location.

use std::sync::OnceLock;

use crate::holder::Holder;
use crate::{Error, IOBase, IOFile, Listing, MediaType, MimeType, Result, Url};

use super::{
    BoundLocation, ByteReader, ByteWriter, FileSystem, Folder, OutputMetadata, RandomAccessReader,
};

/// A file whose streams are supplied by its bound filesystem.
pub struct File {
    bound: BoundLocation,
    declared: Option<MediaType>,
    inferred: OnceLock<MediaType>,
}

impl File {
    /// Bind a known file location without touching the filesystem.
    pub fn new(bound: BoundLocation) -> Self {
        Self {
            bound,
            declared: None,
            inferred: OnceLock::new(),
        }
    }

    /// Bind an injected raw filesystem path.
    pub fn from_path(
        filesystem: std::sync::Arc<dyn FileSystem>,
        path: impl Into<String>,
        uri: Option<String>,
    ) -> Result<Self> {
        BoundLocation::new(filesystem, path, uri).map(Self::new)
    }

    /// Borrow all bound-location facts.
    pub const fn bound(&self) -> &BoundLocation {
        &self.bound
    }

    /// Borrow the exact filesystem instance.
    pub fn filesystem(&self) -> &std::sync::Arc<dyn FileSystem> {
        self.bound.filesystem()
    }

    /// Borrow the exact opaque filesystem path.
    pub fn path(&self) -> &str {
        self.bound.path()
    }

    /// Borrow the safe diagnostic URL.
    pub const fn url(&self) -> &Url {
        self.bound.diagnostic_url()
    }

    /// Open a strict random-access input file.
    pub fn open_input_file(&self) -> Result<Box<dyn RandomAccessReader>> {
        self.filesystem().open_input_file(self.path())
    }

    /// Open a strict sequential input stream.
    pub fn open_input_stream(&self) -> Result<Box<dyn ByteReader>> {
        self.filesystem().open_input_stream(self.path())
    }

    /// Open a truncating output stream.
    pub fn open_output_stream(
        &self,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.filesystem().open_output_stream(self.path(), metadata)
    }

    /// Open an append stream.
    pub fn open_append_stream(
        &self,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.filesystem().open_append_stream(self.path(), metadata)
    }

    /// Stream from one retained input open.
    pub fn byte_stream(
        &self,
        position: u64,
        batch_size: usize,
    ) -> Result<crate::ByteStream<'static>> {
        if position == 0 {
            return match self.open_input_stream() {
                Ok(reader) => crate::ByteStream::from_fs_reader(reader, batch_size),
                Err(error) if error.is_absent() => {
                    crate::ByteStream::from_reader(std::io::empty(), batch_size)
                }
                Err(error) => Err(error),
            };
        }
        let mut reader = match self.open_input_file() {
            Ok(reader) => reader,
            Err(error) if error.is_absent() => {
                return crate::ByteStream::from_reader(std::io::empty(), batch_size);
            }
            Err(error) => return Err(error),
        };
        if let Err(error) = reader.seek(std::io::SeekFrom::Start(position)) {
            let _ = reader.close();
            return Err(error);
        }
        crate::ByteStream::from_fs_random_reader(reader, batch_size)
    }

    fn write_stream(mut writer: Box<dyn ByteWriter>, bytes: &[u8]) -> Result<()> {
        let result = (|| {
            let mut written = 0;
            while written < bytes.len() {
                let count = writer.write(&bytes[written..])?;
                if count == 0 {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "output stream stopped before the complete value was written",
                    )));
                }
                if count > bytes.len() - written {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "output stream reported writing beyond the supplied buffer",
                    )));
                }
                written += count;
            }
            Ok(())
        })();
        let close = writer.close();
        match result {
            Ok(()) => close,
            Err(error) => Err(error),
        }
    }
}

impl IOFile for File {
    fn file_url(&self) -> &Url {
        self.url()
    }

    fn file_exists(&self) -> bool {
        self.filesystem()
            .file_info(self.path())
            .is_ok_and(|info| info.kind == crate::IOKind::File)
    }

    fn clear_file(&mut self) -> Result<()> {
        match self.filesystem().file_info(self.path())? {
            info if info.kind == crate::IOKind::Unknown => Ok(()),
            info if info.kind == crate::IOKind::Directory => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                self.bound.to_string(),
            ))),
            _ => Self::write_stream(self.open_output_stream(None)?, b""),
        }
    }

    fn delete_file(&mut self) -> Result<()> {
        match self.filesystem().delete_file(self.path()) {
            Err(error) if error.is_absent() => Ok(()),
            result => result,
        }
    }
}

impl crate::IOMedia for File {
    crate::impl_default_iomedia!();
}

impl IOBase for File {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let mut reader = match self.open_input_file() {
            Ok(reader) => reader,
            Err(error) if error.is_absent() => return Ok(0),
            Err(error) => return Err(error),
        };
        let result = reader.read_at(offset, buffer);
        let close = reader.close();
        match result {
            Ok(read) => close.map(|()| read),
            Err(error) => Err(error),
        }
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        self.byte_stream(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for chunk in self.byte_stream(0, 64 * 1024)? {
            let chunk = chunk?;
            bytes.try_reserve(chunk.len()).map_err(|error| {
                Error::Io(std::io::Error::other(format!(
                    "cannot grow whole-file read: {error}"
                )))
            })?;
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let mut reader = match self.open_input_file() {
            Ok(reader) => reader,
            Err(error) if error.is_absent() => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        if let Err(error) = bytes.try_reserve_exact(length).map_err(|error| {
            Error::Io(std::io::Error::other(format!(
                "cannot allocate a {length}-byte range read: {error}"
            )))
        }) {
            let _ = reader.close();
            return Err(error);
        }
        bytes.resize(length, 0);
        let result = (|| {
            let mut filled = 0;
            while filled < length {
                let position = offset.checked_add(filled as u64).ok_or_else(|| {
                    Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "range read position exceeds u64::MAX",
                    ))
                })?;
                let read = reader.read_at(position, &mut bytes[filled..])?;
                if read == 0 {
                    break;
                }
                if read > length - filled {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "filesystem read beyond the supplied range buffer",
                    )));
                }
                filled += read;
            }
            bytes.truncate(filled);
            Ok(bytes)
        })();
        let close = reader.close();
        match result {
            Ok(bytes) => close.map(|()| bytes),
            Err(error) => Err(error),
        }
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let info = self.filesystem().file_info(self.path())?;
        let size = info.size.unwrap_or(0);
        let writer = if offset == size && info.kind == crate::IOKind::File {
            self.open_append_stream(None)?
        } else if offset == 0 && info.kind == crate::IOKind::Unknown {
            self.open_output_stream(None)?
        } else {
            return Err(Error::unsupported(
                "positional writes that are not a sequential append",
                self.filesystem().type_name(),
            ));
        };
        Self::write_stream(writer, bytes)?;
        Ok(bytes.len())
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        Self::write_stream(self.open_output_stream(None)?, bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        let writer = self.open_append_stream(None)?;
        let offset = writer.tell();
        Self::write_stream(writer, bytes)?;
        Ok(offset)
    }

    fn size(&self) -> u64 {
        match self.filesystem().file_info(self.path()) {
            Ok(info) => info.size.unwrap_or(0),
            Err(_) => 0,
        }
    }

    fn capacity(&self) -> u64 {
        self.size()
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        if capacity <= self.size() {
            Ok(())
        } else {
            Err(Error::unsupported("reserve", self.filesystem().type_name()))
        }
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        if size == self.size() {
            return Ok(());
        }
        if size == 0 {
            return Self::write_stream(self.open_output_stream(None)?, b"");
        }
        Err(Error::unsupported(
            "non-zero truncation",
            self.filesystem().type_name(),
        ))
    }

    fn url(&self) -> Option<&Url> {
        Some(self.url())
    }

    fn bound_location(&self) -> Option<&BoundLocation> {
        Some(&self.bound)
    }

    fn media_type(&self) -> &MediaType {
        if let Some(declared) = &self.declared {
            return declared;
        }
        self.inferred.get_or_init(|| {
            if self.url().extension().is_none() {
                MediaType::from(MimeType::FILE)
            } else {
                self.url().media_type()
            }
        })
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.declared = Some(media_type);
    }

    fn kind(&self) -> crate::IOKind {
        self.filesystem()
            .file_info(self.path())
            .map_or(crate::IOKind::Unknown, |info| info.kind)
    }

    fn parent(&self) -> Option<Holder> {
        self.bound
            .parent()?
            .ok()
            .map(Folder::new)
            .map(Holder::FsFolder)
    }

    fn clear(&mut self) -> Result<()> {
        self.clear_file()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.file_remove(recursive)
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        self.file_child_by_path(name)
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
        self.file_ls()
    }

    fn is_atomic(&self) -> bool {
        self.file_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.file_is_tabular()
    }
}

impl std::fmt::Debug for File {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("File").field(&self.bound).finish()
    }
}

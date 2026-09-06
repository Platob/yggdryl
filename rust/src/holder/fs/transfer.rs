//! Native and bounded transfers between bound locations.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, IOKind, Result};

use super::{BoundLocation, ByteReader, ByteWriter};

const CHUNK: usize = 64 * 1024;
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Copy between bound locations, using the backend operation when possible.
pub fn copy_bound(source: &BoundLocation, target: &BoundLocation) -> Result<u64> {
    if source
        .filesystem()
        .try_equals(target.filesystem().as_ref())?
    {
        // `copy_file` returns no count, while `IOBase::copy_into` must. Obtain
        // that required result before mutation so a later stat failure can
        // never report failure after a completed copy.
        let size = transfer_size(source)?;
        source
            .filesystem()
            .copy_file(source.path(), target.path())?;
        return Ok(size);
    }

    let mut reader = source.filesystem().open_input_stream(source.path())?;
    let temporary = temporary_path(target.path());
    let mut writer = match target.filesystem().open_output_stream(&temporary, None) {
        Ok(writer) => writer,
        Err(error) => {
            let _ = reader.close();
            return Err(error);
        }
    };
    let copied = transfer(&mut *reader, &mut *writer);
    let source_close = reader.close();
    let output_close = writer.close();
    let copied = match (copied, source_close, output_close) {
        (Ok(copied), Ok(()), Ok(())) => copied,
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            let _ = target.filesystem().delete_file(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = target.filesystem().move_file(&temporary, target.path()) {
        let _ = target.filesystem().delete_file(&temporary);
        return Err(error);
    }
    Ok(copied)
}

/// Move between bound locations.
pub fn move_bound(source: &BoundLocation, target: &BoundLocation) -> Result<u64> {
    if source
        .filesystem()
        .try_equals(target.filesystem().as_ref())?
    {
        let size = transfer_size(source)?;
        source
            .filesystem()
            .move_file(source.path(), target.path())?;
        return Ok(size);
    }
    let size = copy_bound(source, target)?;
    source.filesystem().delete_file(source.path())?;
    Ok(size)
}

fn transfer_size(source: &BoundLocation) -> Result<u64> {
    let info = source.filesystem().file_info(source.path())?;
    match info.kind {
        IOKind::File => info.size.ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("filesystem did not report the source file size at {source}"),
            ))
        }),
        IOKind::Unknown => Err(Error::absent("source file", source)),
        _ => Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::IsADirectory,
            format!("expected a source file at {source}, got a directory"),
        ))),
    }
}

fn transfer(reader: &mut dyn ByteReader, writer: &mut dyn ByteWriter) -> Result<u64> {
    let mut buffer = vec![0_u8; CHUNK];
    let mut copied = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(copied);
        }
        if read > buffer.len() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "cross-filesystem input stream reported {read} bytes for a {}-byte buffer",
                    buffer.len()
                ),
            )));
        }
        let mut written = 0;
        while written < read {
            let count = writer.write(&buffer[written..read])?;
            if count == 0 {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "cross-filesystem copy output stream stopped",
                )));
            }
            if count > read - written {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "cross-filesystem output stream reported {count} bytes for a {}-byte buffer",
                        read - written
                    ),
                )));
            }
            written += count;
        }
        copied = copied.checked_add(read as u64).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "cross-filesystem copy exceeds u64::MAX bytes",
            ))
        })?;
    }
}

fn temporary_path(target: &str) -> String {
    let tag = TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let first = RandomState::new().hash_one((
        target,
        std::process::id(),
        tag,
        now,
        std::thread::current().id(),
    ));
    let second = RandomState::new().hash_one((now, tag, target.len(), first));
    format!("{target}.yggdryl-transfer-{:016x}{:016x}", first, second)
}

//! Native and bounded transfers between bound locations.

use std::sync::atomic::{AtomicU64, Ordering};

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
        source
            .filesystem()
            .copy_file(source.path(), target.path())?;
        return copied_size(target);
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
        source
            .filesystem()
            .move_file(source.path(), target.path())?;
        return copied_size(target);
    }
    let size = copy_bound(source, target)?;
    source.filesystem().delete_file(source.path())?;
    Ok(size)
}

fn copied_size(target: &BoundLocation) -> Result<u64> {
    let info = target.filesystem().file_info(target.path())?;
    match info.kind {
        IOKind::File => info.size.ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("filesystem did not report the copied file size at {target}"),
            ))
        }),
        IOKind::Unknown => Err(Error::absent("copied file", target)),
        _ => Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::IsADirectory,
            format!("expected a copied file at {target}, got a directory"),
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
        let mut written = 0;
        while written < read {
            let count = writer.write(&buffer[written..read])?;
            if count == 0 {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "cross-filesystem copy output stream stopped",
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
    format!("{target}.yggdryl-transfer-{}-{tag}", std::process::id())
}

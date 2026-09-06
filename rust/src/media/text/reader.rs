//! One reused byte window yielding bounded physical-line parts.

use std::io::Read;

use crate::{Error, Result};

use super::sep::{LineSep, next_break};

const WINDOW_SIZE: usize = crate::DEFAULT_STREAM_BATCH_SIZE;

/// One borrowed piece of a physical line.
pub(crate) struct LinePart<'bytes> {
    /// Exact source bytes excluding the physical terminator.
    pub(crate) bytes: &'bytes [u8],
    /// Whether this piece ends the physical line.
    pub(crate) end: bool,
}

/// A fixed-window streaming line splitter.
pub(crate) struct Lines<R> {
    source: R,
    bytes: Vec<u8>,
    filled: usize,
    cursor: usize,
    drained: bool,
    failed: bool,
    line_open: bool,
}

impl<R: Read> Lines<R> {
    pub(crate) fn new(source: R) -> Self {
        Self {
            source,
            bytes: vec![0; WINDOW_SIZE],
            filled: 0,
            cursor: 0,
            drained: false,
            failed: false,
            line_open: false,
        }
    }

    fn refill(&mut self) -> Result<()> {
        if self.cursor > 0 {
            self.bytes.copy_within(self.cursor..self.filled, 0);
            self.filled -= self.cursor;
            self.cursor = 0;
        }
        let read = self
            .source
            .read(&mut self.bytes[self.filled..])
            .map_err(Error::Io)?;
        if read == 0 {
            self.drained = true;
        }
        self.filled += read;
        Ok(())
    }

    /// Yield the next bounded part, without retaining earlier parts.
    pub(crate) fn next_part<'bytes>(
        &'bytes mut self,
        linesep: Option<&LineSep>,
    ) -> Option<Result<LinePart<'bytes>>> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(found) =
                next_break(&self.bytes[self.cursor..self.filled], linesep, self.drained)
            {
                let start = self.cursor;
                let end = start + found.at;
                self.cursor = start + found.end();
                self.line_open = false;
                let bytes = &self.bytes[start..end];
                return Some(Ok(LinePart { bytes, end: true }));
            }
            if self.drained {
                if self.cursor < self.filled {
                    let start = self.cursor;
                    self.cursor = self.filled;
                    self.line_open = false;
                    let bytes = &self.bytes[start..self.filled];
                    return Some(Ok(LinePart { bytes, end: true }));
                }
                if std::mem::take(&mut self.line_open) {
                    return Some(Ok(LinePart {
                        bytes: &[],
                        end: true,
                    }));
                }
                return None;
            }
            if self.cursor > 0 || self.filled < self.bytes.len() {
                if let Err(error) = self.refill() {
                    self.failed = true;
                    return Some(Err(error));
                }
                continue;
            }

            let overlap = linesep.map_or_else(
                || usize::from(self.bytes[self.filled - 1] == b'\r'),
                |linesep| linesep.len().saturating_sub(1),
            );
            let available = self.filled - self.cursor;
            let safe = available.saturating_sub(overlap);
            if safe == 0 {
                // A configured terminator may itself exceed the ordinary
                // window. Its length is explicit configuration, so retaining
                // one such candidate is the bound needed to recognize it.
                self.bytes.resize(
                    self.bytes
                        .len()
                        .saturating_add(WINDOW_SIZE)
                        .max(overlap + 1),
                    0,
                );
                continue;
            }
            let start = self.cursor;
            let end = start + safe;
            self.cursor = end;
            self.line_open = true;
            let bytes = &self.bytes[start..end];
            return Some(Ok(LinePart { bytes, end: false }));
        }
    }

    /// Yield the next complete line without its terminator.
    #[cfg(test)]
    fn next_line(&mut self, linesep: Option<&LineSep>) -> Option<Result<Vec<u8>>> {
        let mut line = Vec::new();
        loop {
            let part = self.next_part(linesep)?;
            match part {
                Ok(part) => {
                    line.extend_from_slice(part.bytes);
                    if part.end {
                        return Some(Ok(line));
                    }
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::{Lines, WINDOW_SIZE};
    use crate::media::text::LineSep;

    struct Chunked {
        bytes: std::io::Cursor<Vec<u8>>,
        size: usize,
    }

    impl Read for Chunked {
        fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
            let size = target.len().min(self.size);
            self.bytes.read(&mut target[..size])
        }
    }

    fn read(input: &[u8], linesep: Option<&LineSep>) -> Vec<Vec<u8>> {
        let mut lines = Lines::new(std::io::Cursor::new(input));
        let mut values = Vec::new();
        while let Some(line) = lines.next_line(linesep) {
            values.push(line.unwrap());
        }
        values
    }

    #[test]
    fn flexible_and_pinned_terminators_stream_lines() {
        assert_eq!(
            read(b"a\r\nb\nc\rd", None),
            [b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]
        );
        assert_eq!(
            read(b"a\nb\r\nc", Some(&LineSep::CRLF)),
            [b"a\nb".to_vec(), b"c".to_vec()]
        );
        assert_eq!(read(b"\xef\xbb\xbfa\n", None), [b"\xef\xbb\xbfa".to_vec()]);
    }

    #[test]
    fn terminators_can_cross_short_source_reads() {
        let mut flexible = Lines::new(Chunked {
            bytes: std::io::Cursor::new(b"a\r\nb\rc\nlast".to_vec()),
            size: 1,
        });
        let mut values = Vec::new();
        while let Some(line) = flexible.next_line(None) {
            values.push(line.unwrap());
        }
        assert_eq!(
            values,
            [
                b"a".to_vec(),
                b"b".to_vec(),
                b"c".to_vec(),
                b"last".to_vec()
            ]
        );

        let mut pinned = Lines::new(Chunked {
            bytes: std::io::Cursor::new(b"a\r\nb\r\nc".to_vec()),
            size: 1,
        });
        let mut values = Vec::new();
        while let Some(line) = pinned.next_line(Some(&LineSep::CRLF)) {
            values.push(line.unwrap());
        }
        assert_eq!(values, [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn a_line_larger_than_the_window_is_returned_as_bounded_parts() {
        let mut input = vec![b'x'; WINDOW_SIZE * 3 + 7];
        input.extend_from_slice(b"\nnext");
        let mut lines = Lines::new(std::io::Cursor::new(input));
        let mut sizes = Vec::new();
        loop {
            let part = lines.next_part(None).unwrap().unwrap();
            sizes.push(part.bytes.len());
            if part.end {
                break;
            }
        }
        assert!(sizes.len() >= 3);
        assert!(sizes.iter().all(|size| *size <= WINDOW_SIZE));
        assert_eq!(lines.next_line(None).unwrap().unwrap(), b"next");
    }
}

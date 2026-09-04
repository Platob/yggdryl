//! One reused byte window yielding one physical line at a time.

use std::io::Read;

use crate::{Error, Result};

use super::sep::{LineSep, next_break};

const WINDOW_SIZE: usize = crate::DEFAULT_STREAM_BATCH_SIZE;

/// A bounded streaming line splitter.
pub(crate) struct Lines<R> {
    source: R,
    bytes: Vec<u8>,
    filled: usize,
    cursor: usize,
    drained: bool,
    failed: bool,
    first: bool,
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
            first: true,
        }
    }

    fn refill(&mut self) -> Result<()> {
        if self.cursor > 0 {
            self.bytes.copy_within(self.cursor..self.filled, 0);
            self.filled -= self.cursor;
            self.cursor = 0;
        }
        if self.filled == self.bytes.len() {
            self.bytes
                .resize(self.bytes.len().saturating_mul(2).max(1), 0);
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

    /// Yield the next line without its terminator.
    pub(crate) fn next_line(&mut self, linesep: Option<&LineSep>) -> Option<Result<&[u8]>> {
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
                let first = std::mem::replace(&mut self.first, false);
                let line = &self.bytes[start..end];
                return Some(Ok(if first { strip_bom(line) } else { line }));
            }
            if self.drained {
                if self.cursor == self.filled {
                    return None;
                }
                let start = self.cursor;
                self.cursor = self.filled;
                let first = std::mem::replace(&mut self.first, false);
                let line = &self.bytes[start..self.filled];
                return Some(Ok(if first { strip_bom(line) } else { line }));
            }
            if let Err(error) = self.refill() {
                self.failed = true;
                return Some(Err(error));
            }
        }
    }
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use super::Lines;
    use crate::text::LineSep;

    fn read(input: &[u8], linesep: Option<&LineSep>) -> Vec<Vec<u8>> {
        let mut lines = Lines::new(std::io::Cursor::new(input));
        let mut values = Vec::new();
        while let Some(line) = lines.next_line(linesep) {
            values.push(line.unwrap().to_vec());
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
        assert_eq!(read(b"\xef\xbb\xbfa\n", None), [b"a".to_vec()]);
    }
}

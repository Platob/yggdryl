//! The quote-aware cell scanner: one record's bytes in, borrowed cells out.
//!
//! Record boundaries are not decided here. They come from the shared physical
//! line splitter in [`crate::media::text::reader`], which already owns window
//! sizing, refills, mixed terminators, and pinned ones. A quoted cell may hold
//! the terminator, so the reader joins physical lines until [`split_cells`]
//! reports the record complete - the splitter is lossless, so the bytes that
//! separated those lines are restored exactly as the resource held them.
//!
//! The scan itself never walks a byte at a time. Each state jumps to the next
//! byte that can end it with `memchr`, which is the same vectorized search the
//! line splitter runs, so a wide row costs one pass per cell rather than one
//! branch per byte.

use std::borrow::Cow;

use crate::media::text::LineSep;

/// The byte spelling one CSV resource uses.
///
/// `linesep` is the record terminator - the plain-text vocabulary, unchanged -
/// and `separator` is what delimits cells inside one record.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Dialect {
    /// The byte between two cells of one record.
    pub(crate) separator: u8,
    /// The byte that opens and closes a quoted cell; `None` reads every cell
    /// literally.
    pub(crate) quote: Option<u8>,
    /// The byte that escapes the next byte inside a quoted cell; `None` reads
    /// a doubled quote as one quote, which is what RFC 4180 spells.
    pub(crate) escape: Option<u8>,
    /// A line opening with this byte carries no record.
    pub(crate) comment: Option<u8>,
    /// The pinned record terminator; `None` accepts LF, CRLF, and CR mixed.
    pub(crate) linesep: Option<LineSep>,
    /// Whether unquoted cells drop their edge ASCII whitespace.
    pub(crate) trim: bool,
}

/// One cell's place in the record that holds it.
///
/// The scan records where the cell is rather than what it says, so reading a
/// cell that needs no unescaping borrows the record's own bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Span {
    /// The cell's first byte, the opening quote included.
    pub(crate) start: usize,
    /// One past the cell's last byte, before the separator that ended it.
    pub(crate) end: usize,
    /// One past the closing quote, or `start` when the cell is not quoted.
    pub(crate) close: usize,
    /// Whether the quoted section holds a doubled quote or an escape byte.
    pub(crate) escaped: bool,
}

impl Span {
    /// Return whether the cell opened with the dialect's quote byte.
    pub(crate) const fn is_quoted(self) -> bool {
        self.close > self.start
    }
}

/// Split one complete record into cell spans, reusing `spans`.
///
/// Returns whether the record closed every quote it opened. A `false` answer
/// means the record continues past these bytes: the caller joins the next
/// physical line, with the terminator that separated them, and asks again.
pub(crate) fn split_cells(record: &[u8], dialect: &Dialect, spans: &mut Vec<Span>) -> bool {
    spans.clear();
    let mut at = 0;
    loop {
        let start = cell_start(record, at, dialect.trim);
        let (span, next) = match dialect.quote {
            Some(quote) if record.get(start) == Some(&quote) => {
                match quoted_cell(record, start, quote, dialect) {
                    Some(cell) => cell,
                    None => {
                        spans.push(Span {
                            start,
                            end: record.len(),
                            close: start,
                            escaped: false,
                        });
                        return false;
                    }
                }
            }
            _ => plain_cell(record, start, dialect.separator),
        };
        spans.push(span);
        match next {
            Some(next) => at = next,
            None => return true,
        }
    }
}

/// Read one cell's bytes, borrowing the record whenever nothing must change.
pub(crate) fn cell_bytes<'record>(
    record: &'record [u8],
    span: Span,
    dialect: &Dialect,
) -> Cow<'record, [u8]> {
    if !span.is_quoted() {
        return Cow::Borrowed(trailing_trimmed(
            &record[span.start..span.end],
            dialect.trim,
        ));
    }
    // The quoted section excludes both quotes; anything between the closing
    // quote and the separator is content a malformed row still carries.
    let inner = &record[span.start + 1..span.close - 1];
    let trailing = &record[span.close..span.end];
    if !span.escaped && trailing.is_empty() {
        return Cow::Borrowed(inner);
    }
    let mut value = Vec::with_capacity(inner.len() + trailing.len());
    unescape_into(inner, dialect, &mut value);
    value.extend_from_slice(trailing);
    Cow::Owned(value)
}

/// Append one rendered cell, quoting only when the bytes need it.
///
/// `force` quotes a cell whose bytes are legal but would read back as
/// something else - the empty string under the default absence spelling, or a
/// value that happens to spell absence itself.
pub(crate) fn render_cell(value: &[u8], dialect: &Dialect, force: bool, output: &mut Vec<u8>) {
    let Some(quote) = dialect.quote else {
        output.extend_from_slice(value);
        return;
    };
    if !force && !needs_quoting(value, quote, dialect) {
        output.extend_from_slice(value);
        return;
    }
    output.push(quote);
    match dialect.escape {
        Some(escape) if escape != quote => {
            for byte in value {
                if *byte == quote || *byte == escape {
                    output.push(escape);
                }
                output.push(*byte);
            }
        }
        _ => {
            for byte in value {
                if *byte == quote {
                    output.push(quote);
                }
                output.push(*byte);
            }
        }
    }
    output.push(quote);
}

/// Return whether a rendered cell must be quoted to read back unchanged.
fn needs_quoting(value: &[u8], quote: u8, dialect: &Dialect) -> bool {
    let terminator = dialect
        .linesep
        .as_ref()
        .map_or(b"\n".as_slice(), LineSep::as_bytes);
    if value.first().is_some_and(|byte| *byte == quote) {
        return true;
    }
    if dialect.trim && value.first().is_some_and(u8::is_ascii_whitespace) {
        return true;
    }
    if dialect.trim && value.last().is_some_and(u8::is_ascii_whitespace) {
        return true;
    }
    if dialect
        .comment
        .is_some_and(|comment| value.first() == Some(&comment))
    {
        return true;
    }
    if memchr::memchr3(dialect.separator, quote, b'\n', value).is_some() {
        return true;
    }
    if memchr::memchr(b'\r', value).is_some() {
        return true;
    }
    terminator.len() > 1 && memchr::memmem::find(value, terminator).is_some()
}

/// Skip the edge whitespace an unquoted cell opens with.
fn cell_start(record: &[u8], at: usize, trim: bool) -> usize {
    if !trim {
        return at;
    }
    let mut start = at;
    while record
        .get(start)
        .is_some_and(|byte| byte.is_ascii_whitespace() && *byte != b'\n' && *byte != b'\r')
    {
        start += 1;
    }
    start
}

/// Drop the edge whitespace an unquoted cell ends with.
fn trailing_trimmed(value: &[u8], trim: bool) -> &[u8] {
    if !trim {
        return value;
    }
    let mut end = value.len();
    while end > 0 && value[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &value[..end]
}

/// Scan one unquoted cell: a single search for the next separator.
fn plain_cell(record: &[u8], start: usize, separator: u8) -> (Span, Option<usize>) {
    match memchr::memchr(separator, &record[start..]) {
        Some(found) => (
            Span {
                start,
                end: start + found,
                close: start,
                escaped: false,
            },
            Some(start + found + 1),
        ),
        None => (
            Span {
                start,
                end: record.len(),
                close: start,
                escaped: false,
            },
            None,
        ),
    }
}

/// Scan one quoted cell, or answer `None` when its quote never closes.
fn quoted_cell(
    record: &[u8],
    start: usize,
    quote: u8,
    dialect: &Dialect,
) -> Option<(Span, Option<usize>)> {
    let mut scan = start + 1;
    let mut escaped = false;
    let close = loop {
        let found = match dialect.escape {
            Some(escape) if escape != quote => {
                let found = memchr::memchr2(quote, escape, &record[scan..])? + scan;
                if record[found] == escape {
                    // The escaped byte cannot close the cell, whatever it is.
                    escaped = true;
                    scan = found + 2;
                    if scan > record.len() {
                        return None;
                    }
                    continue;
                }
                found
            }
            _ => memchr::memchr(quote, &record[scan..])? + scan,
        };
        if record.get(found + 1) == Some(&quote) {
            // A doubled quote spells one quote and stays inside the cell.
            escaped = true;
            scan = found + 2;
            continue;
        }
        break found + 1;
    };
    let (end, next) = match memchr::memchr(dialect.separator, &record[close..]) {
        Some(found) => (close + found, Some(close + found + 1)),
        None => (record.len(), None),
    };
    Some((
        Span {
            start,
            end,
            close,
            escaped,
        },
        next,
    ))
}

/// Collapse doubled quotes and escape bytes into the value they spell.
fn unescape_into(inner: &[u8], dialect: &Dialect, output: &mut Vec<u8>) {
    let Some(quote) = dialect.quote else {
        output.extend_from_slice(inner);
        return;
    };
    let mut at = 0;
    match dialect.escape {
        Some(escape) if escape != quote => {
            while let Some(found) = memchr::memchr(escape, &inner[at..]) {
                let found = at + found;
                output.extend_from_slice(&inner[at..found]);
                match inner.get(found + 1) {
                    Some(byte) => output.push(*byte),
                    None => output.push(escape),
                }
                at = found + 2;
                if at > inner.len() {
                    return;
                }
            }
        }
        _ => {
            while let Some(found) = memchr::memchr(quote, &inner[at..]) {
                let found = at + found;
                output.extend_from_slice(&inner[at..=found]);
                // The pair spells one quote; the second is not content.
                at = found + 2;
                if at > inner.len() {
                    return;
                }
            }
        }
    }
    output.extend_from_slice(&inner[at..]);
}

#[cfg(test)]
mod tests {
    use super::{Dialect, cell_bytes, render_cell, split_cells};

    fn dialect() -> Dialect {
        Dialect {
            separator: b',',
            quote: Some(b'"'),
            escape: None,
            comment: None,
            linesep: None,
            trim: false,
        }
    }

    #[test]
    fn every_rendered_cell_reads_back_as_the_bytes_it_was_given() {
        let dialect = dialect();
        let nasty: [&[u8]; 12] = [
            b"",
            b"plain",
            b",",
            b"\"",
            b"\"\"",
            b"a\"b",
            b"a,b",
            b"a\nb",
            b"a\r\nb",
            b"  spaced  ",
            b"\"quoted\"",
            b"a,\"b\",c",
        ];
        for value in nasty {
            let mut line = Vec::new();
            render_cell(value, &dialect, false, &mut line);
            let mut spans = Vec::new();
            assert!(
                split_cells(&line, &dialect, &mut spans),
                "{value:?} rendered to an incomplete record: {line:?}"
            );
            assert_eq!(
                spans.len(),
                1,
                "{value:?} rendered to {} cells",
                spans.len()
            );
            assert_eq!(
                cell_bytes(&line, spans[0], &dialect).as_ref(),
                value,
                "rendered as {line:?}"
            );
        }
    }

    #[test]
    fn a_malformed_record_splits_without_panicking() {
        let dialect = dialect();
        let mut spans = Vec::new();
        // A quote that never closes is what the scan reports incomplete, so the
        // reader joins the next physical line and asks again.
        for record in [b"\"open".as_slice(), b"\"", b"a,\"b"] {
            assert!(!split_cells(record, &dialect, &mut spans), "{record:?}");
        }
        // Everything else splits, whatever it spells.
        for record in [
            b"a\"b,c".as_slice(),
            b"\"a\"x,b",
            b",,",
            b"a,",
            b",a",
            b"\"a\"\"\"",
        ] {
            assert!(split_cells(record, &dialect, &mut spans), "{record:?}");
            for span in &spans {
                let _ = cell_bytes(record, *span, &dialect);
            }
        }
    }
}

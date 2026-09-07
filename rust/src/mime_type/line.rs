//! Classifying one captured line, without a dictionary.
//!
//! A capture is millions of lines and most of them are not the protocol a
//! reader is after: a FIX frame wrapped in a process's own prose, a bridge's
//! `NAME=VALUE` row, an XML payload, a JSON log record, a sentence. Deciding
//! which is which is transport rather than FIX - every captured line has a
//! shape whatever protocol it carried - so it lives here, beside the
//! [`MimeType`](crate::MimeType) that names the answer, and needs no
//! dictionary to run.
//!
//! One shallow scan answers three questions a reader asks together: what the
//! line is, what message type it declares, and which way it moved. It reads
//! no message, allocates nothing, and every answer is a slice of the caller's
//! bytes.

use crate::MimeType;

/// FIX's official `XmlData` payload tag.
const XML_DATA_TAG: i32 = 213;

/// The one routing name for FIX user-defined MsgTypes (`U` plus a suffix).
const UDF_MSGTYPE: &[u8] = b"UDF";

#[derive(Clone, Copy)]
enum LineKey<'line> {
    Tag(i32),
    Name(&'line [u8]),
}

#[derive(Clone, Copy)]
struct LineEntry<'line> {
    key: LineKey<'line>,
    value: &'line [u8],
    /// Whether the key opened with the bridge's own  marker.
    ///
    /// The marker is what separates a bridge row from an ordinary run of
    /// attributes, and it counts only where the key starts: a  inside a
    /// value is part of that value, so  is one
    /// Text field rather than the start of a marked run.
    marked: bool,
}

#[derive(Clone, Copy)]
struct LineFrame {
    start: usize,
    numeric: bool,
    separator: LineSeparator,
}

#[derive(Clone, Copy)]
enum LineSeparator {
    Byte(u8),
    Marker(&'static [u8]),
    Whitespace,
}

/// What one shallow scan of a line found.
#[derive(Default)]
pub(crate) struct LineInference<'line> {
    has_tag: bool,
    has_pairs: bool,
    has_symbolic: bool,
    has_xml: bool,
    tag_msgtype: Option<&'line [u8]>,
    name_msgtype: Option<&'line [u8]>,
}

impl<'line> LineInference<'line> {
    /// What the line is.
    ///
    /// A FIX frame is numeric tags; a bridge row is `#`-marked keys or a
    /// `MSGTYPE=` key; both together are the mixed form. An `XmlData(213)`
    /// payload that opens with a tag makes the frame FIXML. Failing every
    /// frame rule, a line that is still `key=value` throughout is the generic
    /// key/value shape rather than nothing, and a document that opens as XML
    /// or JSON is that document.
    pub(crate) const fn mime_type(&self) -> MimeType {
        if self.has_xml {
            return MimeType::FIXML;
        }
        match (self.has_tag, self.has_symbolic) {
            (true, true) => MimeType::FIXUL,
            (true, false) => MimeType::FIX,
            (false, true) => MimeType::ULLINK,
            (false, false) if self.has_pairs => MimeType::KEYVALUE,
            (false, false) => MimeType::OCTET_STREAM,
        }
    }

    /// The message type the line declares, routed through the user-defined
    /// range.
    ///
    /// A raw `MSGTYPE=` is checked across the whole line before numeric tag
    /// 35 and therefore wins when both are present: a bridge writes its own
    /// type in front of a frame it is relaying.
    pub(crate) fn msgtype(&self) -> Option<&'line [u8]> {
        self.name_msgtype.or(self.tag_msgtype).map(route_msgtype)
    }
}

/// Find a raw symbolic key/value pair anywhere in a log line.
///
/// This intentionally runs before numeric-frame location: log prefixes can
/// carry Ullink `MSGTYPE=` before an embedded `8=FIX...` frame. The returned
/// value is borrowed and bounded by the first common entry separator.
fn find_named_value<'line>(line: &'line [u8], wanted: &[u8]) -> Option<&'line [u8]> {
    for start in 0..line.len() {
        let Some((LineKey::Name(name), equals)) = pair_at(line, start) else {
            continue;
        };
        if !name.eq_ignore_ascii_case(wanted) {
            continue;
        }
        let value_start = equals + 1;
        let mut end = value_start;
        while end < line.len()
            && !matches!(
                line[end],
                0x01 | b'|'
                    | b' '
                    | b'\t'
                    | b'\r'
                    | b'\n'
                    | b','
                    | b';'
                    | b']'
                    | b')'
                    | b'}'
                    | b'^'
                    | b'<'
                    | b'{'
                    | b'\\'
            )
        {
            end += 1;
        }
        if end > value_start {
            return Some(&line[value_start..end]);
        }
    }
    None
}

/// Route the standard's `U*` user-defined range through one dictionary root.
fn route_msgtype(value: &[u8]) -> &[u8] {
    if value.len() > 1 && value[0] == b'U' && value[1..].iter().all(u8::is_ascii_alphanumeric) {
        UDF_MSGTYPE
    } else {
        value
    }
}

fn locate_frame(line: &[u8]) -> Option<LineFrame> {
    let mut first = None;
    let mut msgtype = None;
    for start in 0..line.len() {
        let Some((key, _)) = pair_at(line, start) else {
            continue;
        };
        let candidate = (start, matches!(key, LineKey::Tag(_)));
        first.get_or_insert(candidate);
        match key {
            LineKey::Tag(8) => return Some(frame(line, candidate)),
            LineKey::Tag(35) if msgtype.is_none() => msgtype = Some(candidate),
            LineKey::Tag(_) | LineKey::Name(_) => {}
        }
    }
    msgtype.or(first).map(|candidate| frame(line, candidate))
}

fn frame(line: &[u8], (start, numeric): (usize, bool)) -> LineFrame {
    LineFrame {
        start,
        numeric,
        separator: LineSeparator::for_line(line, start, numeric),
    }
}

/// How a log spells SOH when it cannot print the byte itself.
///
/// One vocabulary: the separator a frame is located with and the one a reader
/// splits on are the same fact, so a capture that escapes its separator is
/// recognized once rather than in each place that reads a frame.
pub(crate) const SOH_MARKERS: [&[u8]; 4] = [b"^A", b"\\x01", b"<SOH>", b"{SOH}"];

impl LineSeparator {
    fn for_line(line: &[u8], start: usize, numeric: bool) -> Self {
        let tail = &line[start..];
        if !numeric {
            return if memchr::memchr(b'|', tail).is_some() {
                Self::Byte(b'|')
            } else {
                Self::Whitespace
            };
        }

        let mut found: Option<(usize, Self)> = None;
        let markers = SOH_MARKERS.map(Self::Marker);
        for separator in [Self::Byte(0x01), Self::Byte(b'|')]
            .into_iter()
            .chain(markers)
        {
            let position = match separator {
                Self::Byte(byte) => memchr::memchr(byte, tail),
                Self::Marker(marker) => memchr::memmem::find(tail, marker),
                Self::Whitespace => None,
            };
            if let Some(position) = position {
                if found.is_none_or(|(held, _)| position < held) {
                    found = Some((position, separator));
                }
            }
        }
        found.map_or(Self::Whitespace, |(_, separator)| separator)
    }

    fn segment(self, line: &[u8], start: usize) -> (usize, usize) {
        match self {
            Self::Byte(byte) => match memchr::memchr(byte, &line[start..]) {
                Some(relative) => (start + relative, start + relative + 1),
                None => (line.len(), line.len()),
            },
            Self::Marker(marker) => match memchr::memmem::find(&line[start..], marker) {
                Some(relative) => (start + relative, start + relative + marker.len()),
                None => (line.len(), line.len()),
            },
            Self::Whitespace => {
                let mut end = start;
                while end < line.len() && !line[end].is_ascii_whitespace() {
                    end += 1;
                }
                let mut next = end;
                while next < line.len() && line[next].is_ascii_whitespace() {
                    next += 1;
                }
                (end, next)
            }
        }
    }
}

fn next_entry<'line>(
    line: &'line [u8],
    offset: &mut usize,
    separator: LineSeparator,
) -> Option<LineEntry<'line>> {
    while *offset < line.len() {
        while *offset < line.len() && line[*offset].is_ascii_whitespace() {
            *offset += 1;
        }
        let start = *offset;
        let (end, next) = separator.segment(line, start);
        *offset = next;
        let Some((key, equals)) = pair_at(line, start) else {
            if next == line.len() {
                return None;
            }
            continue;
        };
        if equals >= end {
            continue;
        }
        let mut value_end = end;
        while value_end > equals + 1
            && matches!(
                line[value_end - 1],
                b' ' | b'\t' | b'\r' | b'\n' | b']' | b')' | b'}' | b',' | b';'
            )
        {
            value_end -= 1;
        }
        if value_end > equals + 1 {
            return Some(LineEntry {
                key,
                value: &line[equals + 1..value_end],
                marked: line.get(start) == Some(&b'#'),
            });
        }
    }
    None
}

fn pair_at(line: &[u8], start: usize) -> Option<(LineKey<'_>, usize)> {
    if !is_field_start(line, start) {
        return None;
    }
    let mut key_start = start;
    if line.get(key_start) == Some(&b'#') {
        key_start += 1;
    }
    let first = *line.get(key_start)?;
    let (key, equals) = if first.is_ascii_digit() {
        let mut position = key_start;
        let mut tag = Some(0_i32);
        while position < line.len() && line[position].is_ascii_digit() {
            tag = tag.and_then(|tag| {
                tag.checked_mul(10)
                    .and_then(|tag| tag.checked_add(i32::from(line[position] - b'0')))
            });
            position += 1;
        }
        (LineKey::Tag(tag?), position)
    } else if is_name_start(first) {
        let mut position = key_start + 1;
        while position < line.len() && is_name_continue(line[position]) {
            position += 1;
        }
        (LineKey::Name(&line[key_start..position]), position)
    } else {
        return None;
    };
    if line.get(equals) != Some(&b'=') {
        return None;
    }
    let first_value = *line.get(equals + 1)?;
    if matches!(first_value, b'\'' | b'"') || is_field_end(line, equals + 1) {
        return None;
    }
    Some((key, equals))
}

const fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_name_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

fn is_field_start(line: &[u8], position: usize) -> bool {
    position == 0
        || matches!(
            line[position - 1],
            0x01 | b'|' | b' ' | b'\t' | b'[' | b'(' | b'{' | b'>'
        )
        || (position >= 2 && &line[position - 2..position] == b"^A")
        || (position >= 4 && &line[position - 4..position] == b"\\x01")
        || (position >= 5 && &line[position - 5..position] == b"<SOH>")
        || (position >= 5 && &line[position - 5..position] == b"{SOH}")
}

fn is_field_end(line: &[u8], position: usize) -> bool {
    matches!(
        line[position],
        0x01 | b'|' | b' ' | b'\t' | b'\r' | b'\n' | b']' | b')' | b'}' | b',' | b';'
    ) || line[position..].starts_with(b"^A")
        || line[position..].starts_with(b"\\x01")
        || line[position..].starts_with(b"<SOH>")
        || line[position..].starts_with(b"{SOH}")
}
/// Scans one line once, answering everything a reader asks of it.
///
/// The scan is shallow: it locates the frame, walks its entries to the
/// checksum, and stops. It parses no message and allocates nothing.
pub(crate) fn inspect(line: &[u8]) -> LineInference<'_> {
    let raw_msgtype = find_named_value(line, b"MSGTYPE");
    // A -marked key and a raw  are each a bridge's own marker,
    // and neither needs a frame around it to say so.
    let mut inferred = LineInference {
        has_pairs: raw_msgtype.is_some(),
        has_symbolic: raw_msgtype.is_some(),
        name_msgtype: raw_msgtype,
        ..LineInference::default()
    };
    let Some(frame) = locate_frame(line) else {
        // No frame at all: the line is a document, a sentence, or a bare run
        // of pairs. The first two are decided by their opening byte.
        inferred.has_pairs |= has_any_pair(line);
        return inferred;
    };
    let mut offset = frame.start;
    while let Some(entry) = next_entry(line, &mut offset, frame.separator) {
        let mut checksum = false;
        match entry.key {
            LineKey::Tag(tag) => {
                inferred.has_tag = true;
                inferred.has_pairs = true;
                inferred.has_symbolic |= entry.marked;
                if tag == XML_DATA_TAG {
                    if entry.value.starts_with(b"<") {
                        inferred.has_xml = true;
                    } else if memchr::memchr(b'=', entry.value).is_some() {
                        // A pair-shaped  payload is the mixed form:
                        // the envelope is numeric and everything that matters
                        // is symbolic inside it.
                        inferred.has_symbolic = true;
                    }
                }
                checksum = tag == 10;
                if tag == 35 && inferred.tag_msgtype.is_none() {
                    inferred.tag_msgtype = Some(entry.value);
                }
            }
            LineKey::Name(name) => {
                inferred.has_pairs = true;
                inferred.has_symbolic |= entry.marked;
                // A symbolic key *inside a numeric frame* is the mixed form.
                // Outside one it is just an attribute a log wrote, which is
                // the generic key/value shape rather than a bridge row.
                inferred.has_symbolic |= frame.numeric;
                if name.eq_ignore_ascii_case(b"MSGTYPE") && inferred.name_msgtype.is_none() {
                    inferred.has_symbolic = true;
                    if inferred.name_msgtype.is_none() {
                        inferred.name_msgtype = Some(entry.value);
                    }
                }
            }
        }
        if checksum {
            break;
        }
    }
    inferred
}

/// Whether the line holds any pair at all, marked or not.
fn has_any_pair(line: &[u8]) -> bool {
    (0..line.len()).any(|start| matches!(pair_at(line, start), Some((LineKey::Name(_), _))))
}

/// The media type one whole document opens as, before any pair rule runs.
///
/// Only the opening byte decides, because that is all a classifier can know
/// without parsing: a reader that wants certainty parses. A document wins
/// over the pair rules, so an XML body carrying an `=` is XML rather than a
/// run of attributes.
pub(crate) fn document_type(line: &[u8]) -> Option<MimeType> {
    let trimmed = trim_ascii(line);
    match trimmed.first()? {
        b'<' => trimmed
            .get(1)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'?' | b'!' | b'/'))
            .then_some(MimeType::XML),
        b'{' | b'[' => trimmed
            .last()
            .is_some_and(|byte| matches!(byte, b'}' | b']'))
            .then_some(MimeType::JSON),
        _ => None,
    }
}

/// Both ends trimmed of ASCII whitespace.
pub(crate) fn trim_ascii(line: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = line.len();
    while start < end && line[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &line[start..end]
}

/// Where the message starts inside a log line, when one is there.
///
/// Everything before it is the transport's own prose, which is what a
/// direction is read from and what a body strips.
pub(crate) fn payload_at(line: &[u8]) -> Option<usize> {
    locate_frame(line).map(|frame| frame.start)
}

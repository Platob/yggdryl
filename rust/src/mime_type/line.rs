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

/// The namespace every ULBridge MBean is named under.
///
/// One vendor string carries the whole reading: it is what makes a JSON
/// document a bridge's configuration rather than any other object, so a
/// document not naming it stays ordinary JSON. It has to be naming an MBean
/// and not merely spelling a class, which is what [`object_names`] holds it to.
const ULBRIDGE_NAMESPACE: &[u8] = b"com.ullink.ulbridge";

/// The ObjectName property naming what one MBean is.
///
/// Read only where a delimiter opens it, because `plugin-type` ends in the
/// same four bytes and names something else.
pub(crate) const OBJECT_NAME_TYPE: &[u8] = b"type";

/// Jolokia's own key for the operation a document asked for.
const JOLOKIA_TYPE_KEY: &[u8] = b"\"type\"";

/// The key Jolokia answers with, echoing back what was asked.
///
/// A request never carries it and every answer does, error answers included,
/// which is what makes it the whole of the reading. The keys an answer also
/// carries - `value`, `status` - are not: a write request states a `value` of
/// its own, and reading that as an answer would invert the direction.
const JOLOKIA_REQUEST_KEY: &[u8] = b"\"request\"";

#[derive(Clone, Copy)]
enum LineKey<'line> {
    Tag(i32),
    Name(&'line [u8]),
}

#[derive(Clone, Copy)]
struct LineEntry<'line> {
    key: LineKey<'line>,
    value: &'line [u8],
    /// Whether the key opened with the bridge's own `#` marker.
    ///
    /// The marker is what separates a bridge row from an ordinary run of
    /// attributes, and it counts only where the key starts: a `#` inside a
    /// value is part of that value, so `58=quoting #A=1` is one Text field
    /// rather than the start of a marked run.
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
    /// An escaped `SOH`, in whichever of [`SOH_MARKERS`] the line reaches for.
    ///
    /// One variant rather than four, because the four are one fact spelled
    /// four ways and a capture is free to mix them: a relay that rewrites a
    /// frame it received keeps the spelling it was given and writes its own
    /// beside it, so `8=FIX.4.4^A35=D<SOH>11=A` is one frame with three
    /// fields. Pinning the first spelling found would make the rest of that
    /// line one value.
    Marker,
    Whitespace,
}

/// What one shallow scan of a line found.
#[derive(Default)]
pub(crate) struct LineInference<'line> {
    has_tag: bool,
    has_pairs: bool,
    has_symbolic: bool,
    has_xml: bool,
    has_ulconfig: bool,
    tag_msgtype: Option<&'line [u8]>,
    name_msgtype: Option<&'line [u8]>,
    ulconfig_msgtype: Option<&'line [u8]>,
}

impl<'line> LineInference<'line> {
    /// What the line is.
    ///
    /// A FIX frame is numeric tags; a bridge row is `#`-marked keys or a
    /// `MSGTYPE=` key; both together are the mixed form. An `XmlData(213)`
    /// payload that opens with a tag makes the frame FIXML. Failing every
    /// frame rule, a JSON document naming the ULBridge namespace is that
    /// bridge's configuration, a line that is still `key=value` throughout is
    /// the generic key/value shape rather than nothing, and a document that
    /// opens as XML or JSON is that document.
    pub(crate) const fn mime_type(&self) -> MimeType {
        if self.has_xml {
            return MimeType::FIXML;
        }
        match (self.has_tag, self.has_symbolic) {
            (true, true) => MimeType::FIXUL,
            (true, false) => MimeType::FIX,
            (false, true) => MimeType::ULLINK,
            // A document wins over the bare pair rules, exactly as the XML and
            // JSON readings do: what a bridge wrote inside its own
            // configuration is that document's content, never a field.
            (false, false) if self.has_ulconfig => MimeType::ULCONFIG,
            (false, false) if self.has_pairs => MimeType::KEYVALUE,
            (false, false) => MimeType::OCTET_STREAM,
        }
    }

    /// The message type the line declares, routed through the user-defined
    /// range.
    ///
    /// A raw `MSGTYPE=` is checked across the whole line before numeric tag
    /// 35 and therefore wins when both are present: a bridge writes its own
    /// type in front of a frame it is relaying. A bridge configuration
    /// document declares its own, and only where neither of those was
    /// written, because the frame a line carries outranks the document
    /// carrying it.
    pub(crate) fn msgtype(&self) -> Option<&'line [u8]> {
        self.name_msgtype
            .or(self.tag_msgtype)
            .or(self.ulconfig_msgtype)
    }

    /// Reads the bridge configuration document the line carries, if it does.
    ///
    /// Run only where every frame rule has already declined, which is the one
    /// place the answer could change a thing: a line holding a frame is that
    /// frame whatever document quoted it, and the namespace scan a document
    /// costs is one a frame never pays.
    fn read_ulconfig(&mut self, line: &'line [u8]) {
        if self.has_tag || self.has_symbolic || self.has_xml {
            return;
        }
        let Some(at) = ulconfig_at(line) else {
            return;
        };
        self.has_ulconfig = true;
        self.ulconfig_msgtype = ulconfig_msgtype(&line[at..]);
    }
}

/// Find a raw symbolic key/value pair anywhere in a log line.
///
/// This intentionally runs before numeric-frame location: log prefixes can
/// carry Ullink `MSGTYPE=` before an embedded `8=FIX...` frame. The returned
/// value is borrowed and bounded by the first common entry separator.
fn find_named_value<'line>(line: &'line [u8], wanted: &[u8]) -> Option<&'line [u8]> {
    for (_, key, equals) in pairs(line) {
        let LineKey::Name(name) = key else {
            continue;
        };
        if !name.eq_ignore_ascii_case(wanted) {
            continue;
        }
        let value_start = equals + 1;
        let explicit_separator = line[value_start..]
            .iter()
            .any(|byte| matches!(byte, 0x01 | b'|'));
        let mut end = value_start;
        while end < line.len()
            && !matches!(
                line[end],
                0x01 | b'|'
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
            if matches!(line[end], b' ' | b'\t') {
                let next = line[end..]
                    .iter()
                    .position(|byte| !matches!(byte, b' ' | b'\t'))
                    .map_or(line.len(), |offset| end + offset);
                if !explicit_separator || pair_at(line, next).is_some() {
                    break;
                }
                end = next;
                continue;
            }
            end += 1;
        }
        while end > value_start && matches!(line[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        if end > value_start {
            return Some(&line[value_start..end]);
        }
    }
    None
}

fn locate_frame(line: &[u8]) -> Option<LineFrame> {
    let mut first = None;
    let mut msgtype = None;
    for (start, key, _) in pairs(line) {
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

/// The delimiter of a named bridge frame, preserving spaces inside delimited values.
pub(crate) fn ullink_separator(line: &[u8]) -> u8 {
    // The outer delimiter precedes any separators packed inside an indexed
    // group's value. A later inner SOH must not override an earlier pipe.
    memchr::memchr2(0x01, b'|', line).map_or(b' ', |position| line[position])
}

impl LineSeparator {
    fn for_line(line: &[u8], start: usize, numeric: bool) -> Self {
        let tail = &line[start..];
        if !numeric {
            return match ullink_separator(tail) {
                b' ' => Self::Whitespace,
                byte => Self::Byte(byte),
            };
        }

        let mut found: Option<(usize, Self)> = None;
        for separator in [Self::Byte(0x01), Self::Byte(b'|'), Self::Marker] {
            if let Some((position, _)) = separator.find(tail, 0) {
                if found.is_none_or(|(held, _)| position < held) {
                    found = Some((position, separator));
                }
            }
        }
        found.map_or(Self::Whitespace, |(_, separator)| separator)
    }

    /// Where this separator next stands at or after `start`, and how wide it
    /// is there.
    ///
    /// One owner for the question, because choosing a frame's separator and
    /// walking that frame ask it about the same bytes: a spelling that decides
    /// the frame and then fails to end a segment would be two readings of one
    /// line.
    fn find(self, line: &[u8], start: usize) -> Option<(usize, usize)> {
        match self {
            Self::Byte(byte) => memchr::memchr(byte, &line[start..]).map(|at| (start + at, 1)),
            Self::Marker => SOH_MARKERS
                .iter()
                .filter_map(|marker| {
                    memchr::memmem::find(&line[start..], marker)
                        .map(|at| (start + at, marker.len()))
                })
                .min_by_key(|(at, _)| *at),
            Self::Whitespace => line[start..]
                .iter()
                .position(u8::is_ascii_whitespace)
                .map(|at| (start + at, 1)),
        }
    }

    /// Where the segment opening at `start` ends, and where the next one opens.
    fn segment(self, line: &[u8], start: usize) -> (usize, usize) {
        let Some((end, width)) = self.find(line, start) else {
            return (line.len(), line.len());
        };
        let mut next = end + width;
        // A run of whitespace separates two fields once, so the next segment
        // opens past all of it rather than at the second space.
        if matches!(self, Self::Whitespace) {
            while next < line.len() && line[next].is_ascii_whitespace() {
                next += 1;
            }
        }
        (end, next)
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
        inferred.read_ulconfig(line);
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
    inferred.read_ulconfig(line);
    inferred
}

/// What one line is and the message type it declares, from one scan.
///
/// The two readings a text reader fills its classification columns from,
/// answered by the same inspection rather than by one each: a frame beats a
/// document, because an `XmlData` payload is part of a frame rather than a
/// document of its own, and a document beats the bare pair rules, because an
/// attribute inside a tag is not a field.
pub(crate) fn classify(line: &[u8]) -> (MimeType, Option<&[u8]>) {
    let inferred = inspect(line);
    let shape = inferred.mime_type();
    let shape = if shape == MimeType::OCTET_STREAM || shape == MimeType::KEYVALUE {
        document_type(line)
            .or_else(|| document_behind_prefix(line).map(|(shape, _)| shape))
            .unwrap_or(shape)
    } else {
        shape
    };
    (shape, inferred.msgtype())
}

/// An XML document a transport wrote prose in front of, and where it opens.
///
/// `Sending : <FIXML ...>...</FIXML>` states attributes rather than pairs,
/// so it is not read by the pair rules. The document must open before any
/// `=` - a pair arriving first makes the `<` a value - and the line must
/// close on the document's own last byte, so a sentence mentioning `<trade>`
/// stays a sentence. A JSON document is not read this way: one naming the
/// bridge's namespace is answered by the configuration rules, and prose
/// closing on braces is prose.
pub(crate) fn document_behind_prefix(line: &[u8]) -> Option<(MimeType, usize)> {
    let trimmed = trim_ascii(line);
    let open = memchr::memchr(b'<', trimmed)?;
    if memchr::memchr(b'=', &trimmed[..open]).is_some()
        || trimmed.last() != Some(&b'>')
        || !trimmed.get(open + 1).is_some_and(u8::is_ascii_alphabetic)
    {
        return None;
    }
    let shape = if trimmed[open..].starts_with(b"<FIXML") {
        MimeType::FIXML
    } else {
        MimeType::XML
    };
    // The offset is into `line`: the whitespace trimmed off the front, which
    // is not the whitespace trimmed off both ends.
    let leading = line.len() - line.trim_ascii_start().len();
    Some((shape, leading + open))
}

/// One key and value a line declared, as ranges into that line.
///
/// Ranges rather than slices, because the text reader turns them into ranges of
/// the page the line already lives in: nothing here copies a byte.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PairSpan {
    pub(crate) key: std::ops::Range<usize>,
    pub(crate) value: std::ops::Range<usize>,
    /// Whether the value is itself a run of pairs, so descending into it
    /// answers something. This is the mixed form the classifier recognizes:
    /// a numeric envelope whose payload states its own fields.
    pub(crate) nested: bool,
    /// Whether the line wrote a `#` in front of the key.
    ///
    /// The key range excludes the marker, because a reader lifting a bridge
    /// key by name asks for the name the bridge gave the field. That stripping
    /// is what destroys the fact: afterwards `#ORDERID=123` and `ORDERID=123`
    /// are the same bytes, and telling a bridge's restatement of a pair from a
    /// second arrival of it is a judgment with nothing left to read. One bool
    /// beside the ranges keeps it, and it is the same fact [`LineEntry`]
    /// already carries on the classification walk.
    pub(crate) marked: bool,
}

/// One pair, with `nested` read off the value it names.
fn span(
    line: &[u8],
    key: std::ops::Range<usize>,
    value: std::ops::Range<usize>,
    marked: bool,
) -> PairSpan {
    PairSpan {
        nested: memchr::memchr(b'=', &line[value.clone()]).is_some(),
        key,
        value,
        marked,
    }
}

/// One pair where nothing has said which byte separates two fields.
///
/// Every byte that ever ends a field ends this one, because the alternative is
/// reading the rest of a sentence as a value.
fn loose_span(line: &[u8], start: usize, equals: usize) -> PairSpan {
    let marked = line.get(start) == Some(&b'#');
    let mut value_end = equals + 1;
    while value_end < line.len() && !is_field_end(line, value_end) {
        value_end += 1;
    }
    span(
        line,
        start + usize::from(marked)..equals,
        equals + 1..value_end,
        marked,
    )
}

/// One pair out of one segment of a frame, when the segment states one.
///
/// The key is what precedes the segment's first `=` and the value is
/// everything after it, so both ends are the frame's own separator rather than
/// a byte guessed at. A segment stating no `=`, or opening with one, states no
/// pair.
fn segment_span(line: &[u8], start: usize, end: usize) -> Option<PairSpan> {
    let mut key_at = start;
    while key_at < end && matches!(line[key_at], b' ' | b'\t') {
        key_at += 1;
    }
    let marked = key_at < end && line[key_at] == b'#';
    let name_at = key_at + usize::from(marked);
    let equals = name_at + memchr::memchr(b'=', &line[name_at..end])?;
    (equals > name_at).then(|| span(line, name_at..equals, equals + 1..end, marked))
}

/// Every pair the line declares, wherever it sits.
///
/// Deliberately not the walk [`inspect`] runs. That one starts at the located
/// frame and stops at the checksum, because classification is decided by what
/// the frame holds; a pair a transport wrote in front of the frame, or a
/// trailer after it, is not part of that decision and is still part of what the
/// line said. This walk reads them all, at the cost of one more pass over bytes
/// the reader already holds.
///
/// # A frame narrows the scan
///
/// Where [`locate_frame`] finds a frame, the pairs from there on are that
/// frame's segments cut at their first `=`, and a value ends only at the
/// frame's own separator. Outside one - a sentence, a transport's prefix, a
/// bare run of attributes - a value still ends at the first [`is_field_end`]
/// byte, because nothing has said which byte separates two fields.
///
/// The loose rule is wrong inside a frame, and these are the inputs that say
/// so. Closing a value at any of eleven bytes cuts `18=G L` at the space and
/// `48=ABBN SW` with it, and reads `58=quoting #A=1 and #B=2` - one Text field
/// quoting two marked keys - as three pairs. Walking a key backwards over key
/// bytes finds no pair at all in `NoAllocs[0].79=ACCT` or `Symbol[0]=AAPL`,
/// because `]` neither continues a key nor opens a field, so a bridge's
/// indexed keys vanish entirely. A frame answers every one of them with a fact
/// it already had, and answers it the way [`inspect`] already reads the same
/// bytes.
///
/// An empty value is a pair inside a frame and is not one outside it. `58=`
/// standing between two separators says the line wrote the field and gave it
/// nothing, which is a different fact from `58` being absent; an unbounded `=`
/// is punctuation as often as it is a pair - `x = 5`, `a == b` - and the loose
/// walk has nothing to tell those two apart with.
pub(crate) fn entry_spans(line: &[u8]) -> impl Iterator<Item = PairSpan> + '_ {
    let frame = locate_frame(line);
    let opens = frame.map_or(line.len(), |frame| frame.start);
    // Pairs arrive in line order, so the loose walk stops where the frame
    // opens rather than filtering the frame's own `=` signs back out of it.
    let outside = pairs(line)
        .take_while(move |(start, _, _)| *start < opens)
        .map(move |(start, _, equals)| loose_span(line, start, equals));
    let mut offset = opens;
    let inside = std::iter::from_fn(move || {
        let separator = frame?.separator;
        while offset < line.len() {
            let start = offset;
            let (end, next) = separator.segment(line, start);
            offset = next;
            if let Some(span) = segment_span(line, start, end) {
                return Some(span);
            }
        }
        None
    });
    outside.chain(inside)
}

/// Whether the line holds any pair at all, marked or not.
fn has_any_pair(line: &[u8]) -> bool {
    pairs(line).any(|(_, key, _)| matches!(key, LineKey::Name(_)))
}

/// Every pair the line holds, in order: where it starts, its key, and where
/// its `=` sits.
///
/// A pair closes its key at an `=`, so the `=` signs are where the pairs
/// are, and the line is read at those rather than tried at every byte. The
/// key is the run of key bytes closing at the `=`; the pair starts where
/// that run starts, one byte earlier at the bridge's `#`, or just past an
/// escaped separator whose spelling ends in a key byte - `^A` and `\x01` -
/// and [`pair_at`] then reads it exactly as it reads a pair anywhere, so
/// what this yields is what a byte-by-byte scan yielded, in the same order.
fn pairs(line: &[u8]) -> impl Iterator<Item = (usize, LineKey<'_>, usize)> + '_ {
    memchr::memchr_iter(b'=', line).filter_map(move |equals| {
        let mut run = equals;
        while run > 0 && is_name_continue(line[run - 1]) {
            run -= 1;
        }
        let before = run.checked_sub(1).map(|at| line[at]);
        let candidates = [
            run.checked_sub(1).filter(|_| before == Some(b'#')),
            Some(run),
            (before == Some(b'^') && line.get(run) == Some(&b'A')).then_some(run + 1),
            (before == Some(b'\\') && line[run..].starts_with(b"x01")).then_some(run + 3),
        ];
        candidates.into_iter().flatten().find_map(|start| {
            let (key, at) = pair_at(line, start)?;
            (at == equals).then_some((start, key, at))
        })
    })
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
    payload(line).0
}

/// Where one line's payload starts, and what a document there states about
/// which way it moved.
///
/// One reading rather than two, because a caller that bounds the prose needs
/// the statement in the same pass. A frame states nothing - which way it moved
/// is the transport's to say - while a bridge configuration document states
/// its own half of a Jolokia exchange, and `true` is the half that came back.
pub(crate) fn payload(line: &[u8]) -> (Option<usize>, Option<bool>) {
    if let Some(frame) = locate_frame(line) {
        return (Some(frame.start), None);
    }
    match ulconfig_at(line) {
        Some(at) => (Some(at), Some(ulconfig_answered(&line[at..]))),
        None => (None, None),
    }
}

/// Where the ULBridge configuration document one line carries opens.
///
/// Two facts hold together and neither is enough alone: the line has to be a
/// JSON object - the whole line, or the tail of one a transport put prose in
/// front of - and that object has to name the ULBridge namespace. The
/// namespace is what makes the reading unambiguous, so a document not carrying
/// it is ordinary JSON and stays that way.
///
/// The object is the outermost one, found by the `{` a member opens behind, so
/// a `[jolokia]` in the prose is not mistaken for the document and the bound a
/// direction is read against is the whole prefix rather than part of it.
fn ulconfig_at(line: &[u8]) -> Option<usize> {
    Some(ulconfig_span(line)?.start)
}

/// The span of the ULBridge configuration document one line carries.
///
/// Two facts hold together and neither is enough alone: the line has to name
/// the ULBridge namespace, and an object has to open in front of that name and
/// close after it. The namespace is what makes the reading unambiguous, so a
/// document not carrying it is ordinary JSON and stays that way.
///
/// The close is found rather than assumed, so a transport writing prose on
/// both sides of the document - a timestamp in front, a duration behind - is
/// read exactly as one writing prose in front alone. A document the line cut
/// short never closes and is no document.
pub(crate) fn ulconfig_span(line: &[u8]) -> Option<std::ops::Range<usize>> {
    // The cheap half first: the namespace is absent from every line that is
    // not one of these, and finding it is one prefiltered pass.
    let named = object_names(line).next()?.start;
    // A bulk read answers an array of these, so either opener opens one -
    // an object by its first member, an array by the object it holds. A
    // `[Jolokia]` in the prose opens neither.
    let opened = memchr::memchr2_iter(b'{', b'[', &line[..named]).find(|at| {
        if line[*at] == b'[' {
            opens_object(line, at + 1)
        } else {
            opens_member(line, at + 1)
        }
    })?;
    Some(opened..json_end(line, opened)?)
}

/// Where the JSON value opening at `start` closes, one past its last byte.
///
/// A brace inside a string closes nothing, and a bridge writes braces into
/// strings routinely: an init file, a `${placeholder}`, a rejection sentence.
/// So the scan tracks the string it is inside and the escape in front of a
/// quote, and depth alone answers the rest.
fn json_end(line: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, byte) in line[start..].iter().enumerate() {
        if quoted {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                // An unbalanced closer closes nothing: a caller pointing at
                // something that is not an opener gets no span rather than a
                // panic.
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(start + offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every ULBridge MBean the line names, rather than every class it spells.
///
/// An ObjectName is a domain and then a `:`, so the namespace has to be
/// followed by the rest of a domain and that colon. A bridge writes its own
/// class names into these documents - `com.ullink.ulbridge2.plugins.ULMsg` on
/// every `$type`, `className` and init file - and a log record quoting one is
/// not a configuration document however clearly it names the product.
///
/// Each name is answered with the properties that follow it, bounded by the
/// quote closing the JSON string it stands in, so what an ObjectName says is
/// read out of that name rather than out of the document around it.
pub(crate) fn object_names(line: &[u8]) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
    memchr::memmem::find_iter(line, ULBRIDGE_NAMESPACE).filter_map(move |start| {
        let mut at = start + ULBRIDGE_NAMESPACE.len();
        while line
            .get(at)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            at += 1;
        }
        if line.get(at) != Some(&b':') {
            return None;
        }
        let end = memchr::memchr(b'"', &line[at..]).map_or(line.len(), |offset| at + offset);
        Some(start..end)
    })
}

/// Whether a `[` at this position opens an array of objects.
///
/// The bulk shape and nothing else: a transport's own `[jolokia]` opens no
/// document, and reading which is which costs one byte.
fn opens_object(line: &[u8], mut at: usize) -> bool {
    while line.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    line.get(at) == Some(&b'{')
}

/// Whether a `{` at this position opens an object rather than stands in prose.
///
/// A JSON object's first member is a quoted name, so the next thing that is
/// not whitespace is a quote. Nothing else is read: this decides where a
/// document starts, and a classifier that parsed to answer that would be
/// parsing every line of a capture.
fn opens_member(line: &[u8], mut at: usize) -> bool {
    while line.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    line.get(at) == Some(&b'"')
}

/// The message type one ULBridge configuration document declares.
///
/// A Jolokia document says two things about its type and the specific one
/// wins. An MBean's ObjectName carries a `type=` segment - `Plugin` or
/// `ConfigurationPlugin` - which is what that entry *is*; the request carries
/// the operation - `read`, `write`, `exec` - which is only how the document
/// was obtained. A wildcard read names no type in its own MBean and every
/// entry it answers with names one, so the first entry's is the document's.
fn ulconfig_msgtype(document: &[u8]) -> Option<&[u8]> {
    object_name_type(document).or_else(|| jolokia_operation(document))
}

/// The `type=` property of the first ObjectName that states one.
///
/// The property is read inside the name rather than across the document, so a
/// value elsewhere that happens to spell `,type=` is that value's business.
/// It counts only where a `,` or a `:` opens it, which is the one shape an
/// ObjectName property has and is not the shape `plugin-type=` has.
fn object_name_type(document: &[u8]) -> Option<&[u8]> {
    object_names(document).find_map(|name| object_name_property(&document[name], OBJECT_NAME_TYPE))
}

/// One property of one ObjectName, by the name it is keyed under.
///
/// An ObjectName is a domain, a `:`, and then `key=value` properties in any
/// order separated by `,`. A property counts only where one of those two
/// delimiters opens it, which is what keeps `plugin-type` from answering for
/// `type`; its value runs to the next `,` or to the end of the name.
pub(crate) fn object_name_property<'name>(
    name: &'name [u8],
    property: &[u8],
) -> Option<&'name [u8]> {
    memchr::memmem::find_iter(name, property)
        .filter(|at| {
            at.checked_sub(1)
                .is_some_and(|before| matches!(name[before], b',' | b':'))
                && name.get(at + property.len()) == Some(&b'=')
        })
        .find_map(|at| {
            let start = at + property.len() + 1;
            let end = name[start..]
                .iter()
                .position(|byte| *byte == b',')
                .map_or(name.len(), |offset| start + offset);
            (end > start).then(|| &name[start..end])
        })
}

/// The operation the Jolokia request asked for.
///
/// The key is matched with its opening quote, so the `$type` discriminator
/// every nested object carries is never read as this one.
fn jolokia_operation(document: &[u8]) -> Option<&[u8]> {
    let at = memchr::memmem::find(document, JOLOKIA_TYPE_KEY)? + JOLOKIA_TYPE_KEY.len();
    let at = skip_to(document, at, b':')? + 1;
    let start = skip_to(document, at, b'"')? + 1;
    let end = start + memchr::memchr(b'"', document.get(start..)?)?;
    (end > start).then(|| &document[start..end])
}

/// The position of `wanted`, when it is the next byte that is not whitespace.
fn skip_to(document: &[u8], mut at: usize, wanted: u8) -> Option<usize> {
    while document.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    (document.get(at) == Some(&wanted)).then_some(at)
}

/// Whether one ULBridge configuration document is an answer, not a request.
///
/// Jolokia echoes the request back inside every answer it sends, and a request
/// carries no such key of its own - so the echo is the whole of the reading. A
/// failed read echoes it too, which is right: an error is a read that came
/// back rather than one that went out.
fn ulconfig_answered(document: &[u8]) -> bool {
    memchr::memmem::find(document, JOLOKIA_REQUEST_KEY).is_some()
}

#[cfg(test)]
mod tests {
    use super::{PairSpan, classify, entry_spans};
    use crate::MimeType;

    /// Every pair a line declares, each rendered as the line wrote it - the
    /// mark put back in front, so a fixture reads the way the capture does.
    fn read(line: &[u8]) -> Vec<String> {
        entry_spans(line)
            .map(|span| rendered(line, &span))
            .collect()
    }

    fn rendered(line: &[u8], span: &PairSpan) -> String {
        format!(
            "{}{}={}",
            if span.marked { "#" } else { "" },
            String::from_utf8_lossy(&line[span.key.clone()]),
            String::from_utf8_lossy(&line[span.value.clone()])
        )
    }

    #[test]
    fn a_value_inside_a_frame_ends_at_the_frames_separator() {
        // Each of these closes early under the loose rule: at the space, at
        // the space again, and at the space a third time - leaving `A=1` and
        // `B=2` standing as pairs of their own.
        assert_eq!(
            read(b"8=FIX.4.4|35=D|18=G L|48=ABBN SW|58=quoting #A=1 and #B=2|10=0|"),
            [
                "8=FIX.4.4",
                "35=D",
                "18=G L",
                "48=ABBN SW",
                "58=quoting #A=1 and #B=2",
                "10=0",
            ]
        );
    }

    #[test]
    fn every_byte_that_ends_a_loose_value_is_ordinary_inside_a_frame() {
        // Ten of the eleven bytes the loose walk closes a value at, inside one
        // SOH-framed value; the eleventh is SOH itself, which the pipe-framed
        // line below carries raw.
        let framed = b"8=FIX.4.4\x0158=a|b c\td\re\n f]g)h}i,j;k\x0110=0\x01";
        assert_eq!(
            read(framed),
            ["8=FIX.4.4", "58=a|b c\td\re\n f]g)h}i,j;k", "10=0"]
        );
        assert_eq!(
            read(b"8=FIX.4.4|35=D|58=x\x01y|10=123|"),
            ["8=FIX.4.4", "35=D", "58=x\u{1}y", "10=123"],
            "the frame opened on a pipe, so a raw SOH is a byte of the value"
        );
    }

    #[test]
    fn a_frame_separates_on_what_it_opened_with_and_not_on_what_it_could_have() {
        assert_eq!(
            read(b"8=FIX.4.4 35=D 11=A 10=123"),
            ["8=FIX.4.4", "35=D", "11=A", "10=123"],
            "nothing else separates these fields, so whitespace does"
        );
        assert_eq!(
            read(b"8=FIX.4.4;35=D;11=A;10=123"),
            ["8=FIX.4.4;35=D;11=A;10=123"],
            "a semicolon separates no frame this crate reads, so it is a byte \
             of the one value the line stated"
        );
    }

    #[test]
    fn a_frame_mixing_soh_spellings_is_still_one_frame() {
        // Three spellings of one separator on one line, which is what a relay
        // rewriting a frame it was handed produces.
        let mixed = br"8=FIX.4.4^A35=D<SOH>11=A\x0110=123";
        assert_eq!(read(mixed), ["8=FIX.4.4", "35=D", "11=A", "10=123"]);
        // And the classifier reads the same frame, so tag 35 is the message
        // type rather than everything up to the next `^A`.
        assert_eq!(classify(mixed), (MimeType::FIX, Some(&b"D"[..])));
    }

    #[test]
    fn an_indexed_or_dotted_key_is_a_key_inside_a_frame() {
        // The loose walk finds neither: it runs a key backwards over key bytes
        // into the `]`, and `]` opens no field.
        assert_eq!(
            read(b"8=FIX.4.4|35=D|NoAllocs[0].79=ACCT|Symbol[0]=AAPL|10=0|"),
            [
                "8=FIX.4.4",
                "35=D",
                "NoAllocs[0].79=ACCT",
                "Symbol[0]=AAPL",
                "10=0",
            ]
        );
        assert_eq!(
            read(b"MSGTYPE=D|#NOPARTYIDS=3|#NOPARTYIDS[0]=PARTYID=ONE"),
            ["MSGTYPE=D", "#NOPARTYIDS=3", "#NOPARTYIDS[0]=PARTYID=ONE"]
        );
    }

    #[test]
    fn the_same_bytes_read_loosely_in_front_of_the_frame_they_precede() {
        assert_eq!(
            read(b"58=quoting #A=1 and #B=2 : 8=FIX.4.4|35=D|10=0|"),
            ["58=quoting", "#A=1", "#B=2", "8=FIX.4.4", "35=D", "10=0"],
            "nothing bounds a field in front of the frame, so every byte that \
             could end one does"
        );
        assert_eq!(
            read(b"8=FIX.4.4|35=D|58=quoting #A=1 and #B=2|10=0|"),
            ["8=FIX.4.4", "35=D", "58=quoting #A=1 and #B=2", "10=0"],
            "the same bytes inside a frame are one Text field"
        );
    }

    #[test]
    fn an_empty_value_is_a_pair_inside_a_frame_and_punctuation_outside_one() {
        assert_eq!(
            read(b"MSGTYPE=D|SYMBOL=|SIDE=null|PRICE=<null>|ACCOUNT=A"),
            [
                "MSGTYPE=D",
                "SYMBOL=",
                "SIDE=null",
                "PRICE=<null>",
                "ACCOUNT=A",
            ],
            "each spelling of absence is a pair; which of them means absent is \
             a dialect's reading"
        );
        assert_eq!(
            read(b"58= 8=FIX.4.4|35=D|10=0|"),
            ["8=FIX.4.4", "35=D", "10=0"],
            "in front of the frame an `=` with nothing after it states nothing"
        );
    }

    #[test]
    fn the_mark_the_line_wrote_survives_the_key_being_stripped() {
        assert_eq!(
            read(b"MSGTYPE=D|ORDERID=123|#ORDERID=123|#SIDE=1"),
            ["MSGTYPE=D", "ORDERID=123", "#ORDERID=123", "#SIDE=1"]
        );
        let line = b"MSGTYPE=D|ORDERID=123|#ORDERID=123";
        let spans: Vec<PairSpan> = entry_spans(line).collect();
        assert_eq!(
            &line[spans[1].key.clone()],
            &line[spans[2].key.clone()],
            "the two keys are the same bytes once the mark is off"
        );
        assert!(!spans[1].marked && spans[2].marked);
        assert_eq!(
            read(b"MSGTYPE=D|ORDERID=123|#ORDERID=345"),
            ["MSGTYPE=D", "ORDERID=123", "#ORDERID=345"],
            "a restatement that differs is still marked"
        );
    }

    #[test]
    fn a_line_with_no_pair_states_none() {
        assert!(read(b"no pairs here at all").is_empty());
        assert!(read(b"x = 5").is_empty(), "an `=` alone is punctuation");
    }
}

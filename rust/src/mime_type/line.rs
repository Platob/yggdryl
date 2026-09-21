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
    /// The line carries a JSON document, whole or behind prose, and opens no
    /// frame in front of it. What a document is *for* is not this scan's to
    /// say: the codec reads none and names the row `unknown`.
    has_json: bool,
    tag_msgtype: Option<&'line [u8]>,
    name_msgtype: Option<&'line [u8]>,
}

impl<'line> LineInference<'line> {
    /// What the line is.
    ///
    /// A FIX frame is numeric tags; a bridge row is `#`-marked keys or a
    /// `MSGTYPE=` key; both together are the mixed form. An `XmlData(213)`
    /// payload that opens with a tag makes the frame FIXML. Failing every
    /// frame rule, a JSON document behind prose is JSON rather than the
    /// pairs its text looks like, a line that is still `key=value`
    /// throughout is the generic key/value shape rather than nothing, and a
    /// document that opens as XML or JSON is that document.
    pub(crate) const fn mime_type(&self) -> MimeType {
        if self.has_xml {
            return MimeType::FIXML;
        }
        match (self.has_tag, self.has_symbolic) {
            (true, true) => MimeType::FIXUL,
            (true, false) => MimeType::FIX,
            (false, true) => MimeType::ULLINK,
            // A document wins over the bare pair rules, exactly as the XML and
            // JSON readings do: what a bridge wrote inside a document is that
            // document's content, never a field. What the document is *for*
            // is the codec's reading and not a name this scan gives it, so it
            // answers the JSON it is.
            (false, false) if self.has_json => MimeType::JSON,
            (false, false) if self.has_pairs => MimeType::KEYVALUE,
            (false, false) => MimeType::OCTET_STREAM,
        }
    }

    /// The message type the line declares, routed through the user-defined
    /// range.
    ///
    /// A raw `MSGTYPE=` is checked across the whole line before numeric tag
    /// 35 and therefore wins when both are present: a bridge writes its own
    /// type in front of a frame it is relaying. A document declares none:
    /// the codec names such a row `unknown`.
    pub(crate) fn msgtype(&self) -> Option<&'line [u8]> {
        self.name_msgtype.or(self.tag_msgtype)
    }

    /// Reads whether the line carries a JSON document, if it does.
    ///
    /// Run only where every frame rule has already declined, which is the one
    /// place the answer could change a thing: a line holding a frame is that
    /// frame whatever document quoted it, and the scan a document costs is
    /// one a frame never pays.
    fn read_json(&mut self, line: &'line [u8]) {
        if self.has_tag || self.has_symbolic || self.has_xml {
            return;
        }
        self.has_json = json_at(line).is_some();
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
    let separator = LineSeparator::for_line(line, start);
    LineFrame {
        start: separator.head(line, start),
        numeric,
        separator,
    }
}

/// How a log spells SOH when it cannot print the byte itself.
///
/// One vocabulary: the separator a frame is located with and the one a reader
/// splits on are the same fact, so a capture that escapes its separator is
/// recognized once rather than in each place that reads a frame.
pub(crate) const SOH_MARKERS: [&[u8]; 4] = [b"^A", b"\\x01", b"<SOH>", b"{SOH}"];

impl LineSeparator {
    /// Every separator a line can name, in the order the walk keeps them.
    ///
    /// No two open on the same byte, so no two ever stand at one position and
    /// the order never breaks a tie; it is the index [`rank`](Self::rank)
    /// answers and nothing more.
    const CANDIDATES: [Self; 4] = [
        Self::Byte(0x01),
        Self::Byte(b'|'),
        Self::Marker,
        Self::Whitespace,
    ];

    /// This separator's index in [`CANDIDATES`](Self::CANDIDATES).
    const fn rank(self) -> usize {
        match self {
            Self::Byte(0x01) => 0,
            Self::Byte(_) => 1,
            Self::Marker => 2,
            Self::Whitespace => 3,
        }
    }

    /// What separates two fields of the frame opening at `start`.
    ///
    /// The earliest candidate wins, because a frame separates on what it
    /// opened with rather than on what it could have: `8=FIX.4.4 35=D 58=a|b
    /// 10=0` opened on a space, and ranking the pipe over it would cut that
    /// frame inside one of its values and read `b 10` as a key. Whitespace
    /// ranks with the other candidates for that reason, and is also what is
    /// left when the line named nothing - which is the difference
    /// [`stated`](Self::stated) exists to draw.
    ///
    /// A candidate ranks where it first stands, and only if it separated a
    /// field somewhere - which [`separates_at`](Self::separates_at) decides
    /// one occurrence at a time. One walk over the tail asks both questions
    /// in position order: a candidate is first met where it first stands,
    /// each occurrence of one still undecided is asked whether it separated
    /// there, and once one has, every candidate met behind it is out, since
    /// it cannot rank above one that stood earlier. The winner so far only
    /// ever moves earlier - a candidate that stood later was never asked -
    /// which is what makes that pruning safe. The walk goes on only while a
    /// candidate that stood earlier than the winner so far is still
    /// undecided, and stops at the tail's end otherwise. So a frame that
    /// names its separator in its first field costs the bytes up to its
    /// second, and the four full scans a ranking of every candidate would
    /// cost are never paid.
    fn for_line(line: &[u8], start: usize) -> Self {
        let tail = &line[start..];
        // Where each candidate first stands, once the walk has reached it.
        let mut first = [None; Self::CANDIDATES.len()];
        // Whether the walk is done with a candidate: it separated a field, or
        // it stands behind one that did.
        let mut settled = [false; Self::CANDIDATES.len()];
        // The candidate that separated a field and stands earliest: where it
        // stands, and its rank.
        let mut best: Option<(usize, usize)> = None;
        let mut at = 0;
        while at < tail.len() {
            let Some((separator, width)) = Self::candidate_at(tail, at) else {
                at += 1;
                continue;
            };
            let rank = separator.rank();
            let next = separator.past(tail, at + width);
            if !settled[rank] {
                let stands = *first[rank].get_or_insert(at);
                if best.is_some_and(|(leads, _)| leads < stands) {
                    settled[rank] = true;
                } else if separator.separates_at(tail, next) {
                    settled[rank] = true;
                    best = Some((stands, rank));
                }
                // Only a candidate met before the winner can still outrank
                // it; one not met yet stands behind this occurrence.
                if let Some((leads, _)) = best {
                    let undecided = first.iter().zip(&settled).any(|(stands, settled)| {
                        !settled && stands.is_some_and(|stands| stands < leads)
                    });
                    if !undecided {
                        break;
                    }
                }
            }
            at = next;
        }
        best.map_or(Self::Whitespace, |(_, rank)| Self::CANDIDATES[rank])
    }

    /// The candidate standing at `at`, and how wide it is there.
    ///
    /// A byte that opens a marker is a marker only where the rest of the
    /// spelling follows: `<` in front of `x|35=D` stands for nothing, and a
    /// walk that took it for a separator would rank a marker the line never
    /// wrote. Whitespace answers one byte wide here; the run it heads is
    /// [`past`](Self::past)'s to step over.
    fn candidate_at(line: &[u8], at: usize) -> Option<(Self, usize)> {
        match line[at] {
            0x01 => Some((Self::Byte(0x01), 1)),
            b'|' => Some((Self::Byte(b'|'), 1)),
            b'^' | b'\\' | b'<' | b'{' => {
                marker_width(&line[at..]).map(|width| (Self::Marker, width))
            }
            byte if byte.is_ascii_whitespace() => Some((Self::Whitespace, 1)),
            _ => None,
        }
    }

    /// Whether this separator separated a field where its next segment opens
    /// at `next`, rather than merely standing inside one.
    ///
    /// A byte a line holds is not a byte a line separated with, and position
    /// alone cannot tell the two apart: `MSGTYPE=P Report Ack|SYMBOL=AAPL`
    /// holds a space before its first pipe and separated nothing with it.
    /// What separates two fields has a field after it, read as
    /// [`segment_span`] reads every other segment of a frame - or closes the
    /// line, which is how a wire message ends. So that line answers the pipe
    /// and keeps `P Report Ack` whole, which is the value the earlier space
    /// would have cut.
    ///
    /// This decides only whether one occurrence separated anything;
    /// [`Self::for_line`] still ranks a candidate where it first stands, and
    /// whitespace never names a frame however early it stands. So a line
    /// running its fields together with spaces named nothing, and falls to
    /// the loose rule whatever else it holds.
    fn separates_at(self, line: &[u8], next: usize) -> bool {
        if next >= line.len() {
            // A wire message ends with its separator, so the line closing
            // on one is that line naming it as plainly as a field after
            // one would.
            return true;
        }
        let (stop, _) = self.segment(line, next);
        segment_span(line, next, stop).is_some()
    }

    /// Whether the line named this separator, rather than being read under
    /// the fallback.
    ///
    /// Whitespace is what a reader falls back on, never what a line states:
    /// every line that runs words together holds some, so reading a space as
    /// a frame's separator would make a sentence a frame. A line that named
    /// its separator said where each of its fields ends; one that named none
    /// said nothing of the sort, and is read by the rule for text that merely
    /// carries pairs.
    const fn stated(self) -> bool {
        !matches!(self, Self::Whitespace)
    }

    /// Where the frame holding the pair at `start` opens.
    ///
    /// A frame is located at the first pair [`pair_at`] can read, and that
    /// walk cannot read every field a frame states: `SYMBOL=` gives no value,
    /// `SYMBOL="A B"` opens on a quote, and `#INSTRUMENT[DESCRIPTION]=HOLCIM
    /// N` is keyed by a name no backwards run over key bytes spells. A frame
    /// whose first field is one of those would otherwise lose it - and lose
    /// it only there, because the same field two segments later is a segment
    /// like any other, which would make one field's reading depend on where
    /// it sits.
    ///
    /// So where the located pair already stands at a segment head, the
    /// segments in front of it belong to the frame too, as long as each
    /// states an `=` and no pair of its own: a field the locator refused is
    /// still a field, while prose states pairs, and those the loose walk
    /// owns. Where nothing stated a separator there are no segments to walk
    /// back over, and the located pair is the head.
    fn head(self, line: &[u8], start: usize) -> usize {
        if !self.stated() {
            return start;
        }
        let mut head = start;
        while head > 0 {
            let Some((at, width)) = self.find_back(line, head) else {
                break;
            };
            if at + width != head {
                break;
            }
            let previous = self
                .find_back(line, at)
                .map_or(0, |(before, width)| before + width);
            let segment = &line[previous..at];
            if memchr::memchr(b'=', segment).is_none() || pairs(segment).next().is_some() {
                break;
            }
            head = previous;
        }
        head
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
            Self::Marker => find_marker(line, start),
            Self::Whitespace => line[start..]
                .iter()
                .position(u8::is_ascii_whitespace)
                .map(|at| (start + at, 1)),
        }
    }

    /// Where this separator last stands before `end`, and how wide it is
    /// there.
    ///
    /// The mirror of [`find`](Self::find), for the one question that reads a
    /// line backwards: which segment the pair a frame was located at opens.
    fn find_back(self, line: &[u8], end: usize) -> Option<(usize, usize)> {
        let head = &line[..end];
        match self {
            Self::Byte(byte) => memchr::memrchr(byte, head).map(|at| (at, 1)),
            Self::Marker => SOH_MARKERS
                .iter()
                .filter_map(|marker| {
                    memchr::memmem::rfind(head, marker).map(|at| (at, marker.len()))
                })
                .max_by_key(|(at, _)| *at),
            Self::Whitespace => head
                .iter()
                .rposition(u8::is_ascii_whitespace)
                .map(|at| (at, 1)),
        }
    }

    /// Where the segment opening at `start` ends, and where the next one opens.
    fn segment(self, line: &[u8], start: usize) -> (usize, usize) {
        let Some((end, width)) = self.find(line, start) else {
            return (line.len(), line.len());
        };
        (end, self.past(line, end + width))
    }

    /// Where the next segment opens, given where this separator ended.
    ///
    /// A run of whitespace separates two fields once, so the next segment
    /// opens past all of it rather than at the second space; every other
    /// separator is as wide as it is.
    fn past(self, line: &[u8], mut next: usize) -> usize {
        if matches!(self, Self::Whitespace) {
            while next < line.len() && line[next].is_ascii_whitespace() {
                next += 1;
            }
        }
        next
    }
}

/// Where an escaped `SOH` next stands at or after `start`, in whichever
/// spelling comes first, and how wide it is there.
///
/// Every spelling opens on one of four bytes, so the openers are visited in
/// position order and the first that the rest of a spelling follows is the
/// answer - the same answer as the earliest of four searches, one per
/// spelling, at the cost of one scan to the next opener rather than four to
/// the end of the line when a spelling is absent. A frame spelled `^A`
/// throughout would otherwise pay three full scans per field to learn that it
/// never spelled the other three.
fn find_marker(line: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut at = start;
    while let Some(offset) = next_marker_opener(&line[at..]) {
        let opens = at + offset;
        if let Some(width) = marker_width(&line[opens..]) {
            return Some((opens, width));
        }
        at = opens + 1;
    }
    None
}

/// Where a byte that opens one of [`SOH_MARKERS`] first stands.
///
/// Three of the four in one scan, and the fourth only over the bytes in
/// front of where those three first stood, so whichever opener is earliest
/// is the answer and no byte is read twice past it.
fn next_marker_opener(bytes: &[u8]) -> Option<usize> {
    let three = memchr::memchr3(b'^', b'\\', b'<', bytes);
    let bound = three.unwrap_or(bytes.len());
    memchr::memchr(b'{', &bytes[..bound]).or(three)
}

/// How wide the escaped `SOH` opening at the head of `bytes` is, when one
/// does.
fn marker_width(bytes: &[u8]) -> Option<usize> {
    SOH_MARKERS
        .iter()
        .find(|marker| bytes.starts_with(marker))
        .map(|marker| marker.len())
}

/// The next field of the frame, for the walk that decides what a line is.
///
/// One reading of a segment, [`segment_span`], is shared with the walk that
/// records what a line wrote, because both ask the same bytes the same
/// question. They part on one answer only: a field given no value says
/// nothing about what the line is, so this walk steps over it, while the
/// entry walk keeps it because the line wrote it.
fn next_entry<'line>(
    line: &'line [u8],
    offset: &mut usize,
    separator: LineSeparator,
) -> Option<LineEntry<'line>> {
    while *offset < line.len() {
        let start = *offset;
        let (end, next) = separator.segment(line, start);
        *offset = next;
        let Some(span) = segment_span(line, start, end) else {
            continue;
        };
        if span.value.is_empty() {
            continue;
        }
        return Some(LineEntry {
            key: line_key(&line[span.key.clone()]),
            value: &line[span.value],
            marked: span.marked,
        });
    }
    None
}

/// What a frame's segment cut out as a key: a tag where it is digits, a name
/// wherever it is anything else a key may be.
///
/// The tag is the same integer [`pair_at`] reads, and a run of digits too
/// wide to be one is a name, because a key a dictionary cannot number is
/// still the name the line gave the field.
fn line_key(key: &[u8]) -> LineKey<'_> {
    key.iter()
        .try_fold(0_i32, |tag, byte| {
            let digit = byte.is_ascii_digit().then(|| i32::from(*byte - b'0'))?;
            tag.checked_mul(10)?.checked_add(digit)
        })
        .filter(|_| !key.is_empty())
        .map_or(LineKey::Name(key), LineKey::Tag)
}

/// The pair standing at `start`, where a line that bounded nothing states one.
///
/// This is the walk that finds pairs, and it is deliberately strict: with no
/// separator named, a pair has to be recognizable from its own bytes, so the
/// key is a bare name or a tag and the value is neither empty nor quoted -
/// `x = 5` and `a == b` are punctuation, and `Symbol[0]=AAPL` is a spelling
/// only a frame can vouch for. Inside a frame the segment is bounded already
/// and [`segment_span`] reads it instead.
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
    // A `#`-marked key and a raw `MSGTYPE=` are each a bridge's own marker,
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
        inferred.read_json(line);
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
                        // A pair-shaped `XmlData` payload is the mixed form:
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
    inferred.read_json(line);
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
/// stays a sentence. A JSON document is located by [`json_span`] instead,
/// and prose closing on braces is prose.
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
pub struct PairSpan {
    pub key: std::ops::Range<usize>,
    pub value: std::ops::Range<usize>,
    /// The segment's original end, before transport decoration was trimmed.
    /// A protocol may select that span without locating its delimiter again.
    pub(crate) value_end: usize,
    /// Whether the line wrote a `#` in front of the key.
    ///
    /// The key range excludes the marker, because a reader lifting a bridge
    /// key by name asks for the name the bridge gave the field. That stripping
    /// is what destroys the fact: afterwards `#ORDERID=123` and `ORDERID=123`
    /// are the same bytes, and telling a bridge's restatement of a pair from a
    /// second arrival of it is a judgment with nothing left to read. One bool
    /// beside the ranges keeps it, and it is the same fact `LineEntry`
    /// already carries on the classification walk.
    pub marked: bool,
}

impl PairSpan {
    /// Whether the value is itself a run of pairs, so descending into it
    /// answers something. This is the mixed form the classifier recognizes -
    /// a numeric envelope whose payload states its own fields - and it is
    /// also how a Text field that quotes pairs is read: where the field ends
    /// is the frame's to say, and what the field's own text says is read
    /// beneath it rather than beside the frame's fields.
    ///
    /// Asked rather than carried, because only the walk that records what a
    /// line wrote descends: the walk that decides what a line is reads the
    /// same segments and never asks, and a scan of every value for an `=` it
    /// would not act on is a scan of most of the line.
    pub(crate) fn nested(&self, line: &[u8]) -> bool {
        memchr::memchr(b'=', &line[self.value.clone()]).is_some()
    }
}

/// One pair, as ranges of the line.
const fn span(
    key: std::ops::Range<usize>,
    value: std::ops::Range<usize>,
    marked: bool,
) -> PairSpan {
    PairSpan {
        key,
        value_end: value.end,
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
        start + usize::from(marked)..equals,
        equals + 1..value_end,
        marked,
    )
}

/// One pair out of one segment of a frame, when the segment states one.
///
/// The one reading of a frame's segment: both the walk that decides what a
/// line is and the walk that records what it wrote come here, so a frame
/// cannot be cut two ways.
///
/// The key is what precedes the segment's first `=` and the value is what
/// follows it, so both ends are the frame's own separator rather than a byte
/// guessed at. Two bounds keep a segment that states no field from becoming
/// one. The key is a name or a tag, indexed and dotted where the writer
/// indexed and dotted it - `NoAllocs[0].79`, `#INSTRUMENT[DESCRIPTION]`,
/// `Msg Type`. A key runs on name bytes and stops at the first byte no name
/// holds, so `10=0<SOH> sent >> seq=7` states the note's `seq` and no field
/// keyed `sent >> seq`; a remark spelled entirely in words does state one,
/// which is what admitting the space costs and [`is_segment_key`] argues.
/// And the value gives back the punctuation a transport closed the line with,
/// the `)` on a bridge row a log wrapped in parentheses, so one column holds
/// one spelling of one value however the line that carried it was decorated.
fn segment_span(line: &[u8], start: usize, end: usize) -> Option<PairSpan> {
    let mut key_at = start;
    while key_at < end && line[key_at].is_ascii_whitespace() {
        key_at += 1;
    }
    let marked = key_at < end && line[key_at] == b'#';
    let name_at = key_at + usize::from(marked);
    let equals = name_at + memchr::memchr(b'=', &line[name_at..end])?;
    if !is_segment_key(&line[name_at..equals]) {
        return None;
    }
    let value_end = equals + 1 + trimmed_segment_end(&line[equals + 1..end]);
    let mut pair = span(name_at..equals, equals + 1..value_end, marked);
    pair.value_end = end;
    Some(pair)
}

/// The end of a frame value after its transport's trailing decoration.
///
/// A closing bracket is decoration only where no byte in the value opened it.
/// The common unbracketed tail keeps the old backward trim. A tail containing
/// a closer pays one forward pass over this value, tracking each bracket kind
/// independently; the last closer that answered an opener protects the bytes
/// through it from that trim. Brackets are opaque payload bytes here: this
/// answers only which tail the transport owned, never what a field means.
fn trimmed_segment_end(value: &[u8]) -> usize {
    let mut trimmed = value.len();
    let mut has_closer = false;
    while trimmed > 0
        && matches!(
            value[trimmed - 1],
            b' ' | b'\t' | b'\r' | b'\n' | b']' | b')' | b'}' | b',' | b';'
        )
    {
        has_closer |= matches!(value[trimmed - 1], b']' | b')' | b'}');
        trimmed -= 1;
    }
    if !has_closer {
        return trimmed;
    }
    let mut square = 0_usize;
    let mut round = 0_usize;
    let mut curly = 0_usize;
    let mut protected = None;
    for (at, byte) in value.iter().enumerate() {
        match byte {
            b'[' => square += 1,
            b'(' => round += 1,
            b'{' => curly += 1,
            b']' if square > 0 => {
                square -= 1;
                protected = Some(at + 1);
            }
            b')' if round > 0 => {
                round -= 1;
                protected = Some(at + 1);
            }
            b'}' if curly > 0 => {
                curly -= 1;
                protected = Some(at + 1);
            }
            _ => {}
        }
    }
    protected.map_or(trimmed, |end| trimmed.max(end))
}

/// Whether what a segment put in front of its `=` is a key.
///
/// A frame widens which bytes a key may hold - a bridge indexes and qualifies
/// its keys, a renderer spells one `Msg Type`, and `[`, `]`, `.` and a space
/// are part of the name each wrote - never whether a key is a name at all.
///
/// The space is the one that costs something, and the cost is stated rather
/// than dodged. A renderer that spells `Msg Type=D` spelled `MsgType`, because
/// the fold every name resolves by drops a space exactly as it drops `_` and
/// `-`, and reading that segment as prose would lose a field the frame plainly
/// wrote. The price is that a remark spelled entirely in words is a key too:
/// a frame's tail reading ` trailing note=x` states a field keyed
/// `trailing note`, where ` sent >> seq=7` states only `seq`, because `>`
/// is a byte no name holds. Both are what a run of name bytes in front of an
/// `=` looks like, and no rule this walk may hold - it knows no dictionary -
/// tells the renderer's key from the log's sentence. What a key *means* is
/// the reader's, and a reader holding a dictionary answers nothing for
/// `trailing note`.
///
/// A second mark is part of the key. A bridge marks a key to say it restates
/// one it already wrote, and marks a restatement of a marked key again: the
/// walk strips the one mark it reads, so `##ORDERID` arrives here as
/// `#ORDERID` and is the key `##ORDERID` the frame wrote. What two marks mean
/// belongs to whoever holds the dictionary; that the frame stated a field here
/// is this walk's answer.
fn is_segment_key(key: &[u8]) -> bool {
    let name = key.strip_prefix(b"#").unwrap_or(key);
    name.first()
        .is_some_and(|first| is_name_start(*first) || first.is_ascii_digit())
        && name
            .iter()
            .all(|byte| is_name_continue(*byte) || matches!(byte, b'[' | b']' | b' '))
}

/// Every pair the line declares, wherever it sits.
///
/// Deliberately not the walk [`inspect`] runs. That one starts at the located
/// frame and stops at the checksum, because classification is decided by what
/// the frame holds; a pair a transport wrote in front of the frame, or a
/// trailer after it, is not part of that decision and is still part of what the
/// line said. This walk reads them all: it locates the frame, reads the pairs
/// in front of it under the loose rule, and walks the frame's own segments.
///
/// # A frame narrows the scan
///
/// A frame here is a run of pairs the line separated with a byte it named -
/// a `SOH` raw or escaped, or a pipe. Inside one the pairs are that frame's
/// segments cut at their first `=`, and a value ends only at that separator.
/// Everywhere else - a sentence, a transport's prefix, a bare run of
/// attributes, a frame that ran its fields together with spaces - a value
/// still ends at the first [`is_field_end`] byte, because nothing has said
/// which byte separates two fields. Whitespace is never the naming: every
/// line that runs words together holds some, so reading a space as a frame's
/// separator would make `host=srv1, port=8080` one field.
///
/// The loose rule is wrong inside a frame, and these are the inputs that say
/// so. Closing a value at any of eleven bytes cuts `18=G L` at the space and
/// `48=ABBN SW` with it, and reads `58=quoting #A=1 and #B=2` - one Text field
/// quoting two marked keys - as three pairs beside the frame's own. Walking a
/// key backwards over key bytes finds no pair at all in `NoAllocs[0].79=ACCT`
/// or `Symbol[0]=AAPL`, because `]` neither continues a key nor opens a field,
/// so a bridge's indexed keys vanish wherever a frame did not open the walk.
/// A frame answers every one of them from a fact it already had, and answers
/// it through the one segment reading [`segment_span`] gives [`inspect`] too.
///
/// A value that states pairs of its own is still descended into, which is
/// what [`PairSpan::nested`] has always meant: the frame decides where the
/// Text field ends, and what that field's own text says is read under it
/// rather than beside the frame's fields.
///
/// A frame's segment states one field or it states prose - a log's own remark
/// after the frame it quoted - and prose here is read the way prose is read
/// anywhere: `10=0<SOH> sent >> seq=7` states the frame's `10` and the note's
/// `seq`, and no field keyed `sent >> seq`. Where the remark is spelled in
/// nothing but words, the frame's own segment reading claims it -
/// `10=0<SOH> trailing note=x` states a field keyed `trailing note` - which is
/// what admitting a space into a key costs and [`is_segment_key`] argues.
/// Only the frame's own walk stops where the frame does; nothing the line
/// said is dropped.
///
/// An empty value is a pair inside a frame and is not one outside it. `58=`
/// standing between two separators says the line wrote the field and gave it
/// nothing, which is a different fact from `58` being absent; an unbounded `=`
/// is punctuation as often as it is a pair - `x = 5`, `a == b` - and the loose
/// walk has nothing to tell those two apart with. [`inspect`] reads the same
/// segment and steps over that field, because a field given nothing says
/// nothing about what the line is.
pub(crate) fn entry_spans(line: &[u8]) -> impl Iterator<Item = PairSpan> + '_ {
    located_entry_spans(line).1
}

/// [`entry_spans`], beside where the frame it located opens.
///
/// The frame is located once for both answers: a reader that wants the
/// pairs and where the payload starts - the codec, bounding a message to
/// its frame - would otherwise walk the line to the frame twice, and on a
/// bridge row that walk is every pair the row holds. `None` where the line
/// holds no frame at all, which is where [`payload_at`] goes on to ask
/// whether a document opens instead.
pub(crate) fn located_entry_spans(line: &[u8]) -> (Located, impl Iterator<Item = PairSpan> + '_) {
    let located = locate_frame(line);
    let frame = located.filter(|frame| frame.separator.stated());
    let located = Located {
        frame_at: located.map(|frame| frame.start),
        separated: frame.is_some(),
    };
    let opens = frame.map_or(line.len(), |frame| frame.start);
    // Pairs arrive in line order, so the loose walk stops where the frame
    // opens rather than filtering the frame's own `=` signs back out of it.
    let outside = pairs(line)
        .take_while(move |(start, _, _)| *start < opens)
        .map(move |(start, _, equals)| loose_span(line, start, equals));
    let mut offset = opens;
    let segments = std::iter::from_fn(move || {
        let separator = frame?.separator;
        (offset < line.len()).then(|| {
            let start = offset;
            let (end, next) = separator.segment(line, start);
            offset = next;
            start..end
        })
    });
    // A segment states one field, or it states prose - a log's own remark
    // after the frame it quoted - and prose is read here the way prose is
    // read anywhere else on the line, so a pair written inside it is still a
    // pair the line said.
    let inside = segments.flat_map(move |segment| {
        let stated = segment_span(line, segment.start, segment.end);
        let loose = stated.is_none().then(|| {
            let at = segment.start;
            pairs(&line[segment])
                .map(move |(start, _, equals)| loose_span(line, at + start, at + equals))
        });
        stated.into_iter().chain(loose.into_iter().flatten())
    });
    (located, outside.chain(inside))
}

/// What one scan located: where the frame opens, and whether the line
/// named a separator for it.
///
/// The two facts one [`locate_frame`] answers, carried out of the scan
/// that computed them so a reader that bounds a message to its frame and
/// then asks whether the run of pairs was a frame or prose asks the same
/// walk once. A frame is a run of pairs the line named a separator for - a
/// `SOH` raw or escaped, or a pipe - and whitespace names none: a run of
/// *named* keys is a bridge row where the line separated it and prose
/// carrying an `=` where it did not, which is what tells
/// `heartbeat emitted seq=7` from `ACCOUNT=A1|SIDE=1`. A numeric frame is
/// FIX whatever separated it, so the codec asks this only of a run it did
/// not already read as tags.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Located {
    /// Where the message starts inside the line, when one is there.
    pub(crate) frame_at: Option<usize>,
    /// Whether the line named a separator for the run of pairs its payload
    /// opens with: a `SOH` raw or escaped, or a pipe, and never whitespace.
    pub(crate) separated: bool,
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
/// direction is read from and what a body strips: a frame's start, else
/// where the one document that opens a payload does. A payload states
/// nothing about which way it moved; the prose in front of it does.
pub(crate) fn payload_at(line: &[u8]) -> Option<usize> {
    locate_frame(line)
        .map(|frame| frame.start)
        .or_else(|| json_at(line))
}

/// Where the JSON document one line carries opens, if it carries one.
fn json_at(line: &[u8]) -> Option<usize> {
    Some(json_span(line)?.start)
}

/// The span of the JSON document one line carries, whole or behind prose.
///
/// A document is an object, or an array of objects, that opens before any
/// pair the line could be read by - a `=` in front of the opener makes the
/// braces a value rather than a document - and closes on its own last byte,
/// with prose allowed behind it: a transport writes a timestamp in front of
/// one and sometimes a duration behind. What the document says is not read
/// here or anywhere: a body the codec cannot read is a message that states
/// no type, and this is what locates it.
///
/// The object is found by the `{` a member opens behind, so a `[jolokia]` in
/// the prose or a brace in a sentence opens nothing, and the bound a
/// direction is read against is the whole prefix rather than part of it.
pub(crate) fn json_span(line: &[u8]) -> Option<std::ops::Range<usize>> {
    let opened = memchr::memchr2_iter(b'{', b'[', line).find(|at| {
        if line[*at] == b'[' {
            opens_object(line, at + 1)
        } else {
            opens_member(line, at + 1)
        }
    })?;
    if memchr::memchr(b'=', &line[..opened]).is_some() {
        return None;
    }
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/mime_type/line.rs` pins and a caller cannot reach.
    //!
    //! The line classifier is the walk that decides what a line *is* - which
    //! byte it framed its fields with, where a value ends, which pairs it
    //! stated at all - and every caller above it sees only the
    //! [`MimeType`](crate::MimeType) it answered. `mime_type` is a private
    //! module of the crate, so `PairSpan` is `pub` here and still reaches
    //! nobody.
    pub use super::PairSpan;
    use crate::MimeType;

    /// What a line is, and the message type it states where it states one.
    pub fn classify(line: &[u8]) -> (MimeType, Option<&[u8]>) {
        super::classify(line)
    }

    /// Every pair the line states, as ranges into the line itself.
    pub fn entry_spans(line: &[u8]) -> impl Iterator<Item = PairSpan> + '_ {
        super::entry_spans(line)
    }
}

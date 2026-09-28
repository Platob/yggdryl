//! ULBridge's own log line, which is the product's.
//!
//! The messages a line carries are FIX's, generic over the bridge that
//! reports them; what is ULBridge's is the shape of the line it writes,
//! which no other bridge writes. A message is the same wherever it is
//! hosted; a log line is whoever wrote it.

/// The row header a bridge writes in front of every line of its log.
///
/// A clock, a thread bracket, the plugin that wrote the line and its level,
/// which is what a text read frames a bridge log with. The bracket is the
/// line's own statement about the message it handled: the thread that wrote
/// it always, and - on a line handling one message - the session, the
/// message context and the sequence number, separated as the bridge writes
/// them. Those three are optional as a whole, so a line that carries only
/// the thread still frames and leaves them null rather than failing the row.
///
/// The clock is `mtime`, so the header dates the lines it matches: a text
/// read consumes the capture into the line's `currunix` rather than
/// carrying it beside it, and reads it at `datetime64(ns, UTC)` - the
/// text options' `timezone` stating the zone a bridge's local clock is in -
/// so every line it matches is dated by what the bridge wrote in front of
/// it, never by the file's modification time. The line's instant is its messages'
/// `recdunix` and the sending clock of any that states no
/// `SendingTime(52)`; a message stating one keeps its own. The fraction is
/// what bridges write: a point or a comma, three digits, and the grouped
/// microseconds a bridge spells `23:59:46.524_315`, or no fraction at all.
///
/// A capture that names a field fills it, so the registry's one namespace
/// is what lands it and nothing translates in between: `msgctxid` fills the
/// crate's own [`MsgCtxId`](super::MSGCTXID_TAG_NAME); `msgseqnum` fills
/// FIX's own `MsgSeqNum(34)`; `msgsessionid` fills the crate's own
/// [`MsgSessionId`](super::MSGSESSIONID_TAG_NAME); `msgpluginid` is the
/// plugin that logged the line and fills
/// [`MsgPluginId`](super::MSGPLUGINID_TAG_NAME) - the session names the
/// line moved between are what the line itself spells, never the plugin.
///
/// The other two name no field, and are the capture's own columns carried
/// in front of the row: `msgthreadid`, and `loglevel`, the bridge's own log
/// level, which answers no column but its own.
///
/// The bridge writes the session, the context and the sequence in camel
/// case - `senderSessionId`, `msgCtxId`, `seqNum` - and they were captured
/// that way, with a table mapping `seqnum` onto tag 34. Naming the captures for the fields instead
/// retires that table: a capture reaches its field because it is called what
/// the field is called, which is how every other fill in this crate works.
///
/// `msgsessionid` is the session *instance* the bridge handled the line
/// on, and is its own column rather than `SenderSessionId`: a bridge row
/// spells `SESSIONID` for the counterparty session the message is on, and two
/// connections to one counterparty are two instances, so they are two facts.
/// A fill never lands over a value the message already stated, so a row
/// spelling one of these keeps its own reading.
///
/// # Widening or narrowing this expression moves a lifecycle
///
/// A line the expression does not match yields no captures at all rather
/// than failing the row, so it keeps its body, is dated by the handle's
/// modification time - the epoch where the handle has none - and arrives
/// at [`FixCodec::lifecycle`](super::FixCodec::lifecycle)
/// carrying no session instance, no message context and no sequence. Those
/// three with the message type are what the capture's
/// [`MsgSessEventId`](super::MSGSESSEVENTID_TAG_NAME) joins, and that is
/// the key two observations of one session event are merged on, so a line
/// this expression misses is a line the walk cannot fold. Editing the
/// fraction, the bracket or any other part of it therefore changes how many
/// events a walk over one unchanged capture answers - and a missed line's
/// instant, so its identity - while every message still parses and every
/// content digest still agrees. The count of lines a header matched is
/// worth asserting next to the count of messages parsed.
///
/// It admitted only three fractional digits until 0.1.10, which is why it
/// could not read the last fifteen lines of the capture shipped beside it -
/// and why a walk over that capture answered four events more than the
/// same capture read whole. Until 0.1.17 its clock was a capture named
/// `timestamp`, which dated nothing, so every line of a read took the one
/// modification time of its file.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let options = yggdryl::text::TextOptions::new()
///     .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)?;
/// let captures = options.source_field()?;
/// let names: Vec<&str> = captures.fields().iter().map(yggdryl::Field::name).collect();
/// // The clock is consumed into each line's `currunix`, so it leads no column.
/// assert!(names.ends_with(&["msgthreadid", "msgsessionid", "msgctxid", "msgseqnum", "msgpluginid", "loglevel"]));
/// assert!(!names.contains(&"mtime"));
/// assert_eq!(captures.field("msgseqnum")?.dtype(), &yggdryl::DataType::Int64);
/// let lines = yggdryl::text::read_text_lines(
///     &yggdryl::holder::Buffer::from_bytes(
///         b"2026-01-02 10:15:30.125 [7] [OMS] (INFO) one\n2026-01-02 10:15:31,250 [7] [OMS] (INFO) two\n2026-01-02 10:15:32 [7] [OMS] (INFO) three\n".to_vec(),
///     ),
///     &options,
/// )?
/// .collect::<yggdryl::Result<Vec<_>>>()?;
/// let dated: Vec<Option<i64>> = lines.iter().map(|line| line.mtime().ok().flatten()).collect();
/// assert_eq!(
///     dated,
///     [Some(1_767_348_930_125_000_000), Some(1_767_348_931_250_000_000), Some(1_767_348_932_000_000_000)],
/// );
/// # Ok(())
/// # }
/// ```
pub const ULBRIDGE_ROWHEADER: &str = r"^(?P<mtime>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:[.,]\d{3}(?:_\d{3})?)?) \[(?P<msgthreadid>[1-9]\d*)(?:-(?P<msgsessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] \[(?P<msgpluginid>[^\]]+)\] \((?P<loglevel>[A-Z]+)\) ";

/// The plugin a message came into the bridge through, as the prose in front
/// of its payload names it, the first of three sentences that reads:
///
/// - `Message received: Message type [..] from (OMS_X1_OrderOut as
///   XM8NNITE382) ...` - the plugin inside the parentheses;
/// - `Execution report from OMS_X1_OrderOut type trade for ...` - the
///   plugin the report came from;
/// - `Receiving : 8=FIX...` - `msgpluginid`, the plugin that logged the
///   line, which is the one the message arrived at.
///
/// Nothing where none reads. Provenance: a message states it nowhere, so it
/// is never content and never an input of the code the message digests to.
pub(super) fn originator<'a>(prose: &'a [u8], msgpluginid: Option<&'a str>) -> Option<&'a str> {
    after(prose, b"Message received: ")
        .and_then(|rest| after(rest, b" from ("))
        .and_then(|rest| word_before(rest, b" as "))
        .or_else(|| {
            after(prose, b"Execution report from ").and_then(|rest| word_before(rest, b" type"))
        })
        .or_else(|| {
            (contains(prose, b"Receiving :") || contains(prose, b"Receiving:"))
                .then_some(msgpluginid)
                .flatten()
                .filter(|plugin| !plugin.is_empty())
        })
}

/// The conversation the prose in front of a payload files the message under:
/// the text of its `{conversationId: 80d1f8e9-...}`, trimmed, and nothing
/// where it states none or spells an absence.
pub(super) fn conversation(prose: &[u8]) -> Option<&str> {
    let rest = after(prose, b"{conversationId:")?;
    let end = rest.iter().position(|byte| *byte == b'}')?;
    let text = std::str::from_utf8(&rest[..end]).ok()?.trim();
    (!text.is_empty() && !crate::code::is_null_like(text)).then_some(text)
}

/// What follows the first `needle` in `haystack`.
fn after<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| &haystack[at + needle.len()..])
}

/// Whether `needle` occurs in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    after(haystack, needle).is_some()
}

/// The one word `rest` opens with, ending where `terminator` does.
fn word_before<'a>(rest: &'a [u8], terminator: &[u8]) -> Option<&'a str> {
    let end = rest
        .windows(terminator.len())
        .position(|window| window == terminator)?;
    let word = std::str::from_utf8(&rest[..end]).ok()?;
    (!word.is_empty() && !word.contains(char::is_whitespace)).then_some(word)
}

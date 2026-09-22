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
/// A capture that names a field fills it, so the registry's one namespace
/// is what lands it and nothing translates in between: `msgctxid` fills the
/// crate's own [`MsgCtxId`](super::MSGCTXID_TAG_NAME); `msgseqnum` fills
/// FIX's own `MsgSeqNum(34)`; `msgsessionid` fills the crate's own
/// [`MsgSessionId`](super::MSGSESSIONID_TAG_NAME); `msgpluginid` is the
/// plugin that logged the line and fills
/// [`MsgPluginId`](super::MSGPLUGINID_TAG_NAME) - the session names the
/// line moved between are what the line itself spells, never the plugin.
///
/// The other three name no field, and are the capture's own columns carried
/// in front of the row: `timestamp`, `msgthreadid` and `level`.
///
/// `timestamp` dates nothing, and is not this header's failing but its
/// shape. It is not the line's clock: that is the `mtime` column, which
/// only a capture spelled `mtime` fills, so a line read through this header
/// falls back to the handle's own modification time and an unlocated handle
/// leaves every line at the epoch. It is not the message's clock either: a
/// message is dated by the `SendingTime(52)` it states, else the
/// `TransactTime(60)` it states, else the codec's `default_sending_time`.
/// A caller who wants this header to date its lines names the capture
/// `mtime`, and pays two prices for it: an `mtime` capture is consumed into
/// that column instead of being carried beside it, so the `timestamp`
/// column goes; and the column is read at `datetime64(ns, UTC)` whatever
/// fraction the expression spells, so the syntax no longer types the clock.
/// `level` is the bridge's own log level and answers no column but its own,
/// which a caller who wants it elsewhere renames the same way.
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
/// than failing the row, so it keeps its body, settles at the epoch pin, and
/// arrives at [`FixCodec::lifecycle`](super::FixCodec::lifecycle)
/// carrying no session instance, no message context and no sequence. Those
/// three with the message type are what build `msgsesseventid`, and that is
/// the key two observations of one session event are merged on, so a line
/// this expression misses is a line the walk cannot fold. Editing the
/// fraction, the bracket or any other part of it therefore changes how many
/// events a walk over one unchanged capture answers, while every message
/// still parses and every content digest still agrees. The count of lines a
/// header matched is worth asserting next to the count of messages parsed.
///
/// This one matches a fraction of exactly three digits, so it does not read
/// a bridge that writes its microseconds grouped - `23:59:46.524_315` - and
/// leaves those lines undated and unmerged.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let options = yggdryl::text::TextOptions::new()
///     .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)?;
/// let captures = options.source_field()?;
/// let names: Vec<&str> = captures.fields().iter().map(yggdryl::Field::name).collect();
/// assert!(names.ends_with(&["timestamp", "msgthreadid", "msgsessionid", "msgctxid", "msgseqnum", "msgpluginid", "level"]));
/// assert_eq!(captures.field("msgseqnum")?.dtype(), &yggdryl::DataType::Int64);
/// // The expression's own fraction types the capture it is part of, which
/// // is what this pins; which lines the expression matches is the separate
/// // and larger consequence the section above states.
/// assert_eq!(
///     captures.field("timestamp")?.dtype(),
///     &yggdryl::DataType::datetime64(yggdryl::TimeUnit::Millisecond, yggdryl::Timezone::NAIVE)?,
/// );
/// # Ok(())
/// # }
/// ```
pub const ULBRIDGE_ROWHEADER: &str = r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<msgthreadid>[1-9]\d*)(?:-(?P<msgsessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] \[(?P<msgpluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) ";

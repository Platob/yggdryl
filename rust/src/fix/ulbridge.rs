//! ULBridge's own log line, which is the product's and not a plugin's.
//!
//! Decision 18 made the reading of a plugin FIX's, generic over the bridge
//! that reports it, and this is what stayed behind: the shape of the line
//! ULBridge writes, which no other bridge writes. A plugin is the same
//! wherever it is hosted; a log line is whoever wrote it.

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
/// Every capture is named for what it does. `timestamp` is the row's clock,
/// so it stamps the message; `msgCtxId` fills the crate's own
/// [`MsgCtxId`](super::MSGCTXID_TAG_NAME); `seqNum` fills `MsgSeqNum(34)`,
/// through the bridge's own spellings of standard fields; `pluginid` is the
/// plugin that logged the line, which fills the crate's own
/// [`PluginId`](super::PLUGINID_TAG_NAME) - the session names the line
/// moved between are what the line itself spells, never the plugin;
/// `senderSessionId` is the session instance the bridge handled the line
/// on, and fills [`SenderSessionId`](super::SENDERSESSIONID_TAG_NAME) - but
/// only where the
/// message states none of its own, because a fill never lands over a value the
/// message already stated. A row spelling `SESSIONID` therefore keeps its own
/// reading, and a line that spells nothing takes the bracket's.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let options = yggdryl::media::text::TextOptions::new()
///     .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)?;
/// let captures = options.source_field()?;
/// let names: Vec<&str> = captures.fields().iter().map(yggdryl::Field::name).collect();
/// assert!(names.ends_with(&["timestamp", "threadId", "senderSessionId", "msgCtxId", "seqNum", "pluginid", "level"]));
/// assert_eq!(captures.field("seqNum")?.dtype(), &yggdryl::DataType::Int64);
/// // The bridge's own millisecond fraction is what types its stamp, so a
/// // widened probe must leave this capture exactly where it was.
/// assert_eq!(
///     captures.field("timestamp")?.dtype(),
///     &yggdryl::DataType::DateTime64 {
///         unit: yggdryl::TimeUnit::Millisecond,
///         timezone: yggdryl::Timezone::NAIVE,
///     },
/// );
/// # Ok(())
/// # }
/// ```
pub const ULBRIDGE_ROWHEADER: &str = r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<threadId>[1-9]\d*)(?:-(?P<senderSessionId>[0-9a-f]{8}):(?P<msgCtxId>[0-9a-f]{10}):(?P<seqNum>\d+))?\] \[(?P<pluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) ";

/// The standard tag one of the bridge's own capture spellings fills.
///
/// A bridge writes `seqNum` in its thread bracket where FIX says
/// `MsgSeqNum`, and a capture named as the bridge spells it should still
/// land on FIX's own tag. Folded, like every name here, so `SEQNUM` and
/// `seqnum` are one spelling.
#[must_use]
pub(super) fn capture_tag(name: &str) -> Option<i32> {
    CAPTURE_SPELLINGS
        .iter()
        .find(|(spelling, _)| crate::types::folds_equal(spelling, name))
        .map(|(_, tag)| *tag)
}

/// The bridge's own spellings of standard fields, beside the tags they fill.
const CAPTURE_SPELLINGS: [(&str, i32); 1] = [("seqnum", 34)];

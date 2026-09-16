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
/// Every capture is named for the field it fills, so the registry's one
/// namespace is what lands it and nothing translates in between. `timestamp`
/// is the row's clock, so it stamps the message; `msgctxid` fills the
/// crate's own [`MsgCtxId`](super::MSGCTXID_TAG_NAME); `msgseqnum` fills
/// FIX's own `MsgSeqNum(34)`; `bridgesessionid` fills the crate's own
/// [`BridgeSessionId`](super::BRIDGESESSIONID_TAG_NAME); `pluginid` is the
/// plugin that logged the line and fills
/// [`PluginId`](super::PLUGINID_TAG_NAME) - the session names the line moved
/// between are what the line itself spells, never the plugin.
///
/// The bridge writes these three in camel case - `senderSessionId`,
/// `msgCtxId`, `seqNum` - and they were captured that way, with a table
/// mapping `seqnum` onto tag 34. Naming the captures for the fields instead
/// retires that table: a capture reaches its field because it is called what
/// the field is called, which is how every other fill in this crate works.
///
/// `bridgesessionid` is the session *instance* the bridge handled the line
/// on, and is its own column rather than `SenderSessionId`: a bridge row
/// spells `SESSIONID` for the counterparty session the message is on, and two
/// connections to one counterparty are two instances, so they are two facts.
/// A fill never lands over a value the message already stated, so a row
/// spelling one of these keeps its own reading.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let options = yggdryl::media::text::TextOptions::new()
///     .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)?;
/// let captures = options.source_field()?;
/// let names: Vec<&str> = captures.fields().iter().map(yggdryl::Field::name).collect();
/// assert!(names.ends_with(&["timestamp", "threadId", "bridgesessionid", "msgctxid", "msgseqnum", "pluginid", "level"]));
/// assert_eq!(captures.field("msgseqnum")?.dtype(), &yggdryl::DataType::Int64);
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
pub const ULBRIDGE_ROWHEADER: &str = r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<threadId>[1-9]\d*)(?:-(?P<bridgesessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] \[(?P<pluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) ";

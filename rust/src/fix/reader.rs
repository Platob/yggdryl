//! The three readers, and the stream that holds what is constant across rows.
//!
//! Each reader splits its own dialect, rewrites it into the key forms the one
//! [builder](super::build) understands, and hands the pairs over. None of them
//! parses a message and none of them builds a tree of its own.
//!
//! # A row is a log line
//!
//! A capture line is a message wrapped in whatever the process printed around
//! it - `sending >> 8=FIX.4.2|…|10=203| << queued seq=1092` - so the frame is
//! located first and everything before it is prefix. Reading from byte zero
//! would take `sending >> 8` as the first key and pick the wrong dialect on
//! nearly every real line.
//!
//! The dialect is decided once, from the frame, and never re-sniffed: all
//! ASCII digits before the frame's first `=` means numeric FIX, anything else
//! means a bridge row. Once decided it holds for the whole body, so a `#` or a
//! `<` inside a *value* is part of that value.
//!
//! # Nothing is skipped
//!
//! A row with no message type is built anyway and named `unknown`; a value
//! that will not type is null; a group that will not split stays whole. What
//! is left - input that is not a row at all - is an `Err` item carrying it,
//! and the stream continues, because one corrupt line must not end a run over
//! ten million.

use std::sync::Arc;

use crate::mime_type::line;
use crate::{Error, Result, Version};

use super::build::{Builder, root_name};
use super::project::Projections;
use super::{FixBranch, FixMsg, FixRegistry};

/// What separates the members packed inside one bridge group occurrence.
///
/// ULLINK writes EOT then ETX. A bridge relaying into a FIX session writes the
/// protocol's own SOH instead, which is unambiguous inside an occurrence
/// because no FIX value may contain one.
const MEMBER_SEPARATORS: [&[u8]; 2] = [b"\x04\x03", b"\x01"];

/// The spellings that mean "nothing was sent", by default.
///
/// A bridge with nothing to say writes one of these, and a reader that keeps
/// them puts the four characters `null` into a column whose answer is no
/// answer. The match is on the raw bytes after the whitespace trim, compared
/// case-insensitively as ASCII - never through the crate's fold, which serves
/// names and code spellings and would match spellings nobody wrote.
pub const DEFAULT_NULL_VALUES: [&str; 3] = ["", "null", "<null>"];

/// One capture read row by row, holding what is constant across them.
///
/// A capture is millions of lines and calling a singular reader per line
/// re-does per message what is constant for the whole run. Pinning a branch
/// and the two versions skips inference for every row - and a capture is one
/// session, so pinning is the normal case rather than an optimization.
pub struct FixReader {
    registry: Arc<FixRegistry>,
    projections: Arc<Projections>,
    branch: Option<FixBranch>,
    source_version: Option<Version>,
    target_version: Option<Version>,
    null_values: Vec<String>,
}

/// A clone is a new reader, so it starts with a cache of its own.
///
/// The cache holds one version's projections, and two readers differing in
/// version - which is why a reader is usually cloned - would otherwise clear
/// each other's every row. Sharing the work across threads is `Arc<FixReader>`
/// instead, which shares the cache as well.
impl Clone for FixReader {
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            projections: Arc::default(),
            branch: self.branch.clone(),
            source_version: self.source_version,
            target_version: self.target_version,
            null_values: self.null_values.clone(),
        }
    }
}

impl FixReader {
    /// Opens a reader over one dictionary.
    #[must_use]
    pub fn new(registry: Arc<FixRegistry>) -> Self {
        Self {
            registry,
            projections: Arc::default(),
            branch: None,
            source_version: None,
            target_version: None,
            null_values: DEFAULT_NULL_VALUES
                .iter()
                .map(|spelling| (*spelling).to_owned())
                .collect(),
        }
    }

    /// Pins the dialect, so no row infers one.
    #[must_use]
    pub fn branch(mut self, branch: &FixBranch) -> Self {
        self.branch = Some(branch.clone());
        self
    }

    /// Pins the version the arriving rows are written in.
    #[must_use]
    pub const fn source_version(mut self, version: Version) -> Self {
        self.source_version = Some(version);
        self
    }

    /// Pins the version the built messages are expressed in.
    #[must_use]
    pub const fn target_version(mut self, version: Version) -> Self {
        self.target_version = Some(version);
        self
    }

    /// Replaces the spellings that mean "nothing was sent".
    ///
    /// Empty keeps every literal, which is what a venue for whom the text
    /// `null` is a value sets: the default is a convention, and a convention
    /// has to be overridable to be safe.
    #[must_use]
    pub fn null_values<I, S>(mut self, spellings: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.null_values = spellings.into_iter().map(Into::into).collect();
        self
    }

    /// Whether one raw value is a stated absence rather than a value.
    fn is_absent(&self, value: &[u8]) -> bool {
        let trimmed = line::trim_ascii(value);
        self.null_values
            .iter()
            .any(|spelling| spelling.as_bytes().eq_ignore_ascii_case(trimmed))
    }

    /// Reads one log line, picking its dialect from the frame.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for input that is not a row at all. A line
    /// that merely held nothing is `Ok` and says so.
    pub fn text(&self, row: &str) -> Result<FixMsg> {
        self.bytes(row.as_bytes())
    }

    /// Reads one log line of bytes, picking its dialect from the frame.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for input that is not a row at all.
    pub fn bytes(&self, row: &[u8]) -> Result<FixMsg> {
        if row.is_empty() {
            return Err(Error::Parse {
                target: "fix",
                position: 0,
                reason: "expected a captured row, got no bytes".into(),
            });
        }
        let start = line::payload_at(row).unwrap_or(row.len());
        let body = &row[start..];
        if !numeric_frame(body) {
            return self.ultext(body);
        }
        match unescaped(body) {
            Some(held) => self.fixtext(&held, 0x01),
            None => self.fixtext(body, separator_of(body)),
        }
    }

    /// Reads one numeric FIX frame split on `separator`.
    ///
    /// A trailing empty segment is tolerated, because a wire message ends
    /// with its separator; a segment with no `=` is dropped, as an empty key
    /// is; duplicate tags stay in arrival order. Every key and value is a
    /// slice of the input and none of it is validated as UTF-8.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a row's content cannot provoke.
    pub fn fixtext(&self, body: &[u8], separator: u8) -> Result<FixMsg> {
        let mut pairs: Vec<(&[u8], &[u8])> = Vec::new();
        for segment in split(body, separator) {
            let Some((key, value)) = split_pair(segment) else {
                continue;
            };
            pairs.push((key, value));
            if key == b"10" {
                // Nothing after the checksum is part of the message.
                break;
            }
        }
        self.build(&pairs)
    }

    /// Reads one bridge row of `NAME=VALUE` pairs.
    ///
    /// A key opening with `#` names a group: `#NOPARTYIDS=1` is the counter
    /// and `#NOPARTYIDS[0]=…` is one occurrence whose *value* is a run of
    /// member pairs. Residue that will not split stays as one unknown key,
    /// verbatim: never dropped, never fatal.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a row's content cannot provoke.
    pub fn ultext(&self, body: &[u8]) -> Result<FixMsg> {
        let separator = if memchr::memchr(b'|', body).is_some() {
            b'|'
        } else {
            b' '
        };
        let mut owned: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for segment in split(body, separator) {
            let Some((key, value)) = split_pair(segment) else {
                continue;
            };
            let bare = key.strip_prefix(b"#").unwrap_or(key);
            match group_index(bare) {
                Some((group, occurrence)) if memchr::memchr(b'=', value).is_some() => {
                    for (member, held) in members(value) {
                        let mut rendered = Vec::with_capacity(group.len() + member.len() + 8);
                        rendered.extend_from_slice(group);
                        rendered.extend_from_slice(b"[");
                        rendered.extend_from_slice(occurrence.to_string().as_bytes());
                        rendered.extend_from_slice(b"].");
                        rendered.extend_from_slice(member);
                        owned.push((rendered, held.to_vec()));
                    }
                }
                _ => owned.push((bare.to_vec(), value.to_vec())),
            }
        }
        let pairs: Vec<(&[u8], &[u8])> = owned
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
            .collect();
        self.build(&pairs)
    }

    /// Builds one message from pairs the caller already split.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal.
    pub fn pairs<'a, I>(&self, pairs: I) -> Result<FixMsg>
    where
        I: IntoIterator<Item = (&'a [u8], &'a [u8])>,
    {
        let held: Vec<(&[u8], &[u8])> = pairs.into_iter().collect();
        self.build(&held)
    }

    /// The one build every reader funnels into.
    fn build(&self, pairs: &[(&[u8], &[u8])]) -> Result<FixMsg> {
        let branch = self
            .branch
            .clone()
            .unwrap_or_else(|| self.infer_branch(pairs));
        let version = self
            .source_version
            .or_else(|| self.infer_version(pairs, &branch));
        let msgtype = msgtype_of(pairs);

        let mut builder = Builder::new(
            &self.registry,
            &self.projections,
            branch.clone(),
            version,
            pairs.len(),
        );
        for (key, value) in pairs {
            // A stated absence produces no field and no entry: the key is read
            // as never having been sent. Filtering happens before typing, so
            // nothing tries to read `<null>` as a price and file the failure.
            if self.is_absent(value) {
                continue;
            }
            builder.push(key, value);
        }
        let (field, value, entries) = builder.finish(root_name(msgtype.as_deref()).as_str())?;
        FixMsg::from_parts(Arc::clone(&self.registry), field, value, entries)
    }

    /// The dialect a row is written in, when the caller pinned none.
    ///
    /// The first step is identity rather than inference: `SenderCompID(49)`
    /// and `TargetCompID(56)` are in every header and a branch declares that
    /// pair, so one lookup answers exactly. Both orders are tried, because a
    /// dictionary declares the session from its own side and an inbound
    /// message carries the pair reversed.
    fn infer_branch(&self, pairs: &[(&[u8], &[u8])]) -> FixBranch {
        let sender = value_of(pairs, b"49");
        let target = value_of(pairs, b"56");
        if let (Some(sender), Some(target)) = (sender, target) {
            let sender = String::from_utf8_lossy(sender);
            let target = String::from_utf8_lossy(target);
            if let Some(branch) = self
                .registry
                .branch_for_session(&sender, &target)
                .or_else(|| self.registry.branch_for_session(&target, &sender))
            {
                return branch.clone();
            }
        }
        FixBranch::STANDARD
    }

    /// The version an arriving row is written in, when the caller pinned none.
    ///
    /// Each step is a FIX rule rather than a heuristic. `ApplVerID(1128)`
    /// wins, because under FIXT.1.1 the session version says nothing about
    /// the application version. `BeginString(8)` follows, and `FIXT.1.1`
    /// falls through rather than being taken literally. Then the branch's own
    /// default, then the dictionary's real newest - never a sentinel.
    fn infer_version(&self, pairs: &[(&[u8], &[u8])], branch: &FixBranch) -> Option<Version> {
        if let Some(value) = value_of(pairs, b"1128") {
            if let Some(version) = appl_ver_id(&String::from_utf8_lossy(value), &self.registry) {
                return Some(version);
            }
        }
        if let Some(value) = value_of(pairs, b"8") {
            let text = String::from_utf8_lossy(value);
            if let Some(rest) = text.strip_prefix("FIX.") {
                if let Ok(version) = rest.parse::<Version>() {
                    return Some(version);
                }
            }
        }
        if branch.version() != Version::MIN {
            return Some(branch.version());
        }
        self.registry.newest().map(|pedigree| pedigree.version())
    }
}

/// `ApplVerID`'s numeric and symbolic spellings.
fn appl_ver_id(value: &str, registry: &FixRegistry) -> Option<Version> {
    let spelling = match value {
        "0" | "FIX27" => "2.7",
        "1" | "FIX30" => "3.0",
        "2" | "FIX40" => "4.0",
        "3" | "FIX41" => "4.1",
        "4" | "FIX42" => "4.2",
        "5" | "FIX43" => "4.3",
        "6" | "FIX44" => "4.4",
        "7" | "FIX50" => "5.0",
        "8" | "FIX50SP1" => "5.0SP1",
        "9" | "FIX50SP2" => "5.0SP2",
        // "FIX Latest" is a moving label and resolves to the pedigree the
        // dictionary actually carries, never to a sentinel.
        "10" | "FIXLatest" => {
            return registry.newest().map(|pedigree| pedigree.version());
        }
        _ => return None,
    };
    spelling.parse().ok()
}

/// The value one tag carries, without building anything.
fn value_of<'a>(pairs: &[(&'a [u8], &'a [u8])], key: &[u8]) -> Option<&'a [u8]> {
    pairs
        .iter()
        .find(|(held, _)| *held == key)
        .map(|(_, value)| *value)
}

/// The message type a row declares, by `35=` or by `MSGTYPE=`.
fn msgtype_of(pairs: &[(&[u8], &[u8])]) -> Option<String> {
    for (key, value) in pairs {
        // The key folds the way every other key folds, so `MSG_TYPE` and
        // `Msg Type` name the type too.
        let folded =
            std::str::from_utf8(key).is_ok_and(|key| crate::types::folds_equal(key, "MsgType"));
        if folded || *key == b"35" {
            return Some(String::from_utf8_lossy(value).into_owned());
        }
    }
    None
}

/// Whether the frame's own first key is all ASCII digits.
fn numeric_frame(body: &[u8]) -> bool {
    let end = memchr::memchr(b'=', body).unwrap_or(0);
    end > 0 && body[..end].iter().all(u8::is_ascii_digit)
}

/// One body with a printed SOH spelling rewritten to the byte it stands for.
///
/// A capture that cannot print `0x01` writes `^A`, `\x01`, `<SOH>` or `{SOH}`
/// instead. It is the same frame; only the separator was escaped on the way
/// into the log, so it is unescaped once here rather than taught to every
/// splitter. `None` where nothing was escaped, so the ordinary path allocates
/// nothing.
fn unescaped(body: &[u8]) -> Option<Vec<u8>> {
    if memchr::memchr(0x01, body).is_some() {
        return None;
    }
    let marker = line::SOH_MARKERS
        .into_iter()
        .filter_map(|held| memchr::memmem::find(body, held).map(|at| (at, held)))
        .min_by_key(|(at, _)| *at)
        .map(|(_, held)| held)?;
    let mut held = Vec::with_capacity(body.len());
    let mut start = 0;
    while let Some(at) = memchr::memmem::find(&body[start..], marker) {
        held.extend_from_slice(&body[start..start + at]);
        held.push(0x01);
        start += at + marker.len();
    }
    held.extend_from_slice(&body[start..]);
    Some(held)
}

/// The separator a numeric frame uses: SOH when the body holds one, else `|`.
fn separator_of(body: &[u8]) -> u8 {
    if memchr::memchr(0x01, body).is_some() {
        0x01
    } else {
        b'|'
    }
}

/// One body split on its separator, empty segments included.
fn split(body: &[u8], separator: u8) -> impl Iterator<Item = &[u8]> {
    body.split(move |byte| *byte == separator)
}

/// One segment split at its **first** `=` only.
///
/// `Text=a;b` is one value with a semicolon, not two fields.
fn split_pair(segment: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = memchr::memchr(b'=', segment)?;
    let key = line::trim_ascii(&segment[..at]);
    if key.is_empty() {
        return None;
    }
    Some((key, line::trim_ascii(&segment[at + 1..])))
}

/// The group and occurrence a `NAME[0]` key addresses.
fn group_index(key: &[u8]) -> Option<(&[u8], usize)> {
    let open = memchr::memchr(b'[', key)?;
    let close = memchr::memchr(b']', &key[open..])? + open;
    let index = std::str::from_utf8(&key[open + 1..close]).ok()?;
    Some((&key[..open], index.parse().ok()?))
}

/// The member pairs packed inside one occurrence's value.
///
/// ULLINK separates them with EOT then ETX, and sometimes omits the separator
/// after the first member while keeping the index. Where the separator is
/// there it is used; where it is not, the run is handed back whole under one
/// key rather than guessed at, because splitting it needs the group's own
/// declared members and a wrong split invents a field.
fn members(value: &[u8]) -> Vec<(&[u8], &[u8])> {
    split_members(value)
        .into_iter()
        .filter_map(split_pair)
        .collect()
}

/// One occurrence's value split on the bridge's member separator.
///
/// The first spelling the run actually carries wins, and only that one splits
/// it: mixing them would let a value that legitimately holds the other byte
/// break into fields nobody wrote.
fn split_members(value: &[u8]) -> Vec<&[u8]> {
    let Some(separator) = MEMBER_SEPARATORS
        .into_iter()
        .find(|held| memchr::memmem::find(value, held).is_some())
    else {
        return vec![value];
    };
    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(at) = memchr::memmem::find(&value[start..], separator) {
        parts.push(&value[start..start + at]);
        start += at + separator.len();
    }
    parts.push(&value[start..]);
    parts
}

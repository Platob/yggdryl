//! ULBridge's own vocabulary, on a dictionary of its own.
//!
//! A [bridge configuration document](crate::MimeType::ULCONFIG) states what a
//! session interface *is* - which venue it talks to, over which host and port,
//! at which sequence numbers, in which state - and FIX publishes almost none
//! of it. What FIX does publish, ULBridge spells under FIX's own names, so the
//! rule is the one this crate already keeps for its own fields: a field the
//! specification already has is never given a second tag.
//!
//! | the document says | reads as |
//! | --- | --- |
//! | `SenderCompID`, `TargetCompID`, `BeginString` | FIX's own tags 49, 56 and 8 |
//! | everything else | this dictionary's own tags, from [`ULBRIDGE_TAG_MIN`] |
//!
//! # Why a branch
//!
//! Because ULBridge is not FIX and is not this crate. The specification
//! publishes no `PrimaryHost`, so putting one on the standard branch would
//! say it does; this crate did not invent it either, so the
//! [crate branch](super::CRATE_BRANCH) is not its home. It is a vendor's
//! dictionary, which is exactly what a [`FixBranch`] is for - and being a
//! branch is also what lets a venue keep its own 20001 without colliding.
//!
//! A reader pins it with [`FixCodec::with_branch`](super::FixCodec), and the
//! names FIX publishes still resolve, because a name is looked for in the
//! message's own branch first and in the standard one after.
//!
//! # One entry, or fifty
//!
//! Jolokia answers a single read with one attribute map and a wildcard read
//! with a map keyed by ObjectName. Both are the same statement made once or
//! many times, so both read as one repeating group: [`SESSIONINTERFACES_TAG`]
//! is a List whose occurrences are the MBeans the document answered for, in
//! canonical ObjectName order because a JSON object has no order of its own.
//! An attribute that is itself an object or an array is retained as the JSON
//! it is: a group inside a group is one level deeper than a key addresses, and
//! text that says what arrived beats a value silently dropped. The four arrays
//! a session interface always carries are declared, so each has a name and a
//! tag; anything else keeps its own folded spelling.
//!
//! ```
//! # fn main() -> yggdryl::Result<()> {
//! let held = yggdryl::fix_ulbridge_fields()?;
//! assert_eq!(held[0].name(), "MBean");
//! // A venue's own 20001 is a different field, because the branch differs.
//! let mine = held[0].as_fix().id()?.expect("an identity");
//! assert_ne!(mine, yggdryl::FixId::standard(20_001));
//! # Ok(())
//! # }
//! ```

use std::sync::LazyLock;

use smol_str::SmolStr;

use crate::{DataType, Field, Result, Scalar};

use super::FixBranch;

/// The dictionary ULBridge's own attributes are defined on.
pub const ULBRIDGE_BRANCH: &str = "ulbridge";

/// The first tag this dictionary claims.
///
/// Well inside FIX's user-defined range and clear of both the 5000s, where
/// venues actually crowd, and the 30000s this crate's own fields sit in.
pub const ULBRIDGE_TAG_MIN: i32 = 20_001;

/// The tag carrying the MBean a request named.
pub const MBEAN_TAG: i32 = 20_001;

/// The tag carrying the Jolokia operation a document asked for.
pub const OPERATION_TAG: i32 = 20_002;

/// The tag carrying the status a Jolokia answer came back with.
pub const STATUS_TAG: i32 = 20_003;

/// The tag carrying what a Jolokia answer failed with.
pub const ERROR_TAG: i32 = 20_004;

/// The tag carrying the session interfaces a document answered for.
///
/// A repeating group, so its own tag is the occurrence counter exactly as a
/// FIX group's is.
pub const SESSIONINTERFACES_TAG: i32 = 20_005;

/// This dictionary, built once.
fn branch() -> Result<FixBranch> {
    FixBranch::from_str(ULBRIDGE_BRANCH)
}

/// The fields, built once and shared.
static FIELDS: LazyLock<Option<Vec<Field>>> = LazyLock::new(|| build().ok());

/// One field on ULBridge's branch, named exactly as the document spells it.
///
/// The canonical name is the attribute's own spelling, because that spelling
/// is what arrives and resolution folds ASCII case once on the way in - so
/// nothing has to translate a document's keys before they resolve.
fn attribute(name: &str, tag: i32, dtype: DataType, description: &str) -> Result<Field> {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_id(&branch()?, tag)?;
    field.set_display(name)?;
    field.set_description(description)?;
    Ok(field)
}

/// Every attribute a bridge configuration document states, in tag order.
///
/// The order is the tag's, not the document's, because a dictionary is
/// ordered by identity and a document by whatever its writer chose.
fn build() -> Result<Vec<Field>> {
    // The occurrences are the MBeans answered for. Declared as a List of the
    // attributes below, which is what a repeating group is - and every member
    // is a field in its own right, so an occurrence's `SenderCompID` is still
    // tag 49 and its `CurrentPort` is still 20027.
    let members: Vec<Field> = [
        (
            "SessionInterface",
            20_010,
            DataType::Utf8,
            "The ObjectName of the MBean this occurrence answers for.",
        ),
        (
            "MBeanType",
            20_011,
            DataType::Utf8,
            "The ObjectName's own type property: what this MBean is.",
        ),
        (
            "PluginType",
            20_012,
            DataType::Utf8,
            "The ObjectName's plugin-type property: the protocol the plugin speaks.",
        ),
        (
            "Name",
            20_013,
            DataType::Utf8,
            "The name the bridge knows this session interface by.",
        ),
        (
            "Category",
            20_014,
            DataType::Utf8,
            "The category the bridge files this session interface under.",
        ),
        (
            "Guid",
            20_015,
            DataType::Utf8,
            "The identifier the bridge holds this session interface under.",
        ),
        (
            "Prefix",
            20_016,
            DataType::Utf8,
            "Prepended to every identifier this session interface issues.",
        ),
        (
            "Suffix",
            20_017,
            DataType::Utf8,
            "Appended to every identifier this session interface issues.",
        ),
        (
            "Comment",
            20_018,
            DataType::Utf8,
            "Whatever an operator wrote about this session interface.",
        ),
        (
            "State",
            20_019,
            DataType::Utf8,
            "What the session is doing now: logged, stopped, and the rest.",
        ),
        (
            "Type",
            20_020,
            DataType::Utf8,
            "Which side of the connection this is: A accepts, I initiates.",
        ),
        (
            "Version",
            20_021,
            DataType::Utf8,
            "The plugin version this session interface runs.",
        ),
        (
            "PrimaryHost",
            20_022,
            DataType::Utf8,
            "The host the session connects to first.",
        ),
        (
            "PrimaryPort",
            20_023,
            DataType::Int64,
            "The port on the primary host.",
        ),
        (
            "BackupHost",
            20_024,
            DataType::Utf8,
            "The host the session falls back to.",
        ),
        (
            "BackupPort",
            20_025,
            DataType::Int64,
            "The port on the backup host; -1 where none is declared.",
        ),
        (
            "CurrentHost",
            20_026,
            DataType::Utf8,
            "The host the session is connected to now.",
        ),
        (
            "CurrentPort",
            20_027,
            DataType::Int64,
            "The port the session is connected on now.",
        ),
        (
            "IncomingMsgSeqNum",
            20_028,
            DataType::Int64,
            "The sequence number the session expects to receive next.",
        ),
        (
            "OutgoingMsgSeqNum",
            20_029,
            DataType::Int64,
            "The sequence number the session will send next.",
        ),
        (
            "LogLevel",
            20_030,
            DataType::Int64,
            "The log level this session interface runs at; -1 inherits.",
        ),
        (
            "PriorityLevel",
            20_031,
            DataType::Int64,
            "The scheduling priority the bridge gives this session.",
        ),
        (
            "LoadIsolation",
            20_032,
            DataType::Int64,
            "Which isolation pool the session's load is placed in.",
        ),
        (
            "NotificationsStatus",
            20_033,
            DataType::Boolean,
            "Whether the session raises notifications.",
        ),
        (
            "NeedReload",
            20_034,
            DataType::Boolean,
            "Whether the configuration on disk is ahead of the running one.",
        ),
        (
            "BinaryName",
            20_035,
            DataType::Utf8,
            "The jar the plugin class was loaded from.",
        ),
        (
            "ClassName",
            20_036,
            DataType::Utf8,
            "The plugin class this session interface runs.",
        ),
        (
            "RevisionInformation",
            20_037,
            DataType::Utf8,
            "The revision string the plugin build carries.",
        ),
        (
            "MinimumBridgeRevision",
            20_038,
            DataType::Utf8,
            "The oldest bridge revision this plugin will run on.",
        ),
        (
            "InitFileContent",
            20_039,
            DataType::Utf8,
            "The session's init file, as the INI text it is.",
        ),
        // The four arrays a session interface carries. Declared as the text
        // they are retained as, so each has a name and a tag to be addressed
        // by rather than a folded spelling nobody wrote down.
        (
            "ExtendedActions",
            20_040,
            DataType::Utf8,
            "The actions this session interface offers, as the JSON array it is.",
        ),
        (
            "Enrichments",
            20_041,
            DataType::Utf8,
            "The enrichment chain attached to this session interface, as the JSON array it is.",
        ),
        (
            "ClassHierarchy",
            20_042,
            DataType::Utf8,
            "The plugin's class hierarchy and revisions, as the JSON array it is.",
        ),
        (
            "Resources",
            20_043,
            DataType::Utf8,
            "The extensions this session interface declares, as the JSON array it is.",
        ),
    ]
    .into_iter()
    .map(|(name, tag, dtype, description)| attribute(name, tag, dtype, description))
    .collect::<Result<_>>()?;

    let occurrence = DataType::from_fields(members.clone())?.required_field("SessionInterface");
    let mut fields = vec![
        attribute(
            "MBean",
            MBEAN_TAG,
            DataType::Utf8,
            "The MBean the request named, which is a pattern where it named many.",
        )?,
        attribute(
            "Operation",
            OPERATION_TAG,
            DataType::Utf8,
            "The Jolokia operation the document asked for: read, write, exec, list, search.",
        )?,
        attribute(
            "Status",
            STATUS_TAG,
            DataType::Int64,
            "The status the answer came back with; absent on a request.",
        )?,
        attribute(
            "Error",
            ERROR_TAG,
            DataType::Utf8,
            "What the answer failed with, where it failed.",
        )?,
        attribute(
            "SessionInterfaces",
            SESSIONINTERFACES_TAG,
            DataType::list(occurrence),
            "The MBeans this document answered for, one occurrence each.",
        )?,
    ];
    fields.extend(members);
    Ok(fields)
}

/// The fields ULBridge's dictionary defines, in tag order.
///
/// Registering them is a caller's choice rather than a load-time side effect,
/// exactly as it is for [this crate's own](super::fix_crate_fields): a
/// dictionary read from a store is what that store held.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the branch or one of the
/// datatypes does not build, which is a defect in this module rather than
/// anything a caller did.
pub fn fix_ulbridge_fields() -> Result<&'static [Field]> {
    FIELDS
        .as_deref()
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: ULBRIDGE_BRANCH.into(),
            reason: crate::text::expected_got("ULBridge's own fields", "a build failure"),
        })
}

impl super::FixRegistry {
    /// Adds ULBridge's own fields, so a bridge configuration document types.
    ///
    /// A dictionary that has them reads a document's attributes as the ports,
    /// sequence numbers and flags they are; one that does not reads them as
    /// the text they arrived as, because a key no dictionary explains is kept
    /// rather than dropped.
    ///
    /// # Errors
    ///
    /// Returns the registry's own refusal when a field collides with something
    /// already held, which cannot happen on a dictionary that does not already
    /// declare ULBridge's branch.
    pub fn with_ulbridge_fields(mut self) -> Result<Self> {
        for field in fix_ulbridge_fields()? {
            self.insert(field.clone())?;
        }
        Ok(self)
    }
}

/// One bridge configuration document, flattened into the pairs every reader
/// hands the builder.
///
/// The document is JSON, so it is read by the crate's own JSON reader into one
/// [`Scalar`] and walked - there is no second parser here and no second schema.
/// What the walk states is the whole of the mapping:
///
/// | the document | the pairs |
/// | --- | --- |
/// | the `request` it echoes, or the request itself | `MBean`, `Operation` |
/// | `status`, `error` | `Status`, `Error` |
/// | each MBean the `value` answers for | one `SessionInterfaces[i]` occurrence |
/// | that entry's ObjectName | `SessionInterface`, `MBeanType`, `PluginType` |
/// | each of its attributes | that attribute's own name |
///
/// An attribute that is itself an object or an array is retained as the JSON
/// it is, under the name it arrived under: a group inside a group is one level
/// deeper than a key addresses, and text that says what arrived beats a value
/// silently dropped.
///
/// A bulk answer is an array of these, and reads as the first response in it,
/// because one line is one message and a document declares one exchange.
fn ulconfig_pairs(document: &Scalar) -> Vec<(Vec<u8>, Vec<u8>)> {
    let document = match document.as_sequence() {
        Some(bulk) => match bulk.first() {
            Some(first) => first,
            None => return Vec::new(),
        },
        None => document,
    };
    let Some(root) = document.as_record() else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    // A request is echoed inside an answer and stands alone in a request, so
    // the two shapes are one reading with one fallback.
    let request = root
        .get("request")
        .and_then(Scalar::as_record)
        .unwrap_or(root);
    push_leaf(&mut pairs, b"MBean", request.get("mbean"));
    push_leaf(&mut pairs, b"Operation", request.get("type"));
    push_leaf(&mut pairs, b"Status", root.get("status"));
    push_leaf(&mut pairs, b"Error", root.get("error"));

    let Some(value) = root.get("value") else {
        return pairs;
    };
    for (occurrence, (name, attributes)) in entries(value, request.get("mbean")).enumerate() {
        push_occurrence(&mut pairs, occurrence, name.as_deref(), attributes);
    }
    pairs
}

/// The MBeans one `value` answers for, and the attributes of each.
///
/// A wildcard read answers a map keyed by ObjectName and a single read answers
/// one attribute map, so the key that names an MBean is what separates them -
/// and where none does, the request's own MBean names the one entry.
fn entries<'value>(
    value: &'value Scalar,
    named: Option<&'value Scalar>,
) -> impl Iterator<Item = (Option<SmolStr>, &'value Scalar)> {
    let keyed: Vec<(Option<SmolStr>, &Scalar)> = value
        .as_record()
        .map(|held| {
            held.iter()
                .filter(|(key, _)| {
                    crate::mime_type::line::object_names(key.as_bytes())
                        .next()
                        .is_some()
                })
                .map(|(key, held)| (Some(key.clone()), held))
                .collect()
        })
        .unwrap_or_default();
    if keyed.is_empty() {
        let named = named.and_then(Scalar::as_str).map(SmolStr::new);
        return vec![(named, value)].into_iter();
    }
    keyed.into_iter()
}

/// One MBean's ObjectName and attributes, as that occurrence's pairs.
fn push_occurrence(
    pairs: &mut Vec<(Vec<u8>, Vec<u8>)>,
    occurrence: usize,
    name: Option<&str>,
    attributes: &Scalar,
) {
    let mut push = |member: &[u8], value: Vec<u8>| {
        let mut key = Vec::with_capacity(member.len() + 24);
        key.extend_from_slice(b"SessionInterfaces[");
        key.extend_from_slice(occurrence.to_string().as_bytes());
        key.extend_from_slice(b"].");
        key.extend_from_slice(member);
        pairs.push((key, value));
    };
    if let Some(name) = name {
        push(b"SessionInterface", name.as_bytes().to_vec());
        // The ObjectName's own properties, read where the classifier reads
        // them so one spelling answers for both.
        for (member, property) in [
            (
                b"MBeanType".as_slice(),
                crate::mime_type::line::OBJECT_NAME_TYPE,
            ),
            (b"PluginType".as_slice(), b"plugin-type".as_slice()),
        ] {
            if let Some(held) =
                crate::mime_type::line::object_name_property(name.as_bytes(), property)
            {
                push(member, held.to_vec());
            }
        }
    }
    let Some(attributes) = attributes.as_record() else {
        return;
    };
    for (attribute, value) in attributes {
        if let Some(rendered) = rendered(value) {
            push(attribute.as_bytes(), rendered);
        }
    }
}

/// One leaf of the envelope, where the document stated it.
fn push_leaf(pairs: &mut Vec<(Vec<u8>, Vec<u8>)>, key: &[u8], value: Option<&Scalar>) {
    if let Some(rendered) = value.and_then(rendered) {
        pairs.push((key.to_vec(), rendered));
    }
}

/// What one JSON value contributes, or nothing where it states nothing.
///
/// Text is its own bytes; every other leaf is the JSON it is, which is also
/// what an object or an array becomes - the builder types a leaf and keeps
/// what it cannot type, and a rendered subtree is the honest thing to keep.
fn rendered(value: &Scalar) -> Option<Vec<u8>> {
    if value.is_null() {
        return None;
    }
    match value.as_str() {
        Some(text) => (!text.is_empty()).then(|| text.as_bytes().to_vec()),
        None => crate::into_json_scalar(value).ok().map(String::into_bytes),
    }
}

impl super::FixCodec {
    /// Reads one ULBridge configuration document.
    ///
    /// The document is the payload [`MimeType::ULCONFIG`](crate::MimeType)
    /// names, and this is the reader for it beside the one for a numeric
    /// frame, a bridge row and a FIXML row: it produces the same key/value
    /// pairs they do and hands them to the same builder, so what a dictionary
    /// types here is typed by the rules that type everything else.
    ///
    /// Pin [`ULBRIDGE_BRANCH`] to type a document's own attributes; FIX's own
    /// names resolve either way.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error) naming the byte position when
    /// the document is not JSON, and the builder's refusal otherwise.
    pub fn transform_ulconfig_line(&self, body: &[u8], enrich: bool) -> Result<super::FixMsg> {
        let document = crate::from_json_scalar(body)?;
        let owned = ulconfig_pairs(&document);
        let pairs: Vec<(&[u8], &[u8])> = owned
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
            .collect();
        self.build_pairs(&pairs, enrich)
    }
}

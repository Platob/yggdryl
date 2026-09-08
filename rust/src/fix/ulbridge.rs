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
//! # One plugin at a time
//!
//! A row is one exchange and a wildcard read answers fifty plugins in one, so
//! [`UlPlugin`] is one of those answers on its own: the ObjectName the bridge
//! holds it under, beside the attributes it stated. It reads out of the bytes
//! a line carries, out of a parsed document, or out of a typed message, and
//! crosses back to one - which is what a reader walking a hundred plugins
//! wants before it types any of them.
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

/// The name the session-interface group carries.
const SESSIONINTERFACES_NAME: &str = "SessionInterfaces";

/// The name one occurrence's own ObjectName member carries.
const SESSIONINTERFACE_NAME: &str = "SessionInterface";

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
            SESSIONINTERFACE_NAME,
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
        // What a plugin says about the configuration it was built from. A
        // bridge states these beside the session's own facts, and a reader
        // asking which CBlock a session runs asks here.
        (
            "NeedCFBReload",
            20_044,
            DataType::Boolean,
            "Whether the session's CBlock has changed under it since it loaded.",
        ),
        (
            "CFBInfos",
            20_045,
            DataType::Utf8,
            "The CBlocks this session interface loaded and their revisions, as the JSON array it is.",
        ),
        (
            "targetProducts",
            20_046,
            DataType::Utf8,
            "The products this plugin is built for, as the JSON array it is.",
        ),
        (
            "cm-extension",
            20_047,
            DataType::Utf8,
            "The configuration-manager extension version this plugin declares.",
        ),
    ]
    .into_iter()
    .map(|(name, tag, dtype, description)| attribute(name, tag, dtype, description))
    .collect::<Result<_>>()?;

    let occurrence = DataType::from_fields(members.clone())?.required_field(SESSIONINTERFACE_NAME);
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
            SESSIONINTERFACES_NAME,
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
        key.extend_from_slice(SESSIONINTERFACES_NAME.as_bytes());
        key.push(b'[');
        key.extend_from_slice(occurrence.to_string().as_bytes());
        key.extend_from_slice(b"].");
        key.extend_from_slice(member);
        pairs.push((key, value));
    };
    if let Some(name) = name {
        push(SESSIONINTERFACE_NAME.as_bytes(), name.as_bytes().to_vec());
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

/// One plugin a bridge configuration document answers for.
///
/// A Jolokia read answers one MBean's attributes or a map of them keyed by
/// ObjectName, and both are the same statement made once or many times. This
/// is one of those statements: the ObjectName the bridge holds the plugin
/// under, beside the attributes it stated, exactly as the document wrote
/// them. [`FixMsg`](super::FixMsg) is the same facts typed against a
/// dictionary - [`Self::into_fixmsg`] crosses to it and [`Self::from_fixmsg`]
/// crosses back - and this is what a reader walking a hundred plugins holds
/// before it types any of them.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// use yggdryl::UlPlugin;
///
/// // A log line: prose in front of the document, prose behind it.
/// let line = br#"12:00:00 [Jolokia] Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=OrderRouting,plugin-type=FIX,type=Plugin":{"Name":"OrderRouting","Version":"4.7.0","State":"logged"}},"status":200} (12 ms)"#;
///
/// let held: Vec<UlPlugin> = UlPlugin::from_json_bytes(line)?.collect();
/// assert_eq!(held.len(), 1);
/// assert_eq!(held[0].name(), Some("OrderRouting"));
/// assert_eq!(held[0].version(), Some("4.7.0"));
/// assert_eq!(held[0].plugin_type(), Some("FIX"));
/// assert_eq!(held[0].mbean_type(), Some("Plugin"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UlPlugin {
    /// The ObjectName the document answered under, where it named one.
    ///
    /// A single read states its MBean in the request rather than beside the
    /// attributes, and a document that states neither answers a plugin whose
    /// name is whatever its `Name` attribute says.
    mbean: Option<SmolStr>,
    /// The attributes as the document stated them, sorted by name.
    attributes: Scalar,
}

impl UlPlugin {
    /// Every plugin one document answers for, from the bytes a line carries.
    ///
    /// The document is found inside the line the way the classifier finds it:
    /// a transport writes a timestamp in front of one and sometimes a duration
    /// behind it, and both are prose. Bytes that name no MBean are read whole,
    /// because a caller handing the document straight in is handing the
    /// document.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`](crate::Error) naming the byte position when
    /// what is there is not JSON.
    pub fn from_json_bytes(body: &[u8]) -> Result<UlPlugins> {
        let document = crate::from_json_scalar(
            crate::mime_type::line::ulconfig_span(body).map_or(body, |span| &body[span]),
        )?;
        Ok(Self::from_json_scalar(&document))
    }

    /// Every plugin one parsed document answers for, in the order it answered.
    ///
    /// The envelope is optional: a `value` under a Jolokia answer, an array of
    /// those answers for a bulk read, or a bare attribute map a caller pulled
    /// out itself. A document that answers nothing answers no plugins rather
    /// than a refusal - a Jolokia error is a document too.
    #[must_use]
    pub fn from_json_scalar(document: &Scalar) -> UlPlugins {
        let mut held = Vec::new();
        for answer in document.as_sequence().map_or_else(
            || vec![document],
            |bulk| bulk.iter().collect::<Vec<&Scalar>>(),
        ) {
            let Some(root) = answer.as_record() else {
                continue;
            };
            let request = root
                .get("request")
                .and_then(Scalar::as_record)
                .unwrap_or(root);
            let value = root.get("value").unwrap_or(answer);
            for (mbean, attributes) in entries(value, request.get("mbean")) {
                held.push(Self {
                    mbean,
                    attributes: attributes.clone(),
                });
            }
        }
        UlPlugins {
            held: held.into_iter(),
        }
    }

    /// Every plugin one typed message carries.
    ///
    /// The reverse of [`Self::into_fixmsg`], over the occurrences of the
    /// [`SESSIONINTERFACES_TAG`] group: each occurrence is one plugin, its
    /// `SessionInterface` member is the ObjectName, and every other member is
    /// an attribute under the name the dictionary gave it. The two properties
    /// the ObjectName itself states - `MBeanType` and `PluginType` - are read
    /// back off the name rather than kept twice.
    #[must_use]
    pub fn from_fixmsg(message: &super::FixMsg) -> UlPlugins {
        let mut held = Vec::new();
        let occurrences = message
            .get_by_tag(SESSIONINTERFACES_TAG)
            .and_then(Scalar::as_sequence);
        // The member names are the item's, in declaration order, which is the
        // order the values arrive in: a row is an ordered sequence.
        let item = message
            .as_field()
            .dtype()
            .get_field_by_path(SESSIONINTERFACES_NAME)
            .and_then(|group| group.dtype().get_field_at(0));
        if let (Some(occurrences), Some(item)) = (occurrences, item) {
            let names: Vec<&str> = item
                .dtype()
                .as_fields()
                .map(|fields| fields.iter().map(Field::name).collect())
                .unwrap_or_default();
            for occurrence in occurrences {
                let Some(values) = occurrence.as_sequence() else {
                    continue;
                };
                let mut mbean = None;
                let mut attributes: Vec<(SmolStr, Scalar)> = Vec::new();
                for (name, value) in names.iter().zip(values) {
                    if value.is_null() {
                        continue;
                    }
                    match *name {
                        SESSIONINTERFACE_NAME => {
                            mbean = value.as_str().map(SmolStr::new);
                        }
                        // Both are the ObjectName's own properties, and the
                        // name is kept: storing them twice would give one fact
                        // two owners.
                        "MBeanType" | "PluginType" => {}
                        held => attributes.push((SmolStr::new(held), value.clone())),
                    }
                }
                // A duplicate member name cannot happen: the item's names are
                // a Struct's, which the schema grammar already made unique.
                let Ok(attributes) = Scalar::from_record(attributes) else {
                    continue;
                };
                held.push(Self { mbean, attributes });
            }
        }
        UlPlugins {
            held: held.into_iter(),
        }
    }

    /// This plugin as a message typed against `codec`'s dictionary.
    ///
    /// The same build every other reader funnels into, over the pairs this
    /// plugin states: one occurrence of the session-interface group, and the
    /// MBean the document answered under. Nothing is rendered twice - an
    /// attribute that is an object or an array crosses as the JSON it is,
    /// exactly as it does when the line itself is read.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal.
    pub fn into_fixmsg(&self, codec: &super::FixCodec, enrich: bool) -> Result<super::FixMsg> {
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let named = self.mbean.as_deref().map(Scalar::from);
        push_leaf(&mut pairs, b"MBean", named.as_ref());
        push_occurrence(&mut pairs, 0, self.mbean.as_deref(), &self.attributes);
        let borrowed: Vec<(&[u8], &[u8])> = pairs
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
            .collect();
        codec.transform_pairs(borrowed, enrich)
    }

    /// The ObjectName the bridge holds this plugin under.
    #[must_use]
    pub fn mbean(&self) -> Option<&str> {
        self.mbean.as_deref()
    }

    /// What the ObjectName says this MBean is: `Plugin`, `ConfigurationPlugin`.
    #[must_use]
    pub fn mbean_type(&self) -> Option<&str> {
        self.property(crate::mime_type::line::OBJECT_NAME_TYPE)
    }

    /// The protocol the ObjectName says this plugin speaks.
    #[must_use]
    pub fn plugin_type(&self) -> Option<&str> {
        self.property(b"plugin-type")
    }

    /// The name the bridge knows this plugin by.
    ///
    /// The attribute where the document stated one, and the ObjectName's own
    /// `name` property where it did not: a wildcard read names every plugin in
    /// the key it answers under.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.text("Name").or_else(|| self.property(b"name"))
    }

    /// The plugin version this session interface runs.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.text("Version")
    }

    /// The category the bridge files this plugin under.
    #[must_use]
    pub fn category(&self) -> Option<&str> {
        self.text("Category")
    }

    /// What the session is doing now, where the document says.
    #[must_use]
    pub fn state(&self) -> Option<&str> {
        self.text("State")
    }

    /// One attribute as the document stated it.
    ///
    /// The spelling is the document's own, folded the way every other name in
    /// this crate is, so `PrimaryHost`, `primaryhost` and `primary_host` are
    /// one attribute.
    #[must_use]
    pub fn get(&self, attribute: &str) -> Option<&Scalar> {
        let held = self.attributes.as_record()?;
        held.get(attribute).or_else(|| {
            held.iter()
                .find(|(name, _)| crate::types::folds_equal(name, attribute))
                .map(|(_, value)| value)
        })
    }

    /// Every attribute this plugin states, by name.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, &Scalar)> {
        self.attributes
            .as_record()
            .into_iter()
            .flat_map(|held| held.iter().map(|(name, value)| (name.as_str(), value)))
    }

    /// One attribute as text, where it is text.
    fn text(&self, attribute: &str) -> Option<&str> {
        self.get(attribute)?.as_str()
    }

    /// One property of the ObjectName, where the document named one.
    fn property(&self, property: &[u8]) -> Option<&str> {
        let mbean = self.mbean.as_deref()?;
        let held = crate::mime_type::line::object_name_property(mbean.as_bytes(), property)?;
        std::str::from_utf8(held).ok()
    }
}

/// Every plugin one document answers for, in the order it answered.
#[derive(Clone, Debug)]
pub struct UlPlugins {
    held: std::vec::IntoIter<UlPlugin>,
}

impl Iterator for UlPlugins {
    type Item = UlPlugin;

    fn next(&mut self) -> Option<Self::Item> {
        self.held.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.held.size_hint()
    }
}

impl DoubleEndedIterator for UlPlugins {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.held.next_back()
    }
}

impl ExactSizeIterator for UlPlugins {}

impl std::iter::FusedIterator for UlPlugins {}

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
        // The document as the line carries it: a transport writes a timestamp
        // in front of one and sometimes a duration behind it, and the reader
        // that classified the line already knows where both stop. A body that
        // names no MBean is read whole, because a caller handing one straight
        // in is handing the document itself.
        let document = crate::from_json_scalar(
            crate::mime_type::line::ulconfig_span(body).map_or(body, |span| &body[span]),
        )?;
        let owned = ulconfig_pairs(&document);
        let pairs: Vec<(&[u8], &[u8])> = owned
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
            .collect();
        self.build_pairs(&pairs, enrich)
    }
}

//! A plugin as the bridge hosting it reports it, and the dictionary that
//! types one.
//!
//! A plugin is a FIX session endpoint: which venue it talks to, over which
//! host and port, at which sequence numbers, in which state. That is a fact
//! about a FIX session and not about whoever reports it, so the reading here
//! is generic over the bridge (decision 18) - ULBridge's Jolokia answer is
//! one producer of such a report, and its document is what
//! [`Plugin::from_json_bytes`] reads. FIX publishes almost none of these
//! fields; what it does publish, a report spells under FIX's own names, so
//! the rule is the one this crate already keeps for its own fields: a field
//! the specification already has is never given a second tag.
//!
//! | the report says | reads as |
//! | --- | --- |
//! | `SenderCompID`, `TargetCompID`, `BeginString` | FIX's own tags 49, 56 and 8 |
//! | everything else | this dictionary's own tags, from [`PLUGIN_TAG_MIN`] |
//!
//! # Why a dictionary of its own
//!
//! Because a plugin is neither FIX nor this crate. The specification
//! publishes no `PrimaryHost`, so a field claiming to be FIX's would say it
//! does; this crate did not invent it either, so the crate's own tags above
//! [`CRATE_TAG_MIN`](super::CRATE_TAG_MIN) are not its home. It is a
//! dictionary of its own, so every field here is stamped as a member of
//! [`PLUGIN_DIALECT`] in its `fix:branches`, and a venue keeps its own
//! 20010 beside it under its own name: a field is its tag and its name, and
//! a name that differs is a different field.
//!
//! A reader registers them with
//! [`FixRegistry::with_plugin_fields`](super::FixRegistry::with_plugin_fields),
//! and the names FIX publishes still resolve, because the registry is one
//! namespace.
//!
//! # One configuration per message
//!
//! A single read yields one [`Plugin`]. A wildcard or bulk answer yields
//! lazy [`Plugins`], retaining the shared source document and one key cursor.
//! Each configuration converts to one flat [`FixMsg`](super::FixMsg):
//! `SessionInterface` the ObjectName the read answered for, and the
//! attributes ordinary scalar fields. What the Jolokia exchange wrapped them
//! in is the transport's and no part of the configuration, so nothing states
//! it (decision 17). No synthetic collection or count field is introduced.
//!
//! ```
//! # fn main() -> yggdryl::Result<()> {
//! let held = yggdryl::fix_plugin_fields()?;
//! assert_eq!(held[0].name(), "SessionInterface");
//! // A venue's own 20010 under another name is a different field.
//! let mine = held[0].as_fix().id()?.expect("an identity");
//! assert_eq!(mine, yggdryl::FixId::of(20_010, "sessioninterface")?);
//! assert_ne!(mine, yggdryl::FixId::of(20_010, "VenueOwnThing")?);
//! assert!(held[0].as_fix().has_branch("plugin"));
//! // 20001 is the floor of the range this dictionary claims, not a tag it
//! // defines: 20001 to 20004 held the envelope and are retired rather than
//! // reused, so a capture that holds `MBean` on 20001 keeps its meaning.
//! assert!(held.iter().all(|field| field.name() != "MBean"));
//! # Ok(())
//! # }
//! ```

use std::sync::{Arc, LazyLock};

use smol_str::SmolStr;

use super::build::RowStamp;

use crate::{DataType, Field, Result, Scalar};

/// The dictionary a plugin's own attributes are members of.
pub const PLUGIN_DIALECT: &str = "plugin";

/// The first tag this dictionary claims.
///
/// Well inside FIX's user-defined range and clear of both the 5000s, where
/// venues actually crowd, and the 30000s this crate's own fields sit in.
///
/// The floor of the range rather than the smallest tag defined in it:
/// 20001 to 20004 carried the Jolokia envelope, which decision 17 deleted,
/// and they are retired rather than reused - a capture written before it
/// holds `MBean` on 20001, and a dictionary giving 20001 to something else
/// would read that column as the new field.
pub const PLUGIN_TAG_MIN: i32 = 20_001;

/// The name the actual returned ObjectName member carries.
const SESSIONINTERFACE_NAME: &str = "SessionInterface";

/// The document attributes whose spelling names a field of this crate's own,
/// beside the name the dictionary holds them under.
///
/// A registry is one namespace, and every registry holds the crate's `state`
/// (the order's, read out of `OrdStatus`) and its `version` (the FIX version
/// a row was read at). A bridge document spells a plugin's own state and its
/// own version with the same two words, and they are not the same facts, so
/// the dictionary calls them `PluginState` and `PluginVersion` and the
/// document's spelling is translated at the one boundary a document crosses,
/// in both directions: the row fills under the dictionary's name, and the
/// arrival record keeps the document's spelling, exactly as a line keeps
/// what it wrote. Every other attribute is named as the document spells it.
const ATTRIBUTE_SPELLINGS: [(&str, &str); 2] =
    [("State", "PluginState"), ("Version", "PluginVersion")];

/// The dictionary name one document attribute is held under, where it is not
/// the attribute's own spelling.
fn attribute_name(spelling: &str) -> Option<&'static [u8]> {
    ATTRIBUTE_SPELLINGS
        .iter()
        .find(|(document, _)| crate::types::folds_equal(document, spelling))
        .map(|(_, held)| held.as_bytes())
}

/// The document spelling of one dictionary field of the bridge's.
fn attribute_spelling(name: &str) -> &str {
    ATTRIBUTE_SPELLINGS
        .iter()
        .find(|(_, held)| crate::types::folds_equal(held, name))
        .map_or(name, |(document, _)| document)
}

/// The fields, built once and shared.
static FIELDS: LazyLock<Option<Vec<Field>>> = LazyLock::new(|| build().ok());

/// One field of the plugin dictionary, named as the report spells it.
///
/// The canonical name is the attribute's own spelling, because that spelling
/// is what arrives and resolution folds ASCII case once on the way in - so
/// nothing has to translate a document's keys before they resolve - except
/// the two [`ATTRIBUTE_SPELLINGS`] translates, whose spelling is the crate's
/// own.
fn attribute(name: &str, tag: i32, dtype: DataType, description: &str) -> Result<Field> {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_tag(tag)?;
    field.as_fix_mut().set_branches([PLUGIN_DIALECT])?;
    field.set_display(name)?;
    field.set_description(description)?;
    Ok(field)
}

/// Every attribute a bridge configuration document states, in tag order.
///
/// The order is the tag's, not the document's, because a dictionary is
/// ordered by identity and a document by whatever its writer chose.
fn build() -> Result<Vec<Field>> {
    // A returned MBean is one flat message. FIX's own SenderCompID remains
    // tag 49; the bridge's CurrentPort remains tag 20027.
    let members: Vec<Field> = [
        (
            SESSIONINTERFACE_NAME,
            20_010,
            DataType::utf8(),
            "The ObjectName of the MBean this occurrence answers for.",
        ),
        (
            "MBeanType",
            20_011,
            DataType::utf8(),
            "The ObjectName's own type property: what this MBean is.",
        ),
        (
            "PluginType",
            20_012,
            DataType::utf8(),
            "The ObjectName's plugin-type property: the protocol the plugin speaks.",
        ),
        (
            "Name",
            20_013,
            DataType::utf8(),
            "The name the bridge knows this session interface by.",
        ),
        (
            "Category",
            20_014,
            DataType::utf8(),
            "The category the bridge files this session interface under.",
        ),
        (
            "Guid",
            20_015,
            DataType::utf8(),
            "The identifier the bridge holds this session interface under.",
        ),
        (
            "Prefix",
            20_016,
            DataType::utf8(),
            "Prepended to every identifier this session interface issues.",
        ),
        (
            "Suffix",
            20_017,
            DataType::utf8(),
            "Appended to every identifier this session interface issues.",
        ),
        (
            "Comment",
            20_018,
            DataType::utf8(),
            "Whatever an operator wrote about this session interface.",
        ),
        (
            "PluginState",
            20_019,
            DataType::utf8(),
            "What the session is doing now: logged, stopped, and the rest; \
             the document spells it State.",
        ),
        (
            "Type",
            20_020,
            DataType::utf8(),
            "Which side of the connection this is: A accepts, I initiates.",
        ),
        (
            "PluginVersion",
            20_021,
            DataType::utf8(),
            "The plugin version this session interface runs; the document \
             spells it Version.",
        ),
        (
            "PrimaryHost",
            20_022,
            DataType::utf8(),
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
            DataType::utf8(),
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
            DataType::utf8(),
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
            DataType::utf8(),
            "The jar the plugin class was loaded from.",
        ),
        (
            "ClassName",
            20_036,
            DataType::utf8(),
            "The plugin class this session interface runs.",
        ),
        (
            "RevisionInformation",
            20_037,
            DataType::utf8(),
            "The revision string the plugin build carries.",
        ),
        (
            "MinimumBridgeRevision",
            20_038,
            DataType::utf8(),
            "The oldest bridge revision this plugin will run on.",
        ),
        (
            "InitFileContent",
            20_039,
            DataType::utf8(),
            "The session's init file, as the INI text it is.",
        ),
        // The four arrays a session interface carries. Declared as the text
        // they are retained as, so each has a name and a tag to be addressed
        // by rather than a folded spelling nobody wrote down.
        (
            "ExtendedActions",
            20_040,
            DataType::utf8(),
            "The actions this session interface offers, as the JSON array it is.",
        ),
        (
            "Enrichments",
            20_041,
            DataType::utf8(),
            "The enrichment chain attached to this session interface, as the JSON array it is.",
        ),
        (
            "ClassHierarchy",
            20_042,
            DataType::utf8(),
            "The plugin's class hierarchy and revisions, as the JSON array it is.",
        ),
        (
            "Resources",
            20_043,
            DataType::utf8(),
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
            DataType::utf8(),
            "The CBlocks this session interface loaded and their revisions, as the JSON array it is.",
        ),
        (
            "targetProducts",
            20_046,
            DataType::utf8(),
            "The products this plugin is built for, as the JSON array it is.",
        ),
        (
            "cm-extension",
            20_047,
            DataType::utf8(),
            "The configuration-manager extension version this plugin declares.",
        ),
    ]
    .into_iter()
    .map(|(name, tag, dtype, description)| attribute(name, tag, dtype, description))
    .collect::<Result<_>>()?;

    Ok(members)
}

/// The scalar fields the plugin dictionary defines, in tag order.
///
/// Registering them is a caller's choice rather than a load-time side effect,
/// exactly as it is for [this crate's own](super::fix_crate_fields): a
/// dictionary read from a store is what that store held.
///
/// # Errors
///
/// Returns the schema grammar's refusal when one of the datatypes does not
/// build, which is a defect in this module rather than anything a caller
/// did.
pub fn fix_plugin_fields() -> Result<&'static [Field]> {
    FIELDS
        .as_deref()
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: PLUGIN_DIALECT.into(),
            reason: crate::text::expected_got(
                "the plugin dictionary's own fields",
                "a build failure",
            ),
        })
}

/// The message type a plugin configuration is, built once.
static MESSAGE: std::sync::LazyLock<Option<Field>> = std::sync::LazyLock::new(|| message().ok());

/// The code FIX leaves to a user, and this crate spends on a configuration.
///
/// The specification reserves every type opening with `U` for messages it
/// does not define, so a code here collides with no dictionary's own and
/// needs no dictionary edited to be read (decision 19).
pub const PLUGINCONFIG_CODE_NAME: (&str, &str) = ("UCFG", "pluginconfig");

/// The `pluginconfig` component: what a configuration message is made of.
///
/// The plugin attributes beside the three fields FIX publishes that a
/// configuration also states, which is the whole of it. The members are held
/// by value, so registering the component states the shape of a `UCFG`
/// message without registering its attributes as dictionary fields - two
/// different questions, and [`FixRegistry::with_plugin_fields`] is the
/// answer to the other one.
fn message() -> Result<Field> {
    let mut members: Vec<Field> = Vec::new();
    // A message states its type, so the component carries the field that
    // holds it: what a `UCFG` message is, is part of the message.
    members.push(
        msgtype_field()
            .cloned()
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: PLUGIN_DIALECT.into(),
                reason: crate::text::expected_got("FIX's own MsgType", "a build failure"),
            })?,
    );
    members.extend(fix_plugin_fields()?.iter().cloned());
    for (name, tag) in [
        ("BeginString", 8),
        ("SenderCompID", 49),
        ("TargetCompID", 56),
    ] {
        let mut field = DataType::utf8().nullable_field(name);
        field.as_fix_mut().set_tag(tag)?;
        members.push(field);
    }
    let mut message = DataType::from_fields(members)?.required_field(PLUGINCONFIG_CODE_NAME.1);
    message.as_fix_mut().set_msgtype(PLUGINCONFIG_CODE_NAME.0)?;
    Ok(message)
}

/// FIX's own `MsgType`, built here so a configuration can carry one.
///
/// A message states its type, and a registry that holds only the crate's own
/// fields names no tag 35 to hang it on. The code the crate supplies is the
/// crate's, so the field it lands in is the crate's too rather than
/// something a dictionary has to publish first (decision 19).
static MSGTYPE_FIELD: std::sync::LazyLock<Option<Field>> = std::sync::LazyLock::new(|| {
    let mut field = DataType::utf8().nullable_field(super::MSGTYPE_TAG_NAME.1);
    field.as_fix_mut().set_tag(super::MSGTYPE_TAG_NAME.0).ok()?;
    Some(field)
});

/// The field a supplied message code is built into.
pub(super) fn msgtype_field() -> Option<&'static Field> {
    MSGTYPE_FIELD.as_ref()
}

/// The `pluginconfig` message component, for a registry to hold.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the component does not build,
/// which is a defect in this module rather than anything a caller did.
pub fn fix_plugin_message() -> Result<&'static Field> {
    MESSAGE.as_ref().ok_or_else(|| crate::Error::InvalidRecord {
        path: PLUGIN_DIALECT.into(),
        reason: crate::text::expected_got("the plugin message component", "a build failure"),
    })
}

impl super::FixRegistry {
    /// Adds the plugin dictionary's scalar fields, one configuration per message.
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
    /// hold a field under one of these tags and names.
    pub fn with_plugin_fields(mut self) -> Result<Self> {
        self.add_fields(fix_plugin_fields()?.iter().cloned())?;
        // The message type is registered beside the fields, where it belongs
        // - and by `FixRegistry::new` too, so a registry that never came
        // through here still has it (decision 19). Whichever ran first, the
        // component is already there and this is not a second one.
        if self.get_msgtype(PLUGINCONFIG_CODE_NAME.0).is_none() {
            self.create_definition(
                crate::FixCategory::Components,
                fix_plugin_message()?.clone(),
            )?;
        }
        Ok(self)
    }
}

/// The document inside a line a caller handed over whole.
///
/// A transport writes a timestamp in front of a document and sometimes a
/// duration behind it, and the namespace scan knows where both stop. Bytes
/// carrying no document at all are handed on unchanged, because a body that
/// is not a Jolokia answer names no plugin either way and reading is not
/// refusing (decision 17).
fn document_in(body: &[u8]) -> &[u8] {
    crate::mime_type::line::plugin_span(body).map_or(body, |span| &body[span])
}

/// The returned ObjectName and attributes, as flat scalar pairs.
fn push_attributes(pairs: &mut Vec<Pair>, mbean: Option<&str>, attributes: &Scalar) -> Result<()> {
    if let Some(mbean) = mbean {
        pairs.push((
            SESSIONINTERFACE_NAME.as_bytes().to_vec(),
            mbean.as_bytes().to_vec(),
            None,
        ));
        // The ObjectName's own properties, read where the classifier reads
        // them so one spelling answers for both.
        for (member, property) in [
            (
                b"MBeanType".as_slice(),
                crate::mime_type::line::OBJECT_NAME_TYPE,
            ),
            (b"PluginType".as_slice(), b"plugin-type".as_slice()),
        ] {
            if let Some(value) =
                crate::mime_type::line::object_name_property(mbean.as_bytes(), property)
            {
                pairs.push((member.to_vec(), value.to_vec(), None));
            }
        }
    }
    if let Some(attributes) = attributes.as_record() {
        for (attribute, value) in attributes {
            if let Some(value) = rendered(value)? {
                pairs.push((
                    attribute.as_bytes().to_vec(),
                    value,
                    attribute_name(attribute),
                ));
            }
        }
    }
    Ok(())
}

/// One pair a document states: the key as the document spells it, the
/// rendered value, and the dictionary's name for the field where that is
/// not the key.
type Pair = (Vec<u8>, Vec<u8>, Option<&'static [u8]>);

/// What one JSON value contributes, or nothing where it states nothing.
///
/// Text is its own bytes; every other leaf is the JSON it is, which is also
/// what an object or an array becomes - the builder types a leaf and keeps
/// what it cannot type, and a rendered subtree is the honest thing to keep.
fn rendered(value: &Scalar) -> Result<Option<Vec<u8>>> {
    if value.is_null() {
        return Ok(None);
    }
    match value.as_str() {
        Some(text) => Ok((!text.is_empty()).then(|| text.as_bytes().to_vec())),
        None => crate::into_json_scalar(value).map(|text| Some(text.into_bytes())),
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
/// use yggdryl::Plugin;
///
/// // A log line: prose in front of the document, prose behind it.
/// let line = br#"12:00:00 [Jolokia] Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=OrderRouting,plugin-type=FIX,type=Plugin":{"Name":"OrderRouting","Version":"4.7.0","State":"logged"}},"status":200} (12 ms)"#;
///
/// let held: Vec<Plugin> = Plugin::from_json_bytes(line).collect();
/// assert_eq!(held.len(), 1);
/// assert_eq!(held[0].name(), Some("OrderRouting"));
/// assert_eq!(held[0].version(), Some("4.7.0"));
/// assert_eq!(held[0].plugin_type(), Some("FIX"));
/// assert_eq!(held[0].mbean_type(), Some("Plugin"));
///
/// // A body that is not a Jolokia answer names no plugin, and naming none
/// // is what it answers: reading is not refusing (decision 17).
/// assert_eq!(Plugin::from_json_bytes(br#"{"a":1}"#).count(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Plugin {
    /// The ObjectName the read answered for.
    ///
    /// A wildcard answer keys it beside the attributes and a single read
    /// states it in the request, so the two are read from different places
    /// and name the same thing. An answer naming none names no plugin at all
    /// and is walked past (decision 17), so a plugin this crate reads always
    /// carries one; the option is for a caller building one by hand.
    mbean: Option<SmolStr>,
    /// The attributes as the document stated them, sorted by name.
    attributes: Scalar,
    /// What the row this plugin arrived on stated beside its document.
    ///
    /// Not part of this value's identity: a row is where a statement was
    /// read, not what it says.
    stamp: Option<Arc<RowStamp>>,
}

impl PartialEq for Plugin {
    fn eq(&self, other: &Self) -> bool {
        self.mbean == other.mbean && self.attributes == other.attributes
    }
}

impl Eq for Plugin {}

impl std::hash::Hash for Plugin {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.mbean.hash(state);
        self.attributes.hash(state);
    }
}

impl Plugin {
    /// The deterministic hash of this configuration: what it is named and
    /// what it states, which is all of it (decision 17).
    /// Uses one allocation for the shared XXH3 state, independent of value size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// One plugin from the parts a document states: the ObjectName the
    /// answer named it by, and its attributes.
    ///
    /// What the Jolokia exchange wrapped them in is the transport's and no
    /// part of the configuration (decision 17).
    #[must_use]
    pub fn new(mbean: Option<&str>, attributes: Scalar) -> Self {
        Self {
            mbean: mbean.map(SmolStr::new),
            attributes,
            stamp: None,
        }
    }

    /// Every plugin one document answers for, from the bytes a line carries.
    ///
    /// The document is found inside the line the way the namespace scan finds
    /// it: a transport writes a timestamp in front of one and sometimes a
    /// duration behind it, and both are prose.
    ///
    /// Bytes that are not a Jolokia answer name no plugin, and naming none is
    /// what they answer: reading is not refusing, and a row carrying a body
    /// FIX cannot read said nothing FIX can read (decision 17). That covers
    /// bytes that are not JSON at all.
    #[must_use]
    pub fn from_json_bytes(body: &[u8]) -> Plugins {
        Self::from_json_document(document_in(body))
    }

    /// Every plugin one document answers for, the document already bounded.
    ///
    /// [`Self::from_json_bytes`] finds the document inside a line first; this
    /// is the same reading for a caller whose own scan already found it, so
    /// the namespace, the opener and the close are looked for once for the
    /// whole reading (decision 17).
    fn from_json_document(document: &[u8]) -> Plugins {
        let Ok(parsed) = crate::from_json_scalar(document) else {
            return Plugins::none();
        };
        Self::from_json_scalar(&parsed)
    }

    /// Every plugin one parsed document answers for, in the order it answered.
    ///
    /// A Jolokia answer - a `value` beside what was asked - names one plugin
    /// per ObjectName its value keys, or the one its request selected. A
    /// document that is neither, an answer that came back empty, and an
    /// error-only answer all name none, which is what they answer: the
    /// envelope is the transport's and no configuration of its own
    /// (decision 17). A bulk array is traversed lazily without collecting
    /// its results.
    #[must_use]
    pub fn from_json_scalar(document: &Scalar) -> Plugins {
        Plugins {
            document: document.clone(),
            response: 0,
            after: None,
            done: false,
            stamp: None,
        }
    }

    /// Recovers one configuration from a flat typed message.
    ///
    /// Derived crate fields are omitted, and so are the ObjectName and the
    /// two properties read out of it, because [`Self::into_fixmsg`] writes
    /// all three from `mbean` rather than from the attributes. Every other
    /// entry is an attribute the document stated - a message states nothing
    /// of the exchange that carried it (decision 17), so there is nothing
    /// else to leave out.
    pub fn from_fixmsg(message: &super::FixMsg) -> Result<Self> {
        let values =
            message
                .as_value()
                .as_sequence()
                .ok_or_else(|| crate::Error::InvalidRecord {
                    path: message.as_field().name().into(),
                    reason: "expected a flat FIX row sequence".into(),
                })?;
        let mut attributes = Vec::new();
        for (field, value) in message.as_field().fields().iter().zip(values) {
            if value.is_null() || field.as_fix().tag()?.is_some_and(super::is_crate_tag) {
                continue;
            }
            let name = field.name();
            if [SESSIONINTERFACE_NAME, "MBeanType", "PluginType"]
                .iter()
                .any(|held| crate::types::folds_equal(name, held))
            {
                continue;
            }
            attributes.push((SmolStr::new(attribute_spelling(name)), value.clone()));
        }
        Ok(Self {
            mbean: message
                .get_by_name(SESSIONINTERFACE_NAME)
                .and_then(Scalar::as_str)
                .map(SmolStr::new),
            attributes: Scalar::from_record(attributes)?,
            stamp: None,
        })
    }

    /// Converts this configuration to one flat message through the core builder.
    ///
    /// Object/array attributes retain their canonical JSON text. The
    /// ObjectName the answer named this plugin by is its `SessionInterface`
    /// attribute. What the row this plugin arrived on stated is applied last,
    /// so a row's own clock outranks any the document carries.
    pub fn into_fixmsg(&self, codec: &super::FixCodec) -> Result<super::FixMsg> {
        // The attributes and nothing the answer wrapped them in: the
        // ObjectName the read named this plugin by is the `SessionInterface`
        // attribute, which is where it always belonged (decision 17).
        let mut pairs = Vec::new();
        push_attributes(&mut pairs, self.mbean.as_deref(), &self.attributes)?;
        let borrowed: Vec<super::codec::SpelledPair<'_>> = pairs
            .iter()
            .map(|(key, value, name)| (key.as_slice(), value.as_slice(), *name))
            .collect();
        let fills = self
            .stamp
            .as_ref()
            .map(|stamp| stamp.fills())
            .unwrap_or_default();
        let mut extras = RowStamp::held(self.stamp.as_ref(), &fills);
        // What this message is, said by the crate rather than by the
        // document: a configuration carries `35=UCFG` as a built child and
        // reads as `pluginconfig` (decision 19).
        extras.msgtype = Some(&PLUGINCONFIG_CODE_NAME);
        codec.build_pairs_with(&borrowed, extras)
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

    /// The attributes as the one value they are.
    #[must_use]
    pub const fn as_attributes(&self) -> &Scalar {
        &self.attributes
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

/// Configurations in response order, then canonical returned ObjectName order.
///
/// Keeps the shared parsed Scalar and one key cursor; no list of results is
/// materialized. A response that named no configuration - a request with no
/// value, an error-only answer, a member that is not a Jolokia answer at all -
/// is walked past rather than yielded, because there is nothing of the
/// plugin's in it (decision 17).
#[derive(Clone, Debug)]
pub struct Plugins {
    document: Scalar,
    response: usize,
    after: Option<SmolStr>,
    done: bool,
    stamp: Option<Arc<RowStamp>>,
}

impl Plugins {
    /// The plugins a document naming none answers for.
    fn none() -> Self {
        Self {
            document: Scalar::Null,
            response: 0,
            after: None,
            done: true,
            stamp: None,
        }
    }
}

impl Iterator for Plugins {
    type Item = Plugin;

    fn next(&mut self) -> Option<Self::Item> {
        use std::ops::Bound;
        if self.done {
            return None;
        }
        loop {
            let answer = match self.document.as_sequence() {
                Some(responses) => responses.get(self.response),
                None => (self.response == 0).then_some(&self.document),
            };
            let Some(answer) = answer else {
                self.done = true;
                return None;
            };
            // A Jolokia answer, and nothing else: an object stating what was
            // asked beside what came back. Anything else the row happened to
            // carry names no configuration, and a document that names none
            // answers none (decision 17).
            let Some(root) = answer.as_record() else {
                self.response += 1;
                self.after = None;
                continue;
            };
            let request = root
                .get("request")
                .and_then(Scalar::as_record)
                .unwrap_or(root);
            let Some(held) = root.get("value") else {
                self.response += 1;
                self.after = None;
                continue;
            };
            let Some(value) = held.as_record() else {
                self.response += 1;
                self.after = None;
                continue;
            };
            // A wildcard read keys its answer by the ObjectName of every
            // plugin it selected, so each is one message.
            let bounds = (
                self.after
                    .as_deref()
                    .map_or(Bound::Unbounded, Bound::Excluded),
                Bound::Unbounded,
            );
            if let Some((mbean, attributes)) = value.range::<str, _>(bounds).find(|(name, _)| {
                crate::mime_type::line::object_names(name.as_bytes())
                    .next()
                    .is_some()
            }) {
                let config = Plugin {
                    mbean: Some(mbean.clone()),
                    attributes: attributes.clone(),
                    stamp: self.stamp.clone(),
                };
                self.after = Some(mbean.clone());
                return Some(config);
            }
            self.response += 1;
            if self.after.take().is_some() {
                continue;
            }
            // A read of one plugin answers its attributes flat, and the
            // ObjectName it was selected by is the request's.
            let named = request
                .get("mbean")
                .and_then(Scalar::as_str)
                .filter(|name| {
                    crate::mime_type::line::object_names(name.as_bytes())
                        .next()
                        .is_some()
                })
                .filter(|name| {
                    let mut escaped = false;
                    !name.bytes().any(|byte| {
                        if escaped {
                            escaped = false;
                            return false;
                        }
                        escaped = byte == b'\\';
                        matches!(byte, b'*' | b'?')
                    })
                });
            let Some(named) = named else {
                continue;
            };
            return Some(Plugin {
                mbean: Some(SmolStr::new(named)),
                attributes: held.clone(),
                stamp: self.stamp.clone(),
            });
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.done { (0, Some(0)) } else { (0, None) }
    }
}

impl std::iter::FusedIterator for Plugins {}

impl super::FixCodec {
    /// Parses one configuration body into lazily converted flat messages.
    ///
    /// The document is the JSON payload a Jolokia answer
    /// names, and this is the reader for it beside the one for a numeric
    /// frame, a bridge row and a FIXML row: it produces the same key/value
    /// pairs they do and hands them to the same builder, so what a dictionary
    /// types here is typed by the rules that type everything else.
    ///
    /// Bulk responses and wildcard values may yield several messages; each
    /// is exactly one plugin's ObjectName and scalar attributes.
    ///
    /// Register [`fix_plugin_fields`] to type a document's own attributes;
    /// FIX's own names resolve either way.
    ///
    /// It refuses nothing: a body that is not a Jolokia answer - `{"a":1}`,
    /// or bytes that are not JSON at all - names no configuration, and
    /// naming none is what it answers (decision 17). A conversion's own
    /// refusal is an item of the iterator.
    #[must_use]
    pub fn parse_plugin_line(&self, body: &[u8]) -> super::FixMessages {
        let extras = super::build::RowExtras {
            direction: self.msgdirection().read_bytes(body),
            ..super::build::RowExtras::NONE
        };
        self.plugin_with(document_in(body), extras)
    }

    /// [`Self::parse_plugin_line`], with what the row stated beside its
    /// document.
    ///
    /// A row states its clock and its own columns once and the document it
    /// carries answers for as many plugins as it names, so what the row
    /// stated is retained on the expansion rather than borrowed across it:
    /// every message the document yields is stamped by the row it arrived on.
    pub(super) fn plugin_with(
        &self,
        document: &[u8],
        extras: super::build::RowExtras<'_>,
    ) -> super::FixMessages {
        // The document itself, bounded by whichever scan found it: a
        // transport writes a timestamp in front of one and sometimes a
        // duration behind it, and the scan that finds the namespace already
        // knows where both stop, so this one does not look again. A document
        // that is not a Jolokia answer names no plugin and answers none.
        let mut values = Plugin::from_json_document(document);
        values.stamp = RowStamp::retained(extras);
        super::FixMessages::from_plugins(self.clone(), values)
    }
}

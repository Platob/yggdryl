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
//! # One configuration per message
//!
//! A single read yields one [`Ulconfig`]. A wildcard or bulk answer yields
//! lazy [`Ulconfigs`], retaining the shared source document and one key cursor.
//! Each value converts to one flat [`FixMsg`](super::FixMsg): `MBean` retains
//! the request selector, `SessionInterface` the returned ObjectName, and the
//! attributes become ordinary scalar fields. No synthetic collection or count
//! field is introduced.
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

/// The name the actual returned ObjectName member carries.
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
    // A returned MBean is one flat message. FIX's own SenderCompID remains
    // tag 49; the bridge's CurrentPort remains tag 20027.
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
    ];
    fields.extend(members);
    Ok(fields)
}

/// The scalar fields ULBridge's dictionary defines, in tag order.
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
    /// Adds ULBridge's scalar fields for one configuration per message.
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
        self.add_fields(fix_ulbridge_fields()?.iter().cloned())?;
        Ok(self)
    }
}

/// The returned ObjectName and attributes, as flat scalar pairs.
fn push_attributes(
    pairs: &mut Vec<(Vec<u8>, Vec<u8>)>,
    mbean: Option<&str>,
    attributes: &Scalar,
) -> Result<()> {
    if let Some(mbean) = mbean {
        pairs.push((
            SESSIONINTERFACE_NAME.as_bytes().to_vec(),
            mbean.as_bytes().to_vec(),
        ));
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
                pairs.push((member.to_vec(), value.to_vec()));
            }
        }
    }
    if let Some(attributes) = attributes.as_record() {
        for (attribute, value) in attributes {
            if let Some(value) = rendered(value)? {
                pairs.push((attribute.as_bytes().to_vec(), value));
            }
        }
    }
    Ok(())
}

/// One leaf of the envelope, where the document stated it.
fn push_leaf(
    pairs: &mut Vec<(Vec<u8>, Vec<u8>)>,
    key: &[u8],
    value: Option<&Scalar>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if let Some(rendered) = rendered(value)? {
        pairs.push((key.to_vec(), rendered));
    }
    Ok(())
}

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
/// use yggdryl::Ulconfig;
///
/// // A log line: prose in front of the document, prose behind it.
/// let line = br#"12:00:00 [Jolokia] Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=OrderRouting,plugin-type=FIX,type=Plugin":{"Name":"OrderRouting","Version":"4.7.0","State":"logged"}},"status":200} (12 ms)"#;
///
/// let held: Vec<Ulconfig> = Ulconfig::from_json_bytes(line)?.collect();
/// assert_eq!(held.len(), 1);
/// assert_eq!(held[0].name(), Some("OrderRouting"));
/// assert_eq!(held[0].version(), Some("4.7.0"));
/// assert_eq!(held[0].plugin_type(), Some("FIX"));
/// assert_eq!(held[0].mbean_type(), Some("Plugin"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Ulconfig {
    /// The ObjectName the document answered under, where it named one.
    ///
    /// A single read states its MBean in the request rather than beside the
    /// attributes, and a document that states neither answers a plugin whose
    /// name is whatever its `Name` attribute says.
    mbean: Option<SmolStr>,
    /// The attributes as the document stated them, sorted by name.
    attributes: Scalar,
    /// Shared original response; only its request/status/error envelope is read.
    envelope: Scalar,
}

impl PartialEq for Ulconfig {
    fn eq(&self, other: &Self) -> bool {
        self.mbean == other.mbean
            && self.attributes == other.attributes
            && self.exchange() == other.exchange()
    }
}

impl Eq for Ulconfig {}

impl std::hash::Hash for Ulconfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.mbean.hash(state);
        self.attributes.hash(state);
        self.exchange().hash(state);
    }
}

impl Ulconfig {
    /// The deterministic hash of this configuration and its selected exchange.
    /// Uses one allocation for the shared XXH3 state, independent of value size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Only the selected exchange contributes to this value's identity.
    fn exchange(
        &self,
    ) -> (
        Option<&str>,
        Option<&Scalar>,
        Option<&Scalar>,
        Option<&Scalar>,
    ) {
        let Some(root) = self.envelope.as_record() else {
            return (self.mbean(), None, None, None);
        };
        let request = root
            .get("request")
            .and_then(Scalar::as_record)
            .unwrap_or(root);
        (
            request.get("mbean").and_then(Scalar::as_str),
            request.get("type").filter(|value| !value.is_null()),
            root.get("status").filter(|value| !value.is_null()),
            root.get("error").filter(|value| !value.is_null()),
        )
    }

    /// One plugin from the parts a document states.
    ///
    /// The selected ObjectName, attributes, and shared source response.
    /// A null envelope declares no exchange metadata.
    #[must_use]
    pub fn new(mbean: Option<&str>, attributes: Scalar, envelope: Scalar) -> Self {
        Self {
            mbean: mbean.map(SmolStr::new),
            attributes,
            envelope,
        }
    }

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
    pub fn from_json_bytes(body: &[u8]) -> Result<Ulconfigs> {
        let document = crate::from_json_scalar(
            crate::mime_type::line::ulconfig_span(body).map_or(body, |span| &body[span]),
        )?;
        Self::from_json_scalar(&document)
    }

    /// Every plugin one parsed document answers for, in the order it answered.
    ///
    /// The envelope is optional: a `value` under a Jolokia answer, an array of
    /// those answers for a bulk read, or a bare attribute map a caller pulled
    /// out itself. Requests and error-only responses retain their envelope;
    /// a bulk array is traversed lazily without collecting its results.
    ///
    /// # Errors
    ///
    /// Refuses a non-object response, identifying its index in a bulk array.
    /// Validation finishes before an iterator is returned.
    pub fn from_json_scalar(document: &Scalar) -> Result<Ulconfigs> {
        if let Some(responses) = document.as_sequence() {
            for (index, response) in responses.iter().enumerate() {
                if response.as_record().is_none() {
                    return Err(crate::Error::InvalidRecord {
                        path: format!("ulconfig[{index}]").into(),
                        reason: crate::text::expected_got("an object response", response.kind()),
                    });
                }
            }
        } else if document.as_record().is_none() {
            return Err(crate::Error::InvalidRecord {
                path: "ulconfig".into(),
                reason: crate::text::expected_got(
                    "an object response or array of responses",
                    document.kind(),
                ),
            });
        }
        Ok(Ulconfigs {
            document: document.clone(),
            response: 0,
            after: None,
            done: false,
        })
    }

    /// Recovers one configuration from a flat typed message.
    ///
    /// The envelope remains separate from the attributes. Derived crate
    /// fields and the two properties computed from the ObjectName are omitted.
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
            if value.is_null() || field.as_fix().branch()?.name() == super::CRATE_BRANCH {
                continue;
            }
            let name = field.name();
            if [
                "MBean",
                "Operation",
                "Status",
                "Error",
                "SessionInterface",
                "MBeanType",
                "PluginType",
            ]
            .iter()
            .any(|held| crate::types::folds_equal(name, held))
            {
                continue;
            }
            attributes.push((SmolStr::new(name), value.clone()));
        }
        let mut request = Vec::new();
        if let Some(value) = message.get_by_name("MBean") {
            request.push(("mbean", value.clone()));
        }
        if let Some(value) = message.get_by_name("Operation") {
            request.push(("type", value.clone()));
        }
        let mut envelope = vec![("request", Scalar::from_record(request)?)];
        if let Some(value) = message.get_by_name("Status") {
            envelope.push(("status", value.clone()));
        }
        if let Some(value) = message.get_by_name("Error") {
            envelope.push(("error", value.clone()));
        }
        Ok(Self {
            mbean: message
                .get_by_name(SESSIONINTERFACE_NAME)
                .and_then(Scalar::as_str)
                .map(SmolStr::new),
            attributes: Scalar::from_record(attributes)?,
            envelope: Scalar::from_record(envelope)?,
        })
    }

    /// Converts this configuration to one flat message through the core builder.
    ///
    /// Object/array attributes retain their canonical JSON text. The request
    /// selector and the returned ObjectName occupy their distinct scalar fields.
    pub fn into_fixmsg(&self, codec: &super::FixCodec, enrich: bool) -> Result<super::FixMsg> {
        let mut pairs = Vec::new();
        if let Some(root) = self.envelope.as_record() {
            let request = root
                .get("request")
                .and_then(Scalar::as_record)
                .unwrap_or(root);
            push_leaf(&mut pairs, b"MBean", request.get("mbean"))?;
            push_leaf(&mut pairs, b"Operation", request.get("type"))?;
            push_leaf(&mut pairs, b"Status", root.get("status"))?;
            push_leaf(&mut pairs, b"Error", root.get("error"))?;
        } else if let Some(mbean) = &self.mbean {
            pairs.push((b"MBean".to_vec(), mbean.as_bytes().to_vec()));
        }
        push_attributes(&mut pairs, self.mbean.as_deref(), &self.attributes)?;
        let borrowed: Vec<(&[u8], &[u8])> = pairs
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
            .collect();
        codec.build_pairs(&borrowed, enrich)
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

    /// The shared source response, including sibling values in a wildcard read.
    ///
    /// Only its request, status, and error describe this selected exchange.
    #[must_use]
    pub const fn as_envelope(&self) -> &Scalar {
        &self.envelope
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
/// materialized. A request or error-only response still yields its envelope.
#[derive(Clone, Debug)]
pub struct Ulconfigs {
    document: Scalar,
    response: usize,
    after: Option<SmolStr>,
    done: bool,
}

impl Iterator for Ulconfigs {
    type Item = Ulconfig;

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
            let root = answer
                .as_record()
                .expect("ULconfig intake validated every response object");
            let request = root
                .get("request")
                .and_then(Scalar::as_record)
                .unwrap_or(root);
            let has_envelope = ["request", "value", "mbean", "status", "error"]
                .iter()
                .any(|key| root.contains_key(*key));
            let empty = Scalar::Null;
            let value = root
                .get("value")
                .unwrap_or(if has_envelope { &empty } else { answer });
            if let Some(attributes) = value.as_record() {
                let bounds = (
                    self.after
                        .as_deref()
                        .map_or(Bound::Unbounded, Bound::Excluded),
                    Bound::Unbounded,
                );
                if let Some((mbean, attributes)) =
                    attributes.range::<str, _>(bounds).find(|(name, _)| {
                        crate::mime_type::line::object_names(name.as_bytes())
                            .next()
                            .is_some()
                    })
                {
                    let config = Ulconfig {
                        mbean: Some(mbean.clone()),
                        attributes: attributes.clone(),
                        envelope: answer.clone(),
                    };
                    self.after = Some(mbean.clone());
                    return Some(config);
                }
            }
            if self.after.take().is_some() {
                self.response += 1;
                continue;
            }
            let selector = request.get("mbean").and_then(Scalar::as_str);
            let wildcard = selector.is_some_and(|name| {
                let mut escaped = false;
                name.bytes().any(|byte| {
                    if escaped {
                        escaped = false;
                        return false;
                    }
                    escaped = byte == b'\\';
                    matches!(byte, b'*' | b'?')
                })
            });
            if wildcard && value.as_record().is_some_and(|value| value.is_empty()) {
                self.response += 1;
                continue;
            }
            let config = Ulconfig {
                mbean: (!wildcard && root.contains_key("value"))
                    .then_some(selector)
                    .flatten()
                    .map(SmolStr::new),
                attributes: value.clone(),
                envelope: if has_envelope {
                    answer.clone()
                } else {
                    Scalar::Null
                },
            };
            self.response += 1;
            return Some(config);
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.done { (0, Some(0)) } else { (0, None) }
    }
}

impl std::iter::FusedIterator for Ulconfigs {}

impl super::FixCodec {
    /// Parses one configuration body into lazily converted flat messages.
    ///
    /// Bulk responses and wildcard values may yield several messages; each
    /// carries the common envelope and exactly one MBean's scalar attributes.
    pub fn transform_ulconfig_line(&self, body: &[u8], enrich: bool) -> Result<super::FixMessages> {
        Ok(super::FixMessages::from_ulconfigs(
            self.clone(),
            Ulconfig::from_json_bytes(body)?,
            enrich,
        ))
    }
}

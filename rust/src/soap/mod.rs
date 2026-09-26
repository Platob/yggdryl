//! SOAP 1.1: the envelope every XML-over-HTTP protocol here speaks in, its
//! header blocks, its one body element, and the fault that stands in for it.
//!
//! An [`Envelope`] is what the natural XML value of a SOAP message reads as
//! and writes from: the header blocks and the body element are each a
//! [`Fragment`] - one element by its spelled name, its natural value, and the
//! namespace declarations in scope at it - so a protocol built over SOAP reads
//! its own vocabulary through [`Element`] and knows
//! nothing of the envelope. A [`Fault`] is the body an error answers with:
//! its code, its human-readable string, the actor that raised it, and the
//! protocol's own detail.
//!
//! ```
//! use yggdryl::Scalar;
//! use yggdryl::soap::{Body, Envelope, Fragment};
//!
//! let discover = Scalar::from_struct([
//!     ("@xmlns", Scalar::from("urn:schemas-microsoft-com:xml-analysis")),
//!     ("RequestType", Scalar::from("DISCOVER_DATASOURCES")),
//! ])?;
//! let request = Envelope::new(Body::Payload(Fragment::new("Discover", discover)));
//! let bytes = request.into_bytes()?;
//! assert!(bytes.starts_with(b"<?xml version=\"1.0\" encoding=\"utf-8\"?><SOAP-ENV:Envelope"));
//!
//! let read = Envelope::from_bytes(&bytes)?;
//! let payload = read.payload().expect("a payload, not a fault");
//! assert_eq!(payload.element().local_name(), "Discover");
//! assert_eq!(
//!     payload.element().namespace(),
//!     Some("urn:schemas-microsoft-com:xml-analysis")
//! );
//! # Ok::<(), yggdryl::Error>(())
//! ```
//!
//! The writer has a streaming shape as well: [`EnvelopeWriter`] opens the
//! envelope and its header, lends the body to the caller, and closes both, so
//! a response whose body is a stream of rows is written as the rows arrive
//! rather than held whole. [`http`] is the HTTP binding: the request framing a
//! SOAP endpoint reads and the response framing it writes, over any
//! `std::io` stream.

pub mod http;

use std::borrow::Cow;
use std::fmt;
use std::io::Write;

use smol_str::{SmolStr, format_smolstr};

use crate::text::Limits;
use crate::xml::{
    ATTRIBUTE_PREFIX, Element, Scope, TEXT_KEY, from_bytes_with_limits, write_element_text,
    write_fragment,
};
use crate::{Error, Result, Scalar};

/// The SOAP 1.1 envelope namespace.
pub const ENVELOPE_NAMESPACE: &str = "http://schemas.xmlsoap.org/soap/envelope/";

/// The SOAP 1.1 encoding namespace, which `encodingStyle` names.
pub const ENCODING_NAMESPACE: &str = "http://schemas.xmlsoap.org/soap/encoding/";

/// The prefix the envelope is written under, as the specification's own
/// examples spell it.
pub const PREFIX: &str = "SOAP-ENV";

/// The media type of a SOAP 1.1 message on the wire, charset included.
pub const CONTENT_TYPE: &str = "text/xml; charset=utf-8";

/// The HTTP header that names the method a SOAP 1.1 request invokes.
pub const ACTION_HEADER: &str = "SOAPAction";

/// The declaration every message opens with.
const DECLARATION: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>";

/// One element with its in-scope namespace declarations: a header block, the
/// body element, or a fault's detail.
///
/// A fragment built here carries its declarations as the natural value's own
/// `@xmlns` attributes; one read out of a message also remembers the
/// declarations the envelope above it made, so its [`element`](Self::element)
/// resolves a prefix declared anywhere over it.
#[derive(Clone, Debug, Eq)]
pub struct Fragment {
    name: SmolStr,
    value: Scalar,
    scope: Scope,
    /// Whether the scope is a message's: read out of one, the fragment's
    /// namespaces are facts of the document; built, they are whatever the
    /// message it is written into declares around it.
    read: bool,
}

/// Two fragments are one element when they spell the same name over the
/// same content: a fragment built for a message, the same one read back out
/// of it, and that one written and read again are all equal, though each
/// reading remembers the declarations made above it and a rewrite declares
/// them on the element. Declarations bind names rather than carry content,
/// so they are no part of what is compared - where a fragment was read is
/// provenance, not identity.
impl PartialEq for Fragment {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && content_of(&self.value) == content_of(&other.value)
    }
}

/// `value` less its namespace declarations: the entries a record keeps once
/// `@xmlns` and `@xmlns:p` come off, a record left with nothing being null
/// and one left with its text alone being that text - the value the element
/// would have had, written where its names were already bound.
fn content_of(value: &Scalar) -> Cow<'_, Scalar> {
    let Scalar::Struct(entries) = value else {
        return Cow::Borrowed(value);
    };
    let declares = entries.as_map().keys().any(|key| {
        key.strip_prefix(ATTRIBUTE_PREFIX)
            .is_some_and(|attribute| attribute == "xmlns" || attribute.starts_with("xmlns:"))
    });
    if !declares {
        return Cow::Borrowed(value);
    }
    let kept: Vec<(SmolStr, Scalar)> = entries
        .as_map()
        .iter()
        .filter(|(key, _)| {
            !key.strip_prefix(ATTRIBUTE_PREFIX)
                .is_some_and(|attribute| attribute == "xmlns" || attribute.starts_with("xmlns:"))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Cow::Owned(match kept.as_slice() {
        [] => Scalar::Null,
        [(key, text)] if key == TEXT_KEY => text.clone(),
        _ => Scalar::from_struct(kept).unwrap_or_else(|_| value.clone()),
    })
}

impl Fragment {
    /// One element named `name` holding the natural value `value`.
    pub fn new(name: impl Into<SmolStr>, value: Scalar) -> Self {
        Self {
            name: name.into(),
            value,
            scope: Scope::new(),
            read: false,
        }
    }

    /// The element named `name` in `namespace`, declared as the default
    /// namespace of the element itself, holding `value`.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is a record already declaring a default
    /// namespace, or is a leaf that cannot carry an attribute.
    pub fn in_namespace(name: impl Into<SmolStr>, namespace: &str, value: Scalar) -> Result<Self> {
        let name = name.into();
        // A prefixed name is in the namespace its prefix names, which a
        // default declaration never reaches: the prefix is declared instead.
        let key = match name.split_once(':') {
            Some((prefix, _)) => format_smolstr!("{ATTRIBUTE_PREFIX}xmlns:{prefix}"),
            None => format_smolstr!("{ATTRIBUTE_PREFIX}xmlns"),
        };
        let value = match value {
            Scalar::Struct(entries) => {
                let mut entries: Vec<(SmolStr, Scalar)> = entries
                    .as_map()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                if entries.iter().any(|(held, _)| *held == key) {
                    return Err(codec_error(match name.split_once(':') {
                        Some((prefix, _)) => {
                            format_smolstr!("the element already declares `xmlns:{prefix}`")
                        }
                        None => {
                            SmolStr::new_static("the element already declares a default namespace")
                        }
                    }));
                }
                entries.push((key, Scalar::from(namespace)));
                Scalar::from_struct(entries)?
            }
            Scalar::Null => Scalar::from_struct([(key, Scalar::from(namespace))])?,
            leaf if leaf.as_str().is_some() => Scalar::from_struct([
                (key, Scalar::from(namespace)),
                (SmolStr::new_static(TEXT_KEY), leaf),
            ])?,
            other => {
                return Err(codec_error(format_smolstr!(
                    "expected a record or text for a namespaced element, got {}",
                    other.kind()
                )));
            }
        };
        Ok(Self::new(name, value))
    }

    /// The fragment `element` is, remembering the declarations in scope at
    /// it, so a prefix declared anywhere over the element still resolves.
    #[must_use]
    pub fn from_element(element: &Element<'_>) -> Self {
        Self {
            name: SmolStr::new(element.name()),
            value: element.value().clone(),
            scope: element.scope().clone(),
            read: true,
        }
    }

    /// Write this fragment as one element, declaring on it every prefix it
    /// spells - in its name, its attributes, its descendants - that its scope
    /// binds and its value does not declare itself, so a fragment read out of
    /// one message writes into another as namespace-well-formed XML (`as`
    /// still declared once `<as:Batch>` stands alone). The envelope's own
    /// `SOAP-ENV` binding and the default namespace are what the message
    /// around the element declares, and are never repeated on it.
    ///
    /// # Errors
    ///
    /// Returns the XML writer's refusal for a name or a value with no XML
    /// spelling, or the sink's failure.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        self.write_under(writer, None)
    }

    /// [`Self::write`] where the element around it declares `default` as the
    /// default namespace: a fragment read out of a message keeps the
    /// namespace its unprefixed name was in - declaring its own default, or
    /// undeclaring one (`xmlns=""`) it never had - while a fragment built
    /// for the message takes the default it is written under.
    ///
    /// # Errors
    ///
    /// As [`Self::write`].
    pub fn write_under<W: Write>(&self, writer: &mut W, default: Option<&str>) -> Result<()> {
        let declared = self.declared_value(default)?;
        write_fragment(writer, &self.name, declared.as_ref().unwrap_or(&self.value))
    }

    /// The value as the natural document carries it: with the scope's
    /// bindings the element spells declared on it, so a message's natural
    /// value and its bytes describe one message.
    pub(crate) fn natural_value(&self) -> Result<Scalar> {
        Ok(self
            .declared_value(None)?
            .unwrap_or_else(|| self.value.clone()))
    }

    /// The value with the bindings the element spells declared on it -
    /// its default namespace reconciled against `default`, the one in force
    /// where it is written, when the fragment was read - `None` when the
    /// scope adds nothing the value does not already say.
    fn declared_value(&self, default: Option<&str>) -> Result<Option<Scalar>> {
        let mut prefixes: Vec<&str> = Vec::new();
        used_prefixes(&self.name, &self.value, &mut prefixes);
        let mut missing: Vec<(SmolStr, Scalar)> = prefixes
            .into_iter()
            .filter(|prefix| !self.declares(prefix))
            .filter_map(|prefix| {
                let namespace = self.scope.resolve(prefix)?;
                // The envelope declares its own prefix over everything in it.
                (prefix != PREFIX || namespace != ENVELOPE_NAMESPACE).then(|| {
                    (
                        format_smolstr!("{ATTRIBUTE_PREFIX}xmlns:{prefix}"),
                        Scalar::from(namespace),
                    )
                })
            })
            .collect();
        if self.read && !self.name.contains(':') && !self.declares("") {
            let own = self.scope.resolve("");
            if own != default {
                missing.push((
                    format_smolstr!("{ATTRIBUTE_PREFIX}xmlns"),
                    Scalar::from(own.unwrap_or("")),
                ));
            }
        }
        if missing.is_empty() {
            return Ok(None);
        }
        let value = match &self.value {
            Scalar::Struct(entries) => {
                let entries = entries
                    .as_map()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .chain(missing);
                Scalar::from_struct(entries)?
            }
            Scalar::Null => Scalar::from_struct(missing)?,
            leaf if leaf.as_str().is_some() => Scalar::from_struct(
                missing
                    .into_iter()
                    .chain([(SmolStr::new_static(TEXT_KEY), leaf.clone())]),
            )?,
            other => {
                return Err(codec_error(format_smolstr!(
                    "expected a record or text for a namespaced element, got {}",
                    other.kind()
                )));
            }
        };
        Ok(Some(value))
    }

    /// Whether the value itself declares `prefix` (`@xmlns` for the default).
    fn declares(&self, prefix: &str) -> bool {
        let Scalar::Struct(entries) = &self.value else {
            return false;
        };
        let key = if prefix.is_empty() {
            format_smolstr!("{ATTRIBUTE_PREFIX}xmlns")
        } else {
            format_smolstr!("{ATTRIBUTE_PREFIX}xmlns:{prefix}")
        };
        entries.as_map().contains_key(&key)
    }

    /// The name as spelled, prefix included.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The natural value.
    #[must_use]
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// The natural value, owned.
    #[must_use]
    pub fn into_value(self) -> Scalar {
        self.value
    }

    /// View this fragment as the element it is, under every declaration in
    /// scope at it.
    #[must_use]
    pub fn element(&self) -> Element<'_> {
        Element::new(&self.name, &self.value, &self.scope)
    }
}

/// The four fault codes SOAP 1.1 defines, and any other a server spelled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultCode {
    /// The envelope's namespace is not one the server speaks.
    VersionMismatch,
    /// A header block marked `mustUnderstand` was not understood.
    MustUnderstand,
    /// The message was malformed or lacked what the server needs; the same
    /// message will fail again.
    Client,
    /// The server could not process a well-formed message; the same message
    /// may succeed later.
    Server,
    /// A code in another namespace, kept as it was spelled.
    Other(SmolStr),
}

impl FaultCode {
    /// The local name SOAP 1.1 gives this code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::VersionMismatch => "VersionMismatch",
            Self::MustUnderstand => "MustUnderstand",
            Self::Client => "Client",
            Self::Server => "Server",
            Self::Other(spelled) => spelled.as_str(),
        }
    }

    /// The code and the dotted subcode a `faultcode` element spells, under
    /// `scope`: `SOAP-ENV:Client.Authentication` is `Client` with the subcode
    /// `Authentication`, and a code whose prefix names another namespace is
    /// kept whole as [`Self::Other`].
    fn read(text: &str, scope: &Scope) -> (Self, Option<SmolStr>) {
        // The prefix comes off first: an NCName may carry a dot (`soap.v1`),
        // so the subcode's dot is looked for in the local part alone.
        let (prefix, qualified) = match text.split_once(':') {
            Some((prefix, local)) => (Some(prefix), local),
            None => (None, text),
        };
        let (local, subcode) = match qualified.split_once('.') {
            Some((local, subcode)) => (local, Some(SmolStr::new(subcode))),
            None => (qualified, None),
        };
        let in_envelope = match prefix {
            Some(prefix) => scope.resolve(prefix) == Some(ENVELOPE_NAMESPACE),
            // An unprefixed code is read as the envelope's, the way an
            // unqualified element is read as the protocol's.
            None => true,
        };
        let code = match local {
            _ if !in_envelope => Self::Other(SmolStr::new(text)),
            "VersionMismatch" => Self::VersionMismatch,
            "MustUnderstand" => Self::MustUnderstand,
            "Client" => Self::Client,
            "Server" => Self::Server,
            _ => Self::Other(SmolStr::new(text)),
        };
        match code {
            Self::Other(_) => (code, None),
            code => (code, subcode),
        }
    }
}

impl fmt::Display for FaultCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(spelled) => formatter.write_str(spelled),
            code => write!(formatter, "{PREFIX}:{}", code.as_str()),
        }
    }
}

/// A SOAP 1.1 fault: the body an error is answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    code: FaultCode,
    subcode: Option<SmolStr>,
    string: String,
    actor: Option<String>,
    detail: Vec<Fragment>,
}

impl Fault {
    /// A fault of `code`, explained by `string`.
    pub fn new(code: FaultCode, string: impl Into<String>) -> Self {
        Self {
            code,
            subcode: None,
            string: string.into(),
            actor: None,
            detail: Vec::new(),
        }
    }

    /// A `Client` fault: the message itself is what failed.
    pub fn client(string: impl Into<String>) -> Self {
        Self::new(FaultCode::Client, string)
    }

    /// A `Server` fault: processing failed on the server's side.
    pub fn server(string: impl Into<String>) -> Self {
        Self::new(FaultCode::Server, string)
    }

    /// Return this fault with a dotted subcode: `Client.Authentication`.
    #[must_use]
    pub fn with_subcode(mut self, subcode: impl Into<SmolStr>) -> Self {
        self.subcode = Some(subcode.into());
        self
    }

    /// Return this fault naming who raised it.
    #[must_use]
    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }

    /// Return this fault with one more detail element.
    #[must_use]
    pub fn with_detail(mut self, detail: Fragment) -> Self {
        self.detail.push(detail);
        self
    }

    /// The fault code.
    #[must_use]
    pub const fn code(&self) -> &FaultCode {
        &self.code
    }

    /// The dotted subcode, when the code carried one.
    #[must_use]
    pub fn subcode(&self) -> Option<&str> {
        self.subcode.as_deref()
    }

    /// The human-readable explanation.
    #[must_use]
    pub fn string(&self) -> &str {
        &self.string
    }

    /// Who raised the fault, when the message says.
    #[must_use]
    pub fn actor(&self) -> Option<&str> {
        self.actor.as_deref()
    }

    /// The application-specific detail elements, in the order they were
    /// given; a read fault's come in name order, the one order a parsed
    /// document keeps.
    #[must_use]
    pub fn detail(&self) -> &[Fragment] {
        &self.detail
    }

    /// The natural value of the `Fault` element this fault writes as.
    fn natural(&self) -> Result<Scalar> {
        let mut entries: Vec<(SmolStr, Scalar)> = Vec::with_capacity(4);
        let code = match &self.subcode {
            Some(subcode) => format!("{}.{subcode}", self.code),
            None => self.code.to_string(),
        };
        entries.push((SmolStr::new_static("faultcode"), Scalar::from(code)));
        entries.push((
            SmolStr::new_static("faultstring"),
            Scalar::from(self.string.as_str()),
        ));
        if let Some(actor) = &self.actor {
            entries.push((
                SmolStr::new_static("faultactor"),
                Scalar::from(actor.as_str()),
            ));
        }
        if !self.detail.is_empty() {
            let detail = record(
                self.detail
                    .iter()
                    .map(|fragment| Ok((fragment.name.clone(), fragment.natural_value()?)))
                    .collect::<Result<Vec<_>>>()?,
            )?;
            entries.push((SmolStr::new_static("detail"), detail));
        }
        record(entries)
    }

    /// Write the `Fault` element this fault is, its children in the order the
    /// SOAP 1.1 envelope schema declares them: `faultcode`, `faultstring`,
    /// then `faultactor` and `detail` where there are any.
    fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        let code = match &self.subcode {
            Some(subcode) => format!("{}.{subcode}", self.code),
            None => self.code.to_string(),
        };
        write!(writer, "<{PREFIX}:Fault><faultcode>")?;
        write_element_text(writer, &code)?;
        write!(writer, "</faultcode><faultstring>")?;
        write_element_text(writer, &self.string)?;
        write!(writer, "</faultstring>")?;
        if let Some(actor) = &self.actor {
            write!(writer, "<faultactor>")?;
            write_element_text(writer, actor)?;
            write!(writer, "</faultactor>")?;
        }
        if !self.detail.is_empty() {
            write!(writer, "<detail>")?;
            for fragment in &self.detail {
                fragment.write(writer)?;
            }
            write!(writer, "</detail>")?;
        }
        write!(writer, "</{PREFIX}:Fault>")?;
        Ok(())
    }

    fn read(fault: &Element<'_>) -> Result<Self> {
        let code_element = at_most_one(fault, "faultcode")?
            .ok_or_else(|| codec_error("a SOAP fault without a `faultcode`"))?;
        let code_text = code_element
            .text()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| codec_error("a SOAP fault without a `faultcode`"))?;
        // Under the code's own scope: a declaration on the `faultcode`
        // element itself is in force for the text it holds.
        let (code, subcode) = FaultCode::read(&code_text, code_element.scope());
        let string = at_most_one(fault, "faultstring")?
            .and_then(|string| string.text().map(str::to_owned))
            .unwrap_or_default();
        let actor =
            at_most_one(fault, "faultactor")?.and_then(|actor| actor.text().map(str::to_owned));
        let detail = at_most_one(fault, "detail")?
            .map(|detail| {
                detail
                    .children()
                    .map(|child| Fragment::from_element(&child))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            code,
            subcode,
            string,
            actor,
            detail,
        })
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SOAP fault {}", self.code)?;
        if let Some(subcode) = &self.subcode {
            write!(formatter, ".{subcode}")?;
        }
        write!(formatter, ": {}", self.string)
    }
}

impl std::error::Error for Fault {}

/// What a body carries: the one element a method call or its answer is, or
/// the fault that stands in for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// One element: the method invoked, or the response to it.
    Payload(Fragment),
    /// The error that took the place of an answer.
    Fault(Fault),
}

/// One SOAP 1.1 message: header blocks and a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    header: Vec<Fragment>,
    body: Body,
}

impl Envelope {
    /// A message with no header block and `body`.
    #[must_use]
    pub const fn new(body: Body) -> Self {
        Self {
            header: Vec::new(),
            body,
        }
    }

    /// A message carrying `payload` as its body element.
    #[must_use]
    pub const fn from_payload(payload: Fragment) -> Self {
        Self::new(Body::Payload(payload))
    }

    /// A message answering with `fault`.
    #[must_use]
    pub const fn from_fault(fault: Fault) -> Self {
        Self::new(Body::Fault(fault))
    }

    /// Return this message with one more header block.
    #[must_use]
    pub fn with_header(mut self, block: Fragment) -> Self {
        self.header.push(block);
        self
    }

    /// The header blocks, in the order they were given; a read message's
    /// come in name order, the one order a parsed document keeps.
    #[must_use]
    pub fn header(&self) -> &[Fragment] {
        &self.header
    }

    /// The body.
    #[must_use]
    pub const fn body(&self) -> &Body {
        &self.body
    }

    /// The body element, when the body is not a fault.
    #[must_use]
    pub const fn payload(&self) -> Option<&Fragment> {
        match &self.body {
            Body::Payload(fragment) => Some(fragment),
            Body::Fault(_) => None,
        }
    }

    /// The fault, when the body is one.
    #[must_use]
    pub const fn fault(&self) -> Option<&Fault> {
        match &self.body {
            Body::Fault(fault) => Some(fault),
            Body::Payload(_) => None,
        }
    }

    /// Consume this message into its body, a fault becoming the error.
    ///
    /// # Errors
    ///
    /// Returns the fault as [`Error::Codec`] carrying its code and string.
    pub fn into_payload(self) -> Result<Fragment> {
        match self.body {
            Body::Payload(fragment) => Ok(fragment),
            Body::Fault(fault) => Err(codec_error(format_smolstr!("{fault}"))),
        }
    }

    /// Read a message from its bytes under default limits.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] when the bytes are not XML, the document
    /// element is not the SOAP 1.1 `Envelope`, or the body does not hold
    /// exactly one element.
    pub fn from_bytes(input: &[u8]) -> Result<Self> {
        Self::from_bytes_with_limits(input, Limits::default())
    }

    /// Read a message from its bytes with explicit limits.
    ///
    /// # Errors
    ///
    /// As [`Self::from_bytes`].
    pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Self> {
        let document = from_bytes_with_limits(input, limits)?;
        Self::from_natural(&document)
    }

    /// Read a message from the natural value of its document.
    ///
    /// # Errors
    ///
    /// As [`Self::from_bytes`].
    pub fn from_natural(document: &Scalar) -> Result<Self> {
        let envelope = Element::root(document)?;
        if !envelope.is(Some(ENVELOPE_NAMESPACE), "Envelope") {
            return Err(codec_error(format_smolstr!(
                "expected the SOAP 1.1 `Envelope` in {ENVELOPE_NAMESPACE:?}, got `{}` in {}",
                envelope.name(),
                envelope
                    .namespace()
                    .map_or_else(|| "no namespace".to_owned(), |held| format!("{held:?}"))
            )));
        }
        // At most one Header (SOAP 1.1 section 4.2): a second one's blocks
        // are never read past, since one may demand to be understood.
        let mut headers = envelope
            .children()
            .filter(|child| child.is(Some(ENVELOPE_NAMESPACE), "Header"));
        let header = headers
            .next()
            .map(|header| {
                header
                    .children()
                    .map(|block| Fragment::from_element(&block))
                    .collect()
            })
            .unwrap_or_default();
        if headers.next().is_some() {
            return Err(codec_error(
                "`Header` appears twice; a SOAP envelope carries at most one",
            ));
        }
        let body = envelope.one_child_in(ENVELOPE_NAMESPACE, "Body")?;
        let mut children = body.children();
        let body = match (children.next(), children.next()) {
            (Some(child), None) if child.is(Some(ENVELOPE_NAMESPACE), "Fault") => {
                Body::Fault(Fault::read(&child)?)
            }
            (Some(child), None) => Body::Payload(Fragment::from_element(&child)),
            (None, _) => {
                return Err(codec_error("the SOAP body holds no element"));
            }
            (Some(_), Some(_)) => {
                return Err(codec_error(
                    "the SOAP body holds several elements where one method call is expected",
                ));
            }
        };
        Ok(Self { header, body })
    }

    /// The natural value of this message: the document the envelope is.
    ///
    /// # Errors
    ///
    /// Returns an error when a header block or the body repeats a name.
    pub fn into_natural(&self) -> Result<Scalar> {
        let mut entries: Vec<(SmolStr, Scalar)> = Vec::with_capacity(4);
        entries.push((
            format_smolstr!("{ATTRIBUTE_PREFIX}xmlns:{PREFIX}"),
            Scalar::from(ENVELOPE_NAMESPACE),
        ));
        if !self.header.is_empty() {
            let header = record(
                self.header
                    .iter()
                    .map(|block| Ok((block.name.clone(), block.natural_value()?)))
                    .collect::<Result<Vec<_>>>()?,
            )?;
            entries.push((format_smolstr!("{PREFIX}:Header"), header));
        }
        let body = match &self.body {
            Body::Payload(fragment) => {
                record([(fragment.name.clone(), fragment.natural_value()?)])?
            }
            Body::Fault(fault) => record([(format_smolstr!("{PREFIX}:Fault"), fault.natural()?)])?,
        };
        entries.push((format_smolstr!("{PREFIX}:Body"), body));
        record([(format_smolstr!("{PREFIX}:Envelope"), record(entries)?)])
    }

    /// Write this message as one XML document, declaration first.
    ///
    /// # Errors
    ///
    /// Returns the XML writer's refusal for a value with no XML spelling.
    pub fn into_bytes(&self) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        self.into_writer(&mut output)?;
        Ok(output)
    }

    /// Write this message to `writer`, declaration first.
    ///
    /// # Errors
    ///
    /// Returns the XML writer's refusal, or the sink's failure.
    pub fn into_writer<W: Write>(&self, writer: W) -> Result<()> {
        // Through the streaming writer rather than the natural value: a
        // record sorts its entries by name, and SOAP 1.1 wants the Header
        // first and a fault's children in the schema's order. What the
        // natural value refuses - a name two blocks share - is refused here
        // too, so the two writers agree on what a message is.
        distinct(&self.header, "header block")?;
        if let Body::Fault(fault) = &self.body {
            distinct(&fault.detail, "detail element")?;
        }
        let mut envelope = EnvelopeWriter::begin(writer, &self.header)?;
        match &self.body {
            Body::Payload(fragment) => {
                fragment.write(envelope.body())?;
                envelope.finish()?;
            }
            Body::Fault(fault) => {
                envelope.fault(fault)?;
            }
        }
        Ok(())
    }
}

/// A message written as it is produced: the envelope and header first, the
/// body element by element, the closing tags last.
///
/// ```
/// use yggdryl::soap::{Envelope, EnvelopeWriter};
///
/// let mut output = Vec::new();
/// let mut envelope = EnvelopeWriter::begin(&mut output, &[])?;
/// std::io::Write::write_all(envelope.body(), b"<Answer xmlns=\"urn:example\">42</Answer>")?;
/// envelope.finish()?;
///
/// let read = Envelope::from_bytes(&output)?;
/// let payload = read.payload().expect("a payload, not a fault");
/// let answer = payload.element();
/// assert_eq!(answer.text(), Some("42"));
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub struct EnvelopeWriter<W: Write> {
    writer: W,
}

impl<W: Write> EnvelopeWriter<W> {
    /// Open the envelope on `writer`, writing the declaration, the envelope's
    /// start tag, the header blocks and the body's start tag.
    ///
    /// # Errors
    ///
    /// Returns the XML writer's refusal for a header block, or the sink's
    /// failure.
    pub fn begin(mut writer: W, header: &[Fragment]) -> Result<Self> {
        writer.write_all(DECLARATION)?;
        write!(
            writer,
            "<{PREFIX}:Envelope xmlns:{PREFIX}=\"{ENVELOPE_NAMESPACE}\">"
        )?;
        if !header.is_empty() {
            write!(writer, "<{PREFIX}:Header>")?;
            for block in header {
                block.write(&mut writer)?;
            }
            write!(writer, "</{PREFIX}:Header>")?;
        }
        write!(writer, "<{PREFIX}:Body>")?;
        Ok(Self { writer })
    }

    /// The body, open for the caller to write its one element into.
    pub fn body(&mut self) -> &mut W {
        &mut self.writer
    }

    /// Write `fault` as the body and close the envelope.
    ///
    /// # Errors
    ///
    /// Returns the XML writer's refusal, or the sink's failure.
    pub fn fault(mut self, fault: &Fault) -> Result<W> {
        fault.write(&mut self.writer)?;
        self.finish()
    }

    /// Close the body and the envelope, handing the sink back.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure.
    pub fn finish(mut self) -> Result<W> {
        write!(self.writer, "</{PREFIX}:Body></{PREFIX}:Envelope>")?;
        Ok(self.writer)
    }
}

/// The one child of `parent` that is `local` in the envelope namespace, or
/// `local` in none; a second one is refused, because two disagreeing children
/// name two messages and neither is chosen.
fn at_most_one<'a>(parent: &'a Element<'_>, local: &str) -> Result<Option<Element<'a>>> {
    let mut found = parent
        .children()
        .filter(|child| child.is_in(ENVELOPE_NAMESPACE, local));
    let first = found.next();
    if found.next().is_some() {
        return Err(codec_error(format_smolstr!(
            "`{local}` appears twice; a SOAP fault carries it at most once"
        )));
    }
    Ok(first)
}

/// Collect into `prefixes`, once each, every prefix `name` and the element
/// names and attribute names under `value` spell - a namespace declaration
/// itself excepted, since it binds rather than uses.
fn used_prefixes<'a>(name: &'a str, value: &'a Scalar, prefixes: &mut Vec<&'a str>) {
    if let Some((prefix, _)) = name.split_once(':') {
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }
    match value {
        Scalar::Struct(entries) => {
            for (key, held) in entries.as_map() {
                if let Some(attribute) = key.strip_prefix(ATTRIBUTE_PREFIX) {
                    if attribute != "xmlns" && !attribute.starts_with("xmlns:") {
                        if let Some((prefix, _)) = attribute.split_once(':') {
                            if !prefixes.contains(&prefix) {
                                prefixes.push(prefix);
                            }
                        }
                    }
                } else if !key.starts_with('#') {
                    used_prefixes(key, held, prefixes);
                }
            }
        }
        other => {
            if let Some(items) = other.as_sequence() {
                for item in items {
                    used_prefixes("", item, prefixes);
                }
            }
        }
    }
}

/// A record over entries that came from distinct elements.
fn record(entries: impl IntoIterator<Item = (SmolStr, Scalar)>) -> Result<Scalar> {
    Scalar::from_struct(entries)
}

/// Refuse a name two of `fragments` share, naming it as `what`.
pub(crate) fn distinct(fragments: &[Fragment], what: &str) -> Result<()> {
    for (index, fragment) in fragments.iter().enumerate() {
        if fragments[..index]
            .iter()
            .any(|earlier| earlier.name == fragment.name)
        {
            return Err(codec_error(format_smolstr!(
                "{what} `{}` repeats; one element per name",
                fragment.name
            )));
        }
    }
    Ok(())
}

pub(crate) fn codec_error(reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: "soap",
        position: 0,
        reason: reason.into(),
    }
}

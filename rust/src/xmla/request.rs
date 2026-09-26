//! The two requests XML for Analysis defines, read out of a SOAP envelope and
//! written into one.
//!
//! A [`Discover`] names a request type, the restrictions that narrow it and
//! the properties the answer is shaped by; an [`Execute`] carries a command -
//! a statement - with the same properties and, optionally, the parameters the
//! statement names. A [`Request`] is either, with the session header a client
//! may send beside it.
//!
//! ```
//! use yggdryl::xmla::{Discover, PropertyList, Request, RequestType, Restrictions};
//!
//! let discover = Discover::new(RequestType::DbschemaTables)
//!     .with_restrictions(Restrictions::new().with("TABLE_CATALOG", "market"))
//!     .with_properties(PropertyList::new().with("Format", "Tabular"));
//! let bytes = Request::from(discover.clone()).into_bytes()?;
//! assert!(bytes.windows(19).any(|window| window == b"<RequestType>DBSCHE"));
//!
//! let read = Request::from_bytes(&bytes)?;
//! assert_eq!(read.discover(), Some(&discover));
//! # Ok::<(), yggdryl::Error>(())
//! ```

use std::io::Write;

use smol_str::{SmolStr, format_smolstr};

use crate::soap::{Body, Envelope, EnvelopeWriter, Fragment};
use crate::xml::{ATTRIBUTE_PREFIX, Element};
use crate::{Error, Result, Scalar};

use super::NAMESPACE;
use super::vocabulary::{Method, PropertyList, RequestType, Restrictions};

/// A Discover: what to describe, narrowed how, answered in what shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discover {
    request_type: RequestType,
    restrictions: Restrictions,
    properties: PropertyList,
}

impl Discover {
    /// A Discover of `request_type`, unrestricted, with no property.
    #[must_use]
    pub fn new(request_type: RequestType) -> Self {
        Self {
            request_type,
            restrictions: Restrictions::new(),
            properties: PropertyList::new(),
        }
    }

    /// Return this Discover narrowed by `restrictions`.
    #[must_use]
    pub fn with_restrictions(mut self, restrictions: Restrictions) -> Self {
        self.restrictions = restrictions;
        self
    }

    /// Return this Discover shaped by `properties`.
    #[must_use]
    pub fn with_properties(mut self, properties: PropertyList) -> Self {
        self.properties = properties;
        self
    }

    /// The request type.
    #[must_use]
    pub const fn request_type(&self) -> &RequestType {
        &self.request_type
    }

    /// The restrictions.
    #[must_use]
    pub const fn restrictions(&self) -> &Restrictions {
        &self.restrictions
    }

    /// The properties.
    #[must_use]
    pub const fn properties(&self) -> &PropertyList {
        &self.properties
    }

    /// The properties, to set.
    pub fn properties_mut(&mut self) -> &mut PropertyList {
        &mut self.properties
    }

    fn read(element: &Element<'_>) -> Result<Self> {
        let request_type = element
            .one_child_in(NAMESPACE, "RequestType")?
            .text()
            .unwrap_or_default()
            .parse()?;
        let restrictions = match at_most_one(element, "Restrictions")? {
            Some(restrictions) => match at_most_one(&restrictions, "RestrictionList")? {
                Some(list) => {
                    let mut restrictions = Restrictions::new();
                    for entry in list.children() {
                        // xmla4js spells several values as `Value` children;
                        // olap4j and this crate repeat the element.
                        let values: Vec<Element<'_>> = entry
                            .children()
                            .filter(|child| child.local_name() == "Value")
                            .collect();
                        if values.is_empty() {
                            restrictions.push(entry.local_name(), entry.text().unwrap_or_default());
                        } else {
                            for value in values {
                                restrictions
                                    .push(entry.local_name(), value.text().unwrap_or_default());
                            }
                        }
                    }
                    restrictions
                }
                None => Restrictions::new(),
            },
            None => Restrictions::new(),
        };
        let properties = read_properties(element)?;
        Ok(Self {
            request_type,
            restrictions,
            properties,
        })
    }

    fn natural(&self) -> Result<Scalar> {
        let mut entries: Vec<(SmolStr, Scalar)> = vec![
            (
                format_smolstr!("{ATTRIBUTE_PREFIX}xmlns"),
                Scalar::from(NAMESPACE),
            ),
            (
                SmolStr::new_static("RequestType"),
                Scalar::from(self.request_type.as_str()),
            ),
        ];
        let list = record(
            self.restrictions
                .entries()
                .iter()
                .map(|(name, values)| {
                    element_name(name)?;
                    let value = match values.as_slice() {
                        [one] => Scalar::from(one.as_str()),
                        many => Scalar::from_sequence(
                            many.iter().map(|value| Scalar::from(value.as_str())),
                        ),
                    };
                    Ok((name.clone(), value))
                })
                .collect::<Result<Vec<_>>>()?,
        )?;
        entries.push((
            SmolStr::new_static("Restrictions"),
            record([(SmolStr::new_static("RestrictionList"), list)])?,
        ));
        entries.push((
            SmolStr::new_static("Properties"),
            properties_natural(&self.properties)?,
        ));
        record(entries)
    }
}

/// What an Execute runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A statement in the provider's command language.
    Statement(String),
    /// Any other command element, kept as spelled: an Analysis Services
    /// scripting verb, a `Cancel`, a provider's own.
    Other(Fragment),
}

impl Command {
    /// The statement text, when the command is one.
    #[must_use]
    pub fn statement(&self) -> Option<&str> {
        match self {
            Self::Statement(text) => Some(text),
            Self::Other(_) => None,
        }
    }

    /// The command's element name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Statement(_) => "Statement",
            Self::Other(fragment) => fragment.name(),
        }
    }
}

/// An Execute: a command, the properties that shape its answer, and the
/// parameters the command names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execute {
    command: Command,
    properties: PropertyList,
    parameters: Vec<(SmolStr, Scalar)>,
}

impl Execute {
    /// An Execute of `statement`, with no property and no parameter.
    pub fn statement(statement: impl Into<String>) -> Self {
        Self {
            command: Command::Statement(statement.into()),
            properties: PropertyList::new(),
            parameters: Vec::new(),
        }
    }

    /// An Execute of any command.
    #[must_use]
    pub const fn new(command: Command) -> Self {
        Self {
            command,
            properties: PropertyList::new(),
            parameters: Vec::new(),
        }
    }

    /// Return this Execute shaped by `properties`.
    #[must_use]
    pub fn with_properties(mut self, properties: PropertyList) -> Self {
        self.properties = properties;
        self
    }

    /// Return this Execute with one more parameter.
    #[must_use]
    pub fn with_parameter(mut self, name: impl Into<SmolStr>, value: Scalar) -> Self {
        self.parameters.push((name.into(), value));
        self
    }

    /// The command.
    #[must_use]
    pub const fn command(&self) -> &Command {
        &self.command
    }

    /// The properties.
    #[must_use]
    pub const fn properties(&self) -> &PropertyList {
        &self.properties
    }

    /// The properties, to set.
    pub fn properties_mut(&mut self) -> &mut PropertyList {
        &mut self.properties
    }

    /// The parameters, in the order written: each a name and its natural
    /// value, text unless the client typed it.
    #[must_use]
    pub fn parameters(&self) -> &[(SmolStr, Scalar)] {
        &self.parameters
    }

    fn read(element: &Element<'_>) -> Result<Self> {
        let command = element.one_child_in(NAMESPACE, "Command")?;
        let mut children = command.children();
        let command = match (children.next(), children.next()) {
            (Some(child), None) if child.is_in(NAMESPACE, "Statement") => {
                Command::Statement(child.text().unwrap_or_default().to_owned())
            }
            (Some(child), None) => Command::Other(Fragment::from_element(&child)),
            (None, _) => {
                return Err(invalid("the Execute command holds no element"));
            }
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "the Execute command holds several elements where one is expected",
                ));
            }
        };
        let properties = read_properties(element)?;
        let parameters = match at_most_one(element, "Parameters")? {
            Some(parameters) => parameters
                .children_in(Some(NAMESPACE), "Parameter")
                .into_iter()
                .chain(parameters.children_in(None, "Parameter"))
                .map(|parameter| {
                    let name = at_most_one(&parameter, "Name")?
                        .and_then(|name| name.text().map(SmolStr::new))
                        .ok_or_else(|| invalid("a Parameter without a Name"))?;
                    // A nil-marked value is null, as a cell marked nil is.
                    let value = at_most_one(&parameter, "Value")?.map_or(Scalar::Null, |value| {
                        if value.is_nil() {
                            Scalar::Null
                        } else {
                            value.value().clone()
                        }
                    });
                    Ok((name, value))
                })
                .collect::<Result<Vec<_>>>()?,
            None => Vec::new(),
        };
        Ok(Self {
            command,
            properties,
            parameters,
        })
    }

    fn natural(&self) -> Result<Scalar> {
        let command = match &self.command {
            Command::Statement(text) => record([(
                SmolStr::new_static("Statement"),
                Scalar::from(text.as_str()),
            )])?,
            // The natural value carries the declarations the fragment
            // resolves by, so the natural envelope and the bytes describe
            // one message.
            Command::Other(fragment) => {
                record([(SmolStr::new(fragment.name()), fragment.natural_value()?)])?
            }
        };
        let mut entries: Vec<(SmolStr, Scalar)> = vec![
            (
                format_smolstr!("{ATTRIBUTE_PREFIX}xmlns"),
                Scalar::from(NAMESPACE),
            ),
            (SmolStr::new_static("Command"), command),
            (
                SmolStr::new_static("Properties"),
                properties_natural(&self.properties)?,
            ),
        ];
        if !self.parameters.is_empty() {
            let parameters = Scalar::from_sequence(
                self.parameters
                    .iter()
                    .map(|(name, value)| {
                        one_value(name, value)?;
                        record([
                            (SmolStr::new_static("Name"), Scalar::from(name.as_str())),
                            (SmolStr::new_static("Value"), value.clone()),
                        ])
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
            entries.push((
                SmolStr::new_static("Parameters"),
                record([(SmolStr::new_static("Parameter"), parameters)])?,
            ));
        }
        record(entries)
    }
}

/// The session header a client sends beside a request.
///
/// `BeginSession` opens one and the server answers a `Session` header with
/// the identifier it chose; `Session` names an open one; `EndSession` closes
/// it. Each is a header block in the XMLA namespace, and a client marks it
/// `mustUnderstand`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Session {
    /// Open a session; the identifier is the server's to choose.
    Begin,
    /// Continue the session `SessionId` names.
    Continue(SmolStr),
    /// Close the session `SessionId` names.
    End(SmolStr),
}

impl Session {
    /// The header element's name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Begin => "BeginSession",
            Self::Continue(_) => "Session",
            Self::End(_) => "EndSession",
        }
    }

    /// The identifier the header names, if it names one.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Begin => None,
            Self::Continue(id) | Self::End(id) => Some(id),
        }
    }

    /// Read the session header out of a message's header blocks, `None`
    /// when there is none.
    ///
    /// # Errors
    ///
    /// Returns an error for a `Session` or `EndSession` block that names no
    /// identifier, or a message carrying more than one session block.
    pub fn read(header: &[Fragment]) -> Result<Option<Self>> {
        let mut found = None;
        for block in header {
            let element = block.element();
            let session = match element.local_name() {
                "BeginSession" if element.is_in(NAMESPACE, "BeginSession") => Self::Begin,
                "Session" if element.is_in(NAMESPACE, "Session") => {
                    Self::Continue(session_id(&element)?)
                }
                "EndSession" if element.is_in(NAMESPACE, "EndSession") => {
                    Self::End(session_id(&element)?)
                }
                _ => continue,
            };
            if found.is_some() {
                return Err(invalid("a request carries more than one session header"));
            }
            found = Some(session);
        }
        Ok(found)
    }

    /// The header block a client sends this session as, marked
    /// `mustUnderstand="1"` the way Excel marks it and the reference
    /// providers read it - the attribute unprefixed, which is the one
    /// spelling olap4j looks up; [`Self::read`] accepts the SOAP-qualified
    /// one as well.
    ///
    /// # Errors
    ///
    /// Returns an error when the block cannot be built, which no identifier
    /// causes.
    pub fn into_fragment(&self) -> Result<Fragment> {
        self.fragment(true)
    }

    /// The header block a provider answers this session with: the same
    /// element without `mustUnderstand`, so a SOAP stack that does not
    /// process sessions is not asked to fault on it - what the reference
    /// providers answer.
    ///
    /// # Errors
    ///
    /// Returns an error when the block cannot be built, which no identifier
    /// causes.
    pub fn into_answer_fragment(&self) -> Result<Fragment> {
        self.fragment(false)
    }

    fn fragment(&self, must_understand: bool) -> Result<Fragment> {
        let mut entries: Vec<(SmolStr, Scalar)> = vec![(
            format_smolstr!("{ATTRIBUTE_PREFIX}xmlns"),
            Scalar::from(NAMESPACE),
        )];
        if must_understand {
            entries.push((
                format_smolstr!("{ATTRIBUTE_PREFIX}mustUnderstand"),
                Scalar::from("1"),
            ));
        }
        if let Some(id) = self.session_id() {
            entries.push((
                format_smolstr!("{ATTRIBUTE_PREFIX}SessionId"),
                Scalar::from(id),
            ));
        }
        Ok(Fragment::new(self.name(), record(entries)?))
    }
}

fn session_id(element: &Element<'_>) -> Result<SmolStr> {
    element
        .attribute("SessionId")
        .and_then(Scalar::as_str)
        .filter(|id| !id.is_empty())
        .map(SmolStr::new)
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "the {} header names no SessionId",
                element.local_name()
            ))
        })
}

/// One request: a Discover or an Execute, and the header blocks sent with
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    header: Vec<Fragment>,
    method: RequestMethod,
}

/// The method a request invokes, with its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestMethod {
    /// A Discover.
    Discover(Discover),
    /// An Execute.
    Execute(Execute),
}

impl Request {
    /// A request with no header block.
    #[must_use]
    pub const fn new(method: RequestMethod) -> Self {
        Self {
            header: Vec::new(),
            method,
        }
    }

    /// Return this request with one more header block.
    #[must_use]
    pub fn with_header(mut self, block: Fragment) -> Self {
        self.header.push(block);
        self
    }

    /// Return this request with a session header.
    ///
    /// # Errors
    ///
    /// Returns the header block's refusal.
    pub fn with_session(self, session: &Session) -> Result<Self> {
        Ok(self.with_header(session.into_fragment()?))
    }

    /// The header blocks.
    #[must_use]
    pub fn header(&self) -> &[Fragment] {
        &self.header
    }

    /// The method invoked.
    #[must_use]
    pub const fn method(&self) -> &RequestMethod {
        &self.method
    }

    /// Which of the two methods this is.
    #[must_use]
    pub const fn kind(&self) -> Method {
        match &self.method {
            RequestMethod::Discover(_) => Method::Discover,
            RequestMethod::Execute(_) => Method::Execute,
        }
    }

    /// The Discover, when the request is one.
    #[must_use]
    pub const fn discover(&self) -> Option<&Discover> {
        match &self.method {
            RequestMethod::Discover(discover) => Some(discover),
            RequestMethod::Execute(_) => None,
        }
    }

    /// The Execute, when the request is one.
    #[must_use]
    pub const fn execute(&self) -> Option<&Execute> {
        match &self.method {
            RequestMethod::Execute(execute) => Some(execute),
            RequestMethod::Discover(_) => None,
        }
    }

    /// The properties of either method.
    #[must_use]
    pub const fn properties(&self) -> &PropertyList {
        match &self.method {
            RequestMethod::Discover(discover) => &discover.properties,
            RequestMethod::Execute(execute) => &execute.properties,
        }
    }

    /// The session header, when one was sent.
    ///
    /// # Errors
    ///
    /// Returns [`Session::read`]'s refusal.
    pub fn session(&self) -> Result<Option<Session>> {
        Session::read(&self.header)
    }

    /// Read a request out of a message.
    ///
    /// # Errors
    ///
    /// Returns an error when the body is a fault, is not `Discover` or
    /// `Execute` in the XMLA namespace, or lacks what the method requires.
    pub fn from_envelope(envelope: &Envelope) -> Result<Self> {
        let payload = match envelope.body() {
            Body::Payload(fragment) => fragment,
            Body::Fault(fault) => {
                return Err(invalid(format_smolstr!(
                    "expected a Discover or Execute, got a {fault}"
                )));
            }
        };
        let element = payload.element();
        let method = if element.is_in(NAMESPACE, "Discover") {
            RequestMethod::Discover(Discover::read(&element)?)
        } else if element.is_in(NAMESPACE, "Execute") {
            RequestMethod::Execute(Execute::read(&element)?)
        } else {
            return Err(invalid(format_smolstr!(
                "expected `Discover` or `Execute` in {NAMESPACE:?}, got `{}` in {}",
                element.name(),
                element
                    .namespace()
                    .map_or_else(|| "no namespace".to_owned(), |held| format!("{held:?}"))
            )));
        };
        Ok(Self {
            header: envelope.header().to_vec(),
            method,
        })
    }

    /// Read a request out of a message's bytes.
    ///
    /// # Errors
    ///
    /// Returns the envelope's refusal or [`Self::from_envelope`]'s.
    pub fn from_bytes(input: &[u8]) -> Result<Self> {
        Self::from_envelope(&Envelope::from_bytes(input)?)
    }

    /// The message this request is sent as, as a natural value: a record,
    /// which sorts the payload's children by name. The bytes a client sends
    /// come from [`Self::into_bytes`], which writes them in the order the
    /// XMLA schema declares.
    ///
    /// # Errors
    ///
    /// Returns an error when a value cannot be spelled.
    pub fn into_envelope(&self) -> Result<Envelope> {
        let payload = match &self.method {
            RequestMethod::Discover(discover) => {
                Fragment::new(Method::Discover.as_str(), discover.natural()?)
            }
            RequestMethod::Execute(execute) => {
                Fragment::new(Method::Execute.as_str(), execute.natural()?)
            }
        };
        let mut envelope = Envelope::from_payload(payload);
        for block in &self.header {
            envelope = envelope.with_header(block.clone());
        }
        Ok(envelope)
    }

    /// The message this request is sent as, written.
    ///
    /// # Errors
    ///
    /// Returns [`Self::into_writer`]'s refusal.
    pub fn into_bytes(&self) -> Result<Vec<u8>> {
        self.into_writer(Vec::new())
    }

    /// Write the message this request is sent as, declaration first, the
    /// payload's children in the order the XMLA schema declares them -
    /// `RequestType`, `Restrictions`, `Properties` for a Discover; `Command`,
    /// `Properties`, `Parameters` for an Execute - which a natural value,
    /// being a record sorted by name, cannot keep.
    ///
    /// # Errors
    ///
    /// Returns the XML writer's refusal for a name or a value with no XML
    /// spelling, or the sink's failure.
    pub fn into_writer<W: Write>(&self, writer: W) -> Result<W> {
        crate::soap::distinct(&self.header, "header block")?;
        let mut envelope = EnvelopeWriter::begin(writer, &self.header)?;
        let body = envelope.body();
        match &self.method {
            RequestMethod::Discover(discover) => {
                write!(
                    body,
                    "<{} xmlns=\"{NAMESPACE}\"><RequestType>",
                    Method::Discover.as_str()
                )?;
                crate::xml::write_element_text(body, discover.request_type.as_str())?;
                write!(body, "</RequestType><Restrictions>")?;
                if discover.restrictions.is_empty() {
                    write!(body, "<RestrictionList/>")?;
                } else {
                    write!(body, "<RestrictionList>")?;
                    for (name, values) in discover.restrictions.entries() {
                        for value in values {
                            crate::xml::write_fragment(body, name, &Scalar::from(value.as_str()))?;
                        }
                    }
                    write!(body, "</RestrictionList>")?;
                }
                write!(body, "</Restrictions>")?;
                write_properties(body, &discover.properties)?;
                write!(body, "</{}>", Method::Discover.as_str())?;
            }
            RequestMethod::Execute(execute) => {
                write!(
                    body,
                    "<{} xmlns=\"{NAMESPACE}\"><Command>",
                    Method::Execute.as_str()
                )?;
                match &execute.command {
                    Command::Statement(text) => {
                        write!(body, "<Statement>")?;
                        crate::xml::write_element_text(body, text)?;
                        write!(body, "</Statement>")?;
                    }
                    // Under the Execute's default namespace: a command read
                    // in no namespace undeclares it, one built here takes it.
                    Command::Other(fragment) => fragment.write_under(body, Some(NAMESPACE))?,
                }
                write!(body, "</Command>")?;
                write_properties(body, &execute.properties)?;
                if !execute.parameters.is_empty() {
                    write!(body, "<Parameters>")?;
                    for (name, value) in &execute.parameters {
                        one_value(name, value)?;
                        write!(body, "<Parameter><Name>")?;
                        crate::xml::write_element_text(body, name)?;
                        write!(body, "</Name>")?;
                        crate::xml::write_fragment(body, "Value", value)?;
                        write!(body, "</Parameter>")?;
                    }
                    write!(body, "</Parameters>")?;
                }
                write!(body, "</{}>", Method::Execute.as_str())?;
            }
        }
        envelope.finish()
    }
}

/// Write `<Properties><PropertyList>` with one element per property.
fn write_properties<W: Write>(writer: &mut W, properties: &PropertyList) -> Result<()> {
    if properties.is_empty() {
        write!(writer, "<Properties><PropertyList/></Properties>")?;
        return Ok(());
    }
    write!(writer, "<Properties><PropertyList>")?;
    for (name, value) in properties.entries() {
        crate::xml::write_fragment(writer, name, &Scalar::from(value.as_str()))?;
    }
    write!(writer, "</PropertyList></Properties>")?;
    Ok(())
}

impl From<Discover> for Request {
    fn from(discover: Discover) -> Self {
        Self::new(RequestMethod::Discover(discover))
    }
}

impl From<Execute> for Request {
    fn from(execute: Execute) -> Self {
        Self::new(RequestMethod::Execute(execute))
    }
}

/// The `Properties/PropertyList` of either method; absent is empty.
fn read_properties(element: &Element<'_>) -> Result<PropertyList> {
    let Some(properties) = at_most_one(element, "Properties")? else {
        return Ok(PropertyList::new());
    };
    let Some(list) = at_most_one(&properties, "PropertyList")? else {
        return Ok(PropertyList::new());
    };
    let mut read = PropertyList::new();
    for entry in list.children() {
        // One value per property: two spellings of one name, in whatever
        // case, disagree rather than one winning.
        let name = entry.local_name();
        if let Some((held, _)) = read
            .entries()
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
        {
            return Err(invalid(if held == name {
                format_smolstr!("the property `{name}` appears twice in the PropertyList")
            } else {
                format_smolstr!(
                    "the property `{name}` appears twice in the PropertyList, as `{held}` and `{name}`"
                )
            }));
        }
        read.set(name, entry.text().unwrap_or_default());
    }
    Ok(read)
}

/// `<Properties><PropertyList>...</PropertyList></Properties>`.
fn properties_natural(properties: &PropertyList) -> Result<Scalar> {
    let list = record(
        properties
            .entries()
            .iter()
            .map(|(name, value)| {
                element_name(name)?;
                Ok((name.clone(), Scalar::from(value.as_str())))
            })
            .collect::<Result<Vec<_>>>()?,
    )?;
    record([(SmolStr::new_static("PropertyList"), list)])
}

/// Refuse a property, restriction or parameter name that is no XML element
/// name - `@xmlns`, `#text`, `a b` - before the natural value would read it
/// as an attribute, a text or nothing, the way the bytes refuse it.
fn element_name(name: &str) -> Result<&str> {
    let mut characters = name.chars();
    let starts = characters
        .next()
        .is_some_and(|first| crate::xml::is_name_start(first) && first != ':');
    if !starts
        || !characters.all(|character| crate::xml::is_name_char(character) && character != ':')
    {
        return Err(invalid(format_smolstr!(
            "`{name}` is not an XML element name, so no XMLA argument spells it"
        )));
    }
    Ok(name)
}

/// Refuse a parameter value that is a sequence: XMLA's `Value` carries one
/// value, and repeated `Value` elements would read as several parameters'.
fn one_value(name: &str, value: &Scalar) -> Result<()> {
    if value.as_sequence().is_some() {
        return Err(invalid(format_smolstr!(
            "parameter `{name}` holds a sequence, and a parameter's Value is one value"
        )));
    }
    Ok(())
}

/// The one child of `parent` that is `local` in the XMLA namespace or in
/// none, refused when it appears twice: every argument of a request is
/// declared once, and two disagreeing ones name two requests.
fn at_most_one<'a>(parent: &'a Element<'_>, local: &str) -> Result<Option<Element<'a>>> {
    let mut found = parent
        .children()
        .filter(|child| child.is_in(NAMESPACE, local));
    let first = found.next();
    if found.next().is_some() {
        return Err(invalid(format_smolstr!(
            "`{local}` appears twice; a request carries it once"
        )));
    }
    Ok(first)
}

/// A record over entries that name distinct elements.
fn record(entries: impl IntoIterator<Item = (SmolStr, Scalar)>) -> Result<Scalar> {
    Scalar::from_struct(entries)
}

pub(crate) fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.xmla"),
        reason: reason.into(),
    }
}

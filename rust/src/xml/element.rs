//! A namespace-aware view over the natural XML value.
//!
//! The codec keeps a name as the document spells it, prefix included, and a
//! namespace declaration as the `@xmlns` or `@xmlns:prefix` attribute it is
//! written as. A protocol reads an element by the namespace it is *in* rather
//! than by the prefix its author chose - `SOAP-ENV:Envelope`, `soap:Envelope`
//! and a bare `Envelope` under a default declaration are one element - so this
//! is the one place a prefix resolves. An [`Element`] borrows one entry of
//! the natural value together with the declarations in scope above it,
//! answers its namespace and local name, and hands out its children under a
//! scope extended by their own declarations.
//!
//! ```
//! use yggdryl::xml::{Element, from_xml_scalar};
//!
//! let document = from_xml_scalar(
//!     "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
//!        <s:Body><Discover xmlns=\"urn:schemas-microsoft-com:xml-analysis\">\
//!          <RequestType>DISCOVER_DATASOURCES</RequestType>\
//!        </Discover></s:Body></s:Envelope>",
//! )?;
//! let envelope = Element::root(&document)?;
//! assert_eq!(envelope.local_name(), "Envelope");
//! assert_eq!(envelope.namespace(), Some("http://schemas.xmlsoap.org/soap/envelope/"));
//!
//! let body = envelope
//!     .child(Some("http://schemas.xmlsoap.org/soap/envelope/"), "Body")
//!     .expect("the body");
//! let discover = body.children().next().expect("the one method element");
//! assert_eq!(discover.namespace(), Some("urn:schemas-microsoft-com:xml-analysis"));
//! let request_type = discover
//!     .child(Some("urn:schemas-microsoft-com:xml-analysis"), "RequestType")
//!     .expect("the request type");
//! assert_eq!(request_type.text(), Some("DISCOVER_DATASOURCES"));
//! # Ok::<(), yggdryl::Error>(())
//! ```

use std::borrow::Cow;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar};

use super::{ATTRIBUTE_PREFIX, TEXT_KEY};

/// The namespace of XML Schema's own vocabulary: `xsd:schema`,
/// `xsd:element`, the `xsd:string` family of type names.
pub const XSD_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";

/// The namespace of the XML Schema instance attributes, `xsi:nil` and
/// `xsi:type` among them.
pub const XSI_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The reserved `xml` prefix, bound by the XML recommendation itself and
/// never declared.
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// The namespace declarations in scope at one element: each prefix - the
/// empty prefix for the default namespace - and the namespace it names.
///
/// Later bindings shadow earlier ones, so a child that redeclares a prefix
/// pushes its binding and every lookup reads from the end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    bindings: Vec<(SmolStr, SmolStr)>,
}

impl Scope {
    /// The scope with no declaration, which is what a document starts in.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    /// Bind `prefix` to `namespace`; the empty prefix is the default
    /// namespace, and an empty namespace undeclares.
    pub fn bind(&mut self, prefix: impl Into<SmolStr>, namespace: impl Into<SmolStr>) {
        self.bindings.push((prefix.into(), namespace.into()));
    }

    /// Return this scope with `prefix` bound to `namespace`.
    #[must_use]
    pub fn with(mut self, prefix: impl Into<SmolStr>, namespace: impl Into<SmolStr>) -> Self {
        self.bind(prefix, namespace);
        self
    }

    /// The namespace `prefix` names here, the innermost binding winning; the
    /// empty prefix asks for the default namespace, and an undeclared
    /// default is no namespace.
    #[must_use]
    pub fn resolve(&self, prefix: &str) -> Option<&str> {
        if prefix == "xml" {
            return Some(XML_NAMESPACE);
        }
        self.bindings
            .iter()
            .rev()
            .find(|(held, _)| held == prefix)
            .map(|(_, namespace)| namespace.as_str())
            .filter(|namespace| !namespace.is_empty())
    }

    /// Every binding in declaration order, outermost first, a rebound prefix
    /// appearing once per binding: `("", namespace)` is a default namespace,
    /// `("", "")` its undeclaration.
    pub fn bindings(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.bindings
            .iter()
            .map(|(prefix, namespace)| (prefix.as_str(), namespace.as_str()))
    }

    /// The prefix bound to `namespace` here, the innermost binding winning;
    /// `Some("")` when it is the default namespace, and `Some("xml")` for the
    /// namespace the recommendation binds itself. A prefix a later binding
    /// rebound no longer names the namespace, so it is never the answer:
    /// `resolve(prefix_of(namespace))` is `namespace` whenever there is one.
    #[must_use]
    pub fn prefix_of(&self, namespace: &str) -> Option<&str> {
        self.bindings
            .iter()
            .rev()
            .filter(|(_, held)| held == namespace)
            .map(|(prefix, _)| prefix.as_str())
            .find(|prefix| self.resolve(prefix) == Some(namespace))
            .or((namespace == XML_NAMESPACE).then_some("xml"))
    }

    /// Extend this scope with the declarations the attributes of one element
    /// carry: `@xmlns` binds the default namespace, `@xmlns:p` binds `p`.
    fn extended(&self, entries: &std::collections::BTreeMap<SmolStr, Scalar>) -> Self {
        let mut scope = self.clone();
        for (key, value) in entries {
            let Some(attribute) = key.strip_prefix(ATTRIBUTE_PREFIX) else {
                continue;
            };
            let Some(text) = value.as_str() else {
                continue;
            };
            if attribute == "xmlns" {
                scope.bind("", text);
            } else if let Some(prefix) = attribute.strip_prefix("xmlns:") {
                scope.bind(prefix, text);
            }
        }
        scope
    }
}

/// One element of a natural XML value: its name as spelled, its value, and
/// the namespace declarations in scope at it.
///
/// An element borrows from the document it was read out of. The one item it
/// may own is a row read out of a sequence laid out as a column, which lends
/// nothing to borrow; a document the parser answers never holds one.
#[derive(Clone, Debug)]
pub struct Element<'a> {
    name: &'a str,
    value: Cow<'a, Scalar>,
    scope: Scope,
}

impl<'a> Element<'a> {
    /// The document element of a natural document: the one entry the record
    /// names, read in the empty scope.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] when `document` is not a record with exactly
    /// one entry, which is the shape every XML door here answers.
    pub fn root(document: &'a Scalar) -> Result<Self> {
        let Some(entries) = document.as_struct() else {
            return Err(codec_error(format_smolstr!(
                "expected an XML document, the record naming its document element, got {}",
                document.kind()
            )));
        };
        let mut entries = entries.iter();
        let (name, value) = match (entries.next(), entries.next()) {
            (Some(root), None) => root,
            (None, _) => {
                return Err(codec_error(
                    "expected an XML document naming its document element, got an empty record",
                ));
            }
            (Some(_), Some(_)) => {
                return Err(codec_error(
                    "expected an XML document naming one document element, got several entries",
                ));
            }
        };
        Ok(Self::new(name, value, &Scope::new()))
    }

    /// View `value` as the element `name` names, under the declarations of
    /// `scope` extended by the element's own.
    #[must_use]
    pub fn new(name: &'a str, value: &'a Scalar, scope: &Scope) -> Self {
        Self::held(name, Cow::Borrowed(value), scope)
    }

    fn held(name: &'a str, value: Cow<'a, Scalar>, scope: &Scope) -> Self {
        let scope = match value.as_struct() {
            Some(entries) => scope.extended(entries),
            None => scope.clone(),
        };
        Self { name, value, scope }
    }

    /// The name exactly as the document spells it, prefix included.
    #[must_use]
    pub const fn name(&self) -> &'a str {
        self.name
    }

    /// The name past its prefix: `Envelope` for `SOAP-ENV:Envelope`.
    #[must_use]
    pub fn local_name(&self) -> &'a str {
        match self.name.split_once(':') {
            Some((_, local)) => local,
            None => self.name,
        }
    }

    /// The prefix the name carries, `None` for an unprefixed name.
    #[must_use]
    pub fn prefix(&self) -> Option<&'a str> {
        self.name.split_once(':').map(|(prefix, _)| prefix)
    }

    /// The namespace this element is in: what its prefix resolves to, or the
    /// default namespace in scope for an unprefixed name.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.scope.resolve(self.prefix().unwrap_or(""))
    }

    /// The declarations in scope at this element.
    #[must_use]
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The natural value of this element.
    #[must_use]
    pub fn value(&self) -> &Scalar {
        &self.value
    }

    /// Whether this element is `local` in `namespace`.
    ///
    /// `None` matches an element in no namespace; a prefixed name whose prefix
    /// is not declared is in no namespace either, which is what the
    /// recommendation says of it.
    #[must_use]
    pub fn is(&self, namespace: Option<&str>, local: &str) -> bool {
        self.local_name() == local && self.namespace() == namespace
    }

    /// Whether this element is `local` in `namespace`, or `local` in no
    /// namespace at all.
    ///
    /// This is the intake reading of a protocol element: a client that
    /// declares the protocol's namespace and one that qualifies nothing both
    /// mean the element the specification names, and only an element in some
    /// *other* namespace is a different element.
    #[must_use]
    pub fn is_in(&self, namespace: &str, local: &str) -> bool {
        self.local_name() == local && matches!(self.namespace(), Some(held) if held == namespace)
            || self.is(None, local)
    }

    /// The element's own text: a leaf's, or the `#text` entry beside its
    /// attributes or children. `None` for a self-closed element, which is
    /// null, and for an element holding children and no text.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match &*self.value {
            Scalar::Struct(entries) => entries.as_map().get(TEXT_KEY).and_then(Scalar::as_str),
            Scalar::Null => None,
            leaf => leaf.as_str(),
        }
    }

    /// Whether this element is absent: `<a/>`, or one marked `nil` in the
    /// XML Schema instance namespace under whatever prefix the document bound
    /// to it - `xsi:nil="true"`, WCF's `i:nil="true"`, `xsi:nil="1"`.
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.value.is_null()
            || self
                .attribute_in(Some(XSI_NAMESPACE), "nil")
                .is_some_and(|marked| matches!(marked.trim(), "true" | "1"))
    }

    /// The value of the attribute spelled `name`, prefix included.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&Scalar> {
        let entries = self.value.as_struct()?;
        let key = format_smolstr!("{ATTRIBUTE_PREFIX}{name}");
        entries.get(&key)
    }

    /// The text of the attribute whose local name is `local` and whose
    /// prefix resolves to `namespace`; an unprefixed attribute is in no
    /// namespace, as the recommendation reads it, and a namespace
    /// declaration - `xmlns`, `xmlns:p` - is a binding rather than an
    /// attribute, so no lookup reaches one.
    #[must_use]
    pub fn attribute_in(&self, namespace: Option<&str>, local: &str) -> Option<&str> {
        let entries = self.value.as_struct()?;
        entries.iter().find_map(|(key, value)| {
            let attribute = key.strip_prefix(ATTRIBUTE_PREFIX)?;
            let (prefix, held) = match attribute.split_once(':') {
                Some((prefix, held)) => (Some(prefix), held),
                None => (None, attribute),
            };
            if attribute == "xmlns" || prefix == Some("xmlns") {
                return None;
            }
            if held != local {
                return None;
            }
            let resolved = prefix.and_then(|prefix| self.scope.resolve(prefix));
            (resolved == namespace).then(|| value.as_str()).flatten()
        })
    }

    /// Every attribute as `(name as spelled, value)`, in name order.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, &Scalar)> + '_ {
        self.value
            .as_struct()
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter_map(|(key, value)| key.strip_prefix(ATTRIBUTE_PREFIX).map(|name| (name, value)))
    }

    /// Every child element, each under this element's scope.
    ///
    /// Children sharing a name come in document order; children of different
    /// names come in name order, which is the one order the natural value
    /// keeps.
    pub fn children(&self) -> impl Iterator<Item = Element<'_>> + '_ {
        self.value
            .as_struct()
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter(|(key, _)| !key.starts_with(ATTRIBUTE_PREFIX) && !key.starts_with('#'))
            .flat_map(move |(key, value)| Children {
                name: key.as_str(),
                value,
                index: 0,
                scope: &self.scope,
            })
    }

    /// The child elements named `local` in `namespace`, in the order
    /// [`Self::children`] answers them: document order within one spelling,
    /// name order across the spellings one namespace may carry (`k` beside
    /// `p:k`), because a parsed document keeps no other.
    pub fn children_in(&self, namespace: Option<&str>, local: &str) -> Vec<Element<'_>> {
        self.children()
            .filter(|child| child.is(namespace, local))
            .collect()
    }

    /// The first child element that is `local` in `namespace`, in the order
    /// [`Self::children`] answers them.
    #[must_use]
    pub fn child(&self, namespace: Option<&str>, local: &str) -> Option<Element<'_>> {
        self.children().find(|child| child.is(namespace, local))
    }

    /// The first child element that is `local` in `namespace`, or `local` in
    /// no namespace - the intake reading [`Self::is_in`] describes.
    #[must_use]
    pub fn child_in(&self, namespace: &str, local: &str) -> Option<Element<'_>> {
        self.children().find(|child| child.is_in(namespace, local))
    }

    /// Exactly one child that is `local` in `namespace` or unqualified,
    /// refused by name when it occurs no time or more than once.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] naming the element and the count found.
    pub fn one_child_in(&self, namespace: &str, local: &str) -> Result<Element<'_>> {
        let mut found = self
            .children()
            .filter(|child| child.is_in(namespace, local));
        match (found.next(), found.next()) {
            (Some(child), None) => Ok(child),
            (None, _) => Err(codec_error(format_smolstr!(
                "expected one `{local}` element under `{}`, found none",
                self.name
            ))),
            (Some(_), Some(_)) => Err(codec_error(format_smolstr!(
                "expected one `{local}` element under `{}`, found several",
                self.name
            ))),
        }
    }
}

/// The children one entry of a record holds: one element per item of a
/// repeated element, one for a single child.
struct Children<'a> {
    name: &'a str,
    value: &'a Scalar,
    index: usize,
    scope: &'a Scope,
}

impl<'a> Iterator for Children<'a> {
    type Item = Element<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.index;
        let item = match self.value {
            Scalar::Serie(_)
            | Scalar::SerieView(_)
            | Scalar::FixedSizeSerie(_)
            | Scalar::LargeSerie(_)
            | Scalar::LargeSerieView(_) => match self.value.as_sequence() {
                Some(items) => Cow::Borrowed(items.get(index)?),
                // A column lends no row: each is built once, and owned.
                None => Cow::Owned(self.value.as_serie()?.get(index)?.into_owned()),
            },
            single if index == 0 => Cow::Borrowed(single),
            _ => return None,
        };
        self.index += 1;
        Some(Element::held(self.name, item, self.scope))
    }
}

fn codec_error(reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: "xml",
        position: 0,
        reason: reason.into(),
    }
}

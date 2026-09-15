//! The `xml:` vocabulary, on the field views that carry it.
//!
//! A document spells more about a column than a datatype can hold, and losing
//! any of it makes a read one-way. Four facts are kept, and each is kept
//! because something reads it back:
//!
//! - **`xml:namespace`** - the URI the element or attribute was bound in. A
//!   prefix names a document's own vocabulary and is dropped, but the URI is
//!   what says two columns called `price` are the same column or two different
//!   ones, and it is what a write binds again.
//! - **`xml:kind`** - whether the column was written as an element, an
//!   attribute, or the element's own characters. A read accepts all three
//!   spellings for one name; without this a write has to pick one, and a
//!   document that declared `Ccy` an attribute comes back with it as a child.
//! - **`xml:name`** - the name the document used, when it is not the column's
//!   name. XMLA escapes a space as `_x0020_`, so `Order Details` travels as
//!   `Order_x0020_Details`: the column is called what it is called, and the
//!   wire spelling is this.
//! - **`xml:type`** - the XSD type the column was declared as, when a schema
//!   declared one. `xs:decimal` with no facets, `xs:integer`, `xs:anyURI` and
//!   `xs:QName` all have to travel as text because no fixed-width datatype can
//!   hold what they allow; this is what a write spells back, and what a reader
//!   consults before deciding a column is merely a string.
//!
//! Nothing else is stored. A facet that a datatype already carries - a decimal
//! scale, a string bound - has an owner, and a second copy here would be a
//! second answer.

use std::fmt;
use std::str::FromStr;

use super::{XmlField, XmlFieldMut};
use crate::{Error, Result};

/// The namespace URI an element or attribute was bound in.
const NAMESPACE: &str = "namespace";
/// How the column was spelled in the document.
const KIND: &str = "kind";
/// The name the document used, when it differs from the column's.
const NAME: &str = "name";
/// The XSD type a schema declared for the column.
const TYPE: &str = "type";

/// What a spelling is, spelled once for every refusal.
const KIND_SHAPE: &str = "one of element, attribute, text";

/// How a column was spelled in the document.
///
/// XML says the same fact three ways and a row has one cell for it, so a read
/// accepts all three and this is what lets a write put it back the way it was
/// found.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum XmlKind {
    /// A child element: `<Ccy>EUR</Ccy>`. The default, and what an undeclared
    /// column is written as.
    #[default]
    Element,
    /// An attribute of the row element: `<Amt Ccy="EUR"/>`.
    Attribute,
    /// The element's own characters, beside its attributes.
    Text,
}

impl XmlKind {
    /// Every spelling, in canonical order.
    pub const ALL: [Self; 3] = [Self::Element, Self::Attribute, Self::Text];

    /// The canonical spelling of this kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Element => "element",
            Self::Attribute => "attribute",
            Self::Text => "text",
        }
    }

    /// Whether this column is written as an attribute.
    pub const fn is_attribute(self) -> bool {
        matches!(self, Self::Attribute)
    }

    /// Whether this column is the element's own characters.
    pub const fn is_text(self) -> bool {
        matches!(self, Self::Text)
    }
}

impl FromStr for XmlKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "element" => Ok(Self::Element),
            "attribute" => Ok(Self::Attribute),
            "text" => Ok(Self::Text),
            other => Err(Error::Parse {
                target: "xml:kind",
                position: 0,
                reason: crate::text::expected_got(KIND_SHAPE, other),
            }),
        }
    }
}

impl fmt::Display for XmlKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl XmlField<'_> {
    /// The namespace URI this column was bound in, if any.
    pub fn namespace(&self) -> Option<&str> {
        self.get(NAMESPACE)
    }

    /// How this column was spelled in the document.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the stored spelling is not one this
    /// vocabulary names.
    pub fn kind(&self) -> Result<XmlKind> {
        self.get(KIND)
            .map_or(Ok(XmlKind::Element), XmlKind::from_str)
    }

    /// The name the document used, when it is not this column's name.
    pub fn name(&self) -> Option<&str> {
        self.get(NAME)
    }

    /// The XSD type a schema declared for this column, if one did.
    pub fn declared_type(&self) -> Option<&str> {
        self.get(TYPE)
    }

    /// The name a document spells this column with.
    ///
    /// The wire name when one was retained, and the column's own name
    /// otherwise - so a writer asks one question rather than two.
    pub fn wire_name(&self) -> &str {
        self.name().unwrap_or_else(|| self.as_field().name())
    }
}

impl XmlFieldMut<'_> {
    /// Bind this column to a namespace URI, or clear it.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure; the field is unchanged on one.
    pub fn set_namespace(&mut self, namespace: Option<&str>) -> Result<()> {
        self.set_or_clear(NAMESPACE, namespace)
    }

    /// Record how this column is spelled in a document.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure; the field is unchanged on one.
    pub fn set_kind(&mut self, kind: XmlKind) -> Result<()> {
        // The default spelling is what an absent key already means, so it is
        // cleared rather than written: one state, one spelling.
        if matches!(kind, XmlKind::Element) {
            return self.set_or_clear(KIND, None);
        }
        self.set_or_clear(KIND, Some(kind.as_str()))
    }

    /// Record the name a document spells this column with, or clear it.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure; the field is unchanged on one.
    pub fn set_name(&mut self, name: Option<&str>) -> Result<()> {
        // A wire name equal to the column's own says nothing.
        let name = name.filter(|name| *name != self.as_field().name());
        self.set_or_clear(NAME, name)
    }

    /// Record the XSD type a schema declared, or clear it.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure; the field is unchanged on one.
    pub fn set_declared_type(&mut self, declared: Option<&str>) -> Result<()> {
        self.set_or_clear(TYPE, declared)
    }

    /// Set one property, or remove it when there is nothing to say.
    fn set_or_clear(&mut self, property: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => {
                self.insert(property, value)?;
                Ok(())
            }
            None => {
                self.remove(property);
                Ok(())
            }
        }
    }
}

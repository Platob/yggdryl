//! An XML Schema document as one [`Field`].
//!
//! A document states text and shape; a schema states what that text *is*. So
//! this is the door that makes an XML read typed without inference: the schema
//! produces a [`Field`], and the declared-field path every other read already
//! has does the rest. There is no second reader and no schema-shaped option.
//!
//! # What maps, and what is kept instead
//!
//! The eight bounded integer types, the two floats, the binaries, the boolean
//! and a decimal with both digit facets map onto a datatype exactly. The rest
//! are kept as text with `xml:type` recording what the schema called them,
//! because widening them would invent a fact:
//!
//! - **An unfaceted `xs:decimal` is unbounded in magnitude and in scale** - the
//!   value space is every `i / 10ⁿ` - so no fixed precision holds it. Only
//!   `totalDigits` with `fractionDigits` says which decimal it is.
//! - **`xs:integer` has no width.** A conforming instance can exceed any of
//!   them, so `Int64` would be a guess.
//! - **The five `g*` types are partial calendar values**, not instants: reading
//!   `gYear` as a date invents a month and a day.
//! - **A temporal whose timezone is optional is two different columns** - one
//!   value with an offset and one without - so only `explicitTimezone` decides.
//! - **`xs:anyURI` is any string** in XSD 1.1; a URL datatype is narrower and
//!   would refuse conformant instances.
//!
//! Two types are refused by name because no cell of them can exist or travel:
//! `xs:error` has an empty value space, and `xs:NOTATION` means only what the
//! schema that declared it means.

use smol_str::{SmolStr, format_smolstr};

use crate::text::Limits;
use crate::types::protocol::XmlKind;
use crate::{DataType, Error, Field, Result, TimeUnit, Timezone};

use super::reader::{Attr, Cursor, Step, codec_error, quoted};

/// The XML Schema namespace, which is what makes an element a declaration.
const XSD_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";

/// One element of a schema document, in the order it was written.
///
/// A schema is read whole rather than streamed: it is a bounded document read
/// once at a boundary, and its *order* is the column order, which a sorted
/// record would lose.
#[derive(Debug)]
struct Node {
    name: SmolStr,
    attributes: Vec<Attr>,
    children: Vec<Node>,
}

impl Node {
    /// The value of one attribute, by local name.
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name.local() == name)
            .map(|attribute| attribute.value.as_str())
    }

    /// Every child with one local name.
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// The first child with one local name.
    fn child<'a>(&'a self, name: &str) -> Option<&'a Node> {
        self.children.iter().find(|child| child.name == name)
    }
}

/// Read one schema document and answer the field its root element declares.
///
/// `root` names which global element to read when a schema declares more than
/// one; a schema declaring exactly one needs no name.
///
/// # Errors
///
/// Returns a parse or limit failure, or a refusal naming the construct or the
/// type a row cannot hold.
pub fn field_from_xsd(input: &[u8], limits: Limits, root: Option<&str>) -> Result<Field> {
    crate::text::check_input_size(input, limits, "xml")?;
    let schema = read_schema(input, limits)?;
    let globals = Globals::of(&schema);
    let element = globals.root(root)?;
    let field = globals.element_field(element, 0)?;
    Ok(field.with_nullable(false))
}

/// Read the schema document into its ordered tree.
fn read_schema(input: &[u8], limits: Limits) -> Result<Node> {
    let mut cursor = Cursor::new(input, limits);
    let Step::Open {
        name,
        attributes,
        empty,
        ..
    } = cursor.next()?
    else {
        return Err(codec_error(
            cursor.position(),
            "expected an XML Schema document",
        ));
    };
    if name.local() != "schema" || name.namespace() != Some(XSD_NAMESPACE) {
        return Err(codec_error(
            cursor.position(),
            format_smolstr!(
                "expected <xs:schema> in the XML Schema namespace, got <{}>",
                quoted(name.local())
            ),
        ));
    }
    let mut node = Node {
        name: SmolStr::new(name.local()),
        attributes,
        children: Vec::new(),
    };
    if !empty {
        read_children(&mut cursor, &mut node)?;
    }
    Ok(node)
}

/// Fill one node's children from the walk, to its own close.
fn read_children(cursor: &mut Cursor<'_>, parent: &mut Node) -> Result<()> {
    loop {
        match cursor.next()? {
            Step::Open {
                name,
                attributes,
                empty,
                ..
            } => {
                let mut child = Node {
                    name: SmolStr::new(name.local()),
                    attributes,
                    children: Vec::new(),
                };
                if !empty {
                    read_children(cursor, &mut child)?;
                }
                parent.children.push(child);
            }
            Step::Close { .. } => return Ok(()),
            Step::End => {
                return Err(codec_error(
                    cursor.position(),
                    format_smolstr!("expected <{}> to close", quoted(&parent.name)),
                ));
            }
        }
    }
}

/// The named declarations one schema document holds.
struct Globals<'schema> {
    schema: &'schema Node,
    target: Option<&'schema str>,
}

impl<'schema> Globals<'schema> {
    fn of(schema: &'schema Node) -> Self {
        Self {
            target: schema.attribute("targetNamespace"),
            schema,
        }
    }

    /// The global element a caller asked for, or the only one there is.
    fn root(&self, wanted: Option<&str>) -> Result<&'schema Node> {
        let mut elements = self.schema.children_named("element");
        match wanted {
            Some(wanted) => elements
                .find(|element| element.attribute("name") == Some(wanted))
                .ok_or_else(|| {
                    codec_error(
                        0,
                        format_smolstr!(
                            "expected the schema to declare <{}>, and it does not",
                            quoted(wanted)
                        ),
                    )
                }),
            None => {
                let first = elements.next().ok_or_else(|| {
                    codec_error(0, "expected the schema to declare a global element")
                })?;
                if elements.next().is_some() {
                    return Err(codec_error(
                        0,
                        "expected one global element, got several; name the one to read",
                    ));
                }
                Ok(first)
            }
        }
    }

    /// Look one named complex type up.
    fn complex_type(&self, name: &str) -> Option<&'schema Node> {
        self.schema
            .children_named("complexType")
            .find(|node| node.attribute("name") == Some(name))
    }

    /// Look one named simple type up.
    fn simple_type(&self, name: &str) -> Option<&'schema Node> {
        self.schema
            .children_named("simpleType")
            .find(|node| node.attribute("name") == Some(name))
    }

    /// Build the field one `xs:element` declares.
    fn element_field(&self, element: &Node, depth: usize) -> Result<Field> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(codec_error(
                0,
                format_smolstr!("XML Schema nesting exceeds the limit of {MAX_SCHEMA_DEPTH}"),
            ));
        }
        let name = element
            .attribute("name")
            .ok_or_else(|| codec_error(0, "expected every declared element to carry a name"))?;
        // `minOccurs="0"` and `nillable="true"` both say a value may be absent.
        let optional = element
            .attribute("minOccurs")
            .is_some_and(|value| value == "0")
            || element
                .attribute("nillable")
                .is_some_and(|value| value == "true");
        let repeated = element
            .attribute("maxOccurs")
            .is_some_and(|value| value == "unbounded" || value.parse::<u64>().is_ok_and(|n| n > 1));

        let (dtype, declared) = match element.attribute("type") {
            Some(declared) => self.named_type(declared, depth)?,
            None => match element.child("complexType") {
                Some(complex) => (self.complex_type_dtype(complex, depth)?, None),
                None => match element.child("simpleType") {
                    Some(simple) => self.simple_type_dtype(simple, depth)?,
                    // A declaration with no type at all holds anything, which
                    // on the wire is text.
                    None => (DataType::utf8(), None),
                },
            },
        };

        let dtype = if repeated {
            DataType::list(Field::new("item", dtype, true))
        } else {
            dtype
        };
        let mut field = Field::new(name, dtype, optional || repeated);
        if let Some(declared) = declared {
            field.as_xml_mut().set_declared_type(Some(&declared))?;
        }
        field.as_xml_mut().set_kind(XmlKind::Element)?;
        field.as_xml_mut().set_namespace(self.target)?;
        Ok(field)
    }

    /// Build the field one `xs:attribute` declares.
    fn attribute_field(&self, attribute: &Node, depth: usize) -> Result<Field> {
        let name = attribute
            .attribute("name")
            .ok_or_else(|| codec_error(0, "expected every declared attribute to carry a name"))?;
        let required = attribute
            .attribute("use")
            .is_some_and(|use_| use_ == "required");
        let (dtype, declared) = match attribute.attribute("type") {
            Some(declared) => self.named_type(declared, depth)?,
            None => match attribute.child("simpleType") {
                Some(simple) => self.simple_type_dtype(simple, depth)?,
                None => (DataType::utf8(), None),
            },
        };
        let mut field = Field::new(name, dtype, !required);
        if let Some(declared) = declared {
            field.as_xml_mut().set_declared_type(Some(&declared))?;
        }
        field.as_xml_mut().set_kind(XmlKind::Attribute)?;
        Ok(field)
    }

    /// The datatype one `xs:complexType` describes.
    fn complex_type_dtype(&self, complex: &Node, depth: usize) -> Result<DataType> {
        if complex
            .attribute("mixed")
            .is_some_and(|value| value == "true")
        {
            return Err(codec_error(
                0,
                "expected a complex type a row can hold, got one declared mixed",
            ));
        }
        let mut children = Vec::new();
        // Attributes are columns exactly as elements are, and are declared
        // beside the model group rather than inside it.
        for attribute in complex.children_named("attribute") {
            children.push(self.attribute_field(attribute, depth.saturating_add(1))?);
        }
        for group in ["sequence", "all", "choice"] {
            let Some(model) = complex.child(group) else {
                continue;
            };
            if model.child("any").is_some() {
                return Err(codec_error(
                    0,
                    "expected a complex type naming its children, got one holding xs:any",
                ));
            }
            for element in model.children_named("element") {
                let field = self.element_field(element, depth.saturating_add(1))?;
                // Every branch of a choice is optional: a row takes one of
                // them, so none of them is always there.
                let field = if group == "choice" {
                    field.with_nullable(true)
                } else {
                    field
                };
                children.push(field);
            }
        }
        if children.is_empty() {
            return Err(codec_error(
                0,
                "expected a complex type declaring at least one element or attribute",
            ));
        }
        DataType::from_fields(children)
    }

    /// The datatype and declared name one `xs:simpleType` describes.
    fn simple_type_dtype(
        &self,
        simple: &Node,
        depth: usize,
    ) -> Result<(DataType, Option<SmolStr>)> {
        let Some(restriction) = simple.child("restriction") else {
            // A union or a list of anything is text: the value space is wider
            // than any one datatype the crate has.
            return Ok((DataType::utf8(), None));
        };
        let base = restriction.attribute("base").unwrap_or("xs:string");
        let facets = Facets::of(restriction);
        let (dtype, _) = self.named_type_with(base, &facets, depth)?;
        Ok((dtype, Some(SmolStr::new(base))))
    }

    /// Resolve one type name, built in or declared in this document.
    fn named_type(&self, declared: &str, depth: usize) -> Result<(DataType, Option<SmolStr>)> {
        self.named_type_with(declared, &Facets::default(), depth)
    }

    /// Resolve one type name under the facets restricting it.
    fn named_type_with(
        &self,
        declared: &str,
        facets: &Facets,
        depth: usize,
    ) -> Result<(DataType, Option<SmolStr>)> {
        let local = declared.rsplit(':').next().unwrap_or(declared);
        if let Some(dtype) = builtin(local, facets)? {
            // A built-in that maps exactly needs no reminder of its name; one
            // that had to travel as text does.
            let kept = matches!(dtype, DataType::String(_)).then(|| SmolStr::new(declared));
            return Ok((dtype, kept));
        }
        if let Some(simple) = self.simple_type(local) {
            return self.simple_type_dtype(simple, depth.saturating_add(1));
        }
        if let Some(complex) = self.complex_type(local) {
            return Ok((
                self.complex_type_dtype(complex, depth.saturating_add(1))?,
                None,
            ));
        }
        Err(codec_error(
            0,
            format_smolstr!(
                "expected a built-in or declared type, got {}",
                quoted(declared)
            ),
        ))
    }
}

/// How deep a schema may nest before the read stops descending.
pub const MAX_SCHEMA_DEPTH: usize = 64;

/// The facets one restriction states, as far as a datatype cares.
#[derive(Debug, Default)]
struct Facets {
    total_digits: Option<u8>,
    fraction_digits: Option<i8>,
    length: Option<u32>,
    explicit_timezone: Option<SmolStr>,
}

impl Facets {
    fn of(restriction: &Node) -> Self {
        let value = |name: &str| {
            restriction
                .child(name)
                .and_then(|node| node.attribute("value"))
                .map(SmolStr::new)
        };
        Self {
            total_digits: value("totalDigits").and_then(|text| text.parse().ok()),
            fraction_digits: value("fractionDigits").and_then(|text| text.parse().ok()),
            length: value("length").and_then(|text| text.parse().ok()),
            explicit_timezone: value("explicitTimezone"),
        }
    }

    /// Whether a temporal's timezone is stated to be absent.
    fn timezone_prohibited(&self) -> bool {
        self.explicit_timezone.as_deref() == Some("prohibited")
    }

    /// Whether a temporal's timezone is stated to be present.
    fn timezone_required(&self) -> bool {
        self.explicit_timezone.as_deref() == Some("required")
    }
}

/// The datatype one XSD built-in maps onto, under its facets.
///
/// `None` means the name is not a built-in; an error means it is one no column
/// can hold.
fn builtin(local: &str, facets: &Facets) -> Result<Option<DataType>> {
    let dtype = match local {
        // Refused: no instance can exist, and no value travels.
        "error" => {
            return Err(refused(local, "it has an empty value space"));
        }
        "NOTATION" => {
            return Err(refused(
                local,
                "its values mean only what the schema declaring them means",
            ));
        }

        "boolean" => DataType::Boolean,
        "float" => DataType::Float32,
        "double" => DataType::Float64,

        // The eight bounded integers are the only ones with a width.
        "long" => DataType::Int64,
        "int" => DataType::Int32,
        "short" => DataType::Int16,
        "byte" => DataType::Int8,
        "unsignedLong" => DataType::UInt64,
        "unsignedInt" => DataType::UInt32,
        "unsignedShort" => DataType::UInt16,
        "unsignedByte" => DataType::UInt8,

        "decimal" => match (facets.total_digits, facets.fraction_digits) {
            (Some(precision), Some(scale)) => DataType::decimal(precision, scale)?,
            // Unbounded in magnitude and in scale: no fixed precision holds it.
            _ => DataType::utf8(),
        },

        "dateTime" => {
            if facets.timezone_prohibited() {
                DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE)?
            } else if facets.timezone_required() {
                DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?
            } else {
                DataType::utf8()
            }
        }
        // `dateTimeStamp` is `dateTime` with the offset required, by definition.
        "dateTimeStamp" => DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?,
        "date" if facets.timezone_prohibited() => DataType::Date32,
        "time" if facets.timezone_prohibited() => DataType::time64(TimeUnit::Microsecond)?,

        "hexBinary" | "base64Binary" => match facets.length {
            Some(width) => DataType::fixed_size_binary(width)?,
            None => DataType::binary(),
        },

        // Built-in list types: whitespace-separated, and a list of text.
        "NMTOKENS" | "IDREFS" | "ENTITIES" => {
            DataType::list(Field::new("item", DataType::utf8(), true))
        }

        // Everything else a schema can name is text, and `xml:type` says what
        // the schema called it.
        "string" | "normalizedString" | "token" | "language" | "NMTOKEN" | "Name" | "NCName"
        | "ID" | "IDREF" | "ENTITY" | "anyURI" | "QName" | "anySimpleType" | "anyAtomicType"
        | "anyType" | "integer" | "nonPositiveInteger" | "negativeInteger"
        | "nonNegativeInteger" | "positiveInteger" | "duration" | "yearMonthDuration"
        | "dayTimeDuration" | "date" | "time" | "gYearMonth" | "gYear" | "gMonthDay" | "gDay"
        | "gMonth" => DataType::utf8(),

        _ => return Ok(None),
    };
    Ok(Some(dtype))
}

/// Build the refusal one uninhabitable type earns.
fn refused(local: &str, because: &str) -> Error {
    codec_error(
        0,
        format_smolstr!("expected a type a column can hold, got xs:{local}, and {because}"),
    )
}

//! The XMLA rowset: a table spelled as XML, its columns declared once in an
//! XML Schema and its rows written as elements under it.
//!
//! A Discover answers a rowset and a tabular Execute answers one, so this is
//! the one place a [`Field`] becomes the `xsd:schema` a client reads its
//! columns from, a record [`Serie`] becomes `<row>` elements, and both are
//! read back. The document has one shape, XML for Analysis 1.1's:
//!
//! ```xml
//! <root xmlns="urn:schemas-microsoft-com:xml-analysis:rowset"
//!       xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
//!       xmlns:xsd="http://www.w3.org/2001/XMLSchema">
//!   <xsd:schema targetNamespace="urn:schemas-microsoft-com:xml-analysis:rowset"
//!               xmlns:sql="urn:schemas-microsoft-com:xml-sql" elementFormDefault="qualified">
//!     <xsd:element name="root"><xsd:complexType><xsd:sequence minOccurs="0" maxOccurs="unbounded">
//!       <xsd:element name="row" type="row"/>
//!     </xsd:sequence></xsd:complexType></xsd:element>
//!     <xsd:simpleType name="uuid"><xsd:restriction base="xsd:string">
//!       <xsd:pattern value="[0-9a-zA-Z]{8}-[0-9a-zA-Z]{4}-[0-9a-zA-Z]{4}-[0-9a-zA-Z]{4}-[0-9a-zA-Z]{12}"/>
//!     </xsd:restriction></xsd:simpleType>
//!     <xsd:complexType name="row"><xsd:sequence>
//!       <xsd:element sql:field="Order Id" name="Order_x0020_Id" type="xsd:int"/>
//!       <xsd:element sql:field="Symbol" name="Symbol" type="xsd:string" minOccurs="0"/>
//!     </xsd:sequence></xsd:complexType>
//!   </xsd:schema>
//!   <row><Order_x0020_Id>7</Order_x0020_Id><Symbol>AAPL</Symbol></row>
//!   <row><Order_x0020_Id>8</Order_x0020_Id></row>
//! </root>
//! ```
//!
//! A column's element carries the column's name made an XML name - a
//! character no name may hold spelled `_xHHHH_`, as SQL Server spells it -
//! with the name as declared in `sql:field`. A nullable column has
//! `minOccurs="0"` and an absent value is an absent element; a sequence
//! column has `maxOccurs="unbounded"` and repeats its element per item; a
//! struct column declares its children inline. Every leaf is written in the
//! spelling the XML codec gives it, and the schema names the XML Schema type
//! the column reads back as: [`XsdType`] is that mapping, both ways.

use std::fmt;
use std::io::Write;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::xml::{Element, XSD_NAMESPACE, XSI_NAMESPACE};
use crate::{Charset, DataType, Error, Field, Result, Scalar, Serie, StructType, TimeUnit, Timezone};

use super::{EXCEPTION_NAMESPACE, ROWSET_NAMESPACE, SQL_NAMESPACE};

/// The element every row is written as.
pub const ROW_ELEMENT: &str = "row";

/// The element a rowset is written under.
pub const ROOT_ELEMENT: &str = "root";

/// The name the rowset schema gives its identifier type, a restriction of
/// `xsd:string` to the canonical spelling.
const UUID_TYPE: &str = "uuid";

/// The XML Schema type a column is declared as.
///
/// The mapping is the column's datatype family onto the XML Schema type that
/// holds it, and back onto the widest datatype of that family a document can
/// prove: `xsd:int` is `int32`, `xsd:dateTime` is a microsecond datetime,
/// `xsd:integer` is a 38-digit decimal of scale zero, `xsd:decimal` is text
/// because the schema states no scale, and a type this table does not know is
/// text, which is what XML proves of anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum XsdType {
    /// `xsd:string`.
    String,
    /// `xsd:boolean`.
    Boolean,
    /// `xsd:byte`.
    Byte,
    /// `xsd:unsignedByte`.
    UnsignedByte,
    /// `xsd:short`.
    Short,
    /// `xsd:unsignedShort`.
    UnsignedShort,
    /// `xsd:int`.
    Int,
    /// `xsd:unsignedInt`.
    UnsignedInt,
    /// `xsd:long`.
    Long,
    /// `xsd:unsignedLong`.
    UnsignedLong,
    /// `xsd:integer`: unbounded digits, which a scale-free decimal holds.
    Integer,
    /// `xsd:float`.
    Float,
    /// `xsd:double`.
    Double,
    /// `xsd:decimal`: exact digits at a scale the schema does not state.
    Decimal,
    /// `xsd:date`.
    Date,
    /// `xsd:time`.
    Time,
    /// `xsd:dateTime`.
    DateTime,
    /// `xsd:duration`.
    Duration,
    /// `xsd:base64Binary`.
    Base64Binary,
    /// `xsd:anyURI`.
    AnyUri,
    /// The rowset schema's own `uuid`.
    Uuid,
}

impl XsdType {
    /// Every type, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::String,
        Self::Boolean,
        Self::Byte,
        Self::UnsignedByte,
        Self::Short,
        Self::UnsignedShort,
        Self::Int,
        Self::UnsignedInt,
        Self::Long,
        Self::UnsignedLong,
        Self::Integer,
        Self::Float,
        Self::Double,
        Self::Decimal,
        Self::Date,
        Self::Time,
        Self::DateTime,
        Self::Duration,
        Self::Base64Binary,
        Self::AnyUri,
        Self::Uuid,
    ];

    /// The qualified name as the schema writes it, under the `xsd` prefix.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "xsd:string",
            Self::Boolean => "xsd:boolean",
            Self::Byte => "xsd:byte",
            Self::UnsignedByte => "xsd:unsignedByte",
            Self::Short => "xsd:short",
            Self::UnsignedShort => "xsd:unsignedShort",
            Self::Int => "xsd:int",
            Self::UnsignedInt => "xsd:unsignedInt",
            Self::Long => "xsd:long",
            Self::UnsignedLong => "xsd:unsignedLong",
            Self::Integer => "xsd:integer",
            Self::Float => "xsd:float",
            Self::Double => "xsd:double",
            Self::Decimal => "xsd:decimal",
            Self::Date => "xsd:date",
            Self::Time => "xsd:time",
            Self::DateTime => "xsd:dateTime",
            Self::Duration => "xsd:duration",
            Self::Base64Binary => "xsd:base64Binary",
            Self::AnyUri => "xsd:anyURI",
            Self::Uuid => UUID_TYPE,
        }
    }

    /// The local name, past the `xsd:` prefix.
    #[must_use]
    pub fn local_name(self) -> &'static str {
        self.as_str().rsplit(':').next().unwrap_or(UUID_TYPE)
    }

    /// The type a leaf datatype is declared as, `None` for a nested datatype
    /// - a struct, a sequence, a map, a union - which declares no simple type.
    #[must_use]
    pub fn of(dtype: &DataType) -> Option<Self> {
        Some(match dtype {
            DataType::Null => Self::String,
            DataType::Boolean => Self::Boolean,
            DataType::Int8 => Self::Byte,
            DataType::UInt8 => Self::UnsignedByte,
            DataType::Int16 => Self::Short,
            DataType::UInt16 => Self::UnsignedShort,
            DataType::Int32 => Self::Int,
            DataType::UInt32 => Self::UnsignedInt,
            DataType::Int64 => Self::Long,
            DataType::UInt64 => Self::UnsignedLong,
            DataType::Float16 | DataType::Float32 => Self::Float,
            DataType::Float64 => Self::Double,
            DataType::Decimal32 { .. }
            | DataType::Decimal64 { .. }
            | DataType::Decimal128 { .. }
            | DataType::Decimal256 { .. } => Self::Decimal,
            DataType::Date32 | DataType::Date64 => Self::Date,
            DataType::Time32(_) | DataType::Time64(_) => Self::Time,
            DataType::DateTime64 { .. } => Self::DateTime,
            DataType::Duration32(_) | DataType::Duration64(_) => Self::Duration,
            DataType::Uuid => Self::Uuid,
            DataType::Url => Self::AnyUri,
            DataType::Geometry(_) | DataType::Geography(_) => Self::Base64Binary,
            DataType::Dictionary(dictionary) => return Self::of(dictionary.value()),
            DataType::RunEndEncoded(encoded) => return Self::of(encoded.values().dtype()),
            DataType::Struct(_)
            | DataType::Serie(_)
            | DataType::SerieView(_)
            | DataType::FixedSizeSerie(_, _)
            | DataType::LargeSerie(_)
            | DataType::LargeSerieView(_)
            | DataType::Map(_)
            | DataType::SortedMap(_)
            | DataType::Union(_, _) => return None,
            other if other.bytes_parameters().is_some() => Self::Base64Binary,
            // Every string leaf, every registered code, an interval with no
            // one-number spelling, a variant, a version, a zone, a media
            // type: text.
            _ => Self::String,
        })
    }

    /// The datatype a column of this type reads back as when no field
    /// declares one.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::String | Self::Decimal => DataType::utf8(),
            Self::Boolean => DataType::Boolean,
            Self::Byte => DataType::Int8,
            Self::UnsignedByte => DataType::UInt8,
            Self::Short => DataType::Int16,
            Self::UnsignedShort => DataType::UInt16,
            Self::Int => DataType::Int32,
            Self::UnsignedInt => DataType::UInt32,
            Self::Long => DataType::Int64,
            Self::UnsignedLong => DataType::UInt64,
            Self::Integer => DataType::decimal128(38, 0)
                .expect("a 38-digit integer decimal is a valid datatype"),
            Self::Float => DataType::Float32,
            Self::Double => DataType::Float64,
            Self::Date => DataType::Date32,
            Self::Time => DataType::Time64(TimeUnit::Microsecond),
            Self::DateTime => DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::NAIVE,
            },
            Self::Duration => DataType::Duration64(TimeUnit::Microsecond),
            Self::Base64Binary => DataType::binary(),
            Self::AnyUri => DataType::Url,
            Self::Uuid => DataType::Uuid,
        }
    }

    /// The type a schema's `type` attribute names, `None` for one this table
    /// does not have - which a reader takes as text.
    ///
    /// `namespace` is what the name's prefix resolved to; the rowset's own
    /// `uuid` is unprefixed and in no namespace.
    #[must_use]
    pub fn from_qualified(namespace: Option<&str>, local: &str) -> Option<Self> {
        match namespace {
            Some(XSD_NAMESPACE) => Self::ALL
                .iter()
                .copied()
                .filter(|held| *held != Self::Uuid)
                .find(|held| held.local_name() == local),
            None if local == UUID_TYPE => Some(Self::Uuid),
            _ => None,
        }
    }
}

impl fmt::Display for XsdType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Spell `name` as an XML element name, escaping every character an XML
/// name cannot hold as `_xHHHH_` - the convention SQL Server and Analysis
/// Services write column names under.
///
/// A colon is escaped too, because it would read as a prefix; a first
/// character that may not open a name is escaped; and a literal `_x` is
/// escaped as `_x005F_x` so the encoding reads back exactly.
///
/// ```
/// use yggdryl::xmla::rowset::{decode_name, encode_name};
///
/// assert_eq!(encode_name("Order Id"), "Order_x0020_Id");
/// assert_eq!(encode_name("2024"), "_x0032_024");
/// assert_eq!(encode_name("a:b"), "a_x003A_b");
/// assert_eq!(decode_name("Order_x0020_Id"), "Order Id");
/// assert_eq!(decode_name(&encode_name("_x0020_ literal")), "_x0020_ literal");
/// ```
#[must_use]
pub fn encode_name(name: &str) -> SmolStr {
    let mut encoded = String::with_capacity(name.len());
    let mut characters = name.chars().peekable();
    let mut first = true;
    while let Some(character) = characters.next() {
        let allowed = if first {
            crate::xml::is_name_start(character) && character != ':'
        } else {
            crate::xml::is_name_char(character) && character != ':'
        };
        let escape = !allowed || (character == '_' && characters.peek() == Some(&'x'));
        if escape {
            let mut units = [0_u16; 2];
            for unit in character.encode_utf16(&mut units) {
                use std::fmt::Write as _;
                let _ = write!(encoded, "_x{unit:04X}_");
            }
        } else {
            encoded.push(character);
        }
        first = false;
    }
    if encoded.is_empty() {
        // An empty name has no element; the escape of nothing at all.
        encoded.push_str("_x0000_");
    }
    SmolStr::new(encoded)
}

/// Read a column name back out of its element name: every `_xHHHH_` is the
/// character it spells, a surrogate pair joined, and anything else stands.
#[must_use]
pub fn decode_name(encoded: &str) -> SmolStr {
    let mut decoded = String::with_capacity(encoded.len());
    let mut pending: Option<u16> = None;
    let mut rest = encoded;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("_x") {
            if after.len() >= 5 && after.as_bytes()[4] == b'_' {
                if let Ok(unit) = u16::from_str_radix(&after[..4], 16) {
                    rest = &after[5..];
                    match (pending.take(), unit) {
                        (Some(high), low) if (0xDC00..=0xDFFF).contains(&low) => {
                            let code = 0x10000
                                + ((u32::from(high) - 0xD800) << 10)
                                + (u32::from(low) - 0xDC00);
                            decoded.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        (Some(_), unit) | (None, unit) if (0xD800..=0xDBFF).contains(&unit) => {
                            pending = Some(unit);
                        }
                        (_, unit) if unit == 0 && decoded.is_empty() && rest.is_empty() => {}
                        (_, unit) => decoded.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}')),
                    }
                    continue;
                }
            }
        }
        let character = rest.chars().next().unwrap_or('\u{FFFD}');
        if pending.take().is_some() {
            decoded.push('\u{FFFD}');
        }
        decoded.push(character);
        rest = &rest[character.len_utf8()..];
    }
    if pending.is_some() {
        decoded.push('\u{FFFD}');
    }
    SmolStr::new(decoded)
}

/// One column of a rowset: its field, and the element name it is spelled
/// under.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Column {
    element: SmolStr,
    /// How the column's cells are written: resolved from the datatype once,
    /// per rowset, never per row.
    shape: Shape,
}

/// How a column's cells are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// A UTF-8 or US-ASCII text leaf, read as bytes where they lie.
    Text,
    /// A byte leaf, read as bytes where they lie and written as base64.
    Bytes,
    /// Any other leaf, read as the value its slot holds.
    Leaf,
    /// A struct or a sequence, written as its natural elements.
    Nested,
}

/// The columns of a rowset: a record field and, per column, the element it
/// is spelled under.
///
/// ```
/// use yggdryl::xmla::rowset::Rowset;
/// use yggdryl::{DataType, Field, Scalar, Serie, StructType};
///
/// let field = Field::new(
///     "row",
///     DataType::from(StructType::from_fields([
///         DataType::Int32.required_field("Order Id"),
///         DataType::utf8().nullable_field("Symbol"),
///     ])?),
///     false,
/// );
/// let rowset = Rowset::new(field.clone())?;
/// assert_eq!(rowset.element_names(), ["Order_x0020_Id", "Symbol"]);
///
/// let rows = Serie::from_scalars(field, [
///     Scalar::from_struct([("Order Id", Scalar::from(7)), ("Symbol", Scalar::from("AAPL"))])?,
///     Scalar::from_struct([("Order Id", Scalar::from(8)), ("Symbol", Scalar::Null)])?,
/// ])?;
/// let mut document = Vec::new();
/// rowset.write_root(&mut document, std::iter::once(Ok(rows.clone())), true, true)?;
/// let text = String::from_utf8(document.clone())?;
/// assert!(text.contains("<row><Order_x0020_Id>7</Order_x0020_Id><Symbol>AAPL</Symbol></row>"));
/// assert!(text.contains("<row><Order_x0020_Id>8</Order_x0020_Id></row>"));
///
/// let value = yggdryl::from_xml_scalar(&document)?;
/// let root = yggdryl::xml::Element::root(&value)?;
/// let (read, back) = Rowset::read_root(&root, None)?;
/// assert_eq!(read.field(), rowset.field());
/// assert_eq!(back, rows);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rowset {
    field: Arc<Field>,
    columns: Vec<Column>,
}

impl Rowset {
    /// The rowset whose rows are `field`, a struct.
    ///
    /// # Errors
    ///
    /// Returns an error when `field` is not a struct, or a column is a map or
    /// a union, which a rowset cannot spell, or a sequence of sequences,
    /// which has no element to repeat.
    pub fn new(field: Field) -> Result<Self> {
        let field = field.with_nullable(false);
        let DataType::Struct(_) = field.dtype() else {
            return Err(invalid(format_smolstr!(
                "expected a struct field for the rows of a rowset, got {}",
                field.dtype()
            )));
        };
        let columns = field
            .fields()
            .iter()
            .map(|column| {
                check_column(column, column.name())?;
                Ok(Column {
                    element: encode_name(column.name()),
                    shape: shape_of(column.dtype()),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            field: Arc::new(field),
            columns,
        })
    }

    /// The record field the rows are.
    #[must_use]
    pub fn field(&self) -> &Field {
        &self.field
    }

    /// The shared record field.
    #[must_use]
    pub const fn shared_field(&self) -> &Arc<Field> {
        &self.field
    }

    /// The element name each column is spelled under, in column order.
    #[must_use]
    pub fn element_names(&self) -> Vec<&str> {
        self.columns
            .iter()
            .map(|column| column.element.as_str())
            .collect()
    }

    /// Write the whole rowset: `<root>` with its declarations, the schema
    /// when `schema`, every batch's rows when `data`, and the closing tag.
    ///
    /// Each batch is written as it is pulled, so a stream of rows is never
    /// held whole; a batch whose field is not this rowset's is refused by
    /// name before any of its rows is written.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure, a batch's failure, or a cell with no XML
    /// spelling, naming its column.
    pub fn write_root<W: Write>(
        &self,
        writer: &mut W,
        batches: impl IntoIterator<Item = crate::arrow::Result<Serie>>,
        schema: bool,
        data: bool,
    ) -> Result<()> {
        write!(
            writer,
            "<{ROOT_ELEMENT} xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\" \
             xmlns:xsd=\"{XSD_NAMESPACE}\" xmlns:EX=\"{EXCEPTION_NAMESPACE}\">"
        )?;
        if schema {
            self.write_schema(writer)?;
        }
        if data {
            for batch in batches {
                let batch = batch.map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$.rows"),
                    reason: format_smolstr!("{error}"),
                })?;
                self.write_rows(writer, &batch)?;
            }
        }
        write!(writer, "</{ROOT_ELEMENT}>")?;
        Ok(())
    }

    /// Write the `xsd:schema` declaring this rowset's columns.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure.
    pub fn write_schema<W: Write>(&self, writer: &mut W) -> Result<()> {
        write!(
            writer,
            "<xsd:schema targetNamespace=\"{ROWSET_NAMESPACE}\" xmlns:sql=\"{SQL_NAMESPACE}\" \
             elementFormDefault=\"qualified\">\
             <xsd:element name=\"{ROOT_ELEMENT}\"><xsd:complexType>\
             <xsd:sequence minOccurs=\"0\" maxOccurs=\"unbounded\">\
             <xsd:element name=\"{ROW_ELEMENT}\" type=\"{ROW_ELEMENT}\"/>\
             </xsd:sequence></xsd:complexType></xsd:element>\
             <xsd:simpleType name=\"{UUID_TYPE}\"><xsd:restriction base=\"xsd:string\">\
             <xsd:pattern value=\"[0-9a-zA-Z]{{8}}-[0-9a-zA-Z]{{4}}-[0-9a-zA-Z]{{4}}-[0-9a-zA-Z]{{4}}-[0-9a-zA-Z]{{12}}\"/>\
             </xsd:restriction></xsd:simpleType>\
             <xsd:complexType name=\"{ROW_ELEMENT}\"><xsd:sequence>"
        )?;
        for (column, field) in self.columns.iter().zip(self.field.fields()) {
            write_declaration(writer, field, &column.element)?;
        }
        write!(writer, "</xsd:sequence></xsd:complexType></xsd:schema>")?;
        Ok(())
    }

    /// Write every row of one record column as `<row>` elements.
    ///
    /// # Errors
    ///
    /// Returns an error when `batch` is not a record of this rowset's field,
    /// the sink fails, or a cell has no XML spelling.
    pub fn write_rows<W: Write>(&self, writer: &mut W, batch: &Serie) -> Result<()> {
        let Some(records) = batch.as_struct() else {
            return Err(invalid(format_smolstr!(
                "expected a record column for the rows of a rowset, got {}",
                batch.field().map_or("a run", Field::name)
            )));
        };
        let children = records.children();
        if children.len() != self.columns.len() {
            return Err(invalid(format_smolstr!(
                "expected {} columns for the rowset, got {}",
                self.columns.len(),
                children.len()
            )));
        }
        for (child, field) in children.iter().zip(self.field.fields()) {
            let Some(held) = child.field() else {
                return Err(invalid("a rowset column must be a column, not a run"));
            };
            if held.dtype() != field.dtype() {
                return Err(invalid(format_smolstr!(
                    "column `{}` holds {}, the rowset declares {}",
                    field.name(),
                    held.dtype(),
                    field.dtype()
                )));
            }
        }
        let rows = batch.len();
        for row in 0..rows {
            write!(writer, "<{ROW_ELEMENT}>")?;
            for ((column, child), field) in self
                .columns
                .iter()
                .zip(children)
                .zip(self.field.fields())
            {
                write_cell(writer, column, child, field, row)?;
            }
            write!(writer, "</{ROW_ELEMENT}>")?;
        }
        Ok(())
    }

    /// Read a rowset's columns out of its `xsd:schema` element.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema declares no `row` type, or a column
    /// declaration is not one this reader understands.
    pub fn from_schema(schema: &Element<'_>) -> Result<Self> {
        let row = schema
            .children_in(Some(XSD_NAMESPACE), "complexType")
            .into_iter()
            .find(|held| held.attribute("name").and_then(Scalar::as_str) == Some(ROW_ELEMENT))
            .ok_or_else(|| invalid("the rowset schema declares no `row` type"))?;
        let sequence = row
            .child(Some(XSD_NAMESPACE), "sequence")
            .ok_or_else(|| invalid("the rowset schema's `row` type declares no sequence"))?;
        let mut fields = Vec::new();
        let mut columns = Vec::new();
        for declaration in sequence.children_in(Some(XSD_NAMESPACE), "element") {
            let (element, field) = read_declaration(&declaration)?;
            columns.push(Column {
                element,
                shape: shape_of(field.dtype()),
            });
            fields.push(field);
        }
        let field = Field::new(
            crate::media::DEFAULT_ROOT_NAME,
            DataType::from(StructType::from_fields(fields)?),
            false,
        );
        Ok(Self {
            field: Arc::new(field),
            columns,
        })
    }

    /// Read the rows of a `root` element under this rowset's columns.
    ///
    /// An absent element and one marked `xsi:nil` are null; a repeated
    /// element is a sequence; text is read as the column's datatype through
    /// the field's own value contract.
    ///
    /// # Errors
    ///
    /// Returns an error naming the row and column of a cell the column's
    /// datatype cannot read.
    pub fn read_rows(&self, root: &Element<'_>) -> Result<Serie> {
        let names: std::collections::HashMap<&str, &str> = self
            .columns
            .iter()
            .zip(self.field.fields())
            .map(|(column, field)| (column.element.as_str(), field.name()))
            .collect();
        let mut rows = Vec::new();
        for (index, row) in root.children_in(Some(ROWSET_NAMESPACE), ROW_ELEMENT).into_iter()
            .chain(root.children_in(None, ROW_ELEMENT))
            .enumerate()
        {
            let entries: Vec<(SmolStr, Scalar)> = match row.value().as_struct() {
                Some(entries) => entries
                    .iter()
                    .filter(|(key, _)| !key.starts_with('@') && !key.starts_with('#'))
                    .map(|(key, value)| {
                        let local = key.rsplit(':').next().unwrap_or(key);
                        let name = names
                            .get(local)
                            .map_or_else(|| decode_name(local), |name| SmolStr::new(*name));
                        (name, value.clone())
                    })
                    .collect(),
                // `<row/>` and `<row></row>`: every column absent.
                None => Vec::new(),
            };
            let record = Scalar::from_struct(entries)?;
            let shaped = crate::xml::shaped(record, &self.field);
            let canonical = self.field.from_natural_value(shaped).map_err(|error| {
                Error::InvalidRecord {
                    path: format_smolstr!("$[{index}]"),
                    reason: format_smolstr!("{error}"),
                }
            })?;
            rows.push(canonical);
        }
        let borrowed: Vec<&Scalar> = rows.iter().collect();
        crate::serie::from_canonical_rows(Arc::clone(&self.field), &borrowed)
    }

    /// Read a whole `root` element: the columns from its schema - or from
    /// `field` when one is declared, which is what types a document a
    /// foreign provider wrote - and the rows under them.
    ///
    /// A root with no schema and no declared field is refused: its rows
    /// state no columns to read them as.
    ///
    /// # Errors
    ///
    /// Returns the schema's or the rows' refusal.
    pub fn read_root(root: &Element<'_>, field: Option<&Field>) -> Result<(Self, Serie)> {
        let rowset = match (field, root.child(Some(XSD_NAMESPACE), "schema")) {
            (Some(field), _) => Self::new(field.clone())?,
            (None, Some(schema)) => Self::from_schema(&schema)?.zoned_where_the_rows_are(root)?,
            (None, None) => {
                return Err(invalid(
                    "the rowset carries no schema and no field is declared to read its rows by",
                ));
            }
        };
        let rows = rowset.read_rows(root)?;
        Ok((rowset, rows))
    }

    /// This rowset with each `xsd:dateTime` column read as an instant where
    /// its rows spell a zone.
    ///
    /// XML Schema's `dateTime` is a wall reading or an instant, and the schema
    /// does not say which; the rows do. A column whose first present value
    /// ends in `Z` or an offset is a UTC instant column, and one whose first
    /// value carries no zone stays naive - the one decision the document
    /// leaves to its rows, made once per column, never per row.
    fn zoned_where_the_rows_are(mut self, root: &Element<'_>) -> Result<Self> {
        let datetimes: Vec<usize> = self
            .field
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| {
                matches!(
                    field.dtype(),
                    DataType::DateTime64 { timezone, .. } if timezone.is_naive()
                )
            })
            .map(|(index, _)| index)
            .collect();
        if datetimes.is_empty() {
            return Ok(self);
        }
        let mut zoned = vec![None; datetimes.len()];
        'rows: for row in root
            .children_in(Some(ROWSET_NAMESPACE), ROW_ELEMENT)
            .into_iter()
            .chain(root.children_in(None, ROW_ELEMENT))
        {
            for (slot, index) in datetimes.iter().enumerate() {
                if zoned[slot].is_some() {
                    continue;
                }
                let element = &self.columns[*index].element;
                let text = row.children().find(|child| child.local_name() == element).and_then(|child| child.text().map(str::to_owned));
                if let Some(text) = text {
                    let text = text.trim();
                    zoned[slot] = Some(spells_a_zone(text));
                }
            }
            if zoned.iter().all(Option::is_some) {
                break 'rows;
            }
        }
        if zoned.iter().all(|decided| *decided != Some(true)) {
            return Ok(self);
        }
        let fields: Vec<Field> = self
            .field
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let instant = datetimes
                    .iter()
                    .position(|held| *held == index)
                    .is_some_and(|slot| zoned[slot] == Some(true));
                if instant {
                    Field::new(
                        field.name(),
                        DataType::DateTime64 {
                            unit: TimeUnit::Microsecond,
                            timezone: Timezone::UTC,
                        },
                        field.is_nullable(),
                    )
                } else {
                    field.clone()
                }
            })
            .collect();
        self.field = Arc::new(Field::new(
            self.field.name(),
            DataType::from(StructType::from_fields(fields)?),
            false,
        ));
        Ok(self)
    }
}

/// Whether an `xsd:dateTime` text ends in a zone designator: `Z`, or an
/// offset `+hh:mm` / `-hh:mm` after the time of day.
fn spells_a_zone(text: &str) -> bool {
    if text.ends_with(['Z', 'z']) {
        return true;
    }
    // `2024-01-01T10:00:00+02:00`: an offset is a sign six characters from the
    // end, past the `T` that opens the time of day.
    text.find('T').is_some_and(|clock| {
        text.len() >= clock + 6
            && text[clock..]
                .char_indices()
                .rev()
                .nth(5)
                .is_some_and(|(_, sign)| matches!(sign, '+' | '-'))
    })
}

/// Refuse a column a rowset cannot spell.
fn check_column(field: &Field, path: &str) -> Result<()> {
    match field.dtype() {
        DataType::Map(_) | DataType::SortedMap(_) => Err(invalid(format_smolstr!(
            "column `{path}` is a map, which a rowset has no element to spell"
        ))),
        DataType::Union(_, _) => Err(invalid(format_smolstr!(
            "column `{path}` is a union, which a rowset has no element to spell"
        ))),
        DataType::Struct(fields) => fields
            .iter()
            .try_for_each(|child| check_column(child, &format!("{path}.{}", child.name()))),
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => match item.dtype() {
            DataType::Serie(_)
            | DataType::SerieView(_)
            | DataType::FixedSizeSerie(_, _)
            | DataType::LargeSerie(_)
            | DataType::LargeSerieView(_) => Err(invalid(format_smolstr!(
                "column `{path}` is a sequence of sequences, which has no element to repeat"
            ))),
            _ => check_column(item, &format!("{path}[]")),
        },
        _ => Ok(()),
    }
}

/// How a column of `dtype` is written, decided once per rowset.
fn shape_of(dtype: &DataType) -> Shape {
    match dtype {
        DataType::Struct(_)
        | DataType::Serie(_)
        | DataType::SerieView(_)
        | DataType::FixedSizeSerie(_, _)
        | DataType::LargeSerie(_)
        | DataType::LargeSerieView(_) => Shape::Nested,
        _ if dtype
            .string_parameters()
            .is_some_and(|leaf| matches!(leaf.charset(), Charset::Utf8 | Charset::Ascii)) =>
        {
            Shape::Text
        }
        _ if dtype.bytes_parameters().is_some() => Shape::Bytes,
        _ => Shape::Leaf,
    }
}

/// Write one cell: nothing for an absent value, else the column's element.
fn write_cell<W: Write>(
    writer: &mut W,
    column: &Column,
    child: &Serie,
    field: &Field,
    row: usize,
) -> Result<()> {
    let element = &column.element;
    match column.shape {
        Shape::Text if child.is_string_storage() => {
            let Some(bytes) = child.value_bytes(row) else {
                return Ok(());
            };
            // A UTF-8 or US-ASCII leaf's bytes are UTF-8, which is what
            // resolving the shape once per rowset established.
            let text = std::str::from_utf8(bytes).map_err(|error| {
                invalid(format_smolstr!(
                    "column `{}` holds bytes that are not UTF-8 at row {row}: {error}",
                    field.name()
                ))
            })?;
            write!(writer, "<{element}>")?;
            crate::xml::write_element_text(writer, text)?;
            write!(writer, "</{element}>")?;
            Ok(())
        }
        Shape::Bytes if child.is_byte_storage() => {
            let Some(bytes) = child.value_bytes(row) else {
                return Ok(());
            };
            use base64::Engine as _;
            write!(writer, "<{element}>")?;
            writer.write_all(
                base64::engine::general_purpose::STANDARD
                    .encode(bytes)
                    .as_bytes(),
            )?;
            write!(writer, "</{element}>")?;
            Ok(())
        }
        Shape::Nested => {
            let Some(value) = child.get(row) else {
                return Ok(());
            };
            if value.is_null() {
                return Ok(());
            }
            let natural = field.into_natural_value(value.into_owned())?;
            let natural = crate::xml::natural(natural, field);
            match natural.sequence_rows() {
                Some(items) => {
                    for item in items.iter() {
                        crate::xml::write_fragment(writer, element, item)?;
                    }
                    Ok(())
                }
                None => crate::xml::write_fragment(writer, element, &natural),
            }
        }
        Shape::Text | Shape::Bytes | Shape::Leaf => {
            let Some(value) = child.get(row) else {
                return Ok(());
            };
            if value.is_null() {
                return Ok(());
            }
            write!(writer, "<{element}>")?;
            crate::xml::write_leaf_text(writer, &value, field.name())?;
            write!(writer, "</{element}>")?;
            Ok(())
        }
    }
}

/// Write one column's `xsd:element` declaration, its children inline for a
/// struct and its item's type for a sequence.
fn write_declaration<W: Write>(writer: &mut W, field: &Field, element: &str) -> Result<()> {
    let (item, unbounded) = match field.dtype() {
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => (item.as_ref(), true),
        _ => (field, false),
    };
    write!(writer, "<xsd:element sql:field=\"")?;
    crate::xml::write_attribute_text(writer, field.name())?;
    write!(writer, "\" name=\"{element}\"")?;
    let nested = match item.dtype() {
        DataType::Struct(fields) => Some(fields),
        other => {
            let xsd = XsdType::of(other).ok_or_else(|| {
                invalid(format_smolstr!(
                    "column `{}` is {other}, which a rowset has no element to spell",
                    field.name()
                ))
            })?;
            write!(writer, " type=\"{xsd}\"")?;
            None
        }
    };
    if field.is_nullable() || item.is_nullable() || field.dtype() == &DataType::Null {
        write!(writer, " minOccurs=\"0\"")?;
    }
    if unbounded {
        write!(writer, " maxOccurs=\"unbounded\"")?;
    }
    match nested {
        Some(fields) => {
            write!(writer, "><xsd:complexType><xsd:sequence>")?;
            for child in fields.iter() {
                write_declaration(writer, child, &encode_name(child.name()))?;
            }
            write!(writer, "</xsd:sequence></xsd:complexType></xsd:element>")?;
        }
        None => write!(writer, "/>")?,
    }
    Ok(())
}

/// Read one column's `xsd:element` declaration: its element name, and the
/// field it declares.
fn read_declaration(declaration: &Element<'_>) -> Result<(SmolStr, Field)> {
    let element = declaration
        .attribute("name")
        .and_then(Scalar::as_str)
        .ok_or_else(|| invalid("a column declaration without a `name`"))?;
    let name = declaration
        .attribute_in(Some(SQL_NAMESPACE), "field")
        .map_or_else(|| decode_name(element), SmolStr::new);
    let nullable = declaration
        .attribute("minOccurs")
        .and_then(Scalar::as_str)
        .is_some_and(|held| held.trim() == "0");
    let unbounded = declaration
        .attribute("maxOccurs")
        .and_then(Scalar::as_str)
        .is_some_and(|held| held.trim() != "1");
    let dtype = match declaration.attribute("type").and_then(Scalar::as_str) {
        Some(qualified) => {
            let (prefix, local) = match qualified.split_once(':') {
                Some((prefix, local)) => (Some(prefix), local),
                None => (None, qualified),
            };
            let namespace = prefix.and_then(|prefix| declaration.scope().resolve(prefix));
            XsdType::from_qualified(namespace, local).map_or_else(DataType::utf8, XsdType::datatype)
        }
        None => {
            let complex = declaration.child(Some(XSD_NAMESPACE), "complexType");
            let sequence = complex
                .as_ref()
                .and_then(|complex| complex.child(Some(XSD_NAMESPACE), "sequence"));
            match sequence {
                Some(sequence) => {
                    let mut fields = Vec::new();
                    for child in sequence.children_in(Some(XSD_NAMESPACE), "element") {
                        let (_, field) = read_declaration(&child)?;
                        fields.push(field);
                    }
                    DataType::from(StructType::from_fields(fields)?)
                }
                // A declaration with no type and no content model: text.
                None => DataType::utf8(),
            }
        }
    };
    let field = if unbounded {
        Field::new(
            name,
            DataType::serie(Field::new("item", dtype, true)),
            nullable,
        )
    } else {
        Field::new(name, dtype, nullable)
    };
    Ok((SmolStr::new(element), field))
}

pub(crate) fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.rowset"),
        reason: reason.into(),
    }
}

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

use std::borrow::Cow;
use std::fmt;
use std::io::Write;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::xml::{Element, XSD_NAMESPACE, XSI_NAMESPACE};
use crate::{
    Charset, DataType, Error, Field, Result, Scalar, Serie, StructType, TimeUnit, Timezone,
};

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
            Self::Integer => {
                DataType::decimal128(38, 0).expect("a 38-digit integer decimal is a valid datatype")
            }
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
        // An empty name has no element; the escape of nothing at all, which
        // `decode_name` reads back as the empty name. A name carrying U+0000
        // itself would spell the same, and is refused where a rowset is built:
        // no XML document holds that character, escaped or not.
        encoded.push_str(EMPTY_NAME);
    }
    SmolStr::new(encoded)
}

/// The element name of the empty column name: the escape of nothing at all.
const EMPTY_NAME: &str = "_x0000_";

/// Read a column name back out of its element name: every `_xHHHH_` is the
/// character it spells, a surrogate pair joined, and anything else stands.
#[must_use]
pub fn decode_name(encoded: &str) -> SmolStr {
    if encoded == EMPTY_NAME {
        return SmolStr::new_static("");
    }
    let mut decoded = String::with_capacity(encoded.len());
    let mut pending: Option<u16> = None;
    let mut rest = encoded;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("_x") {
            if after.len() >= 5 && after.as_bytes()[4] == b'_' {
                if let Ok(unit) = u16::from_str_radix(&after[..4], 16) {
                    rest = &after[5..];
                    if let Some(high) = pending.take() {
                        if (0xDC00..=0xDFFF).contains(&unit) {
                            let code = 0x10000
                                + ((u32::from(high) - 0xD800) << 10)
                                + (u32::from(unit) - 0xDC00);
                            decoded.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                            continue;
                        }
                        // A high half no low half follows spells no character,
                        // whatever comes next.
                        decoded.push('\u{FFFD}');
                    }
                    if (0xD800..=0xDBFF).contains(&unit) {
                        pending = Some(unit);
                    } else {
                        decoded.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'));
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

/// One column of a rowset: the element name it is spelled under, how its
/// cells are written, and - for a struct, or a sequence of structs - the
/// columns of its children in field order, each under the element the schema
/// declares for it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Column {
    element: SmolStr,
    /// How the column's cells are written: resolved from the datatype once,
    /// per rowset, never per row.
    shape: Shape,
    children: Vec<Column>,
}

impl Column {
    /// The column `field` is spelled as under `element`, each child under the
    /// escape of its name.
    fn of(field: &Field, element: SmolStr) -> Self {
        Self {
            element,
            shape: shape_of(field.dtype()),
            children: nested_fields(field)
                .iter()
                .map(|child| Self::of(child, encode_name(child.name())))
                .collect(),
        }
    }

    /// This column read as `field`, its element names kept: what a declared
    /// field changes is what a cell is read as, never where it is spelled.
    fn retyped(&self, field: &Field) -> Self {
        Self {
            element: self.element.clone(),
            shape: shape_of(field.dtype()),
            children: nested_fields(field)
                .iter()
                .enumerate()
                .map(|(index, child)| match self.children.get(index) {
                    Some(column) => column.retyped(child),
                    None => Self::of(child, encode_name(child.name())),
                })
                .collect(),
        }
    }
}

/// The children a struct field, or a sequence of structs, spells as
/// elements; empty for any other field.
fn nested_fields(field: &Field) -> &[Field] {
    match field.dtype() {
        DataType::Struct(_) => field.fields(),
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => item.fields(),
        _ => &[],
    }
}

/// How a column's cells are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// A UTF-8 or US-ASCII text leaf, read as bytes where they lie.
    Text,
    /// A fixed-width UTF-8 or US-ASCII text leaf: the slot's bytes less the
    /// NUL padding past its text, which is the storage's, never the value's.
    FixedText,
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
                Ok(Column::of(column, encode_name(column.name())))
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
        match self.write_root_with(writer, batches, schema, data, false)? {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Write a whole `root` element as a provider streams one: like
    /// [`Self::write_root`], except that a batch failing once rows have been
    /// written - a batch the source refuses, a cell with no XML spelling - is
    /// reported inside the root as `<Messages><Error .../></Messages>` in the
    /// exception namespace and the root is closed, so the document a client
    /// reads is complete and names the failure. Each row is completed before
    /// it reaches the sink, so a cell refused mid-row leaves no element open.
    /// The error is handed back rather than raised.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure.
    pub fn write_root_reporting<W: Write>(
        &self,
        writer: &mut W,
        batches: impl IntoIterator<Item = crate::arrow::Result<Serie>>,
        schema: bool,
        data: bool,
    ) -> Result<Option<super::response::XmlaError>> {
        Ok(self
            .write_root_with(writer, batches, schema, data, true)?
            .map(|error| reported(&error)))
    }

    fn write_root_with<W: Write>(
        &self,
        writer: &mut W,
        batches: impl IntoIterator<Item = crate::arrow::Result<Serie>>,
        schema: bool,
        data: bool,
        report: bool,
    ) -> Result<Option<Error>> {
        write!(
            writer,
            "<{ROOT_ELEMENT} xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\" \
             xmlns:xsd=\"{XSD_NAMESPACE}\" xmlns:EX=\"{EXCEPTION_NAMESPACE}\">"
        )?;
        if schema {
            self.write_schema(writer)?;
        }
        let mut failed = None;
        if data {
            let mut scratch = Vec::new();
            for batch in batches {
                let written = match batch {
                    Ok(batch) if report => self.write_rows_via(writer, &batch, &mut scratch),
                    Ok(batch) => self.write_rows(writer, &batch),
                    Err(error) => Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.rows"),
                        reason: format_smolstr!("{error}"),
                    }),
                };
                if let Err(error) = written {
                    if !report {
                        return Err(error);
                    }
                    write!(writer, "<Messages xmlns=\"{EXCEPTION_NAMESPACE}\">")?;
                    reported(&error).write(writer)?;
                    write!(writer, "</Messages>")?;
                    failed = Some(error);
                    break;
                }
            }
        }
        write!(writer, "</{ROOT_ELEMENT}>")?;
        Ok(failed)
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
             <xsd:pattern value=\"[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}\"/>\
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
    /// Returns an error when `batch` is not a record of this rowset's field -
    /// a column of another name or datatype, or a null under a column the
    /// rowset declares required - the sink fails, or a cell has no XML
    /// spelling.
    pub fn write_rows<W: Write>(&self, writer: &mut W, batch: &Serie) -> Result<()> {
        let children = self.check_batch(batch)?;
        for row in 0..batch.len() {
            self.write_row(writer, children, row)?;
        }
        Ok(())
    }

    /// [`Self::write_rows`], each row completed in `scratch` before it reaches
    /// `writer`: what a reporting write needs, so a cell refused mid-row leaves
    /// the document well formed at the row before it.
    fn write_rows_via<W: Write>(
        &self,
        writer: &mut W,
        batch: &Serie,
        scratch: &mut Vec<u8>,
    ) -> Result<()> {
        let children = self.check_batch(batch)?;
        for row in 0..batch.len() {
            scratch.clear();
            self.write_row(scratch, children, row)?;
            writer.write_all(scratch)?;
        }
        Ok(())
    }

    /// The columns of `batch`, proven to be this rowset's: one per column, of
    /// its name and datatype, holding no null where the rowset declares the
    /// column required.
    fn check_batch<'a>(&self, batch: &'a Serie) -> Result<&'a [Serie]> {
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
            if held.name() != field.name() {
                return Err(invalid(format_smolstr!(
                    "column `{}` is named otherwise: the rowset declares `{}` in its place",
                    held.name(),
                    field.name()
                )));
            }
            if held.dtype() != field.dtype() {
                return Err(invalid(format_smolstr!(
                    "column `{}` holds {}, the rowset declares {}",
                    field.name(),
                    held.dtype(),
                    field.dtype()
                )));
            }
            if !field.is_nullable() && child.null_count() > 0 {
                return Err(invalid(format_smolstr!(
                    "column `{}` holds {} null rows, and the rowset declares it required",
                    field.name(),
                    child.null_count()
                )));
            }
        }
        Ok(children)
    }

    /// Write row `row` of `children`, the columns [`Self::check_batch`] proved.
    fn write_row<W: Write>(&self, writer: &mut W, children: &[Serie], row: usize) -> Result<()> {
        write!(writer, "<{ROW_ELEMENT}>")?;
        for ((column, child), field) in self.columns.iter().zip(children).zip(self.field.fields()) {
            write_cell(writer, column, child, field, row).map_err(|error| {
                Error::InvalidRecord {
                    path: format_smolstr!("$[{row}].{}", field.name()),
                    reason: format_smolstr!("{error}"),
                }
            })?;
        }
        write!(writer, "</{ROW_ELEMENT}>")?;
        Ok(())
    }

    /// The row element as a column: the rowset's columns are its children.
    fn root_column(&self) -> Column {
        Column {
            element: SmolStr::new_static(ROW_ELEMENT),
            shape: Shape::Nested,
            children: self.columns.clone(),
        }
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
            let (column, field) = read_declaration(&declaration)?;
            columns.push(column);
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

    /// Read the rows of a `root` element under this rowset's columns, each
    /// cell under the element its column is spelled as, a nested child's
    /// included.
    ///
    /// An absent element and one marked `xsi:nil` are null; a repeated
    /// element is a sequence; text is read as the column's datatype through
    /// the field's own value contract, and text where a struct's elements
    /// belong is refused.
    ///
    /// # Errors
    ///
    /// Returns an error naming the row and column of a cell the column's
    /// datatype cannot read.
    pub fn read_rows(&self, root: &Element<'_>) -> Result<Serie> {
        let root_column = self.root_column();
        // An error the provider reported once the answer had begun - olap4j
        // writes it as `Messages/Error` inside the root - is the answer's
        // failure, never a shorter rowset.
        if let Some(reported) = root.children().find_map(|child| match child.local_name() {
            "Error" => super::response::XmlaError::from_element(&child),
            "Messages" => child
                .children()
                .find(|error| error.local_name() == "Error")
                .and_then(|error| super::response::XmlaError::from_element(&error)),
            _ => None,
        }) {
            return Err(invalid(format_smolstr!(
                "the provider reported an error inside the rowset: {} ({})",
                reported.description(),
                reported.code()
            )));
        }
        let mut rows = Vec::new();
        for (index, row) in root
            .children_in(Some(ROWSET_NAMESPACE), ROW_ELEMENT)
            .into_iter()
            .chain(root.children_in(None, ROW_ELEMENT))
            .enumerate()
        {
            let located = |error: Error| Error::InvalidRecord {
                path: format_smolstr!("$[{index}]"),
                reason: format_smolstr!("{error}"),
            };
            let record = read_value(&row, &self.field, &root_column).map_err(located)?;
            let shaped = crate::xml::shaped(record, &self.field);
            let canonical = self.field.from_natural_value(shaped).map_err(located)?;
            rows.push(canonical);
        }
        let borrowed: Vec<&Scalar> = rows.iter().collect();
        crate::serie::from_canonical_rows(Arc::clone(&self.field), &borrowed)
    }

    /// Read a whole `root` element: its rows under the columns `field`
    /// declares - what a declared field means to every medium, and what
    /// types a document a foreign provider wrote without a schema - or,
    /// with no field declared, under the columns the root's own `xsd:schema`
    /// states, each `xsd:dateTime` column an instant where its rows spell a
    /// zone.
    ///
    /// A root with no schema and no declared field is refused: its rows
    /// state no columns to read them as.
    ///
    /// # Errors
    ///
    /// Returns the schema's or the rows' refusal.
    pub fn read_root(root: &Element<'_>, field: Option<&Field>) -> Result<(Self, Serie)> {
        Self::read_root_with(
            root,
            field,
            crate::ArrowCastOptions::default().with_safe(false),
        )
    }

    /// [`Self::read_root`] under `cast`: where the root states its own
    /// columns and a field is declared, a strict cast reads each cell straight
    /// into the declared type and refuses one it cannot hold - an absent cell
    /// under a column the document or the caller requires included - naming
    /// its row and column; a `safe` cast reads the cells as stated and nulls
    /// each value the declared type cannot hold, as a safe cast does in every
    /// other encoding. The declared type of a datetime column is read
    /// directly either way, because whether an `xsd:dateTime` is an instant or
    /// a wall reading is the caller's to say.
    ///
    /// # Errors
    ///
    /// Returns the schema's, the rows' or the cast's refusal.
    pub fn read_root_with(
        root: &Element<'_>,
        field: Option<&Field>,
        cast: crate::ArrowCastOptions,
    ) -> Result<(Self, Serie)> {
        match (field, root.child(Some(XSD_NAMESPACE), "schema")) {
            (Some(field), Some(schema)) => {
                // The document's own schema says which element holds which
                // column - `sql:field`, another writer's element names - and
                // the declared field says what each is read as, so a cell
                // parses straight into the type asked for and the cast onto
                // the field reorders, drops and completes, converting nothing.
                let stated = Self::from_schema(&schema)?.zoned_where_the_rows_are(root)?;
                let stated = if cast.is_safe() {
                    stated.typed_as(field, false, |_, wanted| {
                        matches!(wanted.dtype(), DataType::DateTime64 { .. })
                    })?
                } else {
                    stated.typed_as(field, true, |_, _| true)?
                };
                let rows = stated.read_rows(root)?;
                let rows = rows.cast(field, cast)?;
                Ok((Self::new(field.clone())?, rows))
            }
            (Some(field), None) => {
                let declared = Self::new(field.clone())?;
                let rows = declared.read_rows(root)?;
                Ok((declared, rows))
            }
            (None, Some(schema)) => {
                let stated = Self::from_schema(&schema)?.zoned_where_the_rows_are(root)?;
                let rows = stated.read_rows(root)?;
                Ok((stated, rows))
            }
            (None, None) => Err(invalid(
                "the rowset carries no schema and no field is declared to read its rows by",
            )),
        }
    }

    /// The field a rowset `root` element states for its rows: its own
    /// `xsd:schema`, each `xsd:dateTime` column an instant where the rows
    /// spell a zone, as [`Self::read_root`] reads it. `None` where the root
    /// carries no schema, and for a root that is no rowset - a
    /// multidimensional dataset, an empty answer - since neither states rows.
    ///
    /// # Errors
    ///
    /// Returns the schema's refusal.
    pub fn stated_field(root: &Element<'_>) -> Result<Option<Field>> {
        if !matches!(root.namespace(), Some(ROWSET_NAMESPACE) | None) {
            return Ok(None);
        }
        root.child(Some(XSD_NAMESPACE), "schema")
            .map(|schema| {
                Ok(Self::from_schema(&schema)?
                    .zoned_where_the_rows_are(root)?
                    .field()
                    .clone())
            })
            .transpose()
    }

    /// This rowset with every column a declared field also names, and
    /// `adopt` admits, read as that field's datatype - what the caller asked
    /// for, which a text cell parses into directly - and, when `strict`,
    /// absent only where both the document and the caller allow it; every
    /// other column is kept as stated, so the cast onto the declared field
    /// converts nothing for the columns adopted. The element each column is
    /// spelled under is the document's whatever it is read as.
    fn typed_as(
        mut self,
        declared: &Field,
        strict: bool,
        adopt: impl Fn(&Field, &Field) -> bool,
    ) -> Result<Self> {
        let fields = typed_fields(self.field.fields(), declared.fields(), &adopt, strict);
        let field = Field::new(
            self.field.name(),
            DataType::from(StructType::from_fields(fields)?),
            false,
        );
        self.columns = self
            .columns
            .iter()
            .zip(field.fields())
            .map(|(column, field)| column.retyped(field))
            .collect();
        self.field = Arc::new(field);
        Ok(self)
    }

    /// This rowset with each `xsd:dateTime` leaf - a column, a struct's
    /// child, a sequence's item - read as an instant where its rows spell a
    /// zone.
    ///
    /// XML Schema's `dateTime` is a wall reading or an instant, and the schema
    /// does not say which; the rows do. A leaf whose first present value
    /// ends in `Z` or an offset is a UTC instant, and one whose first value
    /// carries no zone stays naive - the one decision the document leaves to
    /// its rows, made once per leaf, never per row.
    fn zoned_where_the_rows_are(mut self, root: &Element<'_>) -> Result<Self> {
        let mut paths = Vec::new();
        naive_datetime_paths(self.field.fields(), &mut Vec::new(), &mut paths);
        if paths.is_empty() {
            return Ok(self);
        }
        let root_column = self.root_column();
        let mut zoned: Vec<Option<bool>> = vec![None; paths.len()];
        for row in root
            .children_in(Some(ROWSET_NAMESPACE), ROW_ELEMENT)
            .into_iter()
            .chain(root.children_in(None, ROW_ELEMENT))
        {
            for (slot, path) in paths.iter().enumerate() {
                if zoned[slot].is_none() {
                    zoned[slot] = first_zone(&row, &self.field, &root_column, path);
                }
            }
            if zoned.iter().all(Option::is_some) {
                break;
            }
        }
        let mut field = Field::clone(&self.field);
        for (path, _) in paths
            .iter()
            .zip(&zoned)
            .filter(|(_, decided)| **decided == Some(true))
        {
            field = with_instant(&field, path)?;
        }
        // An instant and a wall reading are both leaves, so the columns'
        // shapes and elements stand.
        self.field = Arc::new(field);
        Ok(self)
    }
}

/// The paths - struct child indexes, a sequence's item passed through - of
/// every naive datetime leaf under `fields`, appended to `paths`.
fn naive_datetime_paths(fields: &[Field], prefix: &mut Vec<usize>, paths: &mut Vec<Vec<usize>>) {
    for (index, field) in fields.iter().enumerate() {
        prefix.push(index);
        let leaf = match field.dtype() {
            DataType::Serie(item)
            | DataType::SerieView(item)
            | DataType::FixedSizeSerie(item, _)
            | DataType::LargeSerie(item)
            | DataType::LargeSerieView(item) => item.as_ref(),
            _ => field,
        };
        match leaf.dtype() {
            DataType::DateTime64 { timezone, .. } if timezone.is_naive() => {
                paths.push(prefix.clone());
            }
            DataType::Struct(_) => naive_datetime_paths(leaf.fields(), prefix, paths),
            _ => {}
        }
        prefix.pop();
    }
}

/// Whether the first non-empty text at `path` under `element` - a struct's
/// child by index, each element of a repeated one - spells a zone; `None`
/// where the row holds no such text, which decides nothing.
fn first_zone(
    element: &Element<'_>,
    field: &Field,
    column: &Column,
    path: &[usize],
) -> Option<bool> {
    let Some((head, rest)) = path.split_first() else {
        return element
            .text()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(spells_a_zone);
    };
    let child_field = nested_fields(field).get(*head)?;
    let child_column = column.children.get(*head)?;
    element
        .children()
        .filter(|child| child.local_name() == child_column.element)
        .find_map(|child| first_zone(&child, child_field, child_column, rest))
}

/// `field` with the naive datetime leaf at `path` read as a UTC instant at
/// its own unit, a sequence's item passed through.
fn with_instant(field: &Field, path: &[usize]) -> Result<Field> {
    let dtype = match (field.dtype(), path.split_first()) {
        (DataType::Struct(_), Some((head, rest))) => {
            let fields = field
                .fields()
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    if index == *head {
                        with_instant(child, rest)
                    } else {
                        Ok(child.clone())
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            DataType::from(StructType::from_fields(fields)?)
        }
        (DataType::Serie(item), _) => DataType::Serie(Arc::new(with_instant(item, path)?)),
        (DataType::SerieView(item), _) => DataType::SerieView(Arc::new(with_instant(item, path)?)),
        (DataType::FixedSizeSerie(item, length), _) => {
            DataType::FixedSizeSerie(Arc::new(with_instant(item, path)?), *length)
        }
        (DataType::LargeSerie(item), _) => {
            DataType::LargeSerie(Arc::new(with_instant(item, path)?))
        }
        (DataType::LargeSerieView(item), _) => {
            DataType::LargeSerieView(Arc::new(with_instant(item, path)?))
        }
        (DataType::DateTime64 { unit, .. }, None) => DataType::DateTime64 {
            unit: *unit,
            timezone: Timezone::UTC,
        },
        (other, _) => other.clone(),
    };
    Ok(Field::new(field.name(), dtype, field.is_nullable()))
}

/// Whether an `xsd:dateTime` text ends in a zone designator: `Z`, or an
/// offset `+hh:mm` / `-hh:mm` after the time of day, a bracketed zone name
/// after either (`+01:00[Europe/Paris]`) read past.
fn spells_a_zone(text: &str) -> bool {
    let text = text.split('[').next().unwrap_or(text).trim_end();
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
/// The XMLA error a failure is reported as inside a streamed rowset.
fn reported(error: &Error) -> super::response::XmlaError {
    super::response::XmlaError::new(super::service::code::EXECUTION_FAILED, error.to_string())
        .with_source(super::response::ACTOR)
}

/// Whether a datatype is one of the five sequence layouts.
const fn is_sequence(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Serie(_)
            | DataType::SerieView(_)
            | DataType::FixedSizeSerie(_, _)
            | DataType::LargeSerie(_)
            | DataType::LargeSerieView(_)
    )
}

/// `stated` with each column `declared` also names and `adopt` admits
/// carrying the declared datatype, a struct column's children the same way.
/// Under `strict` a cell may be absent only where both the document and the
/// caller allow it - whether a cell may be absent is the document's fact, and
/// a caller who requires the column holds no absence either - so an absent
/// cell under a column either requires is refused where it is read;
/// otherwise the stated nullability is kept, and the cast nulls what the
/// declared field cannot hold.
fn typed_fields(
    stated: &[Field],
    declared: &[Field],
    adopt: &impl Fn(&Field, &Field) -> bool,
    strict: bool,
) -> Vec<Field> {
    stated
        .iter()
        .map(|column| {
            let Some(wanted) = declared.iter().find(|held| held.name() == column.name()) else {
                return column.clone();
            };
            let dtype = match (column.dtype(), wanted.dtype()) {
                (DataType::Struct(inner), DataType::Struct(want)) => {
                    StructType::from_fields(typed_fields(inner, want, adopt, strict))
                        .map_or_else(|_| wanted.dtype().clone(), DataType::from)
                }
                _ if adopt(column, wanted) => wanted.dtype().clone(),
                _ => return column.clone(),
            };
            let nullable = column.is_nullable() && (!strict || wanted.is_nullable());
            Field::new(column.name(), dtype, nullable)
        })
        .collect()
}

fn check_column(field: &Field, path: &str) -> Result<()> {
    if field.name().contains('\0') {
        return Err(invalid(format_smolstr!(
            "column `{path}` carries U+0000 in its name, which no XML name spells"
        )));
    }
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
        _ => match dtype.string_parameters() {
            Some(leaf) if matches!(leaf.charset(), Charset::Utf8 | Charset::Ascii) => {
                if leaf.is_fixed() {
                    Shape::FixedText
                } else {
                    Shape::Text
                }
            }
            _ if dtype.bytes_parameters().is_some() => Shape::Bytes,
            _ => Shape::Leaf,
        },
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
        Shape::Text | Shape::FixedText if child.is_string_storage() => {
            let Some(bytes) = child.value_bytes(row) else {
                return Ok(());
            };
            let bytes = if column.shape == Shape::FixedText {
                // The NUL padding past a fixed slot's text is the storage's.
                let end = bytes
                    .iter()
                    .rposition(|byte| *byte != 0)
                    .map_or(0, |last| last + 1);
                &bytes[..end]
            } else {
                bytes
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
        Shape::Nested | Shape::Text | Shape::FixedText | Shape::Bytes | Shape::Leaf => {
            let Some(value) = child.get(row) else {
                return Ok(());
            };
            write_nested(writer, column, &value, field)
        }
    }
}

/// Write one canonical value under the column's element: nothing for null, a
/// struct as its children in the order its field declares them - each under
/// the element its own column names, the one the schema declares - a
/// sequence as one element per item, and a leaf as its XML Schema text.
fn write_nested<W: Write>(
    writer: &mut W,
    column: &Column,
    value: &Scalar,
    field: &Field,
) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let element = &column.element;
    match field.dtype() {
        DataType::Struct(fields) => {
            write!(writer, "<{element}>")?;
            for (index, child) in fields.iter().enumerate() {
                // A canonical row is an ordered run; a named record is read by
                // name, so a value built either way writes.
                let held = match value.as_struct() {
                    Some(entries) => entries.get(child.name()).map(Cow::Borrowed),
                    None => value
                        .sequence_rows()
                        .and_then(|items| items.get(index).cloned().map(Cow::Owned)),
                };
                if let Some(held) = held {
                    let spelled;
                    let child_column = match column.children.get(index) {
                        Some(child_column) => child_column,
                        None => {
                            spelled = Column::of(child, encode_name(child.name()));
                            &spelled
                        }
                    };
                    write_nested(writer, child_column, &held, child)?;
                }
            }
            write!(writer, "</{element}>")?;
            Ok(())
        }
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => match value.sequence_rows() {
            // A null item keeps its place as `<item/>`, which reads back as
            // null; a null column is absent altogether.
            Some(items) => items.iter().try_for_each(|held| {
                if held.is_null() {
                    write!(writer, "<{element}/>")?;
                    Ok(())
                } else {
                    write_nested(writer, column, held, item)
                }
            }),
            None => write_nested(writer, column, value, item),
        },
        _ => {
            write!(writer, "<{element}>")?;
            write_leaf(writer, value, field)?;
            write!(writer, "</{element}>")?;
            Ok(())
        }
    }
}

/// Write a leaf's XML Schema text: an instant as `xsd:dateTime` spells one -
/// the offset alone, never the bracketed zone name the crate's own form adds -
/// and every other leaf as the XML codec spells it.
fn write_leaf<W: Write>(writer: &mut W, value: &Scalar, field: &Field) -> Result<()> {
    if let Scalar::DateTime64(instant) = value {
        if !instant.timezone().is_naive() {
            let text = crate::temporal::format_timestamp(
                instant.count(),
                instant.unit(),
                &instant.timezone(),
            )
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "column `{}` holds the datetime count {}, which has no ISO 8601 spelling",
                    field.name(),
                    instant.count()
                ))
            })?;
            let offset_only = text.split('[').next().unwrap_or(&text);
            crate::xml::write_element_text(writer, offset_only)?;
            return Ok(());
        }
    }
    crate::xml::write_leaf_text(writer, value, field.name())
}

/// Read one element as the value of `field`: null where it is marked `nil` in
/// the XML Schema instance namespace under whatever prefix the document
/// bound, a struct as its children - each read as the child whose column
/// names its element, the name the schema's `sql:field` states at every
/// level - a sequence as one item per element, aggregated by the parent, and
/// a leaf as its text, a byte column's with the whitespace XML Schema
/// collapses taken out. A required child missing from a struct is refused by
/// name, and so is text where a struct's elements belong.
fn read_value(element: &Element<'_>, field: &Field, column: &Column) -> Result<Scalar> {
    // A nil leaf is absent; a nil struct that may be absent is absent, and one
    // that may not - the row itself, `<row/>` - is a struct of absent cells,
    // each judged by its own column; a nil sequence element is one null item,
    // since an absent sequence column is spelled by no element at all.
    if element.is_nil()
        && !is_sequence(field.dtype())
        && (field.is_nullable() || !matches!(field.dtype(), DataType::Struct(_)))
    {
        return Ok(Scalar::Null);
    }
    match field.dtype() {
        DataType::Struct(fields) => {
            // A struct's cell holds elements: text of its own, past the
            // whitespace a document is laid out with, is a value no child
            // reads.
            if let Some(text) = element.text() {
                if !text.trim().is_empty() {
                    return Err(invalid(format_smolstr!(
                        "`{}` holds the text {text:?}, and a struct column holds elements",
                        field.name()
                    )));
                }
            }
            let mut entries: Vec<(SmolStr, Scalar)> = Vec::new();
            // A sequence column's elements, one item each, in document order:
            // gathered apart so one item, or one null item (`<sizes/>`), is
            // the one-item sequence it is and never a lone value.
            let mut sequences: Vec<(SmolStr, Vec<Scalar>)> = Vec::new();
            for child in element.children() {
                let local = child.local_name();
                let known = column
                    .children
                    .iter()
                    .zip(fields.iter())
                    .find(|(held, _)| held.element == local);
                let Some((child_column, child_field)) = known else {
                    // Unknown to the schema: kept as spelled, for the field's
                    // own value contract to refuse by name.
                    entries.push((decode_name(local), child.value().clone()));
                    continue;
                };
                let name = SmolStr::new(child_field.name());
                let value = read_value(&child, child_field, child_column)?;
                if is_sequence(child_field.dtype()) {
                    match sequences.iter_mut().find(|(held, _)| *held == name) {
                        Some((_, items)) => items.push(value),
                        None => sequences.push((name, vec![value])),
                    }
                } else {
                    entries.push((name, value));
                }
            }
            entries.extend(
                sequences
                    .into_iter()
                    .map(|(name, items)| (name, Scalar::from_sequence(items))),
            );
            for child in fields.iter() {
                // A sequence occurring no time is the empty sequence.
                if !child.is_nullable()
                    && !is_sequence(child.dtype())
                    && child.dtype() != &DataType::Null
                    && !entries.iter().any(|(held, _)| *held == child.name())
                {
                    return Err(invalid(format_smolstr!(
                        "`{}` is missing from the row, and the rowset declares it required",
                        child.name()
                    )));
                }
            }
            Scalar::from_struct(entries)
        }
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => read_value(element, item, column),
        dtype => match element.text() {
            // olap4j's tabular rows type every cell with `xsi:type` and spell
            // an absent one as the text `null`; under a column that is not
            // text - a byte column included, although `null` is also four
            // base64 digits - that spelling can be nothing but absence.
            Some(text)
                if text.trim() == "null"
                    && dtype.string_parameters().is_none()
                    && element.attribute_in(Some(XSI_NAMESPACE), "type").is_some() =>
            {
                Ok(Scalar::Null)
            }
            Some(text) if dtype.bytes_parameters().is_some() => {
                Ok(Scalar::from(text.split_whitespace().collect::<String>()))
            }
            Some(text) => Ok(Scalar::from(text)),
            None => Ok(element.value().clone()),
        },
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
    // A sequence occurs no time when it is empty, so it is always optional.
    if unbounded || field.is_nullable() || item.is_nullable() || field.dtype() == &DataType::Null {
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

/// Read one column's `xsd:element` declaration: the column - its element
/// name, and its children's - and the field it declares.
fn read_declaration(declaration: &Element<'_>) -> Result<(Column, Field)> {
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
    let mut children = Vec::new();
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
                        let (column, field) = read_declaration(&child)?;
                        children.push(column);
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
    let column = Column {
        element: SmolStr::new(element),
        shape: shape_of(field.dtype()),
        children,
    };
    Ok((column, field))
}

pub(crate) fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.rowset"),
        reason: reason.into(),
    }
}

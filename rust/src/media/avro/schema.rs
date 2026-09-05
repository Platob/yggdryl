//! The Avro schema model: named types, logical types, and fingerprints.
//!
//! A [`Schema`] is parsed from the JSON representation the format defines and
//! keeps that JSON, so a schema read from a container header round-trips with
//! every attribute this implementation does not model - Iceberg's `field-id`
//! rides there and survives byte for byte. Namespaces, aliases, defaults, and
//! recursive references are resolved at parse time into a node tree; a
//! reference to a named type stays a reference, which is what lets a recursive
//! schema be finite.
//!
//! Logical types are modeled because the value model above this codec is
//! typed: a `date` int decodes as a calendar [`Date32`](crate::types::temporal::Date32)
//! rather than a bare count, and a `decimal` keeps its unscaled integer and
//! scale exactly. An annotation this implementation does not know - or one
//! whose attributes are invalid for its underlying type - degrades to the
//! underlying type, as the specification requires, never to an error.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::{Limits, Result, Scalar};

use super::datum::invalid;

/// Maximum structural nesting accepted by the recursive schema parser.
///
/// The JSON the schema arrives in is already bounded by the JSON parser's own
/// hard cap, and this cap mirrors it so a caller-supplied [`Limits`] can only
/// tighten the bound, never widen it past what the stack tolerates.
pub const MAX_SCHEMA_DEPTH: usize = 384;

/// One parsed Avro schema.
///
/// Cloning is cheap: the node tree and the name registry are shared, and the
/// JSON the schema round-trips as is the same shared [`Scalar`] it was parsed
/// from.
#[derive(Clone)]
pub struct Schema {
    /// The root node.
    pub(crate) node: Node,
    /// Every named type in the schema, by fullname.
    pub(crate) names: Arc<HashMap<SmolStr, Node>>,
    /// The JSON this schema round-trips as.
    json: Scalar,
    /// Complete JSON identity with every object normalized to a sorted record.
    identity: Scalar,
    /// Parsing Canonical Form, shared by cheap clones and filled on demand.
    canonical: Arc<OnceLock<String>>,
}

/// One node of the parsed schema tree.
///
/// Logical annotations the value model can honor are their own variants,
/// because they change what a datum decodes *to* even though they never change
/// how it is encoded. A recursive or repeated reference to a named type is a
/// [`Node::Ref`] resolved through the schema's name registry.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    /// The empty type, encoded as zero bytes.
    Null,
    /// A single byte holding zero or one.
    Boolean,
    /// A zig-zag variable-length 32-bit integer.
    Int,
    /// A zig-zag variable-length 64-bit integer.
    Long,
    /// Four little-endian IEEE 754 bytes.
    Float,
    /// Eight little-endian IEEE 754 bytes.
    Double,
    /// A length-prefixed byte string.
    Bytes,
    /// A length-prefixed UTF-8 string.
    String,
    /// `date` over int: days since the Unix epoch.
    Date,
    /// `time-millis` over int: milliseconds since midnight.
    TimeMillis,
    /// `time-micros` over long: microseconds since midnight.
    TimeMicros,
    /// `timestamp-millis` over long: an instant in milliseconds.
    TimestampMillis,
    /// `timestamp-micros` over long: an instant in microseconds.
    TimestampMicros,
    /// `timestamp-nanos` over long: an instant in nanoseconds.
    TimestampNanos,
    /// `local-timestamp-millis` over long: a wall clock in milliseconds.
    LocalTimestampMillis,
    /// `local-timestamp-micros` over long: a wall clock in microseconds.
    LocalTimestampMicros,
    /// `local-timestamp-nanos` over long: a wall clock in nanoseconds.
    LocalTimestampNanos,
    /// `uuid` over string.
    Uuid,
    /// `decimal` over bytes or fixed: unscaled big-endian two's complement.
    Decimal(Arc<DecimalType>),
    /// `duration` over fixed(12): months, days, milliseconds, each u32.
    Duration(Arc<FixedType>),
    /// `uuid` over fixed(16): the raw byte form.
    UuidFixed(Arc<FixedType>),
    /// Named ordered fields, encoded back to back.
    Record(Arc<RecordType>),
    /// A symbol chosen by index.
    Enum(Arc<EnumType>),
    /// Exactly `size` raw bytes, under a name.
    Fixed(Arc<FixedType>),
    /// Length-prefixed blocks of one item type.
    Array(Arc<Node>),
    /// Length-prefixed blocks of string-keyed values.
    Map(Arc<Node>),
    /// A branch index followed by that branch's value.
    Union(Arc<[Node]>),
    /// A reference to a named type declared elsewhere in the schema.
    Ref(SmolStr),
}

/// The fields of one Avro record type, in declaration order.
#[derive(Debug)]
pub(crate) struct RecordType {
    /// The record's fullname.
    pub(crate) name: SmolStr,
    /// Alternate fullnames a resolving reader may match the record by.
    pub(crate) aliases: Vec<SmolStr>,
    /// The fields, in encoding order.
    pub(crate) fields: Vec<FieldType>,
}

/// One field of a record type.
#[derive(Debug)]
pub(crate) struct FieldType {
    /// The field's name.
    pub(crate) name: SmolStr,
    /// Alternate names a resolving reader may match the field by.
    pub(crate) aliases: Vec<SmolStr>,
    /// The field's schema.
    pub(crate) schema: Node,
    /// The Iceberg `field-id` attribute, when the schema carries one.
    ///
    /// Iceberg resolves a column by identifier rather than by name, so the
    /// attribute is surfaced onto the decoded field's metadata rather than
    /// merely surviving as an unmodeled attribute.
    #[cfg(feature = "arrow")]
    pub(crate) field_id: Option<i32>,
    /// The declared default, as the JSON it was written with.
    ///
    /// Kept raw because the specification only consults a default during
    /// schema resolution; converting lazily means a malformed default on a
    /// field nobody fills is never an error.
    pub(crate) default: Option<Scalar>,
}

/// One Avro enum type.
#[derive(Debug)]
pub(crate) struct EnumType {
    /// The enum's fullname.
    pub(crate) name: SmolStr,
    /// Alternate fullnames a resolving reader may match the enum by.
    pub(crate) aliases: Vec<SmolStr>,
    /// The symbols, in index order.
    pub(crate) symbols: Arc<[SmolStr]>,
    /// The symbol an unknown writer symbol resolves to, when declared.
    pub(crate) default: Option<SmolStr>,
}

/// One Avro fixed type.
#[derive(Debug)]
pub(crate) struct FixedType {
    /// The fixed's fullname.
    pub(crate) name: SmolStr,
    /// Alternate fullnames a resolving reader may match the fixed by.
    pub(crate) aliases: Vec<SmolStr>,
    /// The exact encoded size in bytes.
    pub(crate) size: usize,
}

/// A `decimal` annotation over bytes or over a named fixed.
#[derive(Debug)]
pub(crate) struct DecimalType {
    /// Maximum number of decimal digits.
    pub(crate) precision: u32,
    /// Digits after the decimal point.
    pub(crate) scale: u32,
    /// The underlying fixed type, when the decimal is not over bytes.
    pub(crate) fixed: Option<Arc<FixedType>>,
}

impl Schema {
    /// Read an Avro schema from its JSON representation.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not an Avro schema, naming the
    /// construct that was found and where.
    pub fn from_json(document: &Scalar) -> Result<Self> {
        Self::from_json_with_limits(document, Limits::default())
    }

    /// Read an Avro schema from its JSON representation with explicit limits.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not an Avro schema or exceeds
    /// the limits.
    pub fn from_json_with_limits(document: &Scalar, limits: Limits) -> Result<Self> {
        let mut parser = Parser {
            names: HashMap::new(),
            depth_limit: limits.max_depth().min(MAX_SCHEMA_DEPTH),
        };
        let node = parser.parse(document, "", 0)?;
        Ok(Self {
            node,
            names: Arc::new(parser.names),
            json: document.clone(),
            identity: normalized_schema_json(document)?,
            canonical: Arc::new(OnceLock::new()),
        })
    }

    /// Read an Avro schema from its JSON text.
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not JSON or not a schema.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Self> {
        <Self as std::str::FromStr>::from_str(input)
    }

    /// Read an Avro schema from its JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not JSON or not a schema.
    pub fn from_slice(input: &[u8]) -> Result<Self> {
        Self::from_json(&crate::text::json::from_bytes(input)?)
    }

    /// Return the JSON this schema round-trips as.
    ///
    /// A parsed schema answers with the exact document it was parsed from, so
    /// attributes this implementation does not model survive verbatim.
    pub fn into_json(self) -> Scalar {
        self.json
    }

    /// Borrow the exact JSON document this schema was parsed from.
    pub const fn as_json(&self) -> &Scalar {
        &self.json
    }

    /// Return the name a caller would use to refer to the root of this schema.
    pub fn kind(&self) -> &'static str {
        self.node.kind()
    }

    /// Render the Parsing Canonical Form: the one spelling of this schema
    /// every implementation agrees on, which is what fingerprints hash.
    pub fn into_canonical_form(self) -> String {
        self.canonical_form().to_owned()
    }

    fn canonical_form(&self) -> &str {
        self.canonical.get_or_init(|| {
            let mut output = String::new();
            let mut printed = Vec::new();
            canonical(&self.node, &self.names, &mut printed, &mut output);
            output
        })
    }

    /// Return the 64-bit Rabin fingerprint of the canonical form.
    ///
    /// This is the fingerprint single-object encoding frames a datum with,
    /// and the natural cache key for a resolution plan.
    pub fn fingerprint(&self) -> u64 {
        rabin(self.canonical_form().as_bytes())
    }

    /// Return a deterministic hash of the complete retained schema document.
    ///
    /// Unlike an Avro fingerprint, this preserves logical annotations,
    /// defaults, aliases, and extension attributes that affect this schema's
    /// behavior or round trip.
    pub fn stable_hash(&self) -> u64 {
        self.identity.stable_hash()
    }
}

impl fmt::Debug for Schema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Schema")
            .field("json", &self.json)
            .finish()
    }
}

impl PartialEq for Schema {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for Schema {}

impl PartialOrd for Schema {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Schema {
    fn cmp(&self, other: &Self) -> Ordering {
        self.identity.cmp(&other.identity)
    }
}

impl Hash for Schema {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
    }
}

/// Normalize JSON objects independently of how a language boundary represented
/// them (`Mapping` or `Record`) while preserving every key and nested value.
fn normalized_schema_json(value: &Scalar) -> Result<Scalar> {
    use crate::types::Nested;

    match value {
        Scalar::Nested(Nested::Sequence(values)) => Ok(Scalar::from_sequence(
            values
                .as_slice()
                .iter()
                .map(normalized_schema_json)
                .collect::<Result<Vec<_>>>()?,
        )),
        Scalar::Nested(Nested::Record(entries)) => Scalar::from_record(
            entries
                .as_map()
                .iter()
                .map(|(name, value)| {
                    normalized_schema_json(value).map(|value| (name.clone(), value))
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        Scalar::Nested(Nested::Mapping(entries)) => Scalar::from_record(
            entries
                .as_slice()
                .iter()
                .map(|(name, value)| {
                    let name = name.as_str().ok_or_else(|| {
                        invalid(format_smolstr!(
                            "expected every Avro schema object key to be text, got {}",
                            name.kind()
                        ))
                    })?;
                    Ok((name, normalized_schema_json(value)?))
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        scalar => Ok(scalar.clone()),
    }
}

impl std::str::FromStr for Schema {
    type Err = crate::Error;

    fn from_str(input: &str) -> Result<Self> {
        Self::from_json(&crate::text::json::from_utf8(input)?)
    }
}

impl Node {
    /// Return the name a caller would use to refer to this schema node.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Int => "int",
            Self::Long => "long",
            Self::Float => "float",
            Self::Double => "double",
            Self::Bytes => "bytes",
            Self::String => "string",
            Self::Date => "date",
            Self::TimeMillis => "time-millis",
            Self::TimeMicros => "time-micros",
            Self::TimestampMillis => "timestamp-millis",
            Self::TimestampMicros => "timestamp-micros",
            Self::TimestampNanos => "timestamp-nanos",
            Self::LocalTimestampMillis => "local-timestamp-millis",
            Self::LocalTimestampMicros => "local-timestamp-micros",
            Self::LocalTimestampNanos => "local-timestamp-nanos",
            Self::Uuid => "uuid",
            Self::Decimal(_) => "decimal",
            Self::Duration(_) => "duration",
            Self::UuidFixed(_) => "uuid",
            Self::Record(_) => "record",
            Self::Enum(_) => "enum",
            Self::Fixed(_) => "fixed",
            Self::Array(_) => "array",
            Self::Map(_) => "map",
            Self::Union(_) => "union",
            Self::Ref(_) => "reference",
        }
    }

    /// Return the primitive the node encodes as on the wire.
    ///
    /// Logical annotations never change the encoding, so resolution matches
    /// on this - a writer `long` and a reader `timestamp-micros` carry the
    /// same bytes.
    pub(crate) fn wire(&self) -> Wire {
        match self {
            Self::Null => Wire::Null,
            Self::Boolean => Wire::Boolean,
            Self::Int | Self::Date | Self::TimeMillis => Wire::Int,
            Self::Long
            | Self::TimeMicros
            | Self::TimestampMillis
            | Self::TimestampMicros
            | Self::TimestampNanos
            | Self::LocalTimestampMillis
            | Self::LocalTimestampMicros
            | Self::LocalTimestampNanos => Wire::Long,
            Self::Float => Wire::Float,
            Self::Double => Wire::Double,
            Self::Bytes => Wire::Bytes,
            Self::String | Self::Uuid => Wire::String,
            Self::Decimal(decimal) => match &decimal.fixed {
                Some(fixed) => Wire::Fixed(fixed.size),
                None => Wire::Bytes,
            },
            Self::Duration(fixed) | Self::UuidFixed(fixed) | Self::Fixed(fixed) => {
                Wire::Fixed(fixed.size)
            }
            Self::Record(_) => Wire::Record,
            Self::Enum(_) => Wire::Enum,
            Self::Array(_) => Wire::Array,
            Self::Map(_) => Wire::Map,
            Self::Union(_) => Wire::Union,
            Self::Ref(_) => Wire::Ref,
        }
    }

    /// Return the fullname when the node is a named type.
    pub(crate) fn name(&self) -> Option<&SmolStr> {
        match self {
            Self::Record(record) => Some(&record.name),
            Self::Enum(symbols) => Some(&symbols.name),
            Self::Fixed(fixed) | Self::Duration(fixed) | Self::UuidFixed(fixed) => {
                Some(&fixed.name)
            }
            Self::Decimal(decimal) => decimal.fixed.as_deref().map(|fixed| &fixed.name),
            _ => None,
        }
    }

    /// Return the aliases when the node is a named type.
    pub(crate) fn aliases(&self) -> &[SmolStr] {
        match self {
            Self::Record(record) => &record.aliases,
            Self::Enum(symbols) => &symbols.aliases,
            Self::Fixed(fixed) | Self::Duration(fixed) | Self::UuidFixed(fixed) => &fixed.aliases,
            Self::Decimal(decimal) => decimal
                .fixed
                .as_deref()
                .map_or(&[], |fixed| fixed.aliases.as_slice()),
            _ => &[],
        }
    }
}

/// The wire shape a node encodes as, which is what resolution matches on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Wire {
    /// Zero bytes.
    Null,
    /// One byte.
    Boolean,
    /// A zig-zag variable-length 32-bit integer.
    Int,
    /// A zig-zag variable-length 64-bit integer.
    Long,
    /// Four little-endian bytes.
    Float,
    /// Eight little-endian bytes.
    Double,
    /// A length-prefixed byte run.
    Bytes,
    /// A length-prefixed UTF-8 byte run.
    String,
    /// A declared number of raw bytes.
    Fixed(usize),
    /// Fields back to back.
    Record,
    /// A symbol index.
    Enum,
    /// Item blocks.
    Array,
    /// Entry blocks.
    Map,
    /// A branch index and the branch.
    Union,
    /// Resolved through the name registry before use.
    Ref,
}

/// The recursive schema parser and its name registry.
struct Parser {
    /// Every named type declared so far, by fullname.
    names: HashMap<SmolStr, Node>,
    /// The effective nesting bound.
    depth_limit: usize,
}

impl Parser {
    /// Parse one schema node in an enclosing namespace.
    fn parse(&mut self, document: &Scalar, namespace: &str, depth: usize) -> Result<Node> {
        if depth >= self.depth_limit {
            return Err(invalid(format_smolstr!(
                "expected an Avro schema at most {} levels deep",
                self.depth_limit
            )));
        }
        if let Some(name) = document.as_str() {
            return self.resolve(name, namespace);
        }
        if let Some(branches) = document.as_sequence() {
            let mut parsed = Vec::with_capacity(branches.len());
            for branch in branches {
                parsed.push(self.parse(branch, namespace, depth + 1)?);
            }
            return Ok(Node::Union(parsed.into()));
        }
        if document.as_record().is_none() && document.as_mapping().is_none() {
            return Err(invalid(format_smolstr!(
                "expected an Avro schema name, union, or object, got {}",
                document.kind()
            )));
        }

        let type_name = document
            .get_key_str("type")
            .ok_or_else(|| invalid(SmolStr::new_static("expected an Avro schema \"type\"")))?;
        // A `type` may itself be a nested schema, which is how a logical
        // annotation is spelled over an array or a union.
        let Some(type_name) = type_name.as_str() else {
            return self.parse(type_name, namespace, depth + 1);
        };

        let logical = document.get_key_str("logicalType").and_then(Scalar::as_str);
        match type_name {
            "record" | "error" => self.parse_record(document, namespace, depth),
            "enum" => self.parse_enum(document, namespace),
            "fixed" => self.parse_fixed(document, namespace, logical),
            "array" => {
                let items = document.get_key_str("items").ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected an Avro array \"items\" schema",
                    ))
                })?;
                Ok(Node::Array(Arc::new(self.parse(
                    items,
                    namespace,
                    depth + 1,
                )?)))
            }
            "map" => {
                let values = document.get_key_str("values").ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected an Avro map \"values\" schema",
                    ))
                })?;
                Ok(Node::Map(Arc::new(self.parse(
                    values,
                    namespace,
                    depth + 1,
                )?)))
            }
            primitive => {
                let node = self.resolve(primitive, namespace)?;
                Ok(annotate(node, logical, document))
            }
        }
    }

    /// Parse a record, registering it before its own fields are read so a
    /// self-referential field resolves.
    fn parse_record(&mut self, document: &Scalar, namespace: &str, depth: usize) -> Result<Node> {
        let (fullname, child_namespace) = declared_name(document, namespace)?;
        self.check_unregistered(&fullname)?;
        let aliases = declared_aliases(document, &child_namespace);
        let entries = document
            .get_key_str("fields")
            .and_then(Scalar::as_sequence)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an Avro record \"fields\" array on {fullname:?}"
                ))
            })?;

        // Register a placeholder reference first: a field of this record's own
        // type parses as a reference, which the finished registration answers.
        self.names
            .insert(fullname.clone(), Node::Ref(fullname.clone()));

        let mut fields = Vec::with_capacity(entries.len());
        for entry in entries {
            let field_name = entry
                .get_key_str("name")
                .and_then(Scalar::as_str)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected an Avro field \"name\" inside {fullname:?}"
                    ))
                })?;
            let field_type = entry.get_key_str("type").ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an Avro field \"type\" on {fullname:?}.{field_name}"
                ))
            })?;
            let field_aliases = entry
                .get_key_str("aliases")
                .and_then(Scalar::as_sequence)
                .map(|aliases| {
                    aliases
                        .iter()
                        .filter_map(Scalar::as_str)
                        .map(SmolStr::new)
                        .collect()
                })
                .unwrap_or_default();
            fields.push(FieldType {
                name: SmolStr::new(field_name),
                aliases: field_aliases,
                schema: self.parse(field_type, &child_namespace, depth + 1)?,
                #[cfg(feature = "arrow")]
                field_id: entry
                    .get_key_str("field-id")
                    .and_then(Scalar::as_i64)
                    .and_then(|id| i32::try_from(id).ok()),
                default: entry.get_key_str("default").cloned(),
            });
        }

        let node = Node::Record(Arc::new(RecordType {
            name: fullname.clone(),
            aliases,
            fields,
        }));
        self.names.insert(fullname, node.clone());
        Ok(node)
    }

    /// Refuse a second definition of an already-registered name.
    ///
    /// A repeated *reference* is the point of named types; a repeated
    /// *definition* is ambiguous, because the two bodies could disagree and
    /// silently shadow each other in the name table.
    fn check_unregistered(&self, fullname: &SmolStr) -> Result<()> {
        if self.names.contains_key(fullname) {
            return Err(invalid(format_smolstr!(
                "expected one definition of the Avro type {fullname:?}, got a second"
            )));
        }
        Ok(())
    }

    /// Parse an enum and register it.
    fn parse_enum(&mut self, document: &Scalar, namespace: &str) -> Result<Node> {
        let (fullname, child_namespace) = declared_name(document, namespace)?;
        self.check_unregistered(&fullname)?;
        let symbols = document
            .get_key_str("symbols")
            .and_then(Scalar::as_sequence)
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected an Avro enum \"symbols\" array",
                ))
            })?;
        let mut names = Vec::with_capacity(symbols.len());
        for symbol in symbols {
            names.push(SmolStr::new(symbol.as_str().ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an Avro enum symbol string, got {}",
                    symbol.kind()
                ))
            })?));
        }
        let node = Node::Enum(Arc::new(EnumType {
            name: fullname.clone(),
            aliases: declared_aliases(document, &child_namespace),
            symbols: names.into(),
            default: document
                .get_key_str("default")
                .and_then(Scalar::as_str)
                .map(SmolStr::new),
        }));
        self.names.insert(fullname, node.clone());
        Ok(node)
    }

    /// Parse a fixed, apply any logical annotation, and register the result.
    fn parse_fixed(
        &mut self,
        document: &Scalar,
        namespace: &str,
        logical: Option<&str>,
    ) -> Result<Node> {
        let (fullname, child_namespace) = declared_name(document, namespace)?;
        self.check_unregistered(&fullname)?;
        let size = document
            .get_key_str("size")
            .and_then(Scalar::as_i64)
            .and_then(|size| usize::try_from(size).ok())
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a non-negative Avro fixed \"size\"",
                ))
            })?;
        let fixed = Arc::new(FixedType {
            name: fullname.clone(),
            aliases: declared_aliases(document, &child_namespace),
            size,
        });
        let node = match logical {
            Some("decimal") => {
                decimal_over(Some(fixed.clone()), document).unwrap_or(Node::Fixed(fixed))
            }
            Some("duration") if size == 12 => Node::Duration(fixed),
            Some("uuid") if size == 16 => Node::UuidFixed(fixed),
            // An unknown annotation, or one whose attributes do not fit the
            // underlying type, degrades to the underlying type per the
            // specification - it never becomes an error.
            _ => Node::Fixed(fixed),
        };
        self.names.insert(fullname, node.clone());
        Ok(node)
    }

    /// Resolve a bare schema name: a primitive, or a type declared earlier.
    fn resolve(&self, name: &str, namespace: &str) -> Result<Node> {
        Ok(match name {
            "null" => Node::Null,
            "boolean" => Node::Boolean,
            "int" => Node::Int,
            "long" => Node::Long,
            "float" => Node::Float,
            "double" => Node::Double,
            "bytes" => Node::Bytes,
            "string" => Node::String,
            other => {
                let qualified = if other.contains('.') || namespace.is_empty() {
                    SmolStr::new(other)
                } else {
                    format_smolstr!("{namespace}.{other}")
                };
                if self.names.contains_key(&qualified) {
                    Node::Ref(qualified)
                } else if self.names.contains_key(other) {
                    Node::Ref(SmolStr::new(other))
                } else {
                    return Err(invalid(format_smolstr!(
                        "expected an Avro primitive name or a type declared earlier, got {other:?}"
                    )));
                }
            }
        })
    }
}

/// Apply a logical annotation to a primitive node.
///
/// An unknown annotation, or one over an underlying type it does not fit,
/// degrades to the underlying type per the specification.
fn annotate(node: Node, logical: Option<&str>, document: &Scalar) -> Node {
    let Some(logical) = logical else {
        return node;
    };
    match (logical, &node) {
        ("date", Node::Int) => Node::Date,
        ("time-millis", Node::Int) => Node::TimeMillis,
        ("time-micros", Node::Long) => Node::TimeMicros,
        ("timestamp-millis", Node::Long) => Node::TimestampMillis,
        ("timestamp-micros", Node::Long) => Node::TimestampMicros,
        ("timestamp-nanos", Node::Long) => Node::TimestampNanos,
        ("local-timestamp-millis", Node::Long) => Node::LocalTimestampMillis,
        ("local-timestamp-micros", Node::Long) => Node::LocalTimestampMicros,
        ("local-timestamp-nanos", Node::Long) => Node::LocalTimestampNanos,
        ("uuid", Node::String) => Node::Uuid,
        ("decimal", Node::Bytes) => decimal_over(None, document).unwrap_or(node),
        _ => node,
    }
}

/// Build a decimal node when its attributes are valid for the underlying type.
fn decimal_over(fixed: Option<Arc<FixedType>>, document: &Scalar) -> Option<Node> {
    let precision = document
        .get_key_str("precision")
        .and_then(Scalar::as_i64)
        .and_then(|precision| u32::try_from(precision).ok())?;
    let scale = document
        .get_key_str("scale")
        .and_then(Scalar::as_i64)
        .and_then(|scale| u32::try_from(scale).ok())
        .unwrap_or(0);
    if precision == 0 || scale > precision {
        return None;
    }
    // DESIGN: the value model holds a decimal as an i128 unscaled integer, so
    // 38 digits is the widest decimal that decodes losslessly; a wider
    // declaration keeps its raw underlying bytes instead of failing.
    if precision > 38 {
        return None;
    }
    if let Some(fixed) = &fixed {
        if fixed.size == 0 || precision > max_precision_for(fixed.size) {
            return None;
        }
    }
    Some(Node::Decimal(Arc::new(DecimalType {
        precision,
        scale,
        fixed,
    })))
}

/// Return the widest decimal precision a fixed of `size` bytes can carry.
fn max_precision_for(size: usize) -> u32 {
    if size >= 16 {
        // Sixteen bytes already exceed 38 digits, the value model's cap.
        return 38;
    }
    let bits = 8 * size as u32 - 1;
    let max = (1_u128 << bits) - 1;
    max.ilog10() + 1
}

/// Read a named type's fullname and the namespace its children inherit.
fn declared_name(document: &Scalar, namespace: &str) -> Result<(SmolStr, String)> {
    let name = document
        .get_key_str("name")
        .and_then(Scalar::as_str)
        .ok_or_else(|| invalid(SmolStr::new_static("expected an Avro type \"name\"")))?;
    // A dotted name is already a fullname and any namespace attribute is
    // ignored, which is the specification's rule.
    if let Some((space, _)) = name.rsplit_once('.') {
        return Ok((SmolStr::new(name), space.to_owned()));
    }
    let space = document
        .get_key_str("namespace")
        .and_then(Scalar::as_str)
        .unwrap_or(namespace);
    let fullname = if space.is_empty() {
        SmolStr::new(name)
    } else {
        format_smolstr!("{space}.{name}")
    };
    Ok((fullname, space.to_owned()))
}

/// Read a named type's aliases, resolved against its own namespace.
fn declared_aliases(document: &Scalar, namespace: &str) -> Vec<SmolStr> {
    document
        .get_key_str("aliases")
        .and_then(Scalar::as_sequence)
        .map(|aliases| {
            aliases
                .iter()
                .filter_map(Scalar::as_str)
                .map(|alias| {
                    if alias.contains('.') || namespace.is_empty() {
                        SmolStr::new(alias)
                    } else {
                        format_smolstr!("{namespace}.{alias}")
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Render one node's Parsing Canonical Form.
fn canonical(
    node: &Node,
    names: &HashMap<SmolStr, Node>,
    printed: &mut Vec<SmolStr>,
    output: &mut String,
) {
    match node {
        Node::Null => output.push_str("\"null\""),
        Node::Boolean => output.push_str("\"boolean\""),
        Node::Int | Node::Date | Node::TimeMillis => output.push_str("\"int\""),
        Node::Long
        | Node::TimeMicros
        | Node::TimestampMillis
        | Node::TimestampMicros
        | Node::TimestampNanos
        | Node::LocalTimestampMillis
        | Node::LocalTimestampMicros
        | Node::LocalTimestampNanos => output.push_str("\"long\""),
        Node::Float => output.push_str("\"float\""),
        Node::Double => output.push_str("\"double\""),
        Node::Bytes => output.push_str("\"bytes\""),
        Node::String | Node::Uuid => output.push_str("\"string\""),
        Node::Decimal(decimal) => match &decimal.fixed {
            Some(fixed) => canonical_fixed(fixed, printed, output),
            None => output.push_str("\"bytes\""),
        },
        Node::Duration(fixed) | Node::UuidFixed(fixed) | Node::Fixed(fixed) => {
            canonical_fixed(fixed, printed, output);
        }
        Node::Record(record) => {
            if printed.contains(&record.name) {
                canonical_string(&record.name, output);
                return;
            }
            printed.push(record.name.clone());
            output.push_str("{\"name\":");
            canonical_string(&record.name, output);
            output.push_str(",\"type\":\"record\",\"fields\":[");
            for (index, field) in record.fields.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str("{\"name\":");
                canonical_string(&field.name, output);
                output.push_str(",\"type\":");
                canonical(&field.schema, names, printed, output);
                output.push('}');
            }
            output.push_str("]}");
        }
        Node::Enum(symbols) => {
            if printed.contains(&symbols.name) {
                canonical_string(&symbols.name, output);
                return;
            }
            printed.push(symbols.name.clone());
            output.push_str("{\"name\":");
            canonical_string(&symbols.name, output);
            output.push_str(",\"type\":\"enum\",\"symbols\":[");
            for (index, symbol) in symbols.symbols.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_string(symbol, output);
            }
            output.push_str("]}");
        }
        Node::Array(items) => {
            output.push_str("{\"type\":\"array\",\"items\":");
            canonical(items, names, printed, output);
            output.push('}');
        }
        Node::Map(values) => {
            output.push_str("{\"type\":\"map\",\"values\":");
            canonical(values, names, printed, output);
            output.push('}');
        }
        Node::Union(branches) => {
            output.push('[');
            for (index, branch) in branches.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical(branch, names, printed, output);
            }
            output.push(']');
        }
        Node::Ref(name) => match names.get(name) {
            Some(target) if !printed.contains(name) => {
                canonical(target, names, printed, output);
            }
            _ => canonical_string(name, output),
        },
    }
}

/// Render a fixed type's canonical form, or its name once printed.
fn canonical_fixed(fixed: &FixedType, printed: &mut Vec<SmolStr>, output: &mut String) {
    if printed.contains(&fixed.name) {
        canonical_string(&fixed.name, output);
        return;
    }
    printed.push(fixed.name.clone());
    output.push_str("{\"name\":");
    canonical_string(&fixed.name, output);
    output.push_str(",\"type\":\"fixed\",\"size\":");
    output.push_str(&fixed.size.to_string());
    output.push('}');
}

/// Render one JSON string with the minimal escapes canonical form uses.
fn canonical_string(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            control if (control as u32) < 0x20 => {
                output.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

/// The CRC-64-AVRO polynomial, reversed, which is also the empty fingerprint.
const RABIN_EMPTY: u64 = 0xC15D_213A_A4D7_A795;

/// Return the 64-bit Rabin fingerprint of `bytes`, as the specification
/// defines it for schema fingerprinting.
pub(crate) fn rabin(bytes: &[u8]) -> u64 {
    static TABLE: OnceLock<[u64; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0_u64; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut fingerprint = index as u64;
            for _ in 0..8 {
                let mask = if fingerprint & 1 == 1 { RABIN_EMPTY } else { 0 };
                fingerprint = (fingerprint >> 1) ^ mask;
            }
            *entry = fingerprint;
        }
        table
    });
    let mut fingerprint = RABIN_EMPTY;
    for byte in bytes {
        let index = ((fingerprint ^ u64::from(*byte)) & 0xFF) as usize;
        fingerprint = (fingerprint >> 8) ^ table[index];
    }
    fingerprint
}

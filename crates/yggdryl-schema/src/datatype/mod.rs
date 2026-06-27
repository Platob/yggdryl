//! The central [`DataType`] enum, its [`TypeCategory`] classifier, the
//! [`SchemaError`] type, the uniform physical accessors
//! ([`bit_size`](DataType::bit_size) / [`is_large`](DataType::is_large) /
//! [`is_view`](DataType::is_view)) and the canonical string grammar. The
//! category-specific surface lives in the sibling modules ([`primitive`],
//! [`logical`], [`nested`], [`coerce`]).

use std::collections::BTreeMap;
use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::{Charset, Field};
use yggdryl_core::{TimeUnit, Timezone};

mod coerce;
mod logical;
mod nested;
mod numeric;
mod primitive;

pub use coerce::MergeStrategy;
pub use logical::IntervalUnit;
pub use nested::UnionMode;
pub use numeric::Numeric;

/// Error returned when a schema type cannot be parsed, converted or merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// The input was empty.
    Empty,
    /// The input was not a well-formed type / field string.
    Invalid(String),
    /// A well-formed string named an unknown type.
    Unknown(String),
    /// A unit token (time / interval / union mode / merge strategy / charset) was
    /// not recognised.
    UnknownUnit(String),
    /// A [`TypeCategory`] name was not known.
    UnknownCategory(String),
    /// Two types could not be merged into a common type under the chosen strategy.
    Incompatible {
        /// The left operand's canonical string.
        left: String,
        /// The right operand's canonical string.
        right: String,
    },
    /// Two fields with different names were merged.
    NameMismatch {
        /// The left field's name.
        left: String,
        /// The right field's name.
        right: String,
    },
    /// A field expected to be a struct was not.
    NotAStruct(String),
    /// The operation has no equivalent (e.g. converting [`Any`](DataType::Any) to
    /// Arrow). The message names what to do instead.
    Unsupported(String),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Empty => write!(f, "data type is empty"),
            SchemaError::Invalid(value) => write!(f, "data type '{value}' is malformed"),
            SchemaError::Unknown(value) => write!(f, "unknown data type '{value}'"),
            SchemaError::UnknownUnit(value) => write!(f, "unknown unit '{value}'"),
            SchemaError::UnknownCategory(value) => write!(
                f,
                "unknown category '{value}', expected 'any', 'primitive', 'logical' or 'nested'"
            ),
            SchemaError::Incompatible { left, right } => {
                write!(f, "data types '{left}' and '{right}' have no common type")
            }
            SchemaError::NameMismatch { left, right } => {
                write!(
                    f,
                    "cannot merge fields with different names '{left}' and '{right}'"
                )
            }
            SchemaError::NotAStruct(value) => write!(f, "field '{value}' is not a struct"),
            SchemaError::Unsupported(value) => write!(f, "{value}"),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<yggdryl_core::CharsetError> for SchemaError {
    fn from(err: yggdryl_core::CharsetError) -> SchemaError {
        SchemaError::UnknownUnit(err.0)
    }
}

/// The broad category a [`DataType`] belongs to — the three-way split of the type
/// system, plus the [`Any`](TypeCategory::Any) wildcard.
///
/// ```
/// use yggdryl_schema::{DataType, TypeCategory};
/// assert_eq!(DataType::int(32, true).category(), TypeCategory::Primitive);
/// assert_eq!(DataType::date().category(), TypeCategory::Logical);
/// assert_eq!(DataType::struct_(vec![]).category(), TypeCategory::Nested);
/// assert_eq!(DataType::Any.category(), TypeCategory::Any);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TypeCategory {
    /// The wildcard [`Any`](DataType::Any) — matches and merges with every type.
    Any,
    /// A fixed/variable-width scalar: null, boolean, integers, floats, binary, strings.
    Primitive,
    /// A richer logical meaning than its storage: temporal, decimal, dictionary,
    /// JSON / BSON.
    Logical,
    /// A container of other fields: lists, structs, maps, unions, run-end encoding.
    Nested,
}

impl TypeCategory {
    /// Parses a category name (case-insensitive).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<TypeCategory, SchemaError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "any" => Ok(TypeCategory::Any),
            "primitive" | "scalar" => Ok(TypeCategory::Primitive),
            "logical" => Ok(TypeCategory::Logical),
            "nested" | "complex" => Ok(TypeCategory::Nested),
            _ => Err(SchemaError::UnknownCategory(value.to_string())),
        }
    }

    /// The lowercase name (`"any"` / `"primitive"` / `"logical"` / `"nested"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            TypeCategory::Any => "any",
            TypeCategory::Primitive => "primitive",
            TypeCategory::Logical => "logical",
            TypeCategory::Nested => "nested",
        }
    }
}

impl fmt::Display for TypeCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A logical data type. Variants are grouped into three [categories](TypeCategory)
/// — primitive, logical, nested — plus the [`Any`](DataType::Any) wildcard. The
/// design is simpler than Arrow's: width/offset/layout variations are carried as
/// the uniform fields `bits` / `large` / `view` (and read back via
/// [`bit_size`](DataType::bit_size) / [`is_large`](DataType::is_large) /
/// [`is_view`](DataType::is_view)), and all strings are one
/// [`Varchar`](DataType::Varchar) with a [`Charset`].
///
/// ```
/// use yggdryl_schema::{DataType, Charset};
/// assert_eq!(DataType::from_str("int64").unwrap(), DataType::int(64, true));
/// assert_eq!(DataType::from_str("uint8").unwrap(), DataType::int(8, false));
/// assert_eq!(DataType::varchar().is_large(), false);
/// assert_eq!(DataType::int(32, true).bit_size(), Some(32));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataType {
    // ---- wildcard ----
    /// Matches and merges with any other type — the top of the lattice. Has no
    /// Arrow equivalent, so it must be resolved before converting.
    Any,

    // ---- primitive ----
    /// The null type (all values null), the bottom concrete type.
    Null,
    /// `true` / `false`.
    Boolean,
    /// An integer of `bits` width (commonly 8/16/32/64, but any width is allowed),
    /// signed or unsigned.
    Int {
        /// Bit width (commonly 8/16/32/64; any positive width is allowed).
        bits: u16,
        /// Whether the integer is signed.
        signed: bool,
    },
    /// A floating-point number of `bits` width (commonly 16/32/64, but any width is
    /// allowed for custom encodings).
    Float {
        /// Bit width (commonly 16/32/64; any positive width is allowed).
        bits: u16,
    },
    /// A UTF-8 (or other [`Charset`]) string. `large` uses 64-bit offsets; `view`
    /// the view layout; `size` (when set) makes it fixed-length (`char(n)`).
    Varchar {
        /// The character set (default UTF-8).
        charset: Charset,
        /// 64-bit offsets.
        large: bool,
        /// View layout.
        view: bool,
        /// Fixed character length, if any (`None` = variable-length).
        size: Option<i32>,
    },
    /// Opaque bytes. `large` uses 64-bit offsets, `view` the view layout, and
    /// `size` (when set) makes it fixed-width.
    Binary {
        /// 64-bit offsets.
        large: bool,
        /// View layout.
        view: bool,
        /// Fixed byte width, if any.
        size: Option<i32>,
    },

    // ---- logical ----
    /// A decimal with `(precision, scale)` stored in `bits` (32/64/128/256).
    Decimal {
        /// Total number of digits.
        precision: u8,
        /// Digits after the decimal point (may be negative).
        scale: i8,
        /// Storage width: 32, 64, 128 or 256.
        bits: u16,
    },
    /// A calendar date. `large` selects millisecond (64-bit) storage over the
    /// default day (32-bit) storage.
    Date {
        /// Millisecond (64-bit) storage instead of day (32-bit).
        large: bool,
    },
    /// A time of day in the given [`TimeUnit`].
    Time {
        /// Resolution (`s`/`ms` are 32-bit, `us`/`ns` are 64-bit).
        unit: TimeUnit,
    },
    /// A timestamp in the given [`TimeUnit`] with an optional [`Timezone`].
    Timestamp {
        /// Resolution.
        unit: TimeUnit,
        /// Display timezone, if zoned.
        timezone: Option<Timezone>,
    },
    /// Elapsed time in the given [`TimeUnit`].
    Duration {
        /// Resolution.
        unit: TimeUnit,
    },
    /// A calendar interval in the given [`IntervalUnit`].
    Interval {
        /// Interval resolution.
        unit: IntervalUnit,
    },
    /// Dictionary (run-time `key` index into a `value` dictionary) encoding.
    Dictionary {
        /// The integer index type.
        key: Box<DataType>,
        /// The dictionary value type.
        value: Box<DataType>,
    },
    /// JSON text — a string-backed logical type (its physical type is a
    /// [`Varchar`](DataType::Varchar)).
    Json,
    /// A BSON document — a binary-backed logical type (its physical type is a
    /// [`Binary`](DataType::Binary)).
    Bson,

    // ---- nested ----
    /// A list of the `item` field. `large` uses 64-bit offsets, `view` the view
    /// layout, and `size` (when set) makes it fixed-length.
    List {
        /// The element field.
        item: Box<Field>,
        /// 64-bit offsets.
        large: bool,
        /// View layout.
        view: bool,
        /// Fixed length, if any.
        size: Option<i32>,
    },
    /// A composite of named, typed sub-fields. A `Field` of this type is a schema.
    Struct(Vec<Field>),
    /// A map from `key` to `value`; `sorted` records whether the keys are sorted.
    Map {
        /// The key type (non-null by convention).
        key: Box<DataType>,
        /// The value type.
        value: Box<DataType>,
        /// Whether keys are sorted.
        sorted: bool,
    },
    /// A union of typed alternatives.
    Union {
        /// The alternative fields (type ids are assigned `0, 1, …`).
        fields: Vec<Field>,
        /// Sparse or dense layout.
        mode: UnionMode,
    },
    /// Run-end encoding: a `run_ends` integer type and a `values` type.
    RunEndEncoded {
        /// The run-ends integer type.
        run_ends: Box<DataType>,
        /// The values type.
        values: Box<DataType>,
    },
}

impl DataType {
    /// The [`TypeCategory`] this type belongs to.
    pub fn category(&self) -> TypeCategory {
        if self.is_any() {
            TypeCategory::Any
        } else if self.is_nested() {
            TypeCategory::Nested
        } else if self.is_logical() {
            TypeCategory::Logical
        } else {
            TypeCategory::Primitive
        }
    }

    /// Whether this is the [`Any`](DataType::Any) wildcard.
    pub fn is_any(&self) -> bool {
        matches!(self, DataType::Any)
    }

    /// The physical width of a value in **bits** for fixed-width types, or `None`
    /// for variable-width / nested types. `Boolean` is one bit; a fixed-size
    /// `Binary` is `size * 8`.
    ///
    /// ```
    /// use yggdryl_schema::DataType;
    /// assert_eq!(DataType::int(32, true).bit_size(), Some(32));
    /// assert_eq!(DataType::Boolean.bit_size(), Some(1));
    /// assert_eq!(DataType::varchar().bit_size(), None);
    /// ```
    pub fn bit_size(&self) -> Option<u16> {
        use DataType::*;
        let bits = match self {
            Boolean => 1,
            Int { bits, .. } | Float { bits } | Decimal { bits, .. } => *bits,
            Date { large } => {
                if *large {
                    64
                } else {
                    32
                }
            }
            Time { unit } => {
                if matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) {
                    32
                } else {
                    64
                }
            }
            Timestamp { .. } | Duration { .. } => 64,
            Interval { unit } => return Some(unit.bit_size()),
            Binary { size: Some(n), .. } if *n >= 0 => return Some((*n as u16).saturating_mul(8)),
            _ => return None,
        };
        Some(bits)
    }

    /// The physical width in **bytes** for byte-aligned fixed-width types (so
    /// `Boolean`, which is sub-byte, and all variable-width / nested types are `None`).
    pub fn byte_size(&self) -> Option<u16> {
        self.bit_size().filter(|b| b % 8 == 0).map(|b| b / 8)
    }

    /// Whether this type uses the **large** (64-bit offset / wide) form — a large
    /// string/binary/list, or a millisecond date.
    pub fn is_large(&self) -> bool {
        use DataType::*;
        matches!(
            self,
            Varchar { large: true, .. }
                | Binary { large: true, .. }
                | List { large: true, .. }
                | Date { large: true }
        )
    }

    /// Whether this type uses the **view** layout (a string/binary/list view).
    pub fn is_view(&self) -> bool {
        use DataType::*;
        matches!(
            self,
            Varchar { view: true, .. } | Binary { view: true, .. } | List { view: true, .. }
        )
    }

    /// Whether this type has a **fixed** (non-variable) length: a fixed-width scalar
    /// (int / float / decimal / temporal), or a fixed-size binary / string / list.
    /// A variable-length string / binary / list and the unbounded nested types are
    /// not fixed-size.
    ///
    /// ```
    /// use yggdryl_schema::DataType;
    /// assert!(DataType::int(32, true).is_fixed_size());
    /// assert!(DataType::fixed_size_binary(16).is_fixed_size());
    /// assert!(DataType::fixed_size_varchar(10).is_fixed_size());
    /// assert!(!DataType::varchar().is_fixed_size());
    /// assert!(!DataType::binary().is_fixed_size());
    /// ```
    pub fn is_fixed_size(&self) -> bool {
        use DataType::*;
        self.bit_size().is_some()
            || matches!(
                self,
                Varchar { size: Some(_), .. } | List { size: Some(_), .. }
            )
    }

    /// The physical (storage) [`DataType`] backing this type. A [logical](TypeCategory::Logical)
    /// type reports its underlying primitive — a [`Date`](DataType::Date) is an
    /// `int32`/`int64`, a [`Time`](DataType::Time) / [`Timestamp`](DataType::Timestamp)
    /// / [`Duration`](DataType::Duration) / [`Interval`](DataType::Interval) an integer
    /// of its width, a [`Decimal`](DataType::Decimal) an integer of its storage width, a
    /// [`Dictionary`](DataType::Dictionary) its key index type, a [`Json`](DataType::Json)
    /// a [`Varchar`](DataType::Varchar) and a [`Bson`](DataType::Bson) a
    /// [`Binary`](DataType::Binary). Every other type is its own physical type.
    ///
    /// ```
    /// use yggdryl_schema::DataType;
    /// assert_eq!(DataType::date().physical_type(), DataType::int(32, true));
    /// assert_eq!(DataType::json().physical_type(), DataType::varchar());
    /// assert_eq!(DataType::int(32, true).physical_type(), DataType::int(32, true));
    /// ```
    pub fn physical_type(&self) -> DataType {
        use DataType::*;
        match self {
            Date { large } => DataType::int(if *large { 64 } else { 32 }, true),
            Time { unit } => DataType::int(
                if matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) {
                    32
                } else {
                    64
                },
                true,
            ),
            Timestamp { .. } | Duration { .. } => DataType::int(64, true),
            Interval { unit } => DataType::int(unit.bit_size(), true),
            Decimal { bits, .. } => DataType::int(*bits, true),
            Dictionary { key, .. } => key.physical_type(),
            Json => DataType::varchar(),
            Bson => DataType::binary(),
            other => other.clone(),
        }
    }
}

// ---- the canonical string grammar ----

/// Whether `c` opens a parameter group (`[`, `(` or `<`).
fn is_open_bracket(c: char) -> bool {
    matches!(c, '[' | '(' | '<')
}

/// Whether `c` closes a parameter group (`]`, `)` or `>`).
fn is_close_bracket(c: char) -> bool {
    matches!(c, ']' | ')' | '>')
}

/// Splits `input` on top-level (bracket depth 0) occurrences of `sep`, tracking all
/// of `[]` / `()` / `<>` so nested type arguments are not split.
fn split_top_level(input: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in input.char_indices() {
        if is_open_bracket(ch) {
            depth += 1;
        } else if is_close_bracket(ch) {
            depth = depth.saturating_sub(1);
        } else if ch == sep && depth == 0 {
            parts.push(input[start..i].trim());
            start = i + 1;
        }
    }
    parts.push(input[start..].trim());
    parts
}

/// Whether every bracket in `input` is balanced and never closes before it opens
/// — bracket characters inside a `"`/`'`/`` ` `` quoted name are ignored. Catches
/// stray closers (`struct[a]: int]`) that the depth-saturating scanners would
/// otherwise absorb into a name.
fn brackets_balanced(input: &str) -> bool {
    let mut depth: i32 = 0;
    let mut quote: Option<char> = None;
    for ch in input.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None if matches!(ch, '"' | '\'' | '`') => quote = Some(ch),
            None if is_open_bracket(ch) => depth += 1,
            None if is_close_bracket(ch) => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            None => {}
        }
    }
    depth == 0 && quote.is_none()
}

/// The byte index of the first top-level occurrence of `target` (across all bracket
/// kinds).
fn top_level_index(input: &str, target: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, ch) in input.char_indices() {
        if is_open_bracket(ch) {
            depth += 1;
        } else if is_close_bracket(ch) {
            depth = depth.saturating_sub(1);
        } else if ch == target && depth == 0 {
            return Some(i);
        }
    }
    None
}

/// Splits a field token into `(name, type)`. The name may be quoted with `"`, `'`,
/// `` ` `` or `[ ]`, and is separated from the type by a `:` (`qty: int64`) or by
/// whitespace (Hive/SQL `qty int64`, `col struct<a: str>`). Unnamed tokens
/// (`int32`) fall back to `default_name`.
fn split_name_type<'a>(token: &'a str, default_name: &'a str) -> (String, &'a str) {
    // A quoted / bracketed name comes first and may contain spaces or colons.
    if let Some(open) = token.chars().next() {
        let close = match open {
            '"' => Some('"'),
            '\'' => Some('\''),
            '`' => Some('`'),
            '[' => Some(']'),
            _ => None,
        };
        if let Some(close) = close {
            if let Some(end) = token[1..].find(close) {
                let name = token[1..1 + end].to_string();
                let rest = token[1 + end + close.len_utf8()..].trim_start();
                let rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
                return (name, rest);
            }
        }
    }
    // Unquoted: a `:` separates name and type, else the first whitespace does.
    if let Some(i) = top_level_index(token, ':') {
        return (token[..i].trim().to_string(), token[i + 1..].trim());
    }
    if let Some(i) = token.find(char::is_whitespace) {
        return (token[..i].trim().to_string(), token[i + 1..].trim());
    }
    (default_name.to_string(), token)
}

/// Parses a field token (`name: type` / `name type` / unnamed `type`), with an
/// optional trailing ` not null` and an optionally quoted name (see [`split_name_type`]).
pub(crate) fn parse_child_field(token: &str, default_name: &str) -> Result<Field, SchemaError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(SchemaError::Empty);
    }
    // Strip a trailing nullability marker before splitting (so an unnamed
    // `int32 not null` is not mistaken for a `name type` pair). Test the suffix on the
    // raw bytes (case-insensitively) rather than lowercasing the whole token; the
    // matched tail is 9 ASCII bytes, so `len - 9` is always a char boundary.
    const NOT_NULL: &str = " not null";
    let (token, nullable) = if token.len() >= NOT_NULL.len()
        && token.as_bytes()[token.len() - NOT_NULL.len()..]
            .eq_ignore_ascii_case(NOT_NULL.as_bytes())
    {
        (token[..token.len() - NOT_NULL.len()].trim(), false)
    } else {
        (token, true)
    };
    let (name, type_str) = split_name_type(token, default_name);
    let name = if name.is_empty() {
        default_name.to_string()
    } else {
        name
    };
    Ok(Field::new(name, DataType::from_str(type_str)?, nullable))
}

/// Parses the comma-separated body of a `struct[…]` into fields.
pub(crate) fn parse_struct_body(args: &str) -> Result<Vec<Field>, SchemaError> {
    let args = args.trim();
    if args.is_empty() {
        return Ok(Vec::new());
    }
    split_top_level(args, ',')
        .into_iter()
        .enumerate()
        .map(|(i, token)| parse_child_field(token, &format!("f{i}")))
        .collect()
}

/// Parses a standalone field string (`name: type` or `name type`), requiring an
/// explicit name.
pub(crate) fn parse_field_str(input: &str) -> Result<Field, SchemaError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SchemaError::Empty);
    }
    const SENTINEL: &str = "\u{0}unnamed";
    let field = parse_child_field(input, SENTINEL)?;
    if field.name() == SENTINEL {
        return Err(SchemaError::Invalid(input.to_string()));
    }
    Ok(field)
}

/// Parses an integer parameter (precision / size / …).
fn parse_int<T: std::str::FromStr>(value: &str, whole: &str) -> Result<T, SchemaError> {
    value
        .trim()
        .parse::<T>()
        .map_err(|_| SchemaError::Invalid(whole.to_string()))
}

/// Parses a `precision[, scale]` decimal argument.
fn parse_decimal_args(args: &str, whole: &str) -> Result<(u8, i8), SchemaError> {
    let parts = split_top_level(args, ',');
    let precision = parse_int::<u8>(parts.first().copied().unwrap_or(""), whole)?;
    let scale = match parts.get(1) {
        Some(s) => parse_int::<i8>(s, whole)?,
        None => 0,
    };
    Ok((precision, scale))
}

/// Parses a [`TimeUnit`], mapping a bad token to [`SchemaError::UnknownUnit`].
fn parse_time_unit(value: &str) -> Result<TimeUnit, SchemaError> {
    TimeUnit::from_str(value).map_err(|_| SchemaError::UnknownUnit(value.to_string()))
}

/// Parses a generic `int<N>` / `uint<N>` head of any positive bit width (e.g.
/// `int24`, `uint128`), returning `None` for anything else. The common widths are
/// handled by explicit aliases ahead of this; this is the flexible fallback.
fn parse_generic_int(head: &str) -> Option<DataType> {
    let (digits, signed) = match head.strip_prefix("uint") {
        Some(rest) => (rest, false),
        None => (head.strip_prefix("int")?, true),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Any width (including the degenerate `int0`) round-trips through `to_str`; the
    // width is not range-checked here — it mirrors the permissive `int` constructor.
    let bits = digits.parse::<u16>().ok()?;
    Some(DataType::int(bits, signed))
}

/// Parses a generic `float<N>` head of any positive bit width (e.g. `float24`,
/// `float128`), returning `None` for anything else. The common widths are handled by
/// explicit aliases ahead of this; this is the flexible fallback.
fn parse_generic_float(head: &str) -> Option<DataType> {
    let digits = head.strip_prefix("float")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(DataType::float(digits.parse::<u16>().ok()?))
}

/// Parses a time resolution given either a unit token (`us`) or a SQL fractional
/// precision (`0` → s, `1..3` → ms, `4..6` → us, `7..9` → ns).
fn parse_unit_or_precision(value: &str) -> Result<TimeUnit, SchemaError> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        let precision: u32 = parse_int(value, value)?;
        return Ok(match precision {
            0 => TimeUnit::Second,
            1..=3 => TimeUnit::Millisecond,
            4..=6 => TimeUnit::Microsecond,
            _ => TimeUnit::Nanosecond,
        });
    }
    parse_time_unit(value)
}

impl DataType {
    /// Parses a type from its canonical lowercase string. Examples: `int64`,
    /// `uint8`, `decimal128[10, 2]`, `timestamp[us, UTC]`, `varchar[latin1]`,
    /// `list[item: utf8]`, `struct[id: int64 not null, name: utf8]`,
    /// `map[utf8, int64]`. Common aliases are accepted (`bool`, `string`/`str`,
    /// `float`/`double`, `date`). The inverse of [`to_str`](DataType::to_str).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<DataType, SchemaError> {
        log_event!(trace, "DataType::from_str {input:?}");
        let input = input.trim();
        if input.is_empty() {
            return Err(SchemaError::Empty);
        }
        if !brackets_balanced(input) {
            return Err(SchemaError::Invalid(input.to_string()));
        }
        // The argument group may be bracketed with `[ ]` (canonical), `( )` (SQL) or
        // `< >` (Hive), e.g. `varchar(255)`, `struct<a: int>`, `decimal[10, 2]`.
        let (head, args) = match input.find(['[', '(', '<']) {
            Some(i) => {
                let close = match input.as_bytes()[i] {
                    b'[' => ']',
                    b'(' => ')',
                    _ => '>',
                };
                if !input.ends_with(close) {
                    return Err(SchemaError::Invalid(input.to_string()));
                }
                (
                    input[..i].trim(),
                    Some(input[i + 1..input.len() - 1].trim()),
                )
            }
            None => (input, None),
        };
        // Normalise the head: lowercase + single-spaced, so multi-word SQL names
        // (`double precision`, `timestamp with time zone`) match. The common single-word
        // head (`int64` / `utf8` / `struct`) needs no re-spacing, so skip the Vec+join.
        let lower = if head.split_whitespace().nth(1).is_none() {
            head.to_ascii_lowercase()
        } else {
            head.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        };
        match (lower.as_str(), args) {
            ("any", None) => Ok(DataType::Any),
            ("null", None) => Ok(DataType::Null),
            ("bool" | "boolean" | "bit", None) => Ok(DataType::Boolean),
            ("int8" | "tinyint", None) => Ok(DataType::int(8, true)),
            ("int16" | "smallint", None) => Ok(DataType::int(16, true)),
            ("int32" | "int" | "integer", None) => Ok(DataType::int(32, true)),
            ("int64" | "bigint" | "long", None) => Ok(DataType::int(64, true)),
            ("uint8" | "utinyint", None) => Ok(DataType::int(8, false)),
            ("uint16" | "usmallint", None) => Ok(DataType::int(16, false)),
            ("uint32" | "uinteger", None) => Ok(DataType::int(32, false)),
            ("uint64" | "uint" | "ubigint", None) => Ok(DataType::int(64, false)),
            ("float16" | "halffloat", None) => Ok(DataType::float(16)),
            ("float32" | "float" | "real" | "float4", None) => Ok(DataType::float(32)),
            ("float64" | "double" | "double precision" | "float8", None) => Ok(DataType::float(64)),
            (
                "utf8" | "string" | "str" | "text" | "varchar" | "char" | "character" | "nvarchar"
                | "nchar" | "clob",
                None,
            ) => Ok(DataType::varchar()),
            ("large_utf8" | "large_string", None) => {
                Ok(DataType::varchar_with(Charset::Utf8, true, false, None))
            }
            ("utf8_view" | "string_view", None) => {
                Ok(DataType::varchar_with(Charset::Utf8, false, true, None))
            }
            // `char(n)` is a fixed-length string; `varchar(n)` keeps the length only as
            // an (ignored) max-length hint and stays variable-length.
            ("char" | "character" | "nchar", Some(a)) => parse_varchar(a, input, true),
            ("varchar" | "nvarchar" | "string" | "clob", Some(a)) => parse_varchar(a, input, false),
            ("json" | "jsonb", None) => Ok(DataType::Json),
            ("bson", None) => Ok(DataType::Bson),
            ("binary" | "bytea" | "blob" | "varbinary", None) => Ok(DataType::binary()),
            ("varbinary", Some(_)) => Ok(DataType::binary()),
            ("large_binary", None) => Ok(DataType::Binary {
                large: true,
                view: false,
                size: None,
            }),
            ("binary_view", None) => Ok(DataType::Binary {
                large: false,
                view: true,
                size: None,
            }),
            ("fixed_size_binary" | "binary", Some(a)) => {
                Ok(DataType::fixed_size_binary(parse_int(a, input)?))
            }
            ("uuid", None) => Ok(DataType::fixed_size_binary(16)),
            ("date" | "date32", None) => Ok(DataType::date()),
            ("date64", None) => Ok(DataType::Date { large: true }),
            (
                "time" | "time32" | "time64" | "time without time zone" | "time with time zone",
                None,
            ) => Ok(DataType::Time {
                unit: TimeUnit::Microsecond,
            }),
            ("time32" | "time64" | "time", Some(a)) => Ok(DataType::Time {
                unit: parse_unit_or_precision(a)?,
            }),
            ("duration", None) => Ok(DataType::Duration {
                unit: TimeUnit::Microsecond,
            }),
            ("duration", Some(a)) => Ok(DataType::Duration {
                unit: parse_time_unit(a)?,
            }),
            ("interval", None) => Ok(DataType::Interval {
                unit: IntervalUnit::MonthDayNano,
            }),
            ("interval", Some(a)) => Ok(DataType::Interval {
                unit: IntervalUnit::from_str(a)?,
            }),
            ("timestamp" | "datetime" | "timestamp without time zone", None) => {
                Ok(DataType::Timestamp {
                    unit: TimeUnit::Microsecond,
                    timezone: None,
                })
            }
            (
                "timestamptz" | "timestamp with time zone" | "timestamp with local time zone",
                None,
            ) => Ok(DataType::Timestamp {
                unit: TimeUnit::Microsecond,
                timezone: Some(Timezone::Utc),
            }),
            ("timestamp" | "datetime" | "timestamptz", Some(a)) => {
                // Split on only the FIRST top-level comma: a raw POSIX zone
                // (`EST5EDT,M3.2.0,M11.1.0`) itself contains commas, so splitting on
                // every comma would truncate it to its first segment.
                let (unit_str, tz_str) = match top_level_index(a, ',') {
                    Some(i) => (a[..i].trim(), Some(a[i + 1..].trim())),
                    None => (a.trim(), None),
                };
                let unit =
                    parse_unit_or_precision(if unit_str.is_empty() { "us" } else { unit_str })?;
                let timezone = match tz_str.filter(|s| !s.is_empty()) {
                    Some(tz) => Some(
                        Timezone::from_str(tz).map_err(|e| SchemaError::Invalid(e.to_string()))?,
                    ),
                    None if lower == "timestamptz" => Some(Timezone::Utc),
                    None => None,
                };
                Ok(DataType::Timestamp { unit, timezone })
            }
            ("decimal" | "decimal128" | "numeric" | "number" | "dec", Some(a)) => {
                let (p, s) = parse_decimal_args(a, input)?;
                Ok(DataType::decimal_with(p, s, 128))
            }
            ("decimal32", Some(a)) => {
                let (p, s) = parse_decimal_args(a, input)?;
                Ok(DataType::decimal_with(p, s, 32))
            }
            ("decimal64", Some(a)) => {
                let (p, s) = parse_decimal_args(a, input)?;
                Ok(DataType::decimal_with(p, s, 64))
            }
            ("decimal256", Some(a)) => {
                let (p, s) = parse_decimal_args(a, input)?;
                Ok(DataType::decimal_with(p, s, 256))
            }
            ("dictionary", Some(a)) => {
                let parts = split_top_level(a, ',');
                if parts.len() != 2 {
                    return Err(SchemaError::Invalid(input.to_string()));
                }
                Ok(DataType::dictionary(
                    DataType::from_str(parts[0])?,
                    DataType::from_str(parts[1])?,
                ))
            }
            ("list" | "array", Some(a)) => Ok(DataType::list(parse_child_field(a, "item")?)),
            ("list_view", Some(a)) => Ok(DataType::List {
                item: Box::new(parse_child_field(a, "item")?),
                large: false,
                view: true,
                size: None,
            }),
            ("large_list", Some(a)) => Ok(DataType::large_list(parse_child_field(a, "item")?)),
            ("large_list_view", Some(a)) => Ok(DataType::List {
                item: Box::new(parse_child_field(a, "item")?),
                large: true,
                view: true,
                size: None,
            }),
            ("fixed_size_list", Some(a)) => {
                let parts = split_top_level(a, ',');
                if parts.len() != 2 {
                    return Err(SchemaError::Invalid(input.to_string()));
                }
                let item = parse_child_field(parts[0], "item")?;
                Ok(DataType::fixed_size_list(item, parse_int(parts[1], input)?))
            }
            ("struct", Some(a)) => Ok(DataType::Struct(parse_struct_body(a)?)),
            ("map", Some(a)) => {
                let parts = split_top_level(a, ',');
                // Exactly `key, value` or `key, value, sorted`; reject extra args.
                let sorted = match parts.len() {
                    2 => false,
                    3 if parts[2].eq_ignore_ascii_case("sorted") => true,
                    _ => return Err(SchemaError::Invalid(input.to_string())),
                };
                Ok(DataType::map(
                    DataType::from_str(parts[0])?,
                    DataType::from_str(parts[1])?,
                    sorted,
                ))
            }
            ("union" | "sparse_union" | "dense_union", Some(a)) => {
                let mode = if lower == "dense_union" {
                    UnionMode::Dense
                } else {
                    UnionMode::Sparse
                };
                let fields = parse_struct_body(a)?;
                Ok(DataType::Union { fields, mode })
            }
            ("run_end_encoded", Some(a)) => {
                let parts = split_top_level(a, ',');
                if parts.len() != 2 {
                    return Err(SchemaError::Invalid(input.to_string()));
                }
                Ok(DataType::run_end_encoded(
                    DataType::from_str(parts[0])?,
                    DataType::from_str(parts[1])?,
                ))
            }
            // A bare `int<N>` / `uint<N>` / `float<N>` of any width (e.g. `int24`,
            // `uint128`, `float24`).
            (other, None) => parse_generic_int(other)
                .or_else(|| parse_generic_float(other))
                .ok_or_else(|| SchemaError::Unknown(input.to_string())),
            (_, _) => Err(SchemaError::Unknown(input.to_string())),
        }
    }

    /// Builds a [`DataType`] from a `BTreeMap`; reads the single `type` key.
    pub fn from_mapping(fields: &BTreeMap<String, String>) -> Result<DataType, SchemaError> {
        match fields.get("type") {
            Some(value) => DataType::from_str(value),
            None => Err(SchemaError::Empty),
        }
    }

    /// Renders the canonical lowercase string — the inverse of
    /// [`from_str`](DataType::from_str).
    pub fn to_str(&self) -> String {
        use DataType::*;
        match self {
            Any => "any".to_string(),
            Null => "null".to_string(),
            Boolean => "bool".to_string(),
            Int { bits, signed } => format!("{}int{bits}", if *signed { "" } else { "u" }),
            Float { bits } => format!("float{bits}"),
            Varchar {
                charset,
                large,
                view,
                size,
            } => render_varchar(*charset, *large, *view, *size),
            Binary { large, view, size } => match (large, view, size) {
                (_, _, Some(n)) => format!("fixed_size_binary[{n}]"),
                (true, _, None) => "large_binary".to_string(),
                (_, true, None) => "binary_view".to_string(),
                _ => "binary".to_string(),
            },
            Decimal {
                precision,
                scale,
                bits,
            } => format!("decimal{bits}[{precision}, {scale}]"),
            Date { large } => if *large { "date64" } else { "date32" }.to_string(),
            Time { unit } => {
                let width = if matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) {
                    32
                } else {
                    64
                };
                format!("time{width}[{}]", unit.as_str())
            }
            Duration { unit } => format!("duration[{}]", unit.as_str()),
            Interval { unit } => format!("interval[{}]", unit.as_str()),
            Timestamp { unit, timezone } => match timezone {
                Some(tz) => format!("timestamp[{}, {}]", unit.as_str(), tz.name()),
                None => format!("timestamp[{}]", unit.as_str()),
            },
            Dictionary { key, value } => {
                format!("dictionary[{}, {}]", key.to_str(), value.to_str())
            }
            Json => "json".to_string(),
            Bson => "bson".to_string(),
            List {
                item,
                large,
                view,
                size,
            } => {
                let head = match (large, view, size) {
                    (_, _, Some(_)) => "fixed_size_list",
                    (true, true, None) => "large_list_view",
                    (true, false, None) => "large_list",
                    (false, true, None) => "list_view",
                    _ => "list",
                };
                match size {
                    Some(n) => format!("{head}[{}, {n}]", item.to_str()),
                    None => format!("{head}[{}]", item.to_str()),
                }
            }
            Struct(fields) => format!("struct[{}]", render_fields(fields)),
            Map { key, value, sorted } => {
                if *sorted {
                    format!("map[{}, {}, sorted]", key.to_str(), value.to_str())
                } else {
                    format!("map[{}, {}]", key.to_str(), value.to_str())
                }
            }
            Union { fields, mode } => format!("{}_union[{}]", mode.as_str(), render_fields(fields)),
            RunEndEncoded { run_ends, values } => {
                format!(
                    "run_end_encoded[{}, {}]",
                    run_ends.to_str(),
                    values.to_str()
                )
            }
        }
    }

    /// Renders to a component `BTreeMap` (the single `type` key).
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("type".to_string(), self.to_str())])
    }

    /// The canonical string as UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_bytes()
    }

    /// Parses a type from the UTF-8 bytes of its canonical string.
    pub fn from_bytes(bytes: &[u8]) -> Result<DataType, SchemaError> {
        let value =
            std::str::from_utf8(bytes).map_err(|_| SchemaError::Invalid("<bytes>".into()))?;
        DataType::from_str(value)
    }

    /// Serialises to a lossless structural JSON string (preserves field metadata,
    /// unlike the canonical string). Requires the `json` feature.
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("DataType serialises")
    }

    /// Parses a [`DataType`] from the structural JSON of [`to_json`](DataType::to_json).
    #[cfg(feature = "json")]
    pub fn from_json(json: &str) -> Result<DataType, SchemaError> {
        serde_json::from_str(json).map_err(|e| SchemaError::Invalid(e.to_string()))
    }
}

/// Renders a field list as `"name: type[ not null], …"`.
fn render_fields(fields: &[Field]) -> String {
    fields
        .iter()
        .map(Field::to_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders a `Varchar`. A variable-length UTF-8 string uses the friendly `utf8` /
/// `large_utf8` / `utf8_view` spellings (else `varchar[<charset>[, large][, view]]`);
/// a fixed-length string uses the `char` head (`char[n]` for plain UTF-8, else
/// `char[<charset>[, large][, view], <size>]`) so it parses back as fixed.
fn render_varchar(charset: Charset, large: bool, view: bool, size: Option<i32>) -> String {
    let Some(n) = size else {
        // Variable-length.
        if charset == Charset::Utf8 {
            match (large, view) {
                (false, false) => return "utf8".to_string(),
                (true, false) => return "large_utf8".to_string(),
                (false, true) => return "utf8_view".to_string(),
                _ => {}
            }
        }
        let mut out = format!("varchar[{}", charset.as_str());
        if large {
            out.push_str(", large");
        }
        if view {
            out.push_str(", view");
        }
        out.push(']');
        return out;
    };
    // Fixed-length: the `char` head round-trips through `parse_varchar(.., fixed=true)`.
    if charset == Charset::Utf8 && !large && !view {
        return format!("char[{n}]");
    }
    let mut out = format!("char[{}", charset.as_str());
    if large {
        out.push_str(", large");
    }
    if view {
        out.push_str(", view");
    }
    out.push_str(&format!(", {n}"));
    out.push(']');
    out
}

/// Parses a `varchar[<charset>[, large][, view][, <size>]]` argument list. A numeric
/// token sets the fixed `size` when `fixed` is set (the `char(n)` spelling), otherwise
/// it is a SQL max-length hint and ignored (the `varchar(n)` spelling).
fn parse_varchar(args: &str, whole: &str, fixed: bool) -> Result<DataType, SchemaError> {
    let mut charset = Charset::Utf8;
    let mut large = false;
    let mut view = false;
    let mut size = None;
    for (i, part) in split_top_level(args, ',').into_iter().enumerate() {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            "large" => large = true,
            "view" => view = true,
            // A numeric token is a fixed length for `char(n)`, else an ignored
            // `varchar(n)` max-length hint.
            _ if part.bytes().all(|b| b.is_ascii_digit()) => {
                if fixed {
                    size = Some(parse_int::<i32>(part, whole)?);
                }
            }
            _ if i == 0 => charset = Charset::from_str(part)?,
            _ => return Err(SchemaError::Invalid(whole.to_string())),
        }
    }
    Ok(DataType::varchar_with(charset, large, view, size))
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

#[cfg(test)]
mod tests;

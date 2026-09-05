//! Canonical display and recursive Arrow, SQL, Hive, and Spark parsing.

mod field;

use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Field, Result};

use super::{DataType, TimeUnit};
use crate::EdgeAlgorithm;

impl DataType {
    /// Maximum nesting accepted by the recursive string parser.
    pub const PARSE_RECURSION_LIMIT: usize = 64;

    /// Parses a canonical, Arrow-like, SQL, Hive, or Spark datatype.
    ///
    /// This is the stable entry point used by language bindings. It is also
    /// available through the standard [`FromStr`] implementation.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Self> {
        Parser::parse(input)
    }
}

impl FromStr for DataType {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        Self::from_str(input)
    }
}

impl fmt::Display for DataType {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `{:#}` is the readable, indented rendering; the plain form is the
        // canonical expression, which round-trips through `from_str`.
        if formatter.alternate() {
            return fmt::Display::fmt(&self.pretty(), formatter);
        }
        use DataType as D;
        match self {
            D::Null => formatter.write_str("null"),
            D::Boolean => formatter.write_str("boolean"),
            D::Int8 => formatter.write_str("int8"),
            D::Int16 => formatter.write_str("int16"),
            D::Int32 => formatter.write_str("int32"),
            D::Int64 => formatter.write_str("int64"),
            D::UInt8 => formatter.write_str("uint8"),
            D::UInt16 => formatter.write_str("uint16"),
            D::UInt32 => formatter.write_str("uint32"),
            D::UInt64 => formatter.write_str("uint64"),
            D::Float16 => formatter.write_str("float16"),
            D::Float32 => formatter.write_str("float32"),
            D::Float64 => formatter.write_str("float64"),
            D::DateTime64 { unit, timezone } if timezone.is_naive() => {
                write!(formatter, "datetime64({unit})")
            }
            D::DateTime64 { unit, timezone } => {
                write!(formatter, "datetime64({unit},")?;
                fmt_quoted(formatter, timezone.as_str())?;
                formatter.write_char(')')
            }
            D::Date32 => formatter.write_str("date32"),
            D::Date64 => formatter.write_str("date64"),
            D::Time32(unit) => write!(formatter, "time32({unit})"),
            D::Time64(unit) => write!(formatter, "time64({unit})"),
            D::Duration32(unit) => write!(formatter, "duration32({unit})"),
            D::Duration64(unit) => write!(formatter, "duration64({unit})"),
            D::Interval(unit) => write!(formatter, "interval({unit})"),
            D::Binary => formatter.write_str("binary"),
            D::FixedSizeBinary(width) => write!(formatter, "fixed_size_binary({width})"),
            D::LargeBinary => formatter.write_str("large_binary"),
            D::BinaryView => formatter.write_str("binary_view"),
            D::Utf8 => formatter.write_str("utf8"),
            D::LargeUtf8 => formatter.write_str("large_utf8"),
            D::Utf8View => formatter.write_str("utf8_view"),
            D::Ascii => formatter.write_str("ascii"),
            D::FixedAscii(width) => write!(formatter, "ascii({width})"),
            D::Country => formatter.write_str("country"),
            D::Currency => formatter.write_str("currency"),
            D::Mic => formatter.write_str("mic"),
            D::Cfi => formatter.write_str("cfi"),
            D::Uuid => formatter.write_str("uuid"),
            D::List(field) => fmt_single_field_type(formatter, "list", field),
            D::ListView(field) => fmt_single_field_type(formatter, "list_view", field),
            D::FixedSizeList(field, length) => {
                formatter.write_str("fixed_size_list(")?;
                fmt_field(formatter, field)?;
                write!(formatter, ",{length})")
            }
            D::LargeList(field) => fmt_single_field_type(formatter, "large_list", field),
            D::LargeListView(field) => fmt_single_field_type(formatter, "large_list_view", field),
            D::Struct(fields) => {
                formatter.write_str("struct(")?;
                for (index, field) in fields.iter().enumerate() {
                    if index != 0 {
                        formatter.write_char(',')?;
                    }
                    fmt_field(formatter, field)?;
                }
                formatter.write_char(')')
            }
            D::Union(fields, mode) => {
                write!(formatter, "union({mode}")?;
                for (type_id, field) in fields.iter() {
                    write!(formatter, ",{type_id}=")?;
                    fmt_field(formatter, field)?;
                }
                formatter.write_char(')')
            }
            D::Dictionary(dictionary) => {
                write!(
                    formatter,
                    "dictionary({},{})",
                    dictionary.key, dictionary.value
                )
            }
            D::Decimal32 { precision, scale } => {
                write!(formatter, "decimal32({precision},{scale})")
            }
            D::Decimal64 { precision, scale } => {
                write!(formatter, "decimal64({precision},{scale})")
            }
            D::Decimal128 { precision, scale } => {
                write!(formatter, "decimal128({precision},{scale})")
            }
            D::Decimal256 { precision, scale } => {
                write!(formatter, "decimal256({precision},{scale})")
            }
            D::Map(map) => {
                formatter.write_str("map(")?;
                fmt_field(formatter, &map.entries)?;
                write!(formatter, ",keys_sorted={})", map.keys_sorted)
            }
            D::RunEndEncoded(encoded) => {
                formatter.write_str("run_end_encoded(")?;
                fmt_field(formatter, &encoded.run_ends)?;
                formatter.write_char(',')?;
                fmt_field(formatter, &encoded.values)?;
                formatter.write_char(')')
            }
            D::Variant => formatter.write_str("variant"),
            // The defaults display bare, so `geometry` round-trips as itself
            // and a parameter appears exactly when it says something.
            D::Geometry(geospatial) => {
                if geospatial.has_default_crs() {
                    return formatter.write_str("geometry");
                }
                formatter.write_str("geometry(")?;
                fmt_quoted(formatter, geospatial.crs())?;
                formatter.write_char(')')
            }
            D::Geography(geospatial) => {
                let algorithm = geospatial.algorithm().unwrap_or_default();
                if geospatial.has_default_crs() && algorithm == EdgeAlgorithm::Spherical {
                    return formatter.write_str("geography");
                }
                formatter.write_str("geography(")?;
                fmt_quoted(formatter, geospatial.crs())?;
                if algorithm != EdgeAlgorithm::Spherical {
                    formatter.write_char(',')?;
                    fmt_quoted(formatter, algorithm.as_str())?;
                }
                formatter.write_char(')')
            }
        }
    }
}

fn fmt_single_field_type(
    formatter: &mut fmt::Formatter<'_>,
    kind: &str,
    field: &Field,
) -> fmt::Result {
    formatter.write_str(kind)?;
    formatter.write_char('(')?;
    fmt_field(formatter, field)?;
    formatter.write_char(')')
}

fn fmt_field(formatter: &mut fmt::Formatter<'_>, field: &Field) -> fmt::Result {
    fmt::Display::fmt(field, formatter)
}

fn fmt_quoted(formatter: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    formatter.write_char('"')?;
    for character in value.chars() {
        match character {
            '"' => formatter.write_str("\\\"")?,
            '\\' => formatter.write_str("\\\\")?,
            '\n' => formatter.write_str("\\n")?,
            '\r' => formatter.write_str("\\r")?,
            '\t' => formatter.write_str("\\t")?,
            character if character.is_control() => {
                write!(formatter, "\\u{:04x}", u32::from(character))?
            }
            character => formatter.write_char(character)?,
        }
    }
    formatter.write_char('"')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TokenKind {
    Word(SmolStr),
    Quoted(SmolStr),
    Integer(i64),
    Symbol(char),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) struct Parser<'a> {
    pub(crate) source: &'a str,
    pub(crate) tokens: Vec<Token>,
    pub(crate) index: usize,
}

impl<'a> Parser<'a> {
    pub(crate) fn parse(source: &'a str) -> Result<DataType> {
        let tokens = tokenize(source)?;
        let mut parser = Self {
            source,
            tokens,
            index: 0,
        };
        if parser.tokens.is_empty() {
            return Err(parser.error_at(0, "expected a datatype"));
        }
        let value = parser.parse_type(0).map_err(|error| match error {
            error @ Error::Parse { .. } => error,
            error => parser.error_here(format_smolstr!("{error}")),
        })?;
        if !parser.is_done() {
            return Err(parser.error_here("unexpected trailing token"));
        }
        Ok(value)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_type(&mut self, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;

        if let Some(open) = self.peek_symbol() {
            if let Some(close) = matching_close(open) {
                self.index += 1;
                let value = self.parse_type(depth + 1)?;
                self.expect_symbol(close)?;
                return self.parse_postfix_lists(value, depth);
            }
        }

        let token = self
            .next()
            .ok_or_else(|| self.error_here("expected a datatype"))?;
        let word = match token.kind {
            TokenKind::Word(word) => word,
            TokenKind::Quoted(value) => {
                let nested_tokens = tokenize(&value).map_err(|error| match error {
                    Error::Parse {
                        position, reason, ..
                    } => self.error_at(token.start + 1 + position, reason),
                    error => self.error_at(token.start + 1, format_smolstr!("{error}")),
                })?;
                let mut nested = Parser {
                    source: &value,
                    tokens: nested_tokens,
                    index: 0,
                };
                let dtype = nested.parse_type(depth + 1).map_err(|error| match error {
                    Error::Parse {
                        position, reason, ..
                    } => self.error_at(token.start + 1 + position, reason),
                    error => self.error_at(token.start + 1, format_smolstr!("{error}")),
                })?;
                if !nested.is_done() {
                    return Err(self.error_at(token.start, "quoted datatype has trailing tokens"));
                }
                return self.parse_postfix_lists(dtype, depth);
            }
            _ => return Err(self.error_at(token.start, "expected a datatype name")),
        };
        let keyword = normalized(&word);

        let value = match keyword.as_str() {
            "null" | "void" => DataType::Null,
            "boolean" | "bool" => DataType::Boolean,
            "int8" | "tinyint" | "byte" => DataType::Int8,
            "int16" | "smallint" | "short" => DataType::Int16,
            "int32" | "int" | "integer" => DataType::Int32,
            "int64" | "bigint" | "long" => DataType::Int64,
            "uint8" | "utinyint" | "unsignedtinyint" => DataType::UInt8,
            "uint16" | "usmallint" | "unsignedsmallint" => DataType::UInt16,
            "uint32" | "uint" | "unsignedint" | "unsignedinteger" => DataType::UInt32,
            "uint64" | "ubigint" | "unsignedbigint" => DataType::UInt64,
            "float16" | "half" | "halffloat" => DataType::Float16,
            "float32" | "float" | "real" => DataType::Float32,
            "float64" | "double" | "doubleprecision" => {
                self.consume_word("precision");
                DataType::Float64
            }
            "datetime64"
            | "timestamp"
            | "timestampntz"
            | "timestampltz"
            | "timestampwithtimezone" => self.parse_datetime64(&keyword, depth)?,
            "date" | "date32" => DataType::Date32,
            "date64" | "datemillisecond" => DataType::Date64,
            "time" => self.parse_sql_time(depth)?,
            "time32" => {
                let (unit, unit_start) = self.parse_required_time_unit(depth)?;
                DataType::time32(unit)
                    .map_err(|error| self.error_at(unit_start, format_smolstr!("{error}")))?
            }
            "time64" => {
                let (unit, unit_start) = self.parse_required_time_unit(depth)?;
                DataType::time64(unit)
                    .map_err(|error| self.error_at(unit_start, format_smolstr!("{error}")))?
            }
            "duration32" => DataType::duration32(self.parse_required_time_unit(depth)?.0)?,
            // Arrow's Debug form is exactly `Duration(unit)` and has no width
            // because Arrow stores every duration in i64. Preserve that
            // foreign round-trip without reviving lowercase `duration(...)`
            // as a public width-ambiguous alias.
            "duration" if word == "Duration" => {
                DataType::duration64(self.parse_required_time_unit(depth)?.0)?
            }
            "duration64" => DataType::duration64(self.parse_required_time_unit(depth)?.0)?,
            "interval" => DataType::Interval(self.parse_interval_unit(depth)?),
            "binary" | "bytes" | "varbinary" | "blob" | "bytea" => {
                self.ignore_optional_length()?;
                DataType::Binary
            }
            "fixedsizebinary" | "fixedbinary" => {
                DataType::fixed_size_binary(self.parse_single_i32_parameter("binary width")?)?
            }
            "largebinary" => DataType::LargeBinary,
            "binaryview" => DataType::BinaryView,
            "utf8" | "string" | "str" | "text" | "varchar" | "nvarchar" | "char" | "character"
            | "charactervarying" => {
                if keyword == "character" {
                    self.consume_word("varying");
                }
                self.ignore_optional_length()?;
                DataType::Utf8
            }
            "largeutf8" | "largestring" => DataType::LargeUtf8,
            "utf8view" | "stringview" => DataType::Utf8View,

            "uuid" => DataType::Uuid,
            // Bare `ascii` is the variable shape; `ascii(N)` is the fixed
            // one of exactly N bytes.
            "ascii" => match self.consume_opening() {
                Some(close) => {
                    let position = self.current_position();
                    let width = self.parse_i32("ASCII width")?;
                    self.expect_symbol(close)?;
                    DataType::ascii(width)
                        .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?
                }
                None => DataType::Ascii,
            },
            "list" | "array" => self.parse_list(ListKind::List, depth + 1)?,
            "listview" | "arrayview" => self.parse_list(ListKind::ListView, depth + 1)?,
            "fixedsizelist" | "fixedarray" => self.parse_fixed_size_list(depth + 1)?,
            "largelist" | "largearray" => self.parse_list(ListKind::LargeList, depth + 1)?,
            "largelistview" | "largearrayview" => {
                self.parse_list(ListKind::LargeListView, depth + 1)?
            }
            "struct" | "row" => self.parse_struct(depth + 1)?,
            "union" | "denseunion" | "sparseunion" => self.parse_union(&keyword, depth + 1)?,
            // The parenthesis disambiguates, deterministically: bare `variant`
            // is the self-describing semi-structured datatype, and
            // `variant(...)` with members stays the dense-union input sugar.
            "variant" => {
                if self.peek_opening().is_some() {
                    self.parse_union(&keyword, depth + 1)?
                } else {
                    DataType::Variant
                }
            }
            "geometry" => self.parse_geospatial(false)?,
            "geography" => self.parse_geospatial(true)?,
            "dictionary" | "dict" => self.parse_dictionary(depth + 1)?,
            "decimal" | "numeric" => {
                let (precision, scale) = self.parse_decimal_parameters(38)?;
                DataType::decimal(precision, scale)?
            }
            "decimal128" => {
                let (precision, scale) = self.parse_decimal_parameters(38)?;
                DataType::decimal128(precision, scale)?
            }
            "decimal32" => {
                let (precision, scale) = self.parse_decimal_parameters(9)?;
                DataType::decimal32(precision, scale)?
            }
            "decimal64" => {
                let (precision, scale) = self.parse_decimal_parameters(18)?;
                DataType::decimal64(precision, scale)?
            }
            "decimal256" | "bignumeric" => {
                let (precision, scale) = self.parse_decimal_parameters(76)?;
                DataType::decimal256(precision, scale)?
            }
            "map" => self.parse_map(depth + 1)?,
            "runendencoded" | "runend" | "ree" => self.parse_run_end(depth + 1)?,
            // A registered logical name is one more spelling of the datatype
            // it names, resolved through the registry and never a copied
            // list. The keyword is already folded, so the lookup reuses it.
            _ => match super::vocabulary::folded_logical_name(&keyword) {
                Some(dtype) => dtype,
                None => {
                    return Err(
                        self.error_at(token.start, format_smolstr!("unknown datatype {word:?}"))
                    );
                }
            },
        };

        self.parse_postfix_lists(value, depth)
    }

    pub(crate) fn parse_postfix_lists(
        &mut self,
        mut value: DataType,
        depth: usize,
    ) -> Result<DataType> {
        let mut nesting = depth;
        while self.peek_symbol() == Some('[')
            && self
                .tokens
                .get(self.index + 1)
                .is_some_and(|token| token.kind == TokenKind::Symbol(']'))
        {
            self.check_depth(nesting + 1)?;
            self.index += 2;
            value = DataType::list(Field::new("item", value, true));
            nesting += 1;
        }
        Ok(value)
    }

    pub(crate) fn parse_single_i32_parameter(&mut self, label: &str) -> Result<i32> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here(format_smolstr!("expected {label}")))?;
        let value = self.parse_i32(label)?;
        self.expect_symbol(close)?;
        Ok(value)
    }

    pub(crate) fn ignore_optional_length(&mut self) -> Result<()> {
        // Square brackets after a scalar are the SQL/Spark postfix-array
        // operator, not a string length declaration.
        if self.peek_symbol() != Some('(') {
            return Ok(());
        }
        let Some(close) = self.consume_opening() else {
            return Ok(());
        };
        let _ = self.parse_integer("length")?;
        self.expect_symbol(close)
    }

    pub(crate) fn parse_field_or_type(
        &mut self,
        default_name: &str,
        default_nullable: bool,
        depth: usize,
    ) -> Result<Field> {
        self.check_depth(depth)?;
        if self.peek_word_is("field") {
            return self.parse_explicit_field(depth, Some(default_name));
        }
        if self.looks_like_named_field() {
            return self.parse_named_field(depth);
        }
        let dtype = self.parse_type(depth)?;
        let nullable = self.parse_nullability(default_nullable)?;
        Ok(Field::new(default_name, dtype, nullable))
    }

    pub(crate) fn parse_named_field(&mut self, depth: usize) -> Result<Field> {
        self.check_depth(depth)?;
        if self.peek_word_is("field") {
            return self.parse_explicit_field(depth, None);
        }
        let name = self.parse_text("field name")?;
        if !self.consume_symbol(':')
            && !self.consume_symbol('=')
            && (self.peek_symbol().is_some_and(is_closing_or_separator) || self.is_done())
        {
            return Err(self.error_here("expected a datatype after the field name"));
        }
        let dtype = self.parse_type(depth)?;
        let nullable = self.parse_nullability(true)?;
        Ok(Field::new(name, dtype, nullable))
    }

    pub(crate) fn parse_explicit_field(
        &mut self,
        depth: usize,
        default_name: Option<&str>,
    ) -> Result<Field> {
        self.expect_word("field")?;
        if self.peek_symbol() == Some('{') {
            return self.parse_arrow_field(depth, default_name);
        }
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected field(...)"))?;
        let name = self.parse_text("field name")?;
        self.expect_separator("expected datatype after field name")?;
        let dtype = self.parse_type(depth)?;
        self.expect_separator("expected nullable= after datatype")?;
        self.consume_label("nullable");
        let nullable = self.parse_bool("field nullability")?;
        let mut metadata = Vec::new();
        let mut saw_metadata = false;
        let mut dictionary_id = None;
        let mut dictionary_is_ordered = None;
        while self.consume_separator() {
            if self.consume_label("dictionary_id") || self.consume_label("dict_id") {
                if dictionary_id.is_some() {
                    return Err(self.error_here("duplicate dictionary id"));
                }
                dictionary_id = Some(self.parse_integer("dictionary id")?);
            } else if self.consume_label("dictionary_is_ordered")
                || self.consume_label("dict_is_ordered")
            {
                if dictionary_is_ordered.is_some() {
                    return Err(self.error_here("duplicate dictionary ordering flag"));
                }
                dictionary_is_ordered = Some(self.parse_bool("dictionary ordering")?);
            } else if self.consume_label("metadata") {
                if saw_metadata {
                    return Err(self.error_here("duplicate metadata"));
                }
                metadata = self.parse_metadata()?;
                saw_metadata = true;
            } else {
                return Err(self.error_here("unknown field argument"));
            }
        }
        self.expect_symbol(close)?;
        let mut field = Field::from_parts(name, dtype, nullable, metadata)?;
        if dictionary_id.is_some() || dictionary_is_ordered.is_some() {
            field.set_dictionary_options(
                dictionary_id.unwrap_or_default(),
                dictionary_is_ordered.unwrap_or_default(),
            )?;
        }
        Ok(field)
    }

    pub(crate) fn parse_arrow_field(
        &mut self,
        depth: usize,
        default_name: Option<&str>,
    ) -> Result<Field> {
        self.expect_symbol('{')?;
        let mut name = None;
        let mut dtype = None;
        let mut nullable = None;
        let mut dictionary_id = None;
        let mut dictionary_is_ordered = None;
        let mut metadata = Vec::new();
        let mut saw_metadata = false;

        while self.peek_symbol() != Some('}') {
            let key = normalized(&self.parse_text("Arrow field property")?);
            if !self.consume_symbol(':') && !self.consume_symbol('=') {
                return Err(self.error_here("expected ':' after Arrow field property"));
            }
            match key.as_str() {
                "name" => {
                    if name.is_some() {
                        return Err(self.error_here("duplicate field name"));
                    }
                    name = Some(self.parse_text("field name")?);
                }
                // arrow-rs spells this property `data_type` in the Debug
                // output this form parses, and key normalization drops the
                // underscore. That spelling is Arrow's, not this crate's, so
                // it does not follow `Field`'s own `dtype`.
                "datatype" | "type" => {
                    if dtype.is_some() {
                        return Err(self.error_here("duplicate field datatype"));
                    }
                    dtype = Some(self.parse_type(depth)?);
                }
                "nullable" => {
                    if nullable.is_some() {
                        return Err(self.error_here("duplicate field nullability"));
                    }
                    nullable = Some(self.parse_bool("field nullability")?);
                }
                "dictionaryid" | "dictid" => {
                    if dictionary_id.is_some() {
                        return Err(self.error_here("duplicate dictionary id"));
                    }
                    dictionary_id = Some(self.parse_integer("dictionary id")?)
                }
                "dictionaryisordered" | "dictisordered" => {
                    if dictionary_is_ordered.is_some() {
                        return Err(self.error_here("duplicate dictionary ordering flag"));
                    }
                    dictionary_is_ordered = Some(self.parse_bool("dictionary ordering")?)
                }
                "metadata" => {
                    if saw_metadata {
                        return Err(self.error_here("duplicate metadata"));
                    }
                    metadata = self.parse_metadata()?;
                    saw_metadata = true;
                }
                _ => self.skip_value()?,
            }
            if self.peek_symbol() == Some('}') {
                break;
            }
            self.expect_separator("expected ',' between Arrow field properties")?;
        }
        self.expect_symbol('}')?;
        let name = name
            .or_else(|| default_name.map(SmolStr::new))
            .ok_or_else(|| {
                self.error_here("Arrow field is missing a name outside a named child context")
            })?;
        let dtype = dtype.ok_or_else(|| self.error_here("Arrow field is missing dtype"))?;
        // Arrow's Debug formatter omits `nullable` when it is false.
        let mut field = Field::from_parts(name, dtype, nullable.unwrap_or(false), metadata)?;
        if dictionary_id.is_some() || dictionary_is_ordered.is_some() {
            field.set_dictionary_options(
                dictionary_id.unwrap_or_default(),
                dictionary_is_ordered.unwrap_or_default(),
            )?;
        }
        Ok(field)
    }

    pub(crate) fn parse_metadata(&mut self) -> Result<Vec<(SmolStr, SmolStr)>> {
        let close = self
            .consume_opening()
            .filter(|close| *close == '}')
            .ok_or_else(|| self.error_here("expected metadata object"))?;
        let mut values = Vec::new();
        while self.peek_symbol() != Some(close) {
            let key = self.parse_text("metadata key")?;
            if !self.consume_symbol(':') && !self.consume_symbol('=') {
                return Err(self.error_here("expected ':' after metadata key"));
            }
            let value = self.parse_text("metadata value")?;
            values.push((key, value));
            if self.peek_symbol() == Some(close) {
                break;
            }
            self.expect_separator("expected ',' between metadata entries")?;
        }
        self.expect_symbol(close)?;
        Ok(values)
    }

    pub(crate) fn parse_nullability(&mut self, default: bool) -> Result<bool> {
        if self.consume_symbol('?') {
            return Ok(true);
        }
        if self.consume_symbol('!') {
            return Ok(false);
        }
        if self.consume_word("not") {
            self.expect_word("null")?;
            return Ok(false);
        }
        if self.consume_word("required") {
            return Ok(false);
        }
        if self.consume_word("null") || self.consume_word("nullable") {
            if self.consume_symbol('=') {
                return self.parse_bool("field nullability");
            }
            return Ok(true);
        }
        Ok(default)
    }

    pub(crate) fn parse_bool(&mut self, label: &str) -> Result<bool> {
        let token = self
            .next()
            .ok_or_else(|| self.error_here(format_smolstr!("expected {label}")))?;
        match token.kind {
            TokenKind::Word(value) | TokenKind::Quoted(value)
                if value.eq_ignore_ascii_case("true") || value == "1" =>
            {
                Ok(true)
            }
            TokenKind::Word(value) | TokenKind::Quoted(value)
                if value.eq_ignore_ascii_case("false") || value == "0" =>
            {
                Ok(false)
            }
            TokenKind::Integer(1) => Ok(true),
            TokenKind::Integer(0) => Ok(false),
            _ => Err(self.error_at(token.start, format_smolstr!("expected {label} boolean"))),
        }
    }

    pub(crate) fn parse_i32(&mut self, label: &str) -> Result<i32> {
        let position = self.current_position();
        let value = self.parse_integer(label)?;
        i32::try_from(value).map_err(|_| {
            self.error_at(
                position,
                format_smolstr!("{label} must fit in a signed 32-bit integer"),
            )
        })
    }

    pub(crate) fn parse_integer(&mut self, label: &str) -> Result<i64> {
        let token = self
            .next()
            .ok_or_else(|| self.error_here(format_smolstr!("expected {label}")))?;
        match token.kind {
            TokenKind::Integer(value) => Ok(value),
            TokenKind::Word(value) | TokenKind::Quoted(value) => {
                value.parse::<i64>().map_err(|_| {
                    self.error_at(token.start, format_smolstr!("expected integer {label}"))
                })
            }
            _ => Err(self.error_at(token.start, format_smolstr!("expected integer {label}"))),
        }
    }

    pub(crate) fn parse_text(&mut self, label: &str) -> Result<SmolStr> {
        let token = self
            .next()
            .ok_or_else(|| self.error_here(format_smolstr!("expected {label}")))?;
        match token.kind {
            TokenKind::Word(value) | TokenKind::Quoted(value) => Ok(value),
            TokenKind::Integer(value) => Ok(format_smolstr!("{value}")),
            _ => Err(self.error_at(token.start, format_smolstr!("expected {label}"))),
        }
    }

    /// Read a time zone name and canonicalize it where the position is known.
    ///
    /// Parsing is the one place a typo can still be reported against the text
    /// it came from, so the name is validated here rather than left as free
    /// text for a later layer to accept silently.
    pub(crate) fn parse_timezone(&mut self) -> Result<crate::Timezone> {
        let start = self.current_position();
        let text = self.parse_text("timezone")?;
        crate::Timezone::from_smol_str(text)
            .map_err(|error| self.error_at(start, format_smolstr!("{error}")))
    }

    pub(crate) fn skip_value(&mut self) -> Result<()> {
        if let Some(open) = self.peek_symbol() {
            if let Some(close) = matching_close(open) {
                self.index += 1;
                let mut depth = 1_usize;
                while depth != 0 {
                    let token = self
                        .next()
                        .ok_or_else(|| self.error_here("unclosed Arrow field property"))?;
                    match token.kind {
                        TokenKind::Symbol(symbol) if symbol == open => depth += 1,
                        TokenKind::Symbol(symbol) if symbol == close => depth -= 1,
                        _ => {}
                    }
                }
                return Ok(());
            }
        }
        self.next()
            .map(|_| ())
            .ok_or_else(|| self.error_here("expected Arrow field property value"))
    }

    pub(crate) fn looks_like_named_field(&self) -> bool {
        matches!(
            (
                self.tokens.get(self.index).map(|token| &token.kind),
                self.tokens.get(self.index + 1).map(|token| &token.kind)
            ),
            (
                Some(TokenKind::Word(_) | TokenKind::Quoted(_)),
                Some(TokenKind::Symbol(':' | '='))
            )
        )
    }

    pub(crate) fn consume_label(&mut self, label: &str) -> bool {
        if self.peek_word_is(label)
            && self
                .tokens
                .get(self.index + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Symbol('=' | ':')))
        {
            self.index += 2;
            true
        } else {
            false
        }
    }

    pub(crate) fn expect_separator(&mut self, reason: &str) -> Result<()> {
        if self.consume_separator() {
            Ok(())
        } else {
            Err(self.error_here(reason))
        }
    }

    pub(crate) fn consume_separator(&mut self) -> bool {
        self.consume_symbol(',') || self.consume_symbol(';')
    }

    pub(crate) fn consume_opening(&mut self) -> Option<char> {
        let close = matching_close(self.peek_symbol()?)?;
        self.index += 1;
        Some(close)
    }

    pub(crate) fn peek_opening(&self) -> Option<char> {
        matching_close(self.peek_symbol()?)
    }

    pub(crate) fn expect_symbol(&mut self, expected: char) -> Result<()> {
        if self.consume_symbol(expected) {
            Ok(())
        } else {
            Err(self.error_here(format_smolstr!("expected {expected:?}")))
        }
    }

    pub(crate) fn consume_symbol(&mut self, expected: char) -> bool {
        if self.peek_symbol() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub(crate) fn peek_symbol(&self) -> Option<char> {
        match self.tokens.get(self.index).map(|token| &token.kind) {
            Some(TokenKind::Symbol(symbol)) => Some(*symbol),
            _ => None,
        }
    }

    pub(crate) fn expect_word(&mut self, expected: &str) -> Result<()> {
        if self.consume_word(expected) {
            Ok(())
        } else {
            Err(self.error_here(format_smolstr!("expected {expected:?}")))
        }
    }

    pub(crate) fn consume_word(&mut self, expected: &str) -> bool {
        if self.peek_word_is(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub(crate) fn peek_word_is(&self, expected: &str) -> bool {
        match self.tokens.get(self.index).map(|token| &token.kind) {
            Some(TokenKind::Word(value)) => normalized(value) == normalized(expected),
            _ => false,
        }
    }

    pub(crate) fn peek_integer(&self) -> Option<i64> {
        match self.tokens.get(self.index).map(|token| &token.kind) {
            Some(TokenKind::Integer(value)) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn peek_union_mode(&self, close: char) -> bool {
        (self.peek_word_is("dense") || self.peek_word_is("sparse"))
            && matches!(
                self.tokens.get(self.index + 1).map(|token| &token.kind),
                Some(TokenKind::Symbol(symbol))
                    if *symbol == close || matches!(*symbol, ',' | ';')
            )
    }

    pub(crate) fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.index)?.clone();
        self.index += 1;
        Some(token)
    }

    pub(crate) fn is_done(&self) -> bool {
        self.index == self.tokens.len()
    }

    pub(crate) fn current_position(&self) -> usize {
        self.tokens
            .get(self.index)
            .map_or(self.source.len(), |token| token.start)
    }

    pub(crate) fn check_depth(&self, depth: usize) -> Result<()> {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            Err(self.error_here(format_smolstr!(
                "datatype nesting exceeds the limit of {}",
                DataType::PARSE_RECURSION_LIMIT
            )))
        } else {
            Ok(())
        }
    }

    pub(crate) fn error_here(&self, reason: impl Into<SmolStr>) -> Error {
        self.error_at(self.current_position(), reason)
    }

    pub(crate) fn error_at(&self, position: usize, reason: impl Into<SmolStr>) -> Error {
        parse_error(self.source, position, reason)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ListKind {
    List,
    ListView,
    LargeList,
    LargeListView,
}

fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut position = 0_usize;

    while position < source.len() {
        let character = source[position..]
            .chars()
            .next()
            .ok_or_else(|| parse_error(source, position, "invalid UTF-8 boundary"))?;
        if character.is_whitespace() {
            position += character.len_utf8();
            continue;
        }

        if matches!(character, '\'' | '"' | '`') {
            let (value, end) = tokenize_quoted(source, position, character)?;
            tokens.push(Token {
                kind: TokenKind::Quoted(value),
                start: position,
                end,
            });
            position = end;
            continue;
        }

        if is_symbol(character) {
            let end = position + character.len_utf8();
            tokens.push(Token {
                kind: TokenKind::Symbol(character),
                start: position,
                end,
            });
            position = end;
            continue;
        }

        let next_is_digit = source[position + character.len_utf8()..]
            .chars()
            .next()
            .is_some_and(|next| next.is_ascii_digit());
        if character.is_ascii_digit() || ((character == '-' || character == '+') && next_is_digit) {
            let start = position;
            position += character.len_utf8();
            while position < source.len() {
                let next = source[position..]
                    .chars()
                    .next()
                    .ok_or_else(|| parse_error(source, position, "invalid UTF-8 boundary"))?;
                if !next.is_ascii_digit() {
                    break;
                }
                position += next.len_utf8();
            }
            let value = source[start..position].parse::<i64>().map_err(|_| {
                parse_error(source, start, "integer parameter is outside the i64 range")
            })?;
            tokens.push(Token {
                kind: TokenKind::Integer(value),
                start,
                end: position,
            });
            continue;
        }

        let start = position;
        while position < source.len() {
            let next = source[position..]
                .chars()
                .next()
                .ok_or_else(|| parse_error(source, position, "invalid UTF-8 boundary"))?;
            if next.is_whitespace() || is_symbol(next) || matches!(next, '\'' | '"' | '`') {
                break;
            }
            position += next.len_utf8();
        }
        if position == start {
            return Err(parse_error(source, position, "unexpected character"));
        }
        tokens.push(Token {
            kind: TokenKind::Word(source[start..position].into()),
            start,
            end: position,
        });
    }
    Ok(tokens)
}

fn tokenize_quoted(source: &str, start: usize, quote: char) -> Result<(SmolStr, usize)> {
    let mut position = start + quote.len_utf8();
    let mut value = String::new();
    while position < source.len() {
        let character = source[position..]
            .chars()
            .next()
            .ok_or_else(|| parse_error(source, position, "invalid UTF-8 boundary"))?;
        position += character.len_utf8();

        if character == quote {
            if source[position..].starts_with(quote) {
                value.push(quote);
                position += quote.len_utf8();
                continue;
            }
            return Ok((value.into(), position));
        }
        if character != '\\' {
            value.push(character);
            continue;
        }

        let escape_position = position;
        let escaped = source[position..]
            .chars()
            .next()
            .ok_or_else(|| parse_error(source, position, "unterminated escape sequence"))?;
        position += escaped.len_utf8();
        match escaped {
            '\\' => value.push('\\'),
            '\'' => value.push('\''),
            '"' => value.push('"'),
            '`' => value.push('`'),
            'n' => value.push('\n'),
            'r' => value.push('\r'),
            't' => value.push('\t'),
            'b' => value.push('\u{0008}'),
            'f' => value.push('\u{000c}'),
            'u' => {
                let end = position.saturating_add(4);
                if end > source.len() || !source.is_char_boundary(end) {
                    return Err(parse_error(
                        source,
                        escape_position,
                        "incomplete Unicode escape",
                    ));
                }
                let code = u32::from_str_radix(&source[position..end], 16)
                    .map_err(|_| parse_error(source, escape_position, "invalid Unicode escape"))?;
                let decoded = char::from_u32(code).ok_or_else(|| {
                    parse_error(source, escape_position, "invalid Unicode scalar")
                })?;
                value.push(decoded);
                position = end;
            }
            _ => {
                return Err(parse_error(
                    source,
                    escape_position,
                    format_smolstr!("unsupported escape \\{escaped}"),
                ));
            }
        }
    }
    Err(parse_error(source, start, "unterminated quoted value"))
}

fn parse_error(source: &str, position: usize, reason: impl Into<SmolStr>) -> Error {
    let reason = reason.into();
    let position = position.min(source.len());
    let mut start = position.saturating_sub(16);
    while start > 0 && !source.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = position.saturating_add(24).min(source.len());
    while end < source.len() && !source.is_char_boundary(end) {
        end += 1;
    }
    let context = &source[start..end];
    Error::Parse {
        target: "datatype",
        position,
        reason: format_smolstr!("{reason}; near {context:?}"),
    }
}

fn is_symbol(character: char) -> bool {
    matches!(
        character,
        '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '=' | '?' | '!'
    )
}

fn matching_close(open: char) -> Option<char> {
    match open {
        '<' => Some('>'),
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

pub(crate) fn is_closing_or_separator(symbol: char) -> bool {
    matches!(symbol, '>' | ')' | ']' | '}' | ',' | ';')
}

pub(crate) fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn precision_to_unit(precision: i64, position: usize) -> Result<TimeUnit> {
    match precision {
        0 => Ok(TimeUnit::Second),
        1..=3 => Ok(TimeUnit::Millisecond),
        4..=6 => Ok(TimeUnit::Microsecond),
        7..=9 => Ok(TimeUnit::Nanosecond),
        _ => Err(Error::Parse {
            target: "datatype",
            position,
            reason: "temporal precision must be between 0 and 9".into(),
        }),
    }
}

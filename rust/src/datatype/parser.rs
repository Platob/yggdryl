//! Canonical display and recursive Arrow, SQL, Hive, and Spark parsing.

use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Field, Result};

use super::{DataType, TimeUnit, UnionMode};
use crate::enums::EdgeAlgorithm;

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
            D::Timestamp(unit, None) => write!(formatter, "timestamp({unit})"),
            D::Timestamp(unit, Some(timezone)) => {
                write!(formatter, "timestamp({unit},")?;
                fmt_quoted(formatter, timezone.as_str())?;
                formatter.write_char(')')
            }
            D::Date32 => formatter.write_str("date32"),
            D::Date64 => formatter.write_str("date64"),
            D::Time32(unit) => write!(formatter, "time32({unit})"),
            D::Time64(unit) => write!(formatter, "time64({unit})"),
            D::Duration(unit) => write!(formatter, "duration({unit})"),
            D::Interval(unit) => write!(formatter, "interval({unit})"),
            D::Binary => formatter.write_str("binary"),
            D::FixedSizeBinary(width) => write!(formatter, "fixed_size_binary({width})"),
            D::LargeBinary => formatter.write_str("large_binary"),
            D::BinaryView => formatter.write_str("binary_view"),
            D::Utf8 => formatter.write_str("utf8"),
            D::LargeUtf8 => formatter.write_str("large_utf8"),
            D::Utf8View => formatter.write_str("utf8_view"),
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
enum TokenKind {
    Word(SmolStr),
    Quoted(SmolStr),
    Integer(i64),
    Symbol(char),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    index: usize,
}

impl<'a> Parser<'a> {
    fn parse(source: &'a str) -> Result<DataType> {
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
    fn parse_type(&mut self, depth: usize) -> Result<DataType> {
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
                let data_type = nested.parse_type(depth + 1).map_err(|error| match error {
                    Error::Parse {
                        position, reason, ..
                    } => self.error_at(token.start + 1 + position, reason),
                    error => self.error_at(token.start + 1, format_smolstr!("{error}")),
                })?;
                if !nested.is_done() {
                    return Err(self.error_at(token.start, "quoted datatype has trailing tokens"));
                }
                return self.parse_postfix_lists(data_type, depth);
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
            "timestamp" | "timestampntz" | "timestampltz" | "timestampwithtimezone" => {
                self.parse_timestamp(&keyword, depth)?
            }
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
            "duration" => DataType::Duration(self.parse_required_time_unit(depth)?.0),
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
            "list" | "array" => self.parse_list(ListKind::List, depth + 1)?,
            "listview" | "arrayview" => self.parse_list(ListKind::ListView, depth + 1)?,
            "fixedsizelist" | "fixedarray" => self.parse_fixed_size_list(depth + 1)?,
            "largelist" | "largearray" => self.parse_list(ListKind::LargeList, depth + 1)?,
            "largelistview" | "largearrayview" => {
                self.parse_list(ListKind::LargeListView, depth + 1)?
            }
            "struct" | "row" | "record" => self.parse_struct(depth + 1)?,
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
            _ => {
                return Err(
                    self.error_at(token.start, format_smolstr!("unknown datatype {word:?}"))
                );
            }
        };

        self.parse_postfix_lists(value, depth)
    }

    fn parse_postfix_lists(&mut self, mut value: DataType, depth: usize) -> Result<DataType> {
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

    fn parse_timestamp(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        let mut unit = TimeUnit::Microsecond;
        let mut timezone = (keyword == "timestampltz" || keyword == "timestampwithtimezone")
            .then_some(crate::Timezone::UTC);

        if let Some(close) = self.consume_opening() {
            if self.peek_symbol() != Some(close) {
                if let Some(precision) = self.peek_integer() {
                    let precision_start = self.current_position();
                    self.index += 1;
                    unit = precision_to_unit(precision, precision_start)?;
                } else if self.peek_word_is("none") {
                    self.index += 1;
                    timezone = None;
                } else if self.peek_word_is("some") {
                    self.index += 1;
                    let inner_close = self
                        .consume_opening()
                        .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                    timezone = Some(self.parse_timezone()?);
                    self.expect_symbol(inner_close)?;
                } else {
                    let (parsed, unit_start) =
                        self.parse_time_unit_span(Some(close), "timestamp unit")?;
                    unit = parsed;
                    if !unit.is_temporal() {
                        return Err(self.error_at(
                            unit_start,
                            "timestamp requires a temporal resolution unit",
                        ));
                    }
                }

                if self.consume_separator() {
                    self.consume_label("timezone");
                    if self.peek_word_is("none") {
                        self.index += 1;
                        timezone = None;
                    } else if self.peek_word_is("some") {
                        self.index += 1;
                        let inner_close = self
                            .consume_opening()
                            .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                        timezone = Some(self.parse_timezone()?);
                        self.expect_symbol(inner_close)?;
                    } else {
                        timezone = Some(self.parse_timezone()?);
                    }
                }
            }
            self.expect_symbol(close)?;
        }

        if self.consume_word("with") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone.get_or_insert(crate::Timezone::UTC);
        } else if self.consume_word("without") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone = None;
        }

        Ok(DataType::Timestamp(unit, timezone))
    }

    fn parse_sql_time(&mut self, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        if self.peek_opening().is_none() {
            return DataType::time(TimeUnit::Microsecond);
        }
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected time unit"))?;
        let (unit, unit_start) = if let Some(precision) = self.peek_integer() {
            let start = self.current_position();
            self.index += 1;
            (precision_to_unit(precision, start)?, start)
        } else {
            self.parse_time_unit_span(Some(close), "time unit")?
        };
        self.expect_symbol(close)?;
        DataType::time(unit).map_err(|error| self.error_at(unit_start, format_smolstr!("{error}")))
    }

    fn parse_required_time_unit(&mut self, depth: usize) -> Result<(TimeUnit, usize)> {
        self.check_depth(depth)?;
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected a temporal unit parameter"))?;
        let (unit, unit_start) = self.parse_time_unit_span(Some(close), "temporal unit")?;
        if !unit.is_temporal() {
            return Err(self.error_at(unit_start, "expected a temporal resolution"));
        }
        self.expect_symbol(close)?;
        Ok((unit, unit_start))
    }

    fn parse_interval_unit(&mut self, depth: usize) -> Result<TimeUnit> {
        self.check_depth(depth)?;
        let (unit, unit_start) = if self
            .tokens
            .get(self.index)
            .is_none_or(|_| self.is_time_unit_boundary(self.index, None))
        {
            (TimeUnit::MonthDayNano, self.current_position())
        } else if let Some(close) = self.consume_opening() {
            let parsed = self.parse_time_unit_span(Some(close), "interval unit")?;
            self.expect_symbol(close)?;
            parsed
        } else {
            self.parse_time_unit_span(None, "interval unit")?
        };
        if unit.is_interval() {
            Ok(unit)
        } else {
            Err(self.error_at(unit_start, "interval requires an interval layout"))
        }
    }

    fn parse_time_unit_span(
        &mut self,
        close: Option<char>,
        label: &str,
    ) -> Result<(TimeUnit, usize)> {
        let first = self.index;
        let mut end = first;
        while self.tokens.get(end).is_some() {
            if self.is_time_unit_boundary(end, close) {
                break;
            }
            end += 1;
        }
        if first == end {
            return Err(self.error_here(format_smolstr!("expected {label}")));
        }

        let first_token = &self.tokens[first];
        let last_token = &self.tokens[end - 1];
        let (value, source_start, quoted_token) = match &first_token.kind {
            TokenKind::Quoted(value) if end == first + 1 => {
                (value.as_str(), first_token.start + 1, Some(first_token))
            }
            _ => (
                &self.source[first_token.start..last_token.end],
                first_token.start,
                None,
            ),
        };
        let parsed = TimeUnit::from_str(value).map_err(|error| match error {
            Error::Parse {
                position, reason, ..
            } => {
                let position = quoted_token.map_or_else(
                    || source_start.saturating_add(position),
                    |token| self.quoted_source_position(token, position),
                );
                self.error_at(position, reason)
            }
            error => self.error_at(source_start, format_smolstr!("{error}")),
        });
        self.index = end;
        parsed.map(|unit| (unit, source_start))
    }

    fn quoted_source_position(&self, token: &Token, decoded_position: usize) -> usize {
        let Some(quote) = self.source[token.start..].chars().next() else {
            return token.start;
        };
        let mut source_position = token.start.saturating_add(quote.len_utf8());
        let body_end = token.end.saturating_sub(quote.len_utf8());
        let mut decoded_offset = 0_usize;

        while source_position < body_end {
            if decoded_position <= decoded_offset {
                return source_position;
            }
            let logical_start = source_position;
            let Some(character) = self.source[source_position..].chars().next() else {
                return logical_start;
            };
            source_position = source_position.saturating_add(character.len_utf8());

            let decoded_len =
                if character == quote && self.source[source_position..].starts_with(quote) {
                    source_position = source_position.saturating_add(quote.len_utf8());
                    quote.len_utf8()
                } else if character == '\\' {
                    let Some(escaped) = self.source[source_position..].chars().next() else {
                        return logical_start;
                    };
                    source_position = source_position.saturating_add(escaped.len_utf8());
                    if escaped == 'u' {
                        let digits_end = source_position.saturating_add(4);
                        let decoded_len = self
                            .source
                            .get(source_position..digits_end)
                            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                            .and_then(char::from_u32)
                            .map_or(1, char::len_utf8);
                        source_position = digits_end.min(body_end);
                        decoded_len
                    } else {
                        1
                    }
                } else {
                    character.len_utf8()
                };

            if decoded_position < decoded_offset.saturating_add(decoded_len) {
                return logical_start;
            }
            decoded_offset = decoded_offset.saturating_add(decoded_len);
        }
        body_end
    }

    fn is_time_unit_boundary(&self, index: usize, close: Option<char>) -> bool {
        match self.tokens.get(index).map(|token| &token.kind) {
            Some(TokenKind::Symbol(symbol)) => {
                close == Some(*symbol)
                    || matches!(*symbol, ',' | ';')
                    || (close.is_none()
                        && (is_closing_or_separator(*symbol)
                            || matches!(*symbol, '?' | '!')
                            || (*symbol == '['
                                && self
                                    .tokens
                                    .get(index + 1)
                                    .is_some_and(|next| next.kind == TokenKind::Symbol(']')))))
            }
            Some(TokenKind::Word(value)) if close.is_none() => {
                ["not", "required", "null", "nullable"]
                    .iter()
                    .any(|boundary| value.eq_ignore_ascii_case(boundary))
            }
            _ => false,
        }
    }

    fn parse_list(&mut self, kind: ListKind, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected a list child in (), [], {}, or <>"))?;
        let field = self.parse_field_or_type("item", true, depth)?;
        self.expect_symbol(close)?;
        Ok(match kind {
            ListKind::List => DataType::list(field),
            ListKind::ListView => DataType::list_view(field),
            ListKind::LargeList => DataType::large_list(field),
            ListKind::LargeListView => DataType::large_list_view(field),
        })
    }

    fn parse_fixed_size_list(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected fixed-size-list parameters"))?;
        let field = self.parse_field_or_type("item", true, depth)?;
        self.expect_separator("expected a list length after the child")?;
        self.consume_label("length");
        let length = self.parse_i32("list length")?;
        self.expect_symbol(close)?;
        DataType::fixed_size_list(field, length)
    }

    fn parse_struct(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected struct fields"))?;
        let collection_close = if self.peek_symbol() == Some('[') {
            self.index += 1;
            Some(']')
        } else {
            None
        };
        let body_close = collection_close.unwrap_or(close);
        let mut fields = Vec::new();
        while self.peek_symbol() != Some(body_close) {
            fields.push(self.parse_named_field(depth)?);
            if self.peek_symbol() == Some(body_close) {
                break;
            }
            self.expect_separator("expected ',' between struct fields")?;
        }
        self.expect_symbol(body_close)?;
        if collection_close.is_some() {
            self.expect_symbol(close)?;
        }
        DataType::from_fields(fields)
    }

    fn parse_dictionary(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected dictionary key and value types"))?;
        self.consume_label("key");
        let key = self.parse_type(depth)?;
        self.expect_separator("expected dictionary value type")?;
        self.consume_label("value");
        let value = self.parse_type(depth)?;
        self.expect_symbol(close)?;
        DataType::dictionary(key, value)
    }

    fn parse_map(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected map parameters"))?;

        if self.peek_word_is("field") {
            let entries = self.parse_explicit_field(depth, Some("entries"))?;
            let mut keys_sorted = false;
            if self.consume_separator() {
                self.consume_label("keys_sorted");
                keys_sorted = self.parse_bool("keys_sorted")?;
            }
            self.expect_symbol(close)?;
            return DataType::map(entries, keys_sorted);
        }

        self.consume_label("key");
        let key = self.parse_type(depth)?;
        self.expect_separator("expected map value type")?;
        self.consume_label("value");
        let value = self.parse_type(depth)?;
        let mut keys_sorted = false;
        if self.consume_separator() {
            self.consume_label("keys_sorted");
            keys_sorted = self.parse_bool("keys_sorted")?;
        }
        self.expect_symbol(close)?;
        DataType::map_of(key, value, keys_sorted)
    }

    fn parse_run_end(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected run-end and value fields"))?;
        let run_ends = self.parse_field_or_type("run_ends", false, depth)?;
        self.expect_separator("expected encoded values field")?;
        let values = self.parse_field_or_type("values", true, depth)?;
        self.expect_symbol(close)?;
        DataType::run_end_encoded(run_ends, values)
    }

    /// Parse the optional `('crs')` / `('crs', 'algorithm')` parameters.
    ///
    /// Bare `geometry` and `geography` fill the defaults, so the parameters
    /// appear exactly when they say something. A geometry given an edge
    /// algorithm is refused by name at the algorithm's own position -
    /// straight planar lines need none - and an unknown algorithm reports the
    /// accepted vocabulary.
    fn parse_geospatial(&mut self, geography: bool) -> Result<DataType> {
        let build = |crs: Option<&str>, algorithm: Option<EdgeAlgorithm>| {
            if geography {
                DataType::geography(crs, algorithm)
            } else {
                DataType::geometry(crs)
            }
        };
        let Some(close) = self.consume_opening() else {
            return build(None, None).map_err(|error| self.error_here(format_smolstr!("{error}")));
        };
        // Empty parentheses are the bare spelling with punctuation.
        if self.consume_symbol(close) {
            return build(None, None).map_err(|error| self.error_here(format_smolstr!("{error}")));
        }
        let crs_position = self.current_position();
        let crs = self.parse_text("a coordinate reference system")?;
        let mut algorithm = None;
        if self.consume_separator() {
            let algorithm_position = self.current_position();
            let name = self.parse_text("an edge algorithm")?;
            if !geography {
                return Err(self.error_at(
                    algorithm_position,
                    format_smolstr!(
                        "expected no edge algorithm for geometry, got {name:?};                          geography is the type whose edges take one"
                    ),
                ));
            }
            algorithm =
                Some(EdgeAlgorithm::from_str(&name).map_err(|error| {
                    self.error_at(algorithm_position, format_smolstr!("{error}"))
                })?);
        }
        self.expect_symbol(close)?;
        build(Some(&crs), algorithm)
            .map_err(|error| self.error_at(crs_position, format_smolstr!("{error}")))
    }

    fn parse_union(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        let is_variant = keyword == "variant";
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected union or variant members"))?;
        let mut mode = if keyword == "denseunion" || is_variant {
            UnionMode::Dense
        } else {
            UnionMode::Sparse
        };
        if self.peek_union_mode(close) {
            let position = self.current_position();
            mode = self.parse_union_mode()?;
            if is_variant && mode == UnionMode::Sparse {
                return Err(self.error_at(position, "variant layout must be dense"));
            }
            if self.peek_symbol() != Some(close) {
                self.expect_separator("expected ',' after union mode")?;
            }
        }

        let collection_close = if self.peek_symbol() == Some('[') {
            self.index += 1;
            Some(']')
        } else {
            None
        };
        let body_close = collection_close.unwrap_or(close);
        let mut fields = Vec::new();
        let mut next_id = 0_i16;

        while self.peek_symbol() != Some(body_close) {
            let member_close = if self.peek_symbol() == Some('(') {
                self.index += 1;
                Some(')')
            } else {
                None
            };

            let type_id = if let Some(value) = self.peek_integer() {
                let position = self.current_position();
                self.index += 1;
                let id = i8::try_from(value)
                    .map_err(|_| self.error_at(position, "union type id must fit in i8"))?;
                if is_variant && i16::from(id) != next_id {
                    return Err(
                        self.error_at(position, "variant type ids must be sequential from zero")
                    );
                }
                if !self.consume_symbol('=')
                    && !self.consume_symbol(':')
                    && !self.consume_symbol(',')
                {
                    return Err(self.error_here("expected '=', ':', or ',' after union type id"));
                }
                id
            } else {
                i8::try_from(next_id).map_err(|_| {
                    self.error_here(if is_variant {
                        "a variant cannot contain more than 128 members"
                    } else {
                        "a union cannot contain more than 128 members"
                    })
                })?
            };
            next_id = i16::from(type_id) + 1;
            let field =
                self.parse_field_or_type(&format_smolstr!("member_{type_id}"), true, depth)?;
            if let Some(member_close) = member_close {
                self.expect_symbol(member_close)?;
            }
            fields.push((type_id, field));

            if self.peek_symbol() == Some(body_close) {
                break;
            }
            self.expect_separator("expected ',' between union members")?;
        }
        self.expect_symbol(body_close)?;

        if collection_close.is_some() {
            if self.consume_separator() {
                let position = self.current_position();
                mode = self.parse_union_mode()?;
                if is_variant && mode == UnionMode::Sparse {
                    return Err(self.error_at(position, "variant layout must be dense"));
                }
            }
            self.expect_symbol(close)?;
        }
        if is_variant {
            DataType::dense_union(fields.into_iter().map(|(_, field)| field))
        } else {
            DataType::union(fields, mode)
        }
    }

    fn parse_union_mode(&mut self) -> Result<UnionMode> {
        let value = self.parse_text("union mode")?;
        match normalized(&value).as_str() {
            "dense" => Ok(UnionMode::Dense),
            "sparse" => Ok(UnionMode::Sparse),
            _ => Err(self.error_here("union mode must be dense or sparse")),
        }
    }

    fn parse_decimal_parameters(&mut self, default_precision: u8) -> Result<(u8, i8)> {
        let Some(close) = self.consume_opening() else {
            return Ok((default_precision, 0));
        };
        self.consume_label("precision");
        let position = self.current_position();
        let precision_value = self.parse_integer("decimal precision")?;
        let precision = u8::try_from(precision_value)
            .map_err(|_| self.error_at(position, "decimal precision must fit in u8"))?;
        let mut scale = 0_i8;
        if self.consume_separator() {
            self.consume_label("scale");
            let position = self.current_position();
            let scale_value = self.parse_integer("decimal scale")?;
            scale = i8::try_from(scale_value)
                .map_err(|_| self.error_at(position, "decimal scale must fit in i8"))?;
        }
        self.expect_symbol(close)?;
        Ok((precision, scale))
    }

    fn parse_single_i32_parameter(&mut self, label: &str) -> Result<i32> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here(format_smolstr!("expected {label}")))?;
        let value = self.parse_i32(label)?;
        self.expect_symbol(close)?;
        Ok(value)
    }

    fn ignore_optional_length(&mut self) -> Result<()> {
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

    fn parse_field_or_type(
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
        let data_type = self.parse_type(depth)?;
        let nullable = self.parse_nullability(default_nullable)?;
        Ok(Field::new(default_name, data_type, nullable))
    }

    fn parse_named_field(&mut self, depth: usize) -> Result<Field> {
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
        let data_type = self.parse_type(depth)?;
        let nullable = self.parse_nullability(true)?;
        Ok(Field::new(name, data_type, nullable))
    }

    fn parse_explicit_field(&mut self, depth: usize, default_name: Option<&str>) -> Result<Field> {
        self.expect_word("field")?;
        if self.peek_symbol() == Some('{') {
            return self.parse_arrow_field(depth, default_name);
        }
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected field(...)"))?;
        let name = self.parse_text("field name")?;
        self.expect_separator("expected datatype after field name")?;
        let data_type = self.parse_type(depth)?;
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
        let mut field = Field::from_parts(name, data_type, nullable, metadata)?;
        if dictionary_id.is_some() || dictionary_is_ordered.is_some() {
            field.set_dictionary_options(
                dictionary_id.unwrap_or_default(),
                dictionary_is_ordered.unwrap_or_default(),
            )?;
        }
        Ok(field)
    }

    fn parse_arrow_field(&mut self, depth: usize, default_name: Option<&str>) -> Result<Field> {
        self.expect_symbol('{')?;
        let mut name = None;
        let mut data_type = None;
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
                "datatype" | "type" => {
                    if data_type.is_some() {
                        return Err(self.error_here("duplicate field datatype"));
                    }
                    data_type = Some(self.parse_type(depth)?);
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
        let data_type =
            data_type.ok_or_else(|| self.error_here("Arrow field is missing data_type"))?;
        // Arrow's Debug formatter omits `nullable` when it is false.
        let mut field = Field::from_parts(name, data_type, nullable.unwrap_or(false), metadata)?;
        if dictionary_id.is_some() || dictionary_is_ordered.is_some() {
            field.set_dictionary_options(
                dictionary_id.unwrap_or_default(),
                dictionary_is_ordered.unwrap_or_default(),
            )?;
        }
        Ok(field)
    }

    fn parse_metadata(&mut self) -> Result<Vec<(SmolStr, SmolStr)>> {
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

    fn parse_nullability(&mut self, default: bool) -> Result<bool> {
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

    fn parse_bool(&mut self, label: &str) -> Result<bool> {
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

    fn parse_i32(&mut self, label: &str) -> Result<i32> {
        let position = self.current_position();
        let value = self.parse_integer(label)?;
        i32::try_from(value).map_err(|_| {
            self.error_at(
                position,
                format_smolstr!("{label} must fit in a signed 32-bit integer"),
            )
        })
    }

    fn parse_integer(&mut self, label: &str) -> Result<i64> {
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

    fn parse_text(&mut self, label: &str) -> Result<SmolStr> {
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
    fn parse_timezone(&mut self) -> Result<crate::Timezone> {
        let start = self.current_position();
        let text = self.parse_text("timezone")?;
        crate::Timezone::from_smol_str(text)
            .map_err(|error| self.error_at(start, format_smolstr!("{error}")))
    }

    fn skip_value(&mut self) -> Result<()> {
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

    fn looks_like_named_field(&self) -> bool {
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

    fn consume_label(&mut self, label: &str) -> bool {
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

    fn expect_separator(&mut self, reason: &str) -> Result<()> {
        if self.consume_separator() {
            Ok(())
        } else {
            Err(self.error_here(reason))
        }
    }

    fn consume_separator(&mut self) -> bool {
        self.consume_symbol(',') || self.consume_symbol(';')
    }

    fn consume_opening(&mut self) -> Option<char> {
        let close = matching_close(self.peek_symbol()?)?;
        self.index += 1;
        Some(close)
    }

    fn peek_opening(&self) -> Option<char> {
        matching_close(self.peek_symbol()?)
    }

    fn expect_symbol(&mut self, expected: char) -> Result<()> {
        if self.consume_symbol(expected) {
            Ok(())
        } else {
            Err(self.error_here(format_smolstr!("expected {expected:?}")))
        }
    }

    fn consume_symbol(&mut self, expected: char) -> bool {
        if self.peek_symbol() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn peek_symbol(&self) -> Option<char> {
        match self.tokens.get(self.index).map(|token| &token.kind) {
            Some(TokenKind::Symbol(symbol)) => Some(*symbol),
            _ => None,
        }
    }

    fn expect_word(&mut self, expected: &str) -> Result<()> {
        if self.consume_word(expected) {
            Ok(())
        } else {
            Err(self.error_here(format_smolstr!("expected {expected:?}")))
        }
    }

    fn consume_word(&mut self, expected: &str) -> bool {
        if self.peek_word_is(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn peek_word_is(&self, expected: &str) -> bool {
        match self.tokens.get(self.index).map(|token| &token.kind) {
            Some(TokenKind::Word(value)) => normalized(value) == normalized(expected),
            _ => false,
        }
    }

    fn peek_integer(&self) -> Option<i64> {
        match self.tokens.get(self.index).map(|token| &token.kind) {
            Some(TokenKind::Integer(value)) => Some(*value),
            _ => None,
        }
    }

    fn peek_union_mode(&self, close: char) -> bool {
        (self.peek_word_is("dense") || self.peek_word_is("sparse"))
            && matches!(
                self.tokens.get(self.index + 1).map(|token| &token.kind),
                Some(TokenKind::Symbol(symbol))
                    if *symbol == close || matches!(*symbol, ',' | ';')
            )
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.index)?.clone();
        self.index += 1;
        Some(token)
    }

    fn is_done(&self) -> bool {
        self.index == self.tokens.len()
    }

    fn current_position(&self) -> usize {
        self.tokens
            .get(self.index)
            .map_or(self.source.len(), |token| token.start)
    }

    fn check_depth(&self, depth: usize) -> Result<()> {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            Err(self.error_here(format_smolstr!(
                "datatype nesting exceeds the limit of {}",
                DataType::PARSE_RECURSION_LIMIT
            )))
        } else {
            Ok(())
        }
    }

    fn error_here(&self, reason: impl Into<SmolStr>) -> Error {
        self.error_at(self.current_position(), reason)
    }

    fn error_at(&self, position: usize, reason: impl Into<SmolStr>) -> Error {
        parse_error(self.source, position, reason)
    }
}

#[derive(Clone, Copy)]
enum ListKind {
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

fn is_closing_or_separator(symbol: char) -> bool {
    matches!(symbol, '>' | ')' | ']' | '}' | ',' | ';')
}

pub(super) fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn precision_to_unit(precision: i64, position: usize) -> Result<TimeUnit> {
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

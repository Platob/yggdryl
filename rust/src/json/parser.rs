//! Exact, byte-oriented JSON tree decoding.

use std::borrow::Cow;
use std::collections::HashSet;
use std::str;

use crate::text::Limits;
use crate::text::position::line_column_to_byte_offset;
use crate::text::wire::RawValue;
use crate::{Error, Result};

pub(super) fn parse(input: &[u8], limits: Limits) -> Result<RawValue> {
    let mut cursor = Cursor::new(input, limits);
    cursor.begin_document()?;
    let value = raw_value(&mut cursor, 0)?;
    cursor.end_document()?;
    Ok(value)
}

/// The syntax tree of the value the cursor stands before.
fn raw_value(cursor: &mut Cursor<'_>, depth: usize) -> Result<RawValue> {
    Ok(match cursor.value(depth)? {
        Token::Null => RawValue::Null,
        Token::Bool(value) => RawValue::Bool(value),
        Token::Number(number) => number.into(),
        Token::String(text) => RawValue::String(text.into_owned()),
        Token::Array => {
            let mut values = Vec::new();
            let mut first = true;
            while cursor.next_item(first)? {
                first = false;
                values.push(raw_value(cursor, depth + 1)?);
            }
            RawValue::Sequence(values)
        }
        Token::Object => {
            let mut entries = Vec::new();
            let mut key_positions = Vec::new();
            let mut first = true;
            while let Some((position, key)) = cursor.next_key(first)? {
                first = false;
                let value = raw_value(cursor, depth + 1)?;
                entries.push((RawValue::String(key.into_owned()), value));
                key_positions.push(position);
            }
            if !entries.is_empty() {
                validate_unique_keys(&entries, &key_positions)?;
            }
            RawValue::Mapping(entries)
        }
    })
}

/// One JSON number, typed as the grammar reads it: a non-negative integer
/// is unsigned, a negative one signed, each at 64 bits before 128, and a
/// fraction, an exponent or `-0` a finite float.
#[derive(Clone, Copy, Debug)]
pub(super) enum Number {
    UInt64(u64),
    Int64(i64),
    UInt128(u128),
    Int128(i128),
    Float(f64),
}

impl From<Number> for RawValue {
    fn from(number: Number) -> Self {
        match number {
            Number::UInt64(value) => Self::UInt64(value),
            Number::Int64(value) => Self::Int64(value),
            Number::UInt128(value) => Self::UInt128(value),
            Number::Int128(value) => Self::Int128(value),
            Number::Float(value) => Self::Float(value),
        }
    }
}

/// What the cursor read where a value stands: a leaf whole, or the opener
/// of a container whose items or entries follow.
pub(super) enum Token<'a> {
    Null,
    Bool(bool),
    Number(Number),
    /// A string's characters: borrowed where the document holds them as
    /// they are, decoded where it escapes them.
    String(Cow<'a, str>),
    /// `[` was consumed; [`Cursor::next_item`] walks the items.
    Array,
    /// `{` was consumed; [`Cursor::next_key`] walks the entries.
    Object,
}

/// The one JSON grammar in the crate, read value by value at the caller's
/// pace: the tree [`parse`] builds and a field-directed reader both drive
/// it, so they accept the same documents and refuse the rest in the same
/// words. The node count and the nesting depth are the document's, however
/// a caller walks it; a duplicate key is the caller's to judge, at the
/// close of the object that holds it.
pub(super) struct Cursor<'a> {
    input: &'a [u8],
    position: usize,
    limits: Limits,
    nodes: usize,
}

impl<'a> Cursor<'a> {
    pub(super) const fn new(input: &'a [u8], limits: Limits) -> Self {
        Self {
            input,
            position: 0,
            limits,
            nodes: 0,
        }
    }

    /// Refuse a document holding no value at all.
    pub(super) fn begin_document(&mut self) -> Result<()> {
        self.skip_whitespace();
        if self.position == self.input.len() {
            return Err(codec_error(self.position, "expected one JSON value"));
        }
        Ok(())
    }

    /// Refuse anything but whitespace after the document's one value.
    pub(super) fn end_document(mut self) -> Result<()> {
        self.skip_whitespace();
        if self.position != self.input.len() {
            return Err(codec_error(
                self.position,
                "trailing characters after JSON value",
            ));
        }
        Ok(())
    }

    /// Read the value standing at `depth`: a container's opener is
    /// consumed once its depth is admitted.
    pub(super) fn value(&mut self, depth: usize) -> Result<Token<'a>> {
        self.skip_whitespace();
        let position = self.position;
        self.observe_node(position)?;
        match self.peek() {
            Some(b'n') => self.parse_literal(b"null", Token::Null),
            Some(b't') => self.parse_literal(b"true", Token::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", Token::Bool(false)),
            Some(b'"') => self.parse_string().map(Token::String),
            Some(b'[') => {
                self.observe_container_depth(depth, position)?;
                self.position += 1;
                Ok(Token::Array)
            }
            Some(b'{') => {
                self.observe_container_depth(depth, position)?;
                self.position += 1;
                Ok(Token::Object)
            }
            Some(b'-' | b'0'..=b'9') => self.parse_number().map(Token::Number),
            Some(_) => Err(codec_error(position, "expected a JSON value")),
            None => Err(codec_error(position, "unexpected end of JSON input")),
        }
    }

    /// Whether another item of the array being walked follows: `first`
    /// right after its `[`, the punctuation before it consumed.
    pub(super) fn next_item(&mut self, first: bool) -> Result<bool> {
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(false);
        }
        if !first {
            self.expect(b',', "expected ',' or ']' after JSON array value")?;
            self.skip_whitespace();
            if self.peek() == Some(b']') {
                return Err(codec_error(self.position, "trailing comma in JSON array"));
            }
        }
        Ok(true)
    }

    /// The next key of the object being walked and the byte it opens at,
    /// its colon consumed, or `None` at the object's `}`: `first` right
    /// after its `{`.
    pub(super) fn next_key(&mut self, first: bool) -> Result<Option<(usize, Cow<'a, str>)>> {
        self.skip_whitespace();
        if self.consume(b'}') {
            return Ok(None);
        }
        if !first {
            self.expect(b',', "expected ',' or '}' after JSON object value")?;
            self.skip_whitespace();
            if self.peek() == Some(b'}') {
                return Err(codec_error(self.position, "trailing comma in JSON object"));
            }
        }
        let key_position = self.position;
        if self.peek() != Some(b'"') {
            return Err(codec_error(
                key_position,
                "JSON object key must be a string",
            ));
        }
        self.observe_node(key_position)?;
        let key = self.parse_string()?;
        self.skip_whitespace();
        self.expect(b':', "expected ':' after JSON object key")?;
        Ok(Some((key_position, key)))
    }

    fn observe_node(&mut self, position: usize) -> Result<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes() {
            Err(codec_error(position, "decoded node limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn observe_container_depth(&self, depth: usize, position: usize) -> Result<()> {
        let depth = depth.saturating_add(1);
        if depth > super::MAX_PARSER_DEPTH {
            Err(codec_error(
                position,
                "JSON nesting exceeds the parser hard limit of 384",
            ))
        } else if depth > self.limits.max_depth() {
            Err(codec_error(position, "nesting depth limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn parse_literal(&mut self, expected: &[u8], value: Token<'a>) -> Result<Token<'a>> {
        let start = self.position;
        if self.input.get(start..start.saturating_add(expected.len())) == Some(expected) {
            self.position += expected.len();
            Ok(value)
        } else {
            Err(codec_error(start, "invalid JSON literal"))
        }
    }

    fn parse_string(&mut self) -> Result<Cow<'a, str>> {
        let start = self.position;
        // Most strings hold no escape and no control character: one pass
        // finds where such a string ends, and its bytes are its characters
        // once they are UTF-8. Anything else is walked byte by byte below.
        let input: &'a [u8] = self.input;
        let body = &input[start + 1..];
        if let Some(end) = body
            .iter()
            .position(|&byte| byte == b'"' || byte == b'\\' || byte < 0x20)
            && body[end] == b'"'
            && let Ok(text) = str::from_utf8(&body[..end])
        {
            self.position = start + end + 2;
            return Ok(Cow::Borrowed(text));
        }
        self.position += 1;
        let mut escaped = false;
        // Whether every byte so far is one a string holds as it is: no
        // escape to decode and no control character to refuse.
        let mut verbatim = true;
        while let Some(byte) = self.peek() {
            self.position += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
                verbatim = false;
            } else if byte == b'"' {
                let input: &'a [u8] = self.input;
                let quoted = &input[start..self.position];
                // Nothing to decode: the characters are the bytes, once they
                // are UTF-8. Anything else is serde_json's to read, and to
                // refuse in its own words.
                if verbatim && let Ok(text) = str::from_utf8(&quoted[1..quoted.len() - 1]) {
                    return Ok(Cow::Borrowed(text));
                }
                return serde_json::from_slice::<String>(quoted)
                    .map(Cow::Owned)
                    .map_err(|error| serde_error(quoted, start, error));
            } else if byte < 0x20 {
                verbatim = false;
            }
        }
        Err(codec_error(start, "unterminated JSON string"))
    }

    fn parse_number(&mut self) -> Result<Number> {
        let start = self.position;
        let negative = self.consume(b'-');
        match self.peek() {
            Some(b'0') => {
                self.position += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(codec_error(self.position, "leading zero in JSON number"));
                }
            }
            Some(b'1'..=b'9') => {
                self.position += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.position += 1;
                }
            }
            _ => return Err(codec_error(self.position, "invalid JSON number")),
        }

        let mut floating = false;
        if self.consume(b'.') {
            floating = true;
            let digits = self.position;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
            if self.position == digits {
                return Err(codec_error(self.position, "JSON fraction requires a digit"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            floating = true;
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            let digits = self.position;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
            if self.position == digits {
                return Err(codec_error(self.position, "JSON exponent requires a digit"));
            }
        }

        let digits = &self.input[start + usize::from(negative)..self.position];
        // Eighteen digits are an `i64` whatever their sign, and nineteen a
        // `u64`: read straight off the bytes the grammar just admitted,
        // which spell exactly what `str::parse` would read them as.
        if !floating && (digits.len() <= 18 || !negative && digits.len() == 19) {
            let magnitude = digits
                .iter()
                .fold(0_u64, |value, digit| value * 10 + u64::from(digit - b'0'));
            return Ok(match (negative, magnitude) {
                (true, 0) => Number::Float(-0.0),
                (true, magnitude) => Number::Int64(
                    0_i64 - i64::try_from(magnitude).expect("eighteen digits fit an i64"),
                ),
                (false, magnitude) => Number::UInt64(magnitude),
            });
        }
        let spelling = str::from_utf8(&self.input[start..self.position])
            .map_err(|_| codec_error(start, "JSON number is not ASCII"))?;
        if floating {
            let value = spelling
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| codec_error(start, "JSON number is outside the finite f64 range"))?;
            return Ok(Number::Float(value));
        }
        if spelling == "-0" {
            return Ok(Number::Float(-0.0));
        }
        if negative {
            if let Ok(value) = spelling.parse::<i64>() {
                return Ok(Number::Int64(value));
            }
            return spelling.parse::<i128>().map(Number::Int128).map_err(|_| {
                codec_error(start, "JSON integer is outside the signed 128-bit range")
            });
        }
        if let Ok(value) = spelling.parse::<u64>() {
            return Ok(Number::UInt64(value));
        }
        spelling
            .parse::<u128>()
            .map(Number::UInt128)
            .map_err(|_| codec_error(start, "JSON integer is outside the unsigned 128-bit range"))
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: u8, reason: &'static str) -> Result<()> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(codec_error(self.position, reason))
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }
}

fn validate_unique_keys(entries: &[(RawValue, RawValue)], key_positions: &[usize]) -> Result<()> {
    if entries.len() <= 16 {
        for (index, (key, _)) in entries.iter().enumerate() {
            let RawValue::String(key) = key else {
                return Err(codec_error(
                    key_positions.get(index).copied().unwrap_or_default(),
                    "JSON object key must be a string",
                ));
            };
            if entries[..index].iter().any(
                |(existing, _)| matches!(existing, RawValue::String(existing) if existing == key),
            ) {
                return Err(codec_error(
                    key_positions.get(index).copied().unwrap_or_default(),
                    "JSON object contains a duplicate key",
                ));
            }
        }
        return Ok(());
    }

    let mut seen = HashSet::with_capacity(entries.len());
    for (index, (key, _)) in entries.iter().enumerate() {
        let RawValue::String(key) = key else {
            return Err(codec_error(
                key_positions.get(index).copied().unwrap_or_default(),
                "JSON object key must be a string",
            ));
        };
        if !seen.insert(key.as_str()) {
            return Err(codec_error(
                key_positions.get(index).copied().unwrap_or_default(),
                "JSON object contains a duplicate key",
            ));
        }
    }
    Ok(())
}

fn serde_error(input: &[u8], base: usize, error: serde_json::Error) -> Error {
    Error::Codec {
        format: "json",
        position: base.saturating_add(line_column_to_byte_offset(
            input,
            error.line(),
            error.column(),
        )),
        reason: error.to_string().into(),
    }
}

fn codec_error(position: usize, reason: &'static str) -> Error {
    Error::Codec {
        format: "json",
        position,
        reason: reason.into(),
    }
}

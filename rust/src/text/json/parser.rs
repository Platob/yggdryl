//! Exact, byte-oriented JSON tree decoding.

use std::collections::HashSet;
use std::str;

use crate::text::Limits;
use crate::text::position::line_column_to_byte_offset;
use crate::text::wire::RawValue;
use crate::{Error, Result};

pub(super) fn parse(input: &[u8], limits: Limits) -> Result<RawValue> {
    Parser::new(input, limits).parse_document()
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
    limits: Limits,
    nodes: usize,
}

impl<'a> Parser<'a> {
    const fn new(input: &'a [u8], limits: Limits) -> Self {
        Self {
            input,
            position: 0,
            limits,
            nodes: 0,
        }
    }

    fn parse_document(mut self) -> Result<RawValue> {
        self.skip_whitespace();
        if self.position == self.input.len() {
            return Err(codec_error(self.position, "expected one JSON value"));
        }
        let value = self.parse_value(0)?;
        self.skip_whitespace();
        if self.position != self.input.len() {
            return Err(codec_error(
                self.position,
                "trailing characters after JSON value",
            ));
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<RawValue> {
        self.skip_whitespace();
        let position = self.position;
        self.observe_node(position)?;
        match self.peek() {
            Some(b'n') => self.parse_literal(b"null", RawValue::Null),
            Some(b't') => self.parse_literal(b"true", RawValue::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", RawValue::Bool(false)),
            Some(b'"') => self.parse_string().map(RawValue::String),
            Some(b'[') => self.parse_sequence(depth, position),
            Some(b'{') => self.parse_mapping(depth, position),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => Err(codec_error(position, "expected a JSON value")),
            None => Err(codec_error(position, "unexpected end of JSON input")),
        }
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

    fn parse_literal(&mut self, expected: &[u8], value: RawValue) -> Result<RawValue> {
        let start = self.position;
        if self.input.get(start..start.saturating_add(expected.len())) == Some(expected) {
            self.position += expected.len();
            Ok(value)
        } else {
            Err(codec_error(start, "invalid JSON literal"))
        }
    }

    fn parse_sequence(&mut self, depth: usize, position: usize) -> Result<RawValue> {
        self.observe_container_depth(depth, position)?;
        self.position += 1;
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(RawValue::Sequence(Vec::new()));
        }
        let mut values = Vec::new();
        loop {
            values.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            if self.consume(b']') {
                return Ok(RawValue::Sequence(values));
            }
            self.expect(b',', "expected ',' or ']' after JSON array value")?;
            self.skip_whitespace();
            if self.peek() == Some(b']') {
                return Err(codec_error(self.position, "trailing comma in JSON array"));
            }
        }
    }

    fn parse_mapping(&mut self, depth: usize, position: usize) -> Result<RawValue> {
        self.observe_container_depth(depth, position)?;
        self.position += 1;
        self.skip_whitespace();
        if self.consume(b'}') {
            return Ok(RawValue::Mapping(Vec::new()));
        }
        let mut entries = Vec::new();
        let mut key_positions = Vec::new();
        loop {
            let key_position = self.position;
            if self.peek() != Some(b'"') {
                return Err(codec_error(
                    key_position,
                    "JSON object key must be a string",
                ));
            }
            self.observe_node(key_position)?;
            let key = RawValue::String(self.parse_string()?);
            self.skip_whitespace();
            self.expect(b':', "expected ':' after JSON object key")?;
            let value = self.parse_value(depth + 1)?;
            entries.push((key, value));
            key_positions.push(key_position);
            self.skip_whitespace();
            if self.consume(b'}') {
                validate_unique_keys(&entries, &key_positions)?;
                return Ok(RawValue::Mapping(entries));
            }
            self.expect(b',', "expected ',' or '}' after JSON object value")?;
            self.skip_whitespace();
            if self.peek() == Some(b'}') {
                return Err(codec_error(self.position, "trailing comma in JSON object"));
            }
        }
    }

    fn parse_string(&mut self) -> Result<String> {
        let start = self.position;
        self.position += 1;
        let mut escaped = false;
        while let Some(byte) = self.peek() {
            self.position += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return serde_json::from_slice(&self.input[start..self.position])
                    .map_err(|error| serde_error(&self.input[start..self.position], start, error));
            }
        }
        Err(codec_error(start, "unterminated JSON string"))
    }

    fn parse_number(&mut self) -> Result<RawValue> {
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

        let spelling = str::from_utf8(&self.input[start..self.position])
            .map_err(|_| codec_error(start, "JSON number is not ASCII"))?;
        if floating {
            let value = spelling
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| codec_error(start, "JSON number is outside the finite f64 range"))?;
            return Ok(RawValue::Float(value));
        }
        if spelling == "-0" {
            return Ok(RawValue::Float(-0.0));
        }
        if negative {
            if let Ok(value) = spelling.parse::<i64>() {
                return Ok(RawValue::I64(value));
            }
            return spelling.parse::<i128>().map(RawValue::I128).map_err(|_| {
                codec_error(start, "JSON integer is outside the signed 128-bit range")
            });
        }
        if let Ok(value) = spelling.parse::<u64>() {
            return Ok(RawValue::U64(value));
        }
        spelling
            .parse::<u128>()
            .map(RawValue::U128)
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

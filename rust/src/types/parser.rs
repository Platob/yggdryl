//! Canonical display and recursive Arrow, SQL, Hive, and Spark parsing.

use crate::types::enums::EnumType;
use crate::types::sequence::SequenceType;
use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use super::{DataType, TimeUnit};
use crate::UnionMode;
use crate::types::BytesType;
use crate::types::DecimalType;
use crate::types::StringType;
use crate::types::UuidType;
use crate::{EdgeAlgorithm, Error, Field, Result};

/// Recursive field grammar and FromStr implementation.
mod field {

    use std::borrow::Cow;
    use std::str::FromStr;

    use smol_str::SmolStr;

    use crate::{DataType, Error, Field, Metadata, Result};

    impl Field {
        /// Parses canonical, Arrow-like, SQL, Hive, or Spark field syntax.
        #[allow(clippy::should_implement_trait)]
        pub fn from_str(value: &str) -> Result<Self> {
            value.parse()
        }
    }

    impl FromStr for Field {
        type Err = Error;

        fn from_str(value: &str) -> Result<Self> {
            parse_field(value)
        }
    }

    fn parse_field(input: &str) -> Result<Field> {
        let stripped = strip_optional_wrappers(input)?;
        let base = match &stripped {
            Cow::Borrowed(value) => subslice_offset(input, value),
            Cow::Owned(_) => 0,
        };
        let value = stripped.trim();
        if value.is_empty() {
            return Err(field_parse_error(base, "expected a field expression"));
        }

        if let Some(body) =
            function_body(value, "field").map_err(|error| offset_error(error, base))?
        {
            let body_base = base + subslice_offset(value, body);
            return parse_canonical_field(body).map_err(|error| offset_error(error, body_base));
        }
        if value
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("field"))
        {
            let rest = value[5..].trim_start();
            if rest.starts_with('{') {
                let rest_base = base + subslice_offset(value, rest);
                let body = enclosed_body(rest, '{', '}')
                    .map_err(|error| offset_error(error, rest_base))?;
                let body_base = base + subslice_offset(value, body);
                return parse_arrow_field(body).map_err(|error| offset_error(error, body_base));
            }
        }
        parse_shorthand_field(value).map_err(|error| offset_error(error, base))
    }

    fn subslice_offset(parent: &str, child: &str) -> usize {
        (child.as_ptr() as usize).saturating_sub(parent.as_ptr() as usize)
    }

    fn parse_canonical_field(body: &str) -> Result<Field> {
        let values = split_top_level(body, ',')?;
        if values.len() < 2 {
            return Err(field_parse_error(
                0,
                "field(...) requires a name and datatype",
            ));
        }
        let name = parse_string(values[0].1).map_err(|error| offset_error(error, values[0].0))?;
        let dtype = parse_dtype(values[1].1, values[1].0)?;
        let mut nullable = true;
        let mut dictionary_id = 0;
        let mut dictionary_is_ordered = false;
        let mut metadata = Metadata::new();
        let mut saw_nullable = false;
        let mut saw_dictionary_id = false;
        let mut saw_dictionary_is_ordered = false;
        let mut saw_metadata = false;

        for (offset, argument) in values.into_iter().skip(2) {
            let (key, raw_value) = split_assignment(argument, offset)?;
            if key.eq_ignore_ascii_case("nullable") {
                if saw_nullable {
                    return Err(field_parse_error(offset, "duplicate nullable argument"));
                }
                nullable = parse_bool(raw_value, offset)?;
                saw_nullable = true;
            } else if key.eq_ignore_ascii_case("metadata") {
                if saw_metadata {
                    return Err(field_parse_error(offset, "duplicate metadata argument"));
                }
                metadata = parse_metadata(raw_value, offset)?;
                saw_metadata = true;
            } else if key.eq_ignore_ascii_case("dictionary_id")
                || key.eq_ignore_ascii_case("dict_id")
            {
                if saw_dictionary_id {
                    return Err(field_parse_error(
                        offset,
                        "duplicate dictionary id argument",
                    ));
                }
                dictionary_id = parse_i64(raw_value, offset)?;
                saw_dictionary_id = true;
            } else if key.eq_ignore_ascii_case("dictionary_is_ordered")
                || key.eq_ignore_ascii_case("dict_is_ordered")
            {
                if saw_dictionary_is_ordered {
                    return Err(field_parse_error(
                        offset,
                        "duplicate dictionary ordering argument",
                    ));
                }
                dictionary_is_ordered = parse_bool(raw_value, offset)?;
                saw_dictionary_is_ordered = true;
            } else {
                return Err(field_parse_error(
                    offset,
                    format!("unknown field argument {key:?}"),
                ));
            }
        }

        let mut field = Field::new_with_metadata(name, dtype, nullable, metadata);
        if dictionary_id != 0 || dictionary_is_ordered {
            field.set_dictionary_options(dictionary_id, dictionary_is_ordered)?;
        }
        field.validate()?;
        Ok(field)
    }

    fn parse_arrow_field(body: &str) -> Result<Field> {
        let mut name = None;
        let mut dtype = None;
        let mut nullable = None;
        let mut dictionary_id = None;
        let mut dictionary_is_ordered = None;
        let mut metadata = Metadata::new();
        let mut saw_metadata = false;

        for (index, (offset, member)) in split_top_level(body, ',')?.into_iter().enumerate() {
            if member.trim().is_empty() {
                continue;
            }
            if index == 0 {
                if let Some((display_name, display_type)) =
                    split_arrow_display_field(member, offset)?
                {
                    name = Some(display_name);
                    let (display_type, is_nullable) = strip_nullable_prefix(display_type);
                    nullable = Some(is_nullable);
                    let type_offset = offset + member.find(display_type).unwrap_or_default();
                    dtype = Some(parse_dtype(display_type, type_offset)?);
                    continue;
                }
            }
            if member.trim().eq_ignore_ascii_case("dict_is_ordered") {
                if dictionary_is_ordered.is_some() {
                    return Err(field_parse_error(
                        offset,
                        "duplicate dictionary ordering flag",
                    ));
                }
                dictionary_is_ordered = Some(true);
                continue;
            }
            let (key, value) = split_key_value(member, offset, ':')?;
            let value_offset = offset + member.find(value).unwrap_or_default();
            // The same normalization the token grammar uses, so both readings of
            // this one form answer to the same key set rather than two.
            let key_name = crate::types::parser::normalized(key);
            match key_name.as_str() {
                "name" => {
                    if name.is_some() {
                        return Err(field_parse_error(offset, "duplicate field name"));
                    }
                    name = Some(
                        parse_string(value).map_err(|error| offset_error(error, value_offset))?,
                    );
                }
                "datatype" | "dtype" | "type" => {
                    if dtype.is_some() {
                        return Err(field_parse_error(offset, "duplicate field datatype"));
                    }
                    dtype = Some(parse_dtype(value, value_offset)?);
                }
                "nullable" | "isnullable" => {
                    if nullable.is_some() {
                        return Err(field_parse_error(offset, "duplicate nullability"));
                    }
                    nullable = Some(parse_bool(value, offset)?);
                }
                "metadata" => {
                    if saw_metadata {
                        return Err(field_parse_error(offset, "duplicate field metadata"));
                    }
                    metadata = parse_metadata(value, value_offset)?;
                    saw_metadata = true;
                }
                "dictionaryid" | "dictid" => {
                    if dictionary_id.is_some() {
                        return Err(field_parse_error(offset, "duplicate dictionary id"));
                    }
                    dictionary_id = Some(parse_i64(value, offset)?);
                }
                "dictionaryisordered" | "dictisordered" => {
                    if dictionary_is_ordered.is_some() {
                        return Err(field_parse_error(
                            offset,
                            "duplicate dictionary ordering flag",
                        ));
                    }
                    dictionary_is_ordered = Some(parse_bool(value, offset)?);
                }
                _ => {
                    return Err(field_parse_error(
                        offset,
                        format!("unknown Arrow field member {key:?}"),
                    ));
                }
            }
        }

        let mut field = Field::new_with_metadata(
            name.ok_or_else(|| field_parse_error(0, "Arrow field is missing name"))?,
            dtype.ok_or_else(|| field_parse_error(0, "Arrow field is missing dtype"))?,
            // Arrow's Debug implementation omits `nullable` when it is false.
            nullable.unwrap_or(false),
            metadata,
        );
        let dictionary_id = dictionary_id.unwrap_or_default();
        let dictionary_is_ordered = dictionary_is_ordered.unwrap_or_default();
        if dictionary_id != 0 || dictionary_is_ordered {
            field.set_dictionary_options(dictionary_id, dictionary_is_ordered)?;
        }
        field.validate()?;
        Ok(field)
    }

    fn parse_shorthand_field(value: &str) -> Result<Field> {
        let (value, nullable) = parse_nullability_suffix(value)?;
        if let Some((name, dtype, type_offset)) = split_quoted_field_name(value)? {
            let name = parse_identifier(name)?;
            if dtype.trim().is_empty() {
                return Err(field_parse_error(value.len(), "missing field datatype"));
            }
            let field = Field::new(name, parse_dtype(dtype, type_offset)?, nullable);
            field.validate()?;
            return Ok(field);
        }
        let parts = split_top_level(value, ':')?;
        let (name, dtype, type_offset) = if parts.len() == 2 {
            (parts[0].1, parts[1].1, parts[1].0)
        } else if parts.len() > 2 {
            return Err(field_parse_error(parts[2].0, "unexpected top-level colon"));
        } else {
            let index = top_level_whitespace(value)?.ok_or_else(|| {
                field_parse_error(0, "expected `name: datatype` or `name datatype`")
            })?;
            let dtype = value[index..].trim_start();
            (
                &value[..index],
                dtype,
                index + value[index..].len().saturating_sub(dtype.len()),
            )
        };
        let name = parse_identifier(name)?;
        if dtype.trim().is_empty() {
            return Err(field_parse_error(value.len(), "missing field datatype"));
        }
        let field = Field::new(name, parse_dtype(dtype, type_offset)?, nullable);
        field.validate()?;
        Ok(field)
    }

    fn split_quoted_field_name(value: &str) -> Result<Option<(&str, &str, usize)>> {
        let value = value.trim_start();
        let Some(open) = value.chars().next() else {
            return Ok(None);
        };
        let close = match open {
            '\'' | '"' | '`' => open,
            '[' => ']',
            _ => return Ok(None),
        };
        let mut characters = value[open.len_utf8()..].char_indices().peekable();
        let mut escaped = false;
        let mut end = None;
        while let Some((relative, character)) = characters.next() {
            if escaped {
                escaped = false;
                continue;
            }
            if character == '\\' && open != '[' {
                escaped = true;
                continue;
            }
            if character != close {
                continue;
            }
            if characters.peek().is_some_and(|(_, next)| *next == close) {
                characters.next();
                continue;
            }
            end = Some(open.len_utf8() + relative + close.len_utf8());
            break;
        }
        let end = end.ok_or_else(|| field_parse_error(value.len(), "unterminated field name"))?;
        let name = &value[..end];
        let suffix = &value[end..];
        let trimmed = suffix.trim_start();
        let whitespace = suffix.len() - trimmed.len();
        if let Some(dtype) = trimmed.strip_prefix(':') {
            let dtype = dtype.trim_start();
            let offset = end + whitespace + 1 + trimmed[1..].len().saturating_sub(dtype.len());
            return Ok(Some((name, dtype, offset)));
        }
        if whitespace == 0 {
            return Err(field_parse_error(
                end,
                "expected `:` or whitespace after field name",
            ));
        }
        Ok(Some((name, trimmed, end + whitespace)))
    }

    fn parse_nullability_suffix(value: &str) -> Result<(&str, bool)> {
        let value = value.trim();
        if let Some(prefix) = strip_trailing_words(value, &["not", "null"]) {
            return Ok((prefix.trim_end(), false));
        }
        if let Some(prefix) = strip_trailing_words(value, &["nullable"]) {
            return Ok((prefix.trim_end(), true));
        }
        if let Some(prefix) = strip_trailing_words(value, &["null"]) {
            return Ok((prefix.trim_end(), true));
        }
        if let Some(prefix) = value.strip_suffix('?') {
            return Ok((prefix.trim_end(), true));
        }
        if let Some(prefix) = value.strip_suffix('!') {
            return Ok((prefix.trim_end(), false));
        }
        Ok((value, true))
    }

    fn parse_metadata(value: &str, base: usize) -> Result<Metadata> {
        let value = value.trim();
        let body = enclosed_body(value, '{', '}').map_err(|_| {
            field_parse_error(base, "metadata must be an object enclosed by `{` and `}`")
        })?;
        if body.trim().is_empty() {
            return Ok(Metadata::new());
        }
        let mut entries = Vec::new();
        for (offset, member) in split_top_level(body, ',')? {
            let (key, raw_value) = split_key_value(member, base + offset, ':')?;
            let member_base = base + offset;
            let key_offset = member_base + member.find(key).unwrap_or_default();
            let value_offset = member_base + member.find(raw_value).unwrap_or_default();
            entries.push((
                parse_string(key).map_err(|error| offset_error(error, key_offset))?,
                parse_string(raw_value).map_err(|error| offset_error(error, value_offset))?,
            ));
        }
        Metadata::from_entries(entries).map_err(|error| metadata_parse_error(error, base))
    }

    fn parse_identifier(value: &str) -> Result<String> {
        let value = value.trim();
        if value.is_empty() {
            return Err(field_parse_error(0, "field name must not be empty"));
        }
        parse_string(value)
    }

    fn parse_string(value: &str) -> Result<String> {
        let value = value.trim();
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            if let Ok(parsed) = serde_json::from_str(value) {
                return Ok(parsed);
            }
            return unescape_quoted(&value[1..value.len() - 1], '"');
        }
        if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
            return unescape_quoted(&value[1..value.len() - 1], '\'');
        }
        if value.len() >= 2 && value.starts_with('`') && value.ends_with('`') {
            return unescape_quoted(&value[1..value.len() - 1], '`');
        }
        if value.len() >= 2 && value.starts_with('[') && value.ends_with(']') {
            return Ok(value[1..value.len() - 1].replace("]]", "]"));
        }
        if value.contains(['"', '\'', '`']) {
            return Err(field_parse_error(0, "unmatched quote in string"));
        }
        Ok(value.to_owned())
    }

    fn unescape_quoted(value: &str, quote: char) -> Result<String> {
        let mut output = String::with_capacity(value.len());
        let mut chars = value.chars().peekable();
        while let Some(character) = chars.next() {
            if character == quote {
                if chars.peek() == Some(&quote) {
                    chars.next();
                    output.push(quote);
                    continue;
                }
                return Err(field_parse_error(0, "unescaped quote in string"));
            }
            if character != '\\' {
                output.push(character);
                continue;
            }
            let escaped = chars
                .next()
                .ok_or_else(|| field_parse_error(value.len(), "unterminated escape"))?;
            output.push(match escaped {
                '\\' => '\\',
                '\'' => '\'',
                '"' => '"',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => {
                    return Err(field_parse_error(
                        0,
                        format!("unsupported escape `\\{escaped}`"),
                    ));
                }
            });
        }
        Ok(output)
    }

    fn strip_optional_wrappers(value: &str) -> Result<Cow<'_, str>> {
        strip_optional_wrappers_at_depth(value, 0, 0)
    }

    fn strip_optional_wrappers_at_depth(
        value: &str,
        depth: usize,
        offset: usize,
    ) -> Result<Cow<'_, str>> {
        // A wrapper is not a constructor, so this counts the wrappers stripped
        // rather than the datatypes entered: exactly the limit many are accepted.
        // The refusal names the limit and the text it stopped in, as the datatype
        // grammar's does.
        if depth > DataType::PARSE_RECURSION_LIMIT {
            return Err(field_parse_error(
                offset,
                format!(
                    "field nesting exceeds the limit of {}; near {:?}",
                    DataType::PARSE_RECURSION_LIMIT,
                    &value[..value.len().min(24)]
                ),
            ));
        }
        let value = value.trim();
        if value.len() < 2 {
            return Ok(Cow::Borrowed(value));
        }
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            let parsed = parse_string(value)?;
            return Ok(Cow::Owned(
                strip_optional_wrappers_at_depth(&parsed, depth + 1, offset)?.into_owned(),
            ));
        }
        let pair = match value.chars().next() {
            Some('(') => Some(('(', ')')),
            Some('[') => Some(('[', ']')),
            Some('{') => Some(('{', '}')),
            _ => None,
        };
        let Some((open, close)) = pair else {
            return Ok(Cow::Borrowed(value));
        };
        if closing_delimiter(value, open, close)? == value.len() - close.len_utf8() {
            return strip_optional_wrappers_at_depth(
                &value[open.len_utf8()..value.len() - close.len_utf8()],
                depth + 1,
                offset + open.len_utf8(),
            );
        }
        Ok(Cow::Borrowed(value))
    }

    fn function_body<'a>(value: &'a str, name: &str) -> Result<Option<&'a str>> {
        let Some(prefix) = value.get(..name.len()) else {
            return Ok(None);
        };
        if !prefix.eq_ignore_ascii_case(name) {
            return Ok(None);
        }
        let rest = value[name.len()..].trim_start();
        if !rest.starts_with('(') {
            return Ok(None);
        }
        enclosed_body(rest, '(', ')')
            .map(Some)
            .map_err(|error| offset_error(error, subslice_offset(value, rest)))
    }

    fn enclosed_body(value: &str, open: char, close: char) -> Result<&str> {
        let value = value.trim();
        if !value.starts_with(open) {
            return Err(field_parse_error(0, format!("expected opening `{open}`")));
        }
        let end = closing_delimiter(value, open, close)?;
        if !value[end + close.len_utf8()..].trim().is_empty() {
            return Err(field_parse_error(end + 1, "unexpected trailing input"));
        }
        Ok(&value[open.len_utf8()..end])
    }

    fn closing_delimiter(value: &str, open: char, close: char) -> Result<usize> {
        let mut stack = Vec::new();
        let mut quote = None;
        let mut escaped = false;
        for (index, character) in value.char_indices() {
            if let Some(active) = quote {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == active {
                    quote = None;
                }
                continue;
            }
            if matches!(character, '\'' | '"' | '`') {
                quote = Some(character);
                continue;
            }
            if character == open {
                stack.push(index);
            } else if character == close {
                if stack.pop().is_none() {
                    return Err(field_parse_error(
                        index,
                        format!("unexpected closing `{close}`"),
                    ));
                }
                if stack.is_empty() {
                    return Ok(index);
                }
            }
        }
        Err(field_parse_error(
            value.len(),
            format!("missing closing `{close}`"),
        ))
    }

    fn split_top_level(value: &str, separator: char) -> Result<Vec<(usize, &str)>> {
        let mut values = Vec::new();
        let mut stack = Vec::new();
        let mut quote = None;
        let mut escaped = false;
        let mut start = 0;

        for (index, character) in value.char_indices() {
            if let Some(active) = quote {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == active {
                    quote = None;
                }
                continue;
            }
            if matches!(character, '\'' | '"' | '`') {
                quote = Some(character);
                continue;
            }
            match character {
                '(' | '[' | '{' | '<' => stack.push((character, index)),
                ')' | ']' | '}' | '>' => {
                    let expected = if character == ')' {
                        '('
                    } else if character == ']' {
                        '['
                    } else if character == '}' {
                        '{'
                    } else {
                        '<'
                    };
                    let Some((actual, _)) = stack.pop() else {
                        return Err(field_parse_error(index, "unexpected closing delimiter"));
                    };
                    if actual != expected {
                        return Err(field_parse_error(index, "mismatched closing delimiter"));
                    }
                }
                _ if character == separator && stack.is_empty() => {
                    let raw = &value[start..index];
                    let trimmed = raw.trim();
                    let leading = raw.len() - raw.trim_start().len();
                    values.push((start + leading, trimmed));
                    start = index + character.len_utf8();
                }
                _ => {}
            }
        }
        if quote.is_some() {
            return Err(field_parse_error(value.len(), "unterminated quoted string"));
        }
        if let Some((delimiter, position)) = stack.last() {
            return Err(field_parse_error(
                *position,
                format!("unclosed `{delimiter}` delimiter"),
            ));
        }
        let raw = &value[start..];
        let trimmed = raw.trim();
        let leading = raw.len() - raw.trim_start().len();
        values.push((start + leading, trimmed));
        Ok(values)
    }

    fn top_level_whitespace(value: &str) -> Result<Option<usize>> {
        let mut stack = Vec::new();
        let mut quote = None;
        let mut escaped = false;
        for (index, character) in value.char_indices() {
            if let Some(active) = quote {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == active {
                    quote = None;
                }
                continue;
            }
            if matches!(character, '\'' | '"' | '`') {
                quote = Some(character);
            } else if matches!(character, '(' | '[' | '{' | '<') {
                stack.push(character);
            } else if matches!(character, ')' | ']' | '}' | '>') {
                stack.pop();
            } else if character.is_whitespace() && stack.is_empty() {
                return Ok(Some(index));
            }
        }
        if quote.is_some() || !stack.is_empty() {
            return Err(field_parse_error(value.len(), "unclosed delimiter"));
        }
        Ok(None)
    }

    fn split_assignment(value: &str, base: usize) -> Result<(&str, &str)> {
        split_key_value(value, base, '=')
    }

    fn split_key_value(value: &str, base: usize, separator: char) -> Result<(&str, &str)> {
        let parts = split_top_level(value, separator)?;
        if parts.len() != 2 {
            return Err(field_parse_error(
                base,
                format!("expected exactly one top-level `{separator}`"),
            ));
        }
        if parts[0].1.is_empty() || parts[1].1.is_empty() {
            return Err(field_parse_error(base, "key and value must not be empty"));
        }
        Ok((parts[0].1, parts[1].1))
    }

    fn parse_bool(value: &str, position: usize) -> Result<bool> {
        if value.trim().eq_ignore_ascii_case("true") {
            Ok(true)
        } else if value.trim().eq_ignore_ascii_case("false") {
            Ok(false)
        } else {
            Err(field_parse_error(position, "expected `true` or `false`"))
        }
    }

    fn parse_dtype(value: &str, position: usize) -> Result<DataType> {
        DataType::from_str(value).map_err(|error| match error {
            Error::Parse {
                position: nested,
                reason,
                ..
            } => field_parse_error(position.saturating_add(nested), reason),
            Error::UnknownDataType(name) => {
                field_parse_error(position, format!("unknown datatype {name:?}"))
            }
            Error::InvalidDataType { kind, reason } => {
                field_parse_error(position, format!("invalid {kind} datatype: {reason}"))
            }
            error => error,
        })
    }

    fn offset_error(error: Error, position: usize) -> Error {
        match error {
            Error::Parse {
                position: nested,
                reason,
                ..
            } => field_parse_error(position.saturating_add(nested), reason),
            error => error,
        }
    }

    fn metadata_parse_error(error: Error, position: usize) -> Error {
        match error {
            Error::EmptyMetadataKey => {
                field_parse_error(position, "metadata key must not be empty")
            }
            Error::DuplicateMetadataKey(key) => {
                field_parse_error(position, format!("duplicate metadata key {key:?}"))
            }
            Error::InvalidMetadataValue { key, reason } => field_parse_error(
                position,
                format!("invalid metadata value for {key:?}: {reason}"),
            ),
            error => offset_error(error, position),
        }
    }

    fn split_arrow_display_field(member: &str, position: usize) -> Result<Option<(String, &str)>> {
        let parts = split_top_level(member, ':')?;
        if parts.len() != 2 {
            return Ok(None);
        }
        let key = parts[0].1.trim();
        let is_quoted = (key.starts_with('"') && key.ends_with('"'))
            || (key.starts_with('\'') && key.ends_with('\''))
            || (key.starts_with('`') && key.ends_with('`'))
            || (key.starts_with('[') && key.ends_with(']'));
        if !is_quoted {
            return Ok(None);
        }
        let name = parse_string(key).map_err(|error| offset_error(error, position + parts[0].0))?;
        Ok(Some((name, parts[1].1)))
    }

    fn strip_nullable_prefix(value: &str) -> (&str, bool) {
        let value = value.trim_start();
        let Some(prefix) = value.get(..8) else {
            return (value, false);
        };
        if prefix.eq_ignore_ascii_case("nullable")
            && value[8..].chars().next().is_some_and(char::is_whitespace)
        {
            (value[8..].trim_start(), true)
        } else {
            (value, false)
        }
    }

    fn parse_i64(value: &str, position: usize) -> Result<i64> {
        value
            .trim()
            .parse()
            .map_err(|_| field_parse_error(position, "expected a signed 64-bit integer"))
    }

    fn strip_trailing_words<'a>(value: &'a str, words: &[&str]) -> Option<&'a str> {
        let mut end = value.len();
        for expected in words.iter().rev() {
            while let Some((index, character)) = value[..end].char_indices().next_back() {
                if !character.is_whitespace() {
                    break;
                }
                end = index;
            }
            let word_end = end;
            while let Some((index, character)) = value[..end].char_indices().next_back() {
                if character.is_whitespace() {
                    break;
                }
                end = index;
            }
            if end == word_end || !value[end..word_end].eq_ignore_ascii_case(expected) {
                return None;
            }
        }
        if end == value.len() {
            None
        } else {
            Some(&value[..end])
        }
    }

    fn field_parse_error(position: usize, reason: impl Into<SmolStr>) -> Error {
        Error::Parse {
            target: "field",
            position,
            reason: reason.into(),
        }
    }
}

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
            return fmt::Display::fmt(&self.pretty_view(), formatter);
        }
        use DataType as D;
        match self {
            // Every parameter-free type displays as its variant name, which
            // `DataTypeId::as_str` already spells; only parameters need an arm.
            D::Null
            | D::Boolean
            | D::Int8
            | D::Int16
            | D::Int32
            | D::Int64
            | D::UInt8
            | D::UInt16
            | D::UInt32
            | D::UInt64
            | D::Float16
            | D::Float32
            | D::Float64
            | D::Country
            | D::Currency
            | D::Mic
            | D::Cfi
            | D::Isin
            | D::Cusip
            | D::Sedol
            | D::Bloomberg
            | D::Side
            | D::State
            | D::TimeInForce
            | D::Uuid(_)
            | D::Version
            | D::Timezone
            | D::MimeType
            | D::MediaType
            | D::Url
            | D::Variant => formatter.write_str(self.name()),
            // Each temporal family spells its own leaf and parameters.
            D::DateTime(leaf) => fmt::Display::fmt(leaf, formatter),
            D::Date(leaf) => fmt::Display::fmt(leaf, formatter),
            D::Time(leaf) => fmt::Display::fmt(leaf, formatter),
            D::Duration(leaf) => fmt::Display::fmt(leaf, formatter),
            D::Interval(leaf) => fmt::Display::fmt(leaf, formatter),
            D::Bytes(parameters) => fmt::Display::fmt(parameters, formatter),
            D::String(parameters) => fmt::Display::fmt(parameters, formatter),
            D::Sequence(SequenceType::List(field)) => {
                fmt_single_field_type(formatter, "list", field)
            }
            D::Sequence(SequenceType::ListView(field)) => {
                fmt_single_field_type(formatter, "list_view", field)
            }
            D::Sequence(SequenceType::FixedSizeList(field, length)) => {
                formatter.write_str("fixed_size_list(")?;
                fmt_field(formatter, field)?;
                write!(formatter, ",{length})")
            }
            D::Sequence(SequenceType::LargeList(field)) => {
                fmt_single_field_type(formatter, "large_list", field)
            }
            D::Sequence(SequenceType::LargeListView(field)) => {
                fmt_single_field_type(formatter, "large_list_view", field)
            }
            D::Structure(fields) => {
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
            D::Enum(EnumType::Dictionary(dictionary)) => {
                write!(
                    formatter,
                    "dictionary({},{})",
                    dictionary.key, dictionary.value
                )
            }
            D::Decimal(DecimalType::Decimal32 { precision, scale }) => {
                write!(formatter, "decimal32({precision},{scale})")
            }
            D::Decimal(DecimalType::Decimal64 { precision, scale }) => {
                write!(formatter, "decimal64({precision},{scale})")
            }
            D::Decimal(DecimalType::Decimal128 { precision, scale }) => {
                write!(formatter, "decimal128({precision},{scale})")
            }
            D::Decimal(DecimalType::Decimal256 { precision, scale }) => {
                write!(formatter, "decimal256({precision},{scale})")
            }
            D::Mapping(map) => {
                formatter.write_str("map(")?;
                fmt_field(formatter, map.entries())?;
                write!(formatter, ",keys_sorted={})", map.keys_sorted())
            }
            D::RunEndEncoded(encoded) => {
                formatter.write_str("run_end_encoded(")?;
                fmt_field(formatter, &encoded.run_ends)?;
                formatter.write_char(',')?;
                fmt_field(formatter, &encoded.values)?;
                formatter.write_char(')')
            }
            // The defaults display bare, so `geometry` round-trips as itself
            // and a parameter appears exactly when it says something.
            D::Geometry(geospatial) => {
                if geospatial.has_default_crs() {
                    return formatter.write_str(self.name());
                }
                formatter.write_str("geometry(")?;
                fmt_quoted(formatter, geospatial.crs())?;
                formatter.write_char(')')
            }
            D::Geography(geospatial) => {
                let algorithm = geospatial.algorithm().unwrap_or_default();
                if geospatial.has_default_crs() && algorithm == EdgeAlgorithm::Spherical {
                    return formatter.write_str(self.name());
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

pub(crate) fn fmt_quoted(formatter: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
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
            "date" | "date32" => DataType::date32(),
            "date64" | "datemillisecond" => DataType::date64(),
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
            "interval" => DataType::interval(self.parse_interval_unit(depth)?)?,
            // One byte family, one grammar: an optional bound that reads as
            // the width on the fixed leaf and as the maximum on the sized
            // one, which `binary(32)` is the shorter spelling of. Which word
            // names which leaf is the family's own table, so a spelling is
            // never accepted here and refused under a `layout=` argument.
            _ if BytesType::from_spelling(&keyword).is_some() => {
                let leaf =
                    BytesType::from_spelling(&keyword).expect("the guard just answered this leaf");
                self.parse_bytes(leaf)?
            }
            // SQL's `char(n)` is blank-padded to exactly n bytes, which is
            // the fixed leaf. A width is what makes a string fixed, so a
            // bare `char` - which is what FIX's own type is called and what
            // Arrow's debug spelling prints - is the variable one, and so is
            // `character varying` however it is punctuated. This arm sits
            // above the family's guard because the guard would read `char`
            // as the plain leaf before the parenthesis was looked at.
            "char" | "character" | "nchar" => {
                let fixed = !(keyword == "character" && self.consume_word("varying"))
                    && self.peek_opening().is_some();
                let leaf = match fixed {
                    true => StringType::FixedUtf8String(1),
                    false => StringType::Utf8String,
                };
                self.parse_string(leaf, true)?
            }
            // One string family, one grammar: an optional charset, accepted
            // only beside a charset-free spelling, then an optional bound
            // that reads as the width on a fixed leaf and as the maximum on
            // a plain one. Which word names which leaf is the family's own
            // table, so a spelling is never accepted here and refused under
            // a `layout=` argument.
            _ if StringType::from_spelling(&keyword).is_some() => {
                let leaf =
                    StringType::from_spelling(&keyword).expect("the guard just answered this leaf");
                self.parse_string(leaf, StringType::general_spelling(&keyword).is_some())?
            }

            "uuid" => DataType::Uuid(UuidType::Uuid),
            // A version names the leaf that admits it; bare `uuid` admits
            // every one, so the three are spellings beside it, not under it.
            "uuidv4" => DataType::Uuid(UuidType::Uuidv4),
            "uuidv7" => DataType::Uuid(UuidType::Uuidv7),
            "uuidv8" => DataType::Uuid(UuidType::Uuidv8),
            "version" => DataType::Version,
            "timezone" | "timezonename" | "tz" => DataType::Timezone,
            "mimetype" | "mime" => DataType::MimeType,
            "mediatype" | "contenttype" => DataType::MediaType,
            "url" => DataType::Url,
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
        // Nullability is an argument like the others: a field spelled without
        // it is nullable, which is what the standalone reading of this same
        // form answers, so `list(field("a",int32))` is a field either way.
        let mut nullable = true;
        let mut saw_nullable = false;
        let mut metadata = Vec::new();
        let mut saw_metadata = false;
        let mut dictionary_id = None;
        let mut dictionary_is_ordered = None;
        while self.consume_separator() {
            // The canonical spelling names the nullability first, so it is
            // probed first: every other argument is rarer than this one.
            if !saw_nullable && self.consume_label("nullable") {
                nullable = self.parse_bool("field nullability")?;
                saw_nullable = true;
            } else if self.consume_label("dictionary_id") || self.consume_label("dict_id") {
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
            } else if saw_nullable {
                return Err(self.error_here("unknown field argument"));
            } else {
                // A bare argument in this position is the nullability, which
                // the canonical spelling labels and the SQL-ish ones do not.
                nullable = self.parse_bool("field nullability")?;
                saw_nullable = true;
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
                // underscore; `dtype` is this crate's own spelling of the same
                // property, and one form answers to both.
                "datatype" | "dtype" | "type" => {
                    if dtype.is_some() {
                        return Err(self.error_here("duplicate field datatype"));
                    }
                    dtype = Some(self.parse_type(depth)?);
                }
                "nullable" | "isnullable" => {
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

/// The crate's one fold: case folded, and `_`, `-` and space dropped.
///
/// One rule serves every spelling a caller may write for something this crate
/// names - a datatype word, a logical name, a FIX field name, a FIX code's
/// symbolic name - so `UTCTimestamp`, `utc_timestamp`, `utc-timestamp` and
/// `UTC TIMESTAMP` are one spelling everywhere rather than one spelling per
/// layer.
pub(crate) fn folded(value: &str) -> impl Iterator<Item = char> + '_ {
    value
        .chars()
        .filter(|character| !matches!(character, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
}

/// Whether one byte is a separator the fold drops.
const fn is_dropped(byte: u8) -> bool {
    matches!(byte, b'_' | b'-' | b' ')
}

/// Whether two spellings are one under [`folded`].
///
/// Compares the two folds as they are produced, so a caller holding neither
/// spelling folded allocates nothing to find out.
///
/// Almost every spelling this crate compares is ASCII - a datatype word, a
/// FIX field name, a code's symbolic name - and ASCII folds one byte to one
/// byte, so those walk the bytes directly. `char::to_lowercase` answers an
/// iterator because one character can fold to several, which is real but rare
/// enough that paying for it on every comparison would be the wrong trade.
pub(crate) fn folds_equal(left: &str, right: &str) -> bool {
    if left.is_ascii() && right.is_ascii() {
        let mut left = left.bytes().filter(|byte| !is_dropped(*byte));
        let mut right = right.bytes().filter(|byte| !is_dropped(*byte));
        loop {
            return match (left.next(), right.next()) {
                (Some(left), Some(right)) => {
                    if left.eq_ignore_ascii_case(&right) {
                        continue;
                    }
                    false
                }
                (None, None) => true,
                _ => false,
            };
        }
    }
    folded(left).eq(folded(right))
}

pub(crate) fn normalized(value: &str) -> String {
    folded(value).collect()
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

// ------------------------------------------------------------------------
// Nested datatype grammar.
// ------------------------------------------------------------------------
impl Parser<'_> {
    pub(crate) fn parse_list(&mut self, kind: ListKind, depth: usize) -> Result<DataType> {
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

    pub(crate) fn parse_fixed_size_list(&mut self, depth: usize) -> Result<DataType> {
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

    pub(crate) fn parse_struct(&mut self, depth: usize) -> Result<DataType> {
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

    pub(crate) fn parse_dictionary(&mut self, depth: usize) -> Result<DataType> {
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

    pub(crate) fn parse_map(&mut self, depth: usize) -> Result<DataType> {
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

    pub(crate) fn parse_run_end(&mut self, depth: usize) -> Result<DataType> {
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
    pub(crate) fn parse_union(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
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

    pub(crate) fn parse_union_mode(&mut self) -> Result<UnionMode> {
        let value = self.parse_text("union mode")?;
        match normalized(&value).as_str() {
            "dense" => Ok(UnionMode::Dense),
            "sparse" => Ok(UnionMode::Sparse),
            _ => Err(self.error_here("union mode must be dense or sparse")),
        }
    }
}

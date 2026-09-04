//! Recursive field grammar and FromStr implementation.

use std::borrow::Cow;
use std::str::FromStr;
use std::sync::OnceLock;

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
        return Err(parse_error(base, "expected a field expression"));
    }

    if let Some(body) = function_body(value, "field").map_err(|error| offset_error(error, base))? {
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
            let body =
                enclosed_body(rest, '{', '}').map_err(|error| offset_error(error, rest_base))?;
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
        return Err(parse_error(0, "field(...) requires a name and datatype"));
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
                return Err(parse_error(offset, "duplicate nullable argument"));
            }
            nullable = parse_bool(raw_value, offset)?;
            saw_nullable = true;
        } else if key.eq_ignore_ascii_case("metadata") {
            if saw_metadata {
                return Err(parse_error(offset, "duplicate metadata argument"));
            }
            metadata = parse_metadata(raw_value, offset)?;
            saw_metadata = true;
        } else if key.eq_ignore_ascii_case("dictionary_id") || key.eq_ignore_ascii_case("dict_id") {
            if saw_dictionary_id {
                return Err(parse_error(offset, "duplicate dictionary id argument"));
            }
            dictionary_id = parse_i64(raw_value, offset)?;
            saw_dictionary_id = true;
        } else if key.eq_ignore_ascii_case("dictionary_is_ordered")
            || key.eq_ignore_ascii_case("dict_is_ordered")
        {
            if saw_dictionary_is_ordered {
                return Err(parse_error(
                    offset,
                    "duplicate dictionary ordering argument",
                ));
            }
            dictionary_is_ordered = parse_bool(raw_value, offset)?;
            saw_dictionary_is_ordered = true;
        } else {
            return Err(parse_error(
                offset,
                format!("unknown field argument {key:?}"),
            ));
        }
    }

    let field = Field {
        name: name.into(),
        dtype,
        nullable,
        dictionary_id,
        dictionary_is_ordered,
        metadata,
        arrow: OnceLock::new(),
    };
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
            if let Some((display_name, display_type)) = split_arrow_display_field(member, offset)? {
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
                return Err(parse_error(offset, "duplicate dictionary ordering flag"));
            }
            dictionary_is_ordered = Some(true);
            continue;
        }
        let (key, value) = split_key_value(member, offset, ':')?;
        let value_offset = offset + member.find(value).unwrap_or_default();
        let normalized = key.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "name" => {
                if name.is_some() {
                    return Err(parse_error(offset, "duplicate field name"));
                }
                name =
                    Some(parse_string(value).map_err(|error| offset_error(error, value_offset))?);
            }
            "dtype" | "type" => {
                if dtype.is_some() {
                    return Err(parse_error(offset, "duplicate field datatype"));
                }
                dtype = Some(parse_dtype(value, value_offset)?);
            }
            "nullable" | "is_nullable" => {
                if nullable.is_some() {
                    return Err(parse_error(offset, "duplicate nullability"));
                }
                nullable = Some(parse_bool(value, offset)?);
            }
            "metadata" => {
                if saw_metadata {
                    return Err(parse_error(offset, "duplicate field metadata"));
                }
                metadata = parse_metadata(value, value_offset)?;
                saw_metadata = true;
            }
            "dict_id" | "dictionary_id" => {
                if dictionary_id.is_some() {
                    return Err(parse_error(offset, "duplicate dictionary id"));
                }
                dictionary_id = Some(parse_i64(value, offset)?);
            }
            "dict_is_ordered" | "dictionary_is_ordered" => {
                if dictionary_is_ordered.is_some() {
                    return Err(parse_error(offset, "duplicate dictionary ordering flag"));
                }
                dictionary_is_ordered = Some(parse_bool(value, offset)?);
            }
            _ => {
                return Err(parse_error(
                    offset,
                    format!("unknown Arrow field member {key:?}"),
                ));
            }
        }
    }

    let field = Field {
        name: name
            .ok_or_else(|| parse_error(0, "Arrow field is missing name"))?
            .into(),
        dtype: dtype.ok_or_else(|| parse_error(0, "Arrow field is missing dtype"))?,
        // Arrow's Debug implementation omits `nullable` when it is false.
        nullable: nullable.unwrap_or(false),
        dictionary_id: dictionary_id.unwrap_or_default(),
        dictionary_is_ordered: dictionary_is_ordered.unwrap_or_default(),
        metadata,
        arrow: OnceLock::new(),
    };
    field.validate()?;
    Ok(field)
}

fn parse_shorthand_field(value: &str) -> Result<Field> {
    let (value, nullable) = parse_nullability_suffix(value)?;
    if let Some((name, dtype, type_offset)) = split_quoted_field_name(value)? {
        let name = parse_identifier(name)?;
        if dtype.trim().is_empty() {
            return Err(parse_error(value.len(), "missing field datatype"));
        }
        let field = Field::new(name, parse_dtype(dtype, type_offset)?, nullable);
        field.validate()?;
        return Ok(field);
    }
    let parts = split_top_level(value, ':')?;
    let (name, dtype, type_offset) = if parts.len() == 2 {
        (parts[0].1, parts[1].1, parts[1].0)
    } else if parts.len() > 2 {
        return Err(parse_error(parts[2].0, "unexpected top-level colon"));
    } else {
        let index = top_level_whitespace(value)?
            .ok_or_else(|| parse_error(0, "expected `name: datatype` or `name datatype`"))?;
        let dtype = value[index..].trim_start();
        (
            &value[..index],
            dtype,
            index + value[index..].len().saturating_sub(dtype.len()),
        )
    };
    let name = parse_identifier(name)?;
    if dtype.trim().is_empty() {
        return Err(parse_error(value.len(), "missing field datatype"));
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
    let end = end.ok_or_else(|| parse_error(value.len(), "unterminated field name"))?;
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
        return Err(parse_error(
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
    let body = enclosed_body(value, '{', '}')
        .map_err(|_| parse_error(base, "metadata must be an object enclosed by `{` and `}`"))?;
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
        return Err(parse_error(0, "field name must not be empty"));
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
        return Err(parse_error(0, "unmatched quote in string"));
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
            return Err(parse_error(0, "unescaped quote in string"));
        }
        if character != '\\' {
            output.push(character);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or_else(|| parse_error(value.len(), "unterminated escape"))?;
        output.push(match escaped {
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            _ => return Err(parse_error(0, format!("unsupported escape `\\{escaped}`"))),
        });
    }
    Ok(output)
}

fn strip_optional_wrappers(value: &str) -> Result<Cow<'_, str>> {
    strip_optional_wrappers_at_depth(value, 0)
}

fn strip_optional_wrappers_at_depth(value: &str, depth: usize) -> Result<Cow<'_, str>> {
    if depth > DataType::PARSE_RECURSION_LIMIT {
        return Err(parse_error(0, "field wrapper recursion limit exceeded"));
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
            strip_optional_wrappers_at_depth(&parsed, depth + 1)?.into_owned(),
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
    if matching_close(value, open, close)? == value.len() - close.len_utf8() {
        return strip_optional_wrappers_at_depth(
            &value[open.len_utf8()..value.len() - close.len_utf8()],
            depth + 1,
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
        return Err(parse_error(0, format!("expected opening `{open}`")));
    }
    let end = matching_close(value, open, close)?;
    if !value[end + close.len_utf8()..].trim().is_empty() {
        return Err(parse_error(end + 1, "unexpected trailing input"));
    }
    Ok(&value[open.len_utf8()..end])
}

fn matching_close(value: &str, open: char, close: char) -> Result<usize> {
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
                return Err(parse_error(index, format!("unexpected closing `{close}`")));
            }
            if stack.is_empty() {
                return Ok(index);
            }
        }
    }
    Err(parse_error(
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
                    return Err(parse_error(index, "unexpected closing delimiter"));
                };
                if actual != expected {
                    return Err(parse_error(index, "mismatched closing delimiter"));
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
        return Err(parse_error(value.len(), "unterminated quoted string"));
    }
    if let Some((delimiter, position)) = stack.last() {
        return Err(parse_error(
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
        return Err(parse_error(value.len(), "unclosed delimiter"));
    }
    Ok(None)
}

fn split_assignment(value: &str, base: usize) -> Result<(&str, &str)> {
    split_key_value(value, base, '=')
}

fn split_key_value(value: &str, base: usize, separator: char) -> Result<(&str, &str)> {
    let parts = split_top_level(value, separator)?;
    if parts.len() != 2 {
        return Err(parse_error(
            base,
            format!("expected exactly one top-level `{separator}`"),
        ));
    }
    if parts[0].1.is_empty() || parts[1].1.is_empty() {
        return Err(parse_error(base, "key and value must not be empty"));
    }
    Ok((parts[0].1, parts[1].1))
}

fn parse_bool(value: &str, position: usize) -> Result<bool> {
    if value.trim().eq_ignore_ascii_case("true") {
        Ok(true)
    } else if value.trim().eq_ignore_ascii_case("false") {
        Ok(false)
    } else {
        Err(parse_error(position, "expected `true` or `false`"))
    }
}

fn parse_dtype(value: &str, position: usize) -> Result<DataType> {
    DataType::from_str(value).map_err(|error| match error {
        Error::Parse {
            position: nested,
            reason,
            ..
        } => parse_error(position.saturating_add(nested), reason),
        Error::UnknownDataType(name) => parse_error(position, format!("unknown datatype {name:?}")),
        Error::InvalidDataType { kind, reason } => {
            parse_error(position, format!("invalid {kind} datatype: {reason}"))
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
        } => parse_error(position.saturating_add(nested), reason),
        error => error,
    }
}

fn metadata_parse_error(error: Error, position: usize) -> Error {
    match error {
        Error::EmptyMetadataKey => parse_error(position, "metadata key must not be empty"),
        Error::DuplicateMetadataKey(key) => {
            parse_error(position, format!("duplicate metadata key {key:?}"))
        }
        Error::InvalidMetadataValue { key, reason } => parse_error(
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
        .map_err(|_| parse_error(position, "expected a signed 64-bit integer"))
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

fn parse_error(position: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Parse {
        target: "field",
        position,
        reason: reason.into(),
    }
}

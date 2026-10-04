//! Workbook-owned text indexing semantics. XML remains the wire owner.

use super::value::{Operand, Outcome, Unevaluated};

/// The source workbook's authored text behavior, resolved once at intake.
/// A schema-valid future or unspecified explicit version is not guessed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Compatibility {
    #[default]
    Version1,
    Version2,
    Unknown,
}

impl Compatibility {
    pub(crate) const NAMESPACE: &str =
        "http://schemas.microsoft.com/office/spreadsheetml/2024/workbookCompatibilityVersion";
    pub(crate) const EXTENSION: &str = "{D14903EA-33C4-47F7-8F05-3474C54BE107}";

    pub(crate) const fn from_version(version: Option<u32>) -> Self {
        match version {
            Some(1) => Self::Version1,
            Some(2) => Self::Version2,
            _ => Self::Unknown,
        }
    }

    pub(crate) fn length(self, value: Outcome) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let text = match value.text_argument() {
            Ok(text) => text,
            Err(value) => return value,
        };
        let text = text.as_str();
        let Some(count) = self.count(text) else {
            return Outcome::Uncomputed(Unevaluated::TextCompatibility);
        };
        Outcome::Computed(Operand::Number(count as f64))
    }
}

use crate::Str;
use crate::excel::cell::{DateSystem, ExcelError, MAX_CELL_TEXT};
use super::functions::Function;

impl Operand {
    fn text_argument(self) -> Result<Str, Outcome> {
        match self {
            Self::Text(value) => Ok(value),
            Self::Number(value) => Ok(Str::from(crate::excel::format::formula_number(value))),
            // Formula text conversion follows the explicit en-US contract.
            Self::Boolean(value) => Ok(Str::new_static(if value { "TRUE" } else { "FALSE" })),
            Self::Blank => Ok(Str::new_static("")),
            Self::Error(error) => Err(Outcome::Computed(Self::Error(error))),
            _ => Err(Outcome::Uncomputed(Unevaluated::Coercion)),
        }
    }

    fn text_count(self, system: DateSystem) -> Result<usize, Outcome> {
        match self.number(system) {
            Some(Ok(value)) if value >= 0.0 => Ok(value as usize),
            Some(Ok(_)) => Err(Outcome::Computed(Self::Error(ExcelError::Value))),
            Some(Err(error)) => Err(Outcome::Computed(Self::Error(error))),
            None => Err(Outcome::Uncomputed(Unevaluated::Coercion)),
        }
    }
}

impl Compatibility {
    fn count(self, text: &str) -> Option<usize> {
        match self {
            Self::Version1 => Some(text.encode_utf16().count()),
            Self::Version2 => Some(text.chars().count()),
            Self::Unknown => None,
        }
    }

    /// A nonempty UTF-8 needle cannot begin at a low surrogate. Starting
    /// inside a pair therefore advances to the next scalar boundary, while
    /// an empty FIND needle returns the requested native position directly.
    fn search_start(self, text: &str, from: usize) -> Option<usize> {
        if self == Self::Unknown { return None; }
        let mut position = 0;
        for (byte, character) in text.char_indices() {
            if position >= from { return Some(byte); }
            position += if self == Self::Version1 { character.len_utf16() } else { 1 };
        }
        Some(text.len())
    }

    /// A UTF-8 borrow only when both requested boundaries are representable.
    /// Version 1 can request half a surrogate; that result stays held.
    fn window(self, text: &str, from: usize, until: usize) -> Option<&str> {
        match self {
            Self::Unknown => return None,
            Self::Version2 => return Some(Str::char_window(text, from, until)),
            Self::Version1 => {}
        }
        if from >= until { return Some(""); }
        let (mut position, mut begin, mut end) = (0, None, None);
        for (byte, character) in text.char_indices() {
            if position == from { begin = Some(byte); }
            if position == until { end = Some(byte); break; }
            let next = position + character.len_utf16();
            if (position < from && from < next) || (position < until && until < next) { return None; }
            position = next;
        }
        let begin = begin.unwrap_or(text.len());
        let end = end.unwrap_or(text.len());
        Some(if begin >= end { "" } else { &text[begin..end] })
    }
}

impl Function {
    /// Fixed-arity text adapters share typed scalar intake and existing UTF-8
    /// windows; they neither materialize a range nor parse a new expression.
    pub(crate) fn text(self, values: [Outcome; 4], count: usize, mode: Compatibility, system: DateSystem) -> Outcome {
        let [first, second, third, fourth] = values;
        let answer = || -> Result<Operand, Outcome> {
            let first = first.operand().map_err(Outcome::Uncomputed)?;
            if self == Self::T {
                return Ok(match first {
                    value @ (Operand::Text(_) | Operand::Error(_)) => value,
                    Operand::Reference(_) => return Err(Outcome::Uncomputed(Unevaluated::Reference)),
                    _ => Operand::Text(Str::new_static("")),
                });
            }
            if self == Self::Char {
                let number = match first.number(system) {
                    Some(Ok(number)) => number,
                    Some(Err(error)) => return Ok(Operand::Error(error)),
                    None => return Err(Outcome::Uncomputed(Unevaluated::Coercion)),
                };
                if !(1.0..256.0).contains(&number) { return Ok(Operand::Error(ExcelError::Value)); }
                if number >= 128.0 { return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility)); }
                let character = crate::cp1252::scalar_of(number as u8).expect("ASCII is assigned in every source code page");
                let mut bytes = [0; 4];
                return Ok(Operand::Text(Str::new(character.encode_utf8(&mut bytes))));
            }
            let source = first.text_argument()?;
            let text = source.as_str();
            let string = match self {
                Self::Find => {
                    let within = second.operand().map_err(Outcome::Uncomputed)?.text_argument()?;
                    let within = within.as_str();
                    let start = if count == 2 { 1 } else {
                        third.operand().map_err(Outcome::Uncomputed)?.text_count(system)?
                    };
                    if start == 0 { return Ok(Operand::Error(ExcelError::Value)); }
                    let Some(length) = mode.count(within) else {
                        return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility));
                    };
                    let from = start - 1;
                    if from > length { return Ok(Operand::Error(ExcelError::Value)); }
                    if text.is_empty() { return Ok(Operand::Number(start as f64)); }
                    let from = mode.search_start(within, from).expect("the mode was resolved above");
                    let Some(found) = within[from..].find(text) else {
                        return Ok(Operand::Error(ExcelError::Value));
                    };
                    return Ok(Operand::Number((mode.count(&within[..from + found]).expect("the mode was resolved above") + 1) as f64));
                }
                Self::Replace => {
                    let start = second.operand().map_err(Outcome::Uncomputed)?.text_count(system)?;
                    let length = third.operand().map_err(Outcome::Uncomputed)?.text_count(system)?;
                    let replacement = fourth.operand().map_err(Outcome::Uncomputed)?.text_argument()?;
                    if start == 0 { return Ok(Operand::Error(ExcelError::Value)); }
                    if length == 0 && replacement.as_str().is_empty() { return Ok(Operand::Text(source)); }
                    let from = start - 1;
                    let (Some(prefix), Some(suffix)) = (mode.window(text,0,from), mode.window(text,from.saturating_add(length),usize::MAX)) else {
                        return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility));
                    };
                    let removed = &text[prefix.len()..text.len()-suffix.len()];
                    if removed == replacement.as_str() { return Ok(Operand::Text(source)); }
                    let units = prefix.encode_utf16().count().checked_add(replacement.as_str().encode_utf16().count())
                        .and_then(|units| units.checked_add(suffix.encode_utf16().count()));
                    if units.is_none_or(|units| units > MAX_CELL_TEXT) { return Ok(Operand::Error(ExcelError::Value)); }
                    let mut output = smol_str::SmolStrBuilder::new();
                    output.push_str(prefix);
                    output.push_str(replacement.as_str());
                    output.push_str(suffix);
                    output.finish()
                }
                Self::Lower | Self::Upper => {
                    // The workbook supplies no locale for casing. ASCII and
                    // uncased Unicode have one portable mapping; do not import
                    // Rust expansions into native simple/locale-sensitive case.
                    if source.has_non_ascii_case() {
                        return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility));
                    }
                    return Ok(Operand::Text(if self == Self::Lower {
                        source.lowercase()
                    } else { source.uppercase() }));
                }
                Self::Code => {
                    let Some(character) = text.chars().next() else { return Ok(Operand::Error(ExcelError::Value)); };
                    if !character.is_ascii() { return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility)); }
                    let code = crate::cp1252::byte_of(character).expect("ASCII has one source code");
                    return Ok(Operand::Number(f64::from(code)));
                }
                Self::Proper => {
                    if source.has_non_ascii_case() { return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility)); }
                    return Ok(Operand::Text(source.ascii_titlecase()));
                }
                Self::Clean => {
                    if !text.bytes().any(|byte| byte < 32) { return Ok(Operand::Text(source)); }
                    let mut output = smol_str::SmolStrBuilder::new();
                    for character in text.chars().filter(|character| *character >= ' ') { output.push(character); }
                    output.finish()
                }
                Self::Trim => {
                    let trimmed = text.trim_matches(' ');
                    if !trimmed.contains("  ") {
                        return Ok(Operand::Text(if trimmed.len() == text.len() { source } else { Str::new(trimmed) }));
                    }
                    let mut output = smol_str::SmolStrBuilder::new();
                    let mut space = false;
                    for character in trimmed.chars() {
                        if character != ' ' || !space { output.push(character); }
                        space = character == ' ';
                    }
                    output.finish()
                }
                Self::Left | Self::Right => {
                    let count = if count == 1 { 1 } else {
                        second.operand().map_err(Outcome::Uncomputed)?.text_count(system)?
                    };
                    let (from, until) = if self == Self::Left { (0, count) } else {
                        let length = text.chars().count();
                        (length.saturating_sub(count), length)
                    };
                    let window = Str::char_window(text, from, until);
                    return Ok(Operand::Text(if window.len() == text.len() { source } else { Str::new(window) }));
                }
                Self::Mid => {
                    let from = second.operand().map_err(Outcome::Uncomputed)?.text_count(system)?;
                    let count = third.operand().map_err(Outcome::Uncomputed)?.text_count(system)?;
                    if from == 0 { return Ok(Operand::Error(ExcelError::Value)); }
                    let from = from - 1;
                    let Some(window) = mode.window(text, from, from.saturating_add(count)) else {
                        return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility));
                    };
                    return Ok(Operand::Text(if window.len() == text.len() { source } else { Str::new(window) }));
                }
                Self::Exact => {
                    let other = second.operand().map_err(Outcome::Uncomputed)?.text_argument()?;
                    return Ok(Operand::Boolean(text == other.as_str()));
                }
                Self::Rept => {
                    let count = second.operand().map_err(Outcome::Uncomputed)?.text_count(system)?;
                    if text.encode_utf16().count().checked_mul(count).is_none_or(|units| units > MAX_CELL_TEXT) {
                        return Ok(Operand::Error(ExcelError::Value));
                    }
                    if text.is_empty() || count == 0 { return Ok(Operand::Text(Str::new_static(""))); }
                    if count == 1 { return Ok(Operand::Text(source)); }
                    let mut output = smol_str::SmolStrBuilder::new();
                    for _ in 0..count { output.push_str(text); }
                    output.finish()
                }
                Self::Substitute => {
                    let pattern = second.operand().map_err(Outcome::Uncomputed)?.text_argument()?;
                    let replacement = third.operand().map_err(Outcome::Uncomputed)?.text_argument()?;
                    let instance = if count == 4 {
                        let selected = fourth.operand().map_err(Outcome::Uncomputed)?.text_count(system)?;
                        if selected == 0 { return Ok(Operand::Error(ExcelError::Value)); }
                        Some(selected)
                    } else { None };
                    if pattern.as_str().is_empty() || pattern == replacement { return Ok(Operand::Text(source)); }
                    let matches = text.matches(pattern.as_str()).count();
                    let replaced = instance.map_or(matches, |selected| usize::from(selected <= matches));
                    if replaced == 0 { return Ok(Operand::Text(source)); }
                    let removed = pattern.as_str().encode_utf16().count() * replaced;
                    let added = replacement.as_str().encode_utf16().count().checked_mul(replaced);
                    if added.and_then(|added| (text.encode_utf16().count() - removed).checked_add(added)).is_none_or(|length| length > MAX_CELL_TEXT) {
                        return Ok(Operand::Error(ExcelError::Value));
                    }
                    let mut output = smol_str::SmolStrBuilder::new();
                    let mut from = 0;
                    for (index, (at, matched)) in text.match_indices(pattern.as_str()).enumerate() {
                        if instance.is_none_or(|selected| selected == index + 1) {
                            output.push_str(&text[from..at]);
                            output.push_str(replacement.as_str());
                            from = at + matched.len();
                            if instance.is_some() { break; }
                        }
                    }
                    output.push_str(&text[from..]);
                    output.finish()
                }
                _ => unreachable!("the caller selected fixed-arity text functions"),
            };
            Ok(Operand::Text(Str::from(string)))
        };
        match answer() { Ok(value) => Outcome::Computed(value), Err(value) => value }
    }
}

/// One bounded output, fed in reference order without retaining source cells.
/// Blank multiplicities never expand before the UTF-16 output bound is proven.
pub(crate) struct Join {
    delimiter: Str,
    delimiter_units: usize,
    ignore_empty: bool,
    output: smol_str::SmolStrBuilder,
    units: usize,
    has_item: bool,
    error: Option<ExcelError>,
    unresolved: Option<Unevaluated>,
}

impl Join {
    pub(crate) fn new(delimiter: Outcome, ignore: Outcome) -> Self {
        let mut result = Self {
            delimiter: Str::new_static(""), delimiter_units: 0, ignore_empty: false,
            output: smol_str::SmolStrBuilder::new(), units: 0, has_item: false,
            error: None, unresolved: None,
        };
        match delimiter.operand().map_err(Outcome::Uncomputed).and_then(Operand::text_argument) {
            Ok(value) => {
                result.delimiter_units = value.as_str().encode_utf16().count();
                result.delimiter = value;
            }
            Err(value) => result.refuse(value),
        }
        match ignore.operand() {
            Err(reason) => { result.unresolved.get_or_insert(reason); }
            Ok(value) => match value.logical() {
                Some(Ok(value)) => result.ignore_empty = value,
                Some(Err(error)) => { result.error.get_or_insert(error); }
                None => { result.unresolved.get_or_insert(Unevaluated::Coercion); }
            },
        }
        result
    }

    fn refuse(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Uncomputed(reason) => { self.unresolved.get_or_insert(reason); }
            Outcome::Computed(Operand::Error(error)) => { self.error.get_or_insert(error); }
            _ => unreachable!("text intake returned an error or named refusal"),
        }
    }

    pub(crate) fn push(&mut self, value: Outcome, count: u64) {
        debug_assert!(count > 0);
        let text = match value.operand().map_err(Outcome::Uncomputed).and_then(Operand::text_argument) {
            Ok(value) => value,
            Err(value) => { self.refuse(value); return; }
        };
        if self.error.is_some() || self.unresolved.is_some()
            || (self.ignore_empty && text.as_str().is_empty()) { return; }
        let units = text.as_str().encode_utf16().count();
        let separators = count - u64::from(!self.has_item);
        let total = self.units as u128 + count as u128 * units as u128
            + separators as u128 * self.delimiter_units as u128;
        if total > MAX_CELL_TEXT as u128 {
            self.error = Some(ExcelError::Value);
            return;
        }
        // An all-empty range with an empty delimiter can contain 2^34 cells;
        // it contributes an item but no bytes and needs no per-position loop.
        if units != 0 || self.delimiter_units != 0 {
            for _ in 0..count {
                if self.has_item { self.output.push_str(self.delimiter.as_str()); }
                self.output.push_str(text.as_str());
                self.has_item = true;
            }
        }
        self.has_item = true;
        self.units = total as usize;
    }

    pub(crate) fn finish(self) -> Outcome {
        if let Some(reason) = self.unresolved { return Outcome::Uncomputed(reason); }
        if let Some(error) = self.error { return Outcome::Computed(Operand::Error(error)); }
        Outcome::Computed(Operand::Text(Str::from(self.output.finish())))
    }
}

impl super::criteria::Search {
    pub(crate) fn evaluate(&mut self, values: [Outcome; 3], count: usize, mode: Compatibility, system: DateSystem) -> Outcome {
        let [pattern, within, start] = values;
        let answer = || -> Result<Operand, Outcome> {
            let pattern = pattern.operand().map_err(Outcome::Uncomputed)?.text_argument()?;
            let within = within.operand().map_err(Outcome::Uncomputed)?.text_argument()?;
            let start = if count == 2 { 1 } else {
                start.operand().map_err(Outcome::Uncomputed)?.text_count(system)?
            };
            let Some(length) = mode.count(within.as_str()) else {
                return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility));
            };
            if start == 0 || start > length { return Ok(Operand::Error(ExcelError::Value)); }
            if pattern.has_non_ascii_case() || within.has_non_ascii_case() {
                return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility));
            }
            // The native matcher consumes UTF-16 units even in version 2;
            // only admitted start positions and the public index are scalars.
            let from = if mode == Compatibility::Version1 { start - 1 } else {
                within.as_str().chars().take(start - 1).map(char::len_utf16).sum()
            };
            let Some((found, _)) = self.find(&pattern, within.as_str(), from, mode == Compatibility::Version2) else {
                return Ok(Operand::Error(ExcelError::Value));
            };
            let position = if mode == Compatibility::Version1 { found } else {
                let mut units = 0;
                within.as_str().chars().take_while(|character| {
                    if units >= found { return false; }
                    units += character.len_utf16(); true
                }).count()
            };
            Ok(Operand::Number((position + 1) as f64))
        };
        match answer() { Ok(value) => Outcome::Computed(value), Err(value) => value }
    }
}

/// One resolved TEXT code, including a refused code's shared input handle.
/// The FormatCode owns both its spelling and parsed sections; no second key.
#[derive(Debug, Default)]
pub(crate) struct Formatter {
    code: Option<Result<crate::excel::FormatCode, Str>>,
}

impl Formatter {
    pub(crate) fn text(&mut self, value: Outcome, code: Outcome, system: DateSystem) -> Outcome {
        let answer = || -> Result<Operand, Outcome> {
            let value = value.operand().map_err(Outcome::Uncomputed)?;
            if let Operand::Error(error) = value { return Ok(Operand::Error(error)); }
            let code = code.operand().map_err(Outcome::Uncomputed)?;
            // Native TEXT rejects a Boolean format argument, independently
            // of Boolean value-to-text conversion in the first argument.
            if matches!(code, Operand::Boolean(_)) { return Ok(Operand::Error(ExcelError::Value)); }
            let code = code.text_argument()?;
            let same = self.code.as_ref().is_some_and(|prior| match prior {
                Ok(format) => format.code() == code.as_str(),
                Err(prior) => prior == &code,
            });
            if !same {
                self.code = Some(crate::excel::FormatCode::from_code(code.as_str()).map_err(|_| code));
            }
            let format = match self.code.as_ref().expect("the code was resolved") {
                Ok(format) => format,
                Err(_) => return Ok(Operand::Error(ExcelError::Value)),
            };
            if format.is_localized() { return Err(Outcome::Uncomputed(Unevaluated::TextCompatibility)); }
            let scalar = match value {
                Operand::Blank => crate::Scalar::from(0.0),
                Operand::Number(value) => crate::Scalar::from(value),
                Operand::Boolean(value) => crate::Scalar::from(value),
                Operand::Text(text) => match crate::excel::entry::value_number(text.as_str(), system) {
                    Some(Ok(value)) => crate::Scalar::from(value),
                    Some(Err(_)) => crate::Scalar::from(text),
                    None => return Err(Outcome::Uncomputed(Unevaluated::Coercion)),
                },
                Operand::Error(_) => unreachable!("the source error returned above"),
                Operand::Reference(_) => return Err(Outcome::Uncomputed(Unevaluated::Reference)),
            };
            match format.render_formula(&scalar, system) {
                Ok(text) => Ok(Operand::Text(Str::from(text))),
                Err(error) => Ok(Operand::Error(error)),
            }
        };
        match answer() { Ok(value) => Outcome::Computed(value), Err(value) => value }
    }
}

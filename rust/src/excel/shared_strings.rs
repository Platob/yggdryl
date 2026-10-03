//! The shared string table: the one place a workbook's text lives.
//!
//! A string cell states `t="s"` and an index into `sharedStrings.xml`, whose
//! `si` items hold the text - as one `t`, or as rich-text runs `r/t` whose
//! text is concatenated, phonetic runs (`rPh`, a reading guide over East
//! Asian text) left out. Reading resolves every index once into a
//! [`Str`], which a cell then clones without copying; writing interns each
//! distinct text once and answers its index, so a column of a thousand
//! `AAPL` costs one entry.
//!
//! Text crosses the part under ECMA-376's `ST_Xstring`: a character XML 1.0
//! cannot carry, and a carriage return every parser would fold into a line
//! feed, is written as `_xHHHH_`, and a literal `_x` run is escaped so it
//! reads back as itself. Excel writes and reads that grammar; openpyxl
//! writes it and leaves it unread, which is why `_x000D_` is a familiar
//! sight in its output and never in this crate's.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;

use quick_xml::events::Event;
use smol_str::{SmolStr, format_smolstr};

use crate::{Result, Str};

use super::package::{codec_error, local_name, text_piece};

/// The shared strings as read: every item's text, by index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SharedStrings {
    items: Vec<Str>,
}

impl SharedStrings {
    /// Read `sharedStrings.xml`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`](crate::Error::Codec) when the bytes are not
    /// the part.
    pub(crate) fn from_xml(bytes: &[u8]) -> Result<Self> {
        let mut reader = super::styles::reader(bytes);
        let mut buffer = Vec::new();
        let mut items = Vec::new();
        let mut text = String::new();
        let mut in_item = false;
        // Whether the cursor is inside a `t` whose text is the item's - one
        // directly under `si`, or under a rich run `r`.
        let mut in_text = false;
        // Elements deep inside a subtree the text is not read from.
        let mut skipping = 0_usize;
        loop {
            let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| codec_error(position, error.to_string()))?;
            match event {
                Event::Start(ref start) => {
                    let qualified = start.name();
                    let name = local_name(qualified.as_ref());
                    if skipping > 0 {
                        skipping += 1;
                    } else {
                        match name {
                            b"si" => {
                                in_item = true;
                                text.clear();
                            }
                            b"t" if in_item => in_text = true,
                            b"rPh" | b"phoneticPr" if in_item => skipping = 1,
                            _ => {}
                        }
                    }
                }
                Event::Empty(ref start) => {
                    if skipping == 0 && local_name(start.name().as_ref()) == b"si" {
                        items.push(Str::new_static(""));
                    }
                }
                Event::End(end) => {
                    if skipping > 0 {
                        skipping -= 1;
                        continue;
                    }
                    match local_name(end.name().as_ref()) {
                        b"si" => {
                            in_item = false;
                            items.push(Str::new(decode(&text)));
                        }
                        b"t" => in_text = false,
                        _ => {}
                    }
                }
                Event::Text(ref held) if in_text && skipping == 0 => {
                    text.push_str(&text_piece(held.xml10_content(), position)?);
                }
                Event::CData(ref held) if in_text && skipping == 0 => {
                    text.push_str(&text_piece(held.xml10_content(), position)?);
                }
                Event::GeneralRef(_) if in_text && skipping == 0 => {
                    text.push_str(&super::package::reference_text(&event, position)?);
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(Self { items })
    }

    /// The text at `index`, `None` past the table.
    pub(crate) fn get(&self, index: usize) -> Option<&Str> {
        self.items.get(index)
    }

    /// How many items the table holds.
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    /// The text a shared-string cell names: its content read as an index
    /// into the table, or the reason it names none.
    pub(crate) fn resolve(&self, content: &str) -> std::result::Result<&Str, SmolStr> {
        let index = crate::integer::integer_from_text_as::<usize>(content)
            .ok_or_else(|| format_smolstr!("expected a shared string index, got {content:?}"))?;
        self.get(index).ok_or_else(|| {
            format_smolstr!(
                "expected a shared string index below {}, got {index}",
                self.len()
            )
        })
    }
}

/// The shared strings as written: each distinct text once, in first-seen
/// order, and the count of cells that reference one.
#[derive(Debug, Default)]
pub(crate) struct SharedStringTable {
    items: Vec<Str>,
    indexes: HashMap<Str, u32>,
    references: u64,
}

impl SharedStringTable {
    /// An empty table.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A table opening with the strings a package already holds, each at
    /// the index a stored sheet refers to it by; the reference count starts
    /// at zero and counts the cells written over it.
    pub(crate) fn from_existing(strings: &SharedStrings) -> Self {
        let mut table = Self::new();
        for (index, text) in strings.items.iter().enumerate() {
            let index = u32::try_from(index).expect("fewer strings than u32::MAX");
            table.items.push(text.clone());
            table.indexes.entry(text.clone()).or_insert(index);
        }
        table
    }

    /// The index `text` is stored at, adding it on first sight.
    pub(crate) fn intern(&mut self, text: &str) -> u32 {
        self.references += 1;
        if let Some(index) = self.indexes.get(text) {
            return *index;
        }
        let index = u32::try_from(self.items.len()).expect("fewer strings than u32::MAX");
        let held = Str::new(text);
        self.items.push(held.clone());
        self.indexes.insert(held, index);
        index
    }

    /// Write `sharedStrings.xml`: `count` the references, `uniqueCount` the
    /// items, every `t` preserving its whitespace.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure, or the codec's refusal of a character no
    /// escape covers.
    pub(crate) fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        write!(
            writer,
            "<sst xmlns=\"{}\" count=\"{}\" uniqueCount=\"{}\">",
            super::NAMESPACE,
            self.references,
            self.items.len()
        )?;
        for item in &self.items {
            write_item(writer, item)?;
        }
        write!(writer, "</sst>")?;
        Ok(())
    }
}

/// Write one `si` holding `text`, escaped for XML and for `ST_Xstring`.
fn write_item<W: Write>(writer: &mut W, text: &str) -> Result<()> {
    write!(writer, "<si>")?;
    write_text_element(writer, text)?;
    write!(writer, "</si>")?;
    Ok(())
}

/// Write a `t` element holding `text`, whitespace preserved.
///
/// # Errors
///
/// Returns the sink's failure.
pub(crate) fn write_text_element<W: Write>(writer: &mut W, text: &str) -> Result<()> {
    write!(writer, "<t xml:space=\"preserve\">")?;
    crate::xml::write_element_text(writer, &encode(text))?;
    write!(writer, "</t>")?;
    Ok(())
}

/// Escape what a string cell cannot carry: a character XML 1.0 refuses, a
/// carriage return, and a literal `_xHHHH` run - one that would read as an
/// escape as it stands, or once the escape of the character after it is
/// written behind it - each as `_xHHHH_`.
pub(crate) fn encode(text: &str) -> Cow<'_, str> {
    if !text.chars().any(needs_escape) && !has_literal_escape(text) {
        return Cow::Borrowed(text);
    }
    let mut encoded = String::with_capacity(text.len() + 8);
    let mut rest = text;
    while let Some(character) = rest.chars().next() {
        if needs_escape(character) {
            crate::xml::write_x_escape(&mut encoded, character);
        } else if character == '_' && opens_escape(rest) {
            // `_x0041_` spelled literally is written as `_x005F_x0041_`: the
            // underscore escaped, the rest standing. `_x1234` before an
            // escaped character is written the same way, so the escape's own
            // `_` cannot close it.
            encoded.push_str("_x005F_");
        } else {
            encoded.push(character);
        }
        rest = &rest[character.len_utf8()..];
    }
    Cow::Owned(encoded)
}

/// Read a string cell's text: every `_xHHHH_` back into its character.
pub(crate) fn decode(text: &str) -> Cow<'_, str> {
    crate::xml::decode_x_escapes(text)
}

/// Whether `text` starts with `_xHHHH` followed by what closes an escape:
/// a `_` of its own, or a character whose escape opens with one.
fn opens_escape(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 6
        || bytes[0] != b'_'
        || bytes[1] != b'x'
        || !bytes[2..6].iter().all(u8::is_ascii_hexdigit)
    {
        return false;
    }
    text[6..]
        .chars()
        .next()
        .is_some_and(|next| next == '_' || needs_escape(next))
}

/// Whether `text` holds an `_xHHHH_` run anywhere, as written or as it would
/// be written once the character after it is escaped.
fn has_literal_escape(text: &str) -> bool {
    text.match_indices("_x")
        .any(|(at, _)| opens_escape(&text[at..]))
}

/// Whether a character is written as an escape: one XML 1.0 refuses, or a
/// carriage return every parser would fold into a line feed.
fn needs_escape(character: char) -> bool {
    matches!(
        character,
        '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{D}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}'
    )
}

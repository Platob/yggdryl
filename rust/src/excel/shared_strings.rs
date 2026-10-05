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
use std::collections::{HashMap, HashSet};
use std::io::Write;

use quick_xml::events::Event;

use crate::{Result, Str};

use super::package::{codec_error, local_name, text_piece};

/// The shared strings as read: every item's text, by index, and which
/// items a cell of the same text may be written as.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SharedStrings {
    items: Vec<Str>,
    /// Whether each item is plain - one `t`, no run, no phonetic text -
    /// and the first plain item of its text: the one a cell of that text
    /// is written as.
    reusable: Vec<bool>,
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
        let mut plain = Vec::new();
        let mut text = String::new();
        let mut in_item = false;
        // Whether the cursor is inside a `t` whose text is the item's - one
        // directly under `si`, or under a rich run `r`.
        let mut in_text = false;
        // Elements deep inside a subtree the text is not read from.
        let mut skipping = 0_usize;
        // Elements open inside the current item, and whether the item is
        // one `t` directly under `si` and nothing else.
        let mut depth = 0_usize;
        let mut simple = true;
        let mut texts = 0_usize;
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
                        if in_item {
                            depth += 1;
                            if depth == 1 && name == b"t" {
                                texts += 1;
                            } else if depth == 1 {
                                simple = false;
                            }
                        }
                        match name {
                            b"si" => {
                                in_item = true;
                                text.clear();
                                depth = 0;
                                simple = true;
                                texts = 0;
                            }
                            b"t" if in_item => in_text = true,
                            b"rPh" | b"phoneticPr" if in_item => {
                                skipping = 1;
                                depth -= 1;
                            }
                            _ => {}
                        }
                    }
                }
                Event::Empty(ref start) => {
                    if skipping == 0 {
                        let qualified = start.name();
                        let name = local_name(qualified.as_ref());
                        if name == b"si" {
                            items.push(Str::new_static(""));
                            plain.push(false);
                        } else if in_item && depth == 0 {
                            if name == b"t" {
                                texts += 1;
                            } else {
                                simple = false;
                            }
                        }
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
                            plain.push(simple && texts == 1);
                        }
                        b"t" => in_text = false,
                        _ => {}
                    }
                    depth = depth.saturating_sub(1);
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
        // A plain item is reusable when no plain item before it holds its
        // text: the one a cell of that text interns to.
        let mut seen: HashSet<&str> = HashSet::with_capacity(items.len());
        let reusable = items
            .iter()
            .zip(&plain)
            .map(|(item, plain)| *plain && seen.insert(item.as_str()))
            .collect();
        Ok(Self { items, reusable })
    }

    /// The text at `index`, `None` past the table.
    pub(crate) fn get(&self, index: usize) -> Option<&Str> {
        self.items.get(index)
    }

    /// How many items the table holds.
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether a cell holding the text of item `index` is written as that
    /// item by interning its text: the item is plain and the first plain
    /// one of its text. A cell reading any other item keeps its index
    /// beside it.
    pub(crate) fn is_reusable(&self, index: usize) -> bool {
        self.reusable.get(index).copied().unwrap_or(false)
    }
}

/// The shared strings a save writes cells against: the items the package
/// holds, each at its index, and the plain items this save appends.
#[derive(Debug)]
pub(crate) struct SharedStringsWriter<'a> {
    held: &'a SharedStrings,
    /// The index a text interns to: the first plain item of it, built on
    /// the first cell written.
    plain: Option<HashMap<Str, u32>>,
    appended: Vec<Str>,
}

impl<'a> SharedStringsWriter<'a> {
    /// A writer over the items `held` states.
    pub(crate) fn new(held: &'a SharedStrings) -> Self {
        Self {
            held,
            plain: None,
            appended: Vec::new(),
        }
    }

    /// The index a cell holding `text` is written with: `kept`, the item it
    /// was read from, while that item still holds the text; else the first
    /// plain item holding it; else a plain item appended for it.
    pub(crate) fn index(&mut self, text: &str, kept: Option<u32>) -> u32 {
        if let Some(index) = kept.filter(|index| {
            self.held
                .get(*index as usize)
                .is_some_and(|held| held.as_str() == text)
        }) {
            return index;
        }
        let held = self.held;
        let plain = self.plain.get_or_insert_with(|| {
            let mut plain = HashMap::with_capacity(held.items.len());
            for (index, item) in held.items.iter().enumerate() {
                if held.reusable[index] {
                    plain.insert(item.clone(), index as u32);
                }
            }
            plain
        });
        if let Some(index) = plain.get(text) {
            return *index;
        }
        let index = u32::try_from(held.items.len() + self.appended.len())
            .expect("fewer strings than u32::MAX");
        let item = Str::new(text);
        plain.insert(item.clone(), index);
        self.appended.push(item);
        index
    }

    /// Whether the save appended an item.
    pub(crate) fn has_appended(&self) -> bool {
        !self.appended.is_empty()
    }

    /// The table again: `original`, the part as the package stores it, with
    /// every item it holds copied as it stands - rich runs, phonetic text
    /// and duplicates kept - the appended items after them, `uniqueCount`
    /// the items it now holds and `count`, which no save keeps true, gone;
    /// a fresh table where the package held none.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal of a part that is not well-formed, or of
    /// a character no escape covers.
    pub(crate) fn into_part(
        self,
        original: Option<&[u8]>,
        family: super::package::NamespaceFamily,
    ) -> Result<Vec<u8>> {
        let mut items = Vec::new();
        for item in &self.appended {
            write_item(&mut items, item)?;
        }
        let unique = (self.held.len() + self.appended.len()).to_string();
        let Some(original) = original else {
            let mut part = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
                 <sst xmlns=\"{}\" uniqueCount=\"{unique}\">",
                family.namespace()
            )
            .into_bytes();
            part.extend_from_slice(&items);
            part.extend_from_slice(b"</sst>");
            return Ok(part);
        };
        let items = String::from_utf8(items).unwrap_or_default();
        super::package::rewrite(
            original,
            &super::package::Rewrite {
                before_end: &|name| (name == b"sst").then(|| items.clone()),
                attributes: &|name| {
                    if name == b"sst" {
                        vec![("uniqueCount", Some(unique.clone())), ("count", None)]
                    } else {
                        Vec::new()
                    }
                },
                ..super::package::Rewrite::default()
            },
        )
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

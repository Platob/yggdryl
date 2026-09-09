//! A deterministic reader for the small XML documents an object store answers.
//!
//! Two of the three stores answer XML: listings, bulk deletes, multipart
//! uploads, and every failure are small documents of a fixed shape. That is
//! narrow enough for a scanner - elements by local name in document order, the
//! declaration, comments, CDATA, the five named entities and numeric character
//! references - to cover without an XML dependency. Attributes (the `xmlns` on
//! every root) are skipped, namespace prefixes are dropped so names match
//! locally, unknown elements are ignored, and anything malformed is an
//! [`XmlError`].
//!
//! Only the *document vocabulary* differs per store, so that is what each
//! dialect owns: `aws::xml` reads `ListBucketResult`, `azure::xml` reads
//! `EnumerationResults`. This module reads neither, and knows the name of no
//! element.

use std::fmt;

/// Nesting past which a scan stops. The deepest document either store answers
/// is four levels, so this bounds recursion on a hostile body without touching
/// a real one.
const MAX_DEPTH: usize = 32;

/// A body that is not the document it was expected to be: malformed XML, a
/// different root, or a required element missing or unreadable.
#[derive(Debug)]
pub(crate) struct XmlError(pub(crate) String);

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XmlError {}

/// Escape `& < > " '` for element text.
pub(crate) fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[derive(Debug, Default)]
pub(crate) struct Element {
    /// The tag name without any namespace prefix.
    name: String,
    /// The element's own character data, children excluded.
    text: String,
    /// Child elements in document order.
    children: Vec<Element>,
}

impl Element {
    /// The tag name, without any namespace prefix.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The first child named `name`.
    pub(crate) fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Every child named `name`, in document order.
    pub(crate) fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// The text of the first child named `name`.
    pub(crate) fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|child| child.text.as_str())
    }

    /// The text of the first child named `name`, whose absence is an error.
    pub(crate) fn required(&self, name: &str) -> Result<&str, XmlError> {
        self.child_text(name)
            .ok_or_else(|| XmlError(format!("<{}> without <{name}>", self.name)))
    }
}

/// The root of `xml`, which must be named `expected`.
pub(crate) fn parse_root(xml: &[u8], expected: &str) -> Result<Element, XmlError> {
    let root = parse_document(xml)?;
    if root.name == expected {
        Ok(root)
    } else {
        Err(XmlError(format!(
            "expected <{expected}>, found <{}>",
            root.name
        )))
    }
}

/// The root element of `xml`: prolog and trailing comments allowed, a byte
/// order mark tolerated, anything else outside the root refused.
pub(crate) fn parse_document(xml: &[u8]) -> Result<Element, XmlError> {
    let text = std::str::from_utf8(xml).map_err(|error| XmlError(format!("not UTF-8: {error}")))?;
    let mut scanner = Scanner {
        text: text.strip_prefix('\u{FEFF}').unwrap_or(text),
        at: 0,
    };
    scanner.skip_misc()?;
    if !scanner.starts_with("<") {
        return Err(scanner.error("expected the root element"));
    }
    let root = scanner.element(0)?;
    scanner.skip_misc()?;
    if scanner.at < scanner.text.len() {
        return Err(scanner.error("content after the root element"));
    }
    Ok(root)
}

/// The cursor of one scan; `at` is always on a character boundary because
/// it only ever lands after ASCII markup.
struct Scanner<'a> {
    /// The whole document.
    text: &'a str,
    /// The byte offset of the next unread character.
    at: usize,
}

impl<'a> Scanner<'a> {
    /// What is left to scan.
    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn starts_with(&self, token: &str) -> bool {
        self.rest().starts_with(token)
    }

    /// An error naming where the scan stopped.
    fn error(&self, message: impl fmt::Display) -> XmlError {
        XmlError(format!("{message} at byte {}", self.at))
    }

    /// Advance past `token` or fail.
    fn expect(&mut self, token: &str) -> Result<(), XmlError> {
        if !self.starts_with(token) {
            return Err(self.error(format!("expected `{token}`")));
        }
        self.at += token.len();
        Ok(())
    }

    /// The text up to the next `marker`, which is consumed too.
    fn take_until(&mut self, marker: &str, what: &str) -> Result<&'a str, XmlError> {
        let rest = self.rest();
        let end = rest
            .find(marker)
            .ok_or_else(|| self.error(format!("unterminated {what}")))?;
        self.at += end + marker.len();
        Ok(&rest[..end])
    }

    fn skip_whitespace(&mut self) {
        let skipped = self
            .rest()
            .bytes()
            .take_while(|byte| is_space(*byte))
            .count();
        self.at += skipped;
    }

    /// Skip whitespace, comments, and processing instructions (the XML
    /// declaration is one), which may surround the root.
    fn skip_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_whitespace();
            if self.starts_with("<?") {
                self.at += 2;
                self.take_until("?>", "processing instruction")?;
            } else if self.starts_with("<!--") {
                self.at += 4;
                self.take_until("-->", "comment")?;
            } else {
                return Ok(());
            }
        }
    }

    /// The name under the cursor: everything up to whitespace or markup.
    fn name(&mut self) -> Result<&'a str, XmlError> {
        let rest = self.rest();
        let end = rest
            .bytes()
            .position(|byte| {
                is_space(byte) || matches!(byte, b'/' | b'>' | b'<' | b'=' | b'"' | b'\'')
            })
            .unwrap_or(rest.len());
        if end == 0 {
            return Err(self.error("expected a name"));
        }
        self.at += end;
        Ok(&rest[..end])
    }

    /// Skip one `name="value"` pair. The value is skipped as a quoted unit so
    /// a `>` inside it does not end the tag.
    fn attribute(&mut self) -> Result<(), XmlError> {
        self.name()?;
        self.skip_whitespace();
        self.expect("=")?;
        self.skip_whitespace();
        let quote = match self.rest().as_bytes().first() {
            Some(b'"') => "\"",
            Some(b'\'') => "'",
            _ => return Err(self.error("expected a quoted attribute value")),
        };
        self.at += 1;
        self.take_until(quote, "attribute value")?;
        Ok(())
    }

    /// The element whose `<` is under the cursor, with everything inside it.
    fn element(&mut self, depth: usize) -> Result<Element, XmlError> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("elements nested deeper than {MAX_DEPTH}")));
        }
        self.at += 1;
        let full_name = self.name()?;
        let mut element = Element {
            name: full_name.rsplit(':').next().unwrap_or(full_name).to_owned(),
            ..Element::default()
        };
        loop {
            self.skip_whitespace();
            if self.starts_with("/>") {
                self.at += 2;
                return Ok(element);
            }
            if self.starts_with(">") {
                self.at += 1;
                break;
            }
            self.attribute()?;
        }
        loop {
            if self.starts_with("</") {
                self.at += 2;
                let closing = self.name()?;
                if closing != full_name {
                    return Err(self.error(format!("<{full_name}> closed by </{closing}>")));
                }
                self.skip_whitespace();
                self.expect(">")?;
                return Ok(element);
            }
            if self.starts_with("<![CDATA[") {
                self.at += 9;
                element
                    .text
                    .push_str(self.take_until("]]>", "CDATA section")?);
            } else if self.starts_with("<!--") {
                self.at += 4;
                self.take_until("-->", "comment")?;
            } else if self.starts_with("<?") {
                self.at += 2;
                self.take_until("?>", "processing instruction")?;
            } else if self.starts_with("<") {
                let child = self.element(depth + 1)?;
                element.children.push(child);
            } else if self.rest().is_empty() {
                return Err(self.error(format!("<{full_name}> never closed")));
            } else {
                let rest = self.rest();
                let end = rest.find('<').unwrap_or(rest.len());
                decode_entities(&rest[..end], &mut element.text)
                    .map_err(|message| self.error(message))?;
                self.at += end;
            }
        }
    }
}

/// XML's own whitespace set, narrower than ASCII's.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// Append `raw` character data to `out` with the five named entities and
/// numeric character references resolved. Anything else after `&` is the
/// error text.
fn decode_entities(raw: &str, out: &mut String) -> Result<(), String> {
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find(';').ok_or_else(|| {
            format!(
                "unterminated reference `&{}`",
                after.chars().take(8).collect::<String>()
            )
        })?;
        let reference = &after[..end];
        let character = match reference {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => character_reference(reference),
        };
        out.push(character.ok_or_else(|| format!("unknown reference `&{reference};`"))?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(())
}

/// The character `#NNN` or `#xHH` names, when it names one XML allows.
fn character_reference(reference: &str) -> Option<char> {
    let (digits, radix) = match reference.strip_prefix("#x") {
        Some(hex) => (hex, 16),
        None => (reference.strip_prefix('#')?, 10),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| char::from(byte).is_digit(radix)) {
        return None;
    }
    u32::from_str_radix(digits, radix)
        .ok()
        .and_then(char::from_u32)
        .filter(|character| *character != '\0')
}

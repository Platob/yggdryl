//! Web linking, RFC 8288: the `Link` header field, one or more link values,
//! each a target in angle brackets with its parameters - which is how a
//! paginating API says where the next page is.

use std::borrow::Cow;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

/// One link value of a `Link` header: `<target>; rel="next"; title="x"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    /// The URI reference between the angle brackets, as written: absolute or
    /// relative to the message's URL.
    pub target: String,
    /// The relation types of the first `rel` parameter, split at whitespace,
    /// as written; RFC 8288 compares them ignoring case, and so does
    /// [`Self::has_rel`].
    pub rel: Vec<SmolStr>,
    /// Every other parameter, in order, the name lower case and the value
    /// unquoted; a parameter written without a value holds the empty string.
    pub parameters: Vec<(SmolStr, String)>,
}

impl Link {
    /// Whether `rel` is one of this link's relation types, compared ignoring
    /// case as RFC 8288 section 2.1 asks.
    pub fn has_rel(&self, rel: &str) -> bool {
        self.rel.iter().any(|known| known.eq_ignore_ascii_case(rel))
    }

    /// The value of the parameter `name`, compared ignoring case.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Parse a `Link` header value into its link values.
///
/// Several `Link` fields joined with a comma parse as one list, as RFC 9110
/// section 5.3 lets a receiver join them. A `rel` parameter after the first
/// is ignored, as RFC 8288 section 3.3 asks; every other parameter is kept.
///
/// ```
/// use yggdryl::http::parse_links;
///
/// # fn main() -> yggdryl::Result<()> {
/// let links = parse_links(
///     r#"<https://api.example/items?page=2>; rel="next", <https://api.example/items?page=9>; rel="last""#,
/// )?;
/// assert_eq!(links.len(), 2);
/// assert!(links[0].has_rel("next"));
/// assert_eq!(links[1].target, "https://api.example/items?page=9");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::Parse`] with `target` `http header` and the byte
/// position of a link value that opens with anything but `<`, a target
/// never closed by `>`, a parameter that is not a token, or a quoted value
/// never closed.
pub fn parse_links(value: &str) -> Result<Vec<Link>> {
    parse_raw_links(value).map(|links| links.into_iter().map(Link::from).collect())
}

/// One link value borrowed out of the header text: what [`Link`] is built
/// from, and what a reader wanting one target reads without building any.
pub(crate) struct RawLink<'a> {
    pub(crate) target: &'a str,
    /// Every parameter in order, the name as written and the value unquoted.
    pub(crate) parameters: Vec<(&'a str, Cow<'a, str>)>,
}

impl RawLink<'_> {
    /// The first `rel` parameter's value, which is the one RFC 8288 reads.
    fn rel(&self) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("rel"))
            .map(|(_, value)| value.as_ref())
    }

    /// Whether `rel` is one of the link's relation types, ignoring case.
    pub(crate) fn has_rel(&self, rel: &str) -> bool {
        self.rel().is_some_and(|value| {
            value
                .split_ascii_whitespace()
                .any(|known| known.eq_ignore_ascii_case(rel))
        })
    }
}

impl From<RawLink<'_>> for Link {
    fn from(raw: RawLink<'_>) -> Self {
        let rel = raw
            .rel()
            .map(|value| value.split_ascii_whitespace().map(SmolStr::new).collect())
            .unwrap_or_default();
        let parameters = raw
            .parameters
            .into_iter()
            .filter(|(name, _)| !name.eq_ignore_ascii_case("rel"))
            .map(|(name, value)| (SmolStr::new(name.to_ascii_lowercase()), value.into_owned()))
            .collect();
        Self {
            target: raw.target.to_owned(),
            rel,
            parameters,
        }
    }
}

/// Parse the link values of a `Link` header without copying a target.
pub(crate) fn parse_raw_links(value: &str) -> Result<Vec<RawLink<'_>>> {
    let mut links = Vec::new();
    let mut cursor = Cursor {
        text: value,
        position: 0,
    };
    loop {
        cursor.skip_ows();
        match cursor.peek() {
            None => return Ok(links),
            // An empty list element, which RFC 9110 section 5.6.1 ignores.
            Some(b',') => {
                cursor.position += 1;
                continue;
            }
            Some(b'<') => {}
            Some(_) => return Err(cursor.refuse("expected a <target>")),
        }
        cursor.position += 1;
        let target_start = cursor.position;
        let Some(length) = value[target_start..].find('>') else {
            return Err(cursor.refuse_at(target_start - 1, "expected the target closed by >"));
        };
        cursor.position = target_start + length + 1;
        let target = &value[target_start..target_start + length];
        let mut parameters = Vec::new();
        loop {
            cursor.skip_ows();
            match cursor.peek() {
                Some(b';') => {
                    cursor.position += 1;
                    cursor.skip_ows();
                    parameters.push(cursor.parameter()?);
                }
                Some(b',') => {
                    cursor.position += 1;
                    break;
                }
                None => break,
                Some(_) => return Err(cursor.refuse("expected ; , or the end of the link")),
            }
        }
        links.push(RawLink { target, parameters });
    }
}

struct Cursor<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.position).copied()
    }

    fn skip_ows(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.position += 1;
        }
    }

    fn refuse(&self, reason: &str) -> Error {
        self.refuse_at(self.position, reason)
    }

    fn refuse_at(&self, position: usize, reason: &str) -> Error {
        Error::Parse {
            target: "http header",
            position,
            reason: format_smolstr!(
                "Link: {reason}, got {:?}",
                crate::text::elide_to(self.text, 64)
            ),
        }
    }

    /// `token BWS [ "=" BWS ( token / quoted-string ) ]`.
    fn parameter(&mut self) -> Result<(&'a str, Cow<'a, str>)> {
        let name = self.token()?;
        self.skip_ows();
        if self.peek() != Some(b'=') {
            return Ok((name, Cow::Borrowed("")));
        }
        self.position += 1;
        self.skip_ows();
        let value = if self.peek() == Some(b'"') {
            self.quoted_string()?
        } else {
            Cow::Borrowed(self.token()?)
        };
        Ok((name, value))
    }

    fn token(&mut self) -> Result<&'a str> {
        let start = self.position;
        while self.peek().is_some_and(is_token_byte) {
            self.position += 1;
        }
        if self.position == start {
            return Err(self.refuse("expected a parameter name"));
        }
        Ok(&self.text[start..self.position])
    }

    /// `DQUOTE *( qdtext / quoted-pair ) DQUOTE`, the pairs unescaped only
    /// when there is one.
    fn quoted_string(&mut self) -> Result<Cow<'a, str>> {
        let open = self.position;
        self.position += 1;
        let start = self.position;
        loop {
            let Some(character) = self.text[self.position..].chars().next() else {
                return Err(self.refuse_at(open, "expected the quoted value closed"));
            };
            match character {
                '"' => {
                    let raw = &self.text[start..self.position];
                    self.position += 1;
                    return Ok(Cow::Borrowed(raw));
                }
                '\\' => {
                    let mut text = self.text[start..self.position].to_owned();
                    self.position += 1;
                    let Some(escaped) = self.text[self.position..].chars().next() else {
                        return Err(self.refuse_at(open, "expected the quoted value closed"));
                    };
                    self.position += escaped.len_utf8();
                    text.push(escaped);
                    return self.quoted_tail(open, text);
                }
                character => self.position += character.len_utf8(),
            }
        }
    }

    /// The rest of a quoted string once one pair has forced a copy.
    fn quoted_tail(&mut self, open: usize, mut text: String) -> Result<Cow<'a, str>> {
        loop {
            let Some(character) = self.text[self.position..].chars().next() else {
                return Err(self.refuse_at(open, "expected the quoted value closed"));
            };
            self.position += character.len_utf8();
            match character {
                '"' => return Ok(Cow::Owned(text)),
                '\\' => {
                    let Some(escaped) = self.text[self.position..].chars().next() else {
                        return Err(self.refuse_at(open, "expected the quoted value closed"));
                    };
                    self.position += escaped.len_utf8();
                    text.push(escaped);
                }
                character => text.push(character),
            }
        }
    }
}

const fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

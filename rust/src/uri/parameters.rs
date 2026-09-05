//! The `key=value` pairs a URI query carries.
//!
//! A query is one text component, so this is the view that reads it as the
//! pairs it spells: `?symbol=AAPL&venue=XNAS` addressed by key, iterated in the
//! order it was written, and written back as one component.
//!
//! Reads borrow the query rather than copying it. Only a pair a caller changed,
//! or one whose escapes were decoded, owns its text.

use std::borrow::Cow;

use smol_str::SmolStr;

use super::parser::{is_query_fragment_byte, percent_decode, percent_encode, validate_component};
use crate::Result;

/// Whether one byte may stand for itself inside a pair's key or value.
///
/// The query charset minus the three bytes that would end the key or the pair.
/// `+` is encoded as well: RFC 3986 reads it as a literal plus, HTML form
/// decoding reads it as a space, and a value that survives both is worth the
/// three bytes.
const fn is_parameter_byte(byte: u8) -> bool {
    is_query_fragment_byte(byte) && !matches!(byte, b'&' | b'=' | b'+')
}

/// One query, addressed as its pairs.
///
/// Built by [`Uri::parameters`](crate::Uri::parameters) and written back by
/// [`Uri::set_parameters`](crate::Uri::set_parameters), so a caller reads and
/// edits pairs without ever spelling the query syntax.
///
/// `decode` chooses which text the view speaks. A decoding view answers with
/// the text the escapes stand for and encodes what it is given; a raw view
/// answers with the query's own bytes and takes them as written, refusing text
/// the query syntax cannot carry.
///
/// Duplicate keys are kept: a query may name one key more than once, so
/// [`get`](Self::get) answers with the first, [`get_all`](Self::get_all) with
/// every one, [`insert`](Self::insert) replaces the first and drops the rest,
/// and [`append`](Self::append) adds another.
///
/// ```
/// use yggdryl::Url;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut url = Url::from_str("https://example.com/trades?symbol=AAPL&venue=XNAS")?;
///
/// // The view borrows the URL, so owning its pairs is what lets the edit be
/// // written back to that same URL.
/// let mut parameters = url.parameters(true)?.into_owned();
/// assert_eq!(parameters.get("symbol"), Some("AAPL"));
///
/// parameters.insert("symbol", "MSFT")?;
/// parameters.remove("venue");
/// parameters.append("as of", "2026-01-02 09:30")?;
/// url.set_parameters(&parameters)?;
///
/// // Writing encodes what the query syntax cannot carry literally.
/// assert_eq!(
///     url.to_string(),
///     "https://example.com/trades?symbol=MSFT&as%20of=2026-01-02%2009:30"
/// );
/// assert_eq!(url.parameters(true)?.get("as of"), Some("2026-01-02 09:30"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameters<'uri> {
    pairs: Vec<(Cow<'uri, str>, Cow<'uri, str>)>,
    decoded: bool,
}

impl<'uri> Parameters<'uri> {
    /// Read one query's pairs, decoding their escapes when asked.
    ///
    /// A pair carrying no `=` has an empty value, and empty pairs - the `&&` in
    /// `a=1&&b=2` - are not pairs at all.
    ///
    /// # Errors
    ///
    /// Returns a parse error when `decode` is set and an escape does not stand
    /// for UTF-8.
    pub fn parse(query: &'uri str, decode: bool) -> Result<Self> {
        let mut pairs = Vec::new();
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            pairs.push(if decode {
                (
                    percent_decode(key, "uri query key")?,
                    percent_decode(value, "uri query value")?,
                )
            } else {
                (Cow::Borrowed(key), Cow::Borrowed(value))
            });
        }
        Ok(Self {
            pairs,
            decoded: decode,
        })
    }

    /// Own every pair, releasing the borrow on the query behind them.
    ///
    /// A view borrows the URI it was read from, so this is what lets an edited
    /// view be written back to that same URI:
    /// [`set_parameters`](crate::Uri::set_parameters) needs the URI mutably,
    /// which a borrowing view still holds.
    #[must_use]
    pub fn into_owned(self) -> Parameters<'static> {
        Parameters {
            pairs: self
                .pairs
                .into_iter()
                .map(|(key, value)| (Cow::Owned(key.into_owned()), Cow::Owned(value.into_owned())))
                .collect(),
            decoded: self.decoded,
        }
    }

    /// Return whether this view speaks decoded text.
    pub const fn decoded(&self) -> bool {
        self.decoded
    }

    /// Return how many pairs the query holds, counting repeated keys once each.
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Return whether the query holds no pair at all.
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Return whether any pair names `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.position(key).is_some()
    }

    /// Borrow the first value named `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.position(key).map(|index| self.pairs[index].1.as_ref())
    }

    /// Borrow every value named `key`, in the order the query spells them.
    pub fn get_all<'view>(&'view self, key: &'view str) -> impl Iterator<Item = &'view str> {
        self.pairs
            .iter()
            .filter(move |(name, _)| name == key)
            .map(|(_, value)| value.as_ref())
    }

    /// Borrow every pair, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()))
    }

    /// Borrow every key, in order, repeated keys included.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.pairs.iter().map(|(key, _)| key.as_ref())
    }

    /// Borrow every value, in order.
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.pairs.iter().map(|(_, value)| value.as_ref())
    }

    /// Set `key` to `value`, returning the value it replaced.
    ///
    /// The first pair named `key` keeps its position and every later one is
    /// dropped, so a key names one value afterwards. A key the query does not
    /// hold is appended.
    ///
    /// # Errors
    ///
    /// On a raw view, returns a parse error when `key` or `value` is text the
    /// query syntax cannot carry. A decoding view encodes instead of refusing.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<Option<String>> {
        let (key, value) = self.accepted(key, value)?;
        let Some(index) = self.position(&key) else {
            self.pairs.push((Cow::Owned(key), Cow::Owned(value)));
            return Ok(None);
        };
        let replaced = std::mem::replace(&mut self.pairs[index].1, Cow::Owned(value)).into_owned();
        let mut position = 0;
        self.pairs.retain(|(name, _)| {
            position += 1;
            position - 1 == index || name != &key
        });
        Ok(Some(replaced))
    }

    /// Add another pair named `key`, keeping the pairs already there.
    ///
    /// # Errors
    ///
    /// As [`insert`](Self::insert).
    pub fn append(&mut self, key: &str, value: &str) -> Result<()> {
        let (key, value) = self.accepted(key, value)?;
        self.pairs.push((Cow::Owned(key), Cow::Owned(value)));
        Ok(())
    }

    /// Remove every pair named `key`, returning the first value removed.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        let index = self.position(key)?;
        let removed = self.pairs.remove(index).1.into_owned();
        self.pairs.retain(|(name, _)| name != key);
        Some(removed)
    }

    /// Drop every pair, leaving a query that spells nothing.
    pub fn clear(&mut self) {
        self.pairs.clear();
    }

    /// Spell these pairs as one query component, or `None` when there is none.
    ///
    /// A decoding view encodes each key and value; a raw view already holds
    /// the query's own text and joins it unchanged.
    pub fn to_query(&self) -> Option<SmolStr> {
        if self.pairs.is_empty() {
            return None;
        }
        let mut query = String::new();
        for (key, value) in &self.pairs {
            if !query.is_empty() {
                query.push('&');
            }
            query.push_str(&self.written(key));
            query.push('=');
            query.push_str(&self.written(value));
        }
        Some(SmolStr::from(query))
    }

    /// Return the index of the first pair named `key`.
    fn position(&self, key: &str) -> Option<usize> {
        self.pairs.iter().position(|(name, _)| name == key)
    }

    /// Take one pair's text as this view speaks it, refusing what it cannot.
    fn accepted(&self, key: &str, value: &str) -> Result<(String, String)> {
        if !self.decoded {
            validate_component(key, "uri query key", 0, is_query_fragment_byte)?;
            validate_component(value, "uri query value", 0, is_query_fragment_byte)?;
            for (text, target) in [(key, "uri query key"), (value, "uri query value")] {
                if let Some(position) = text.bytes().position(|byte| matches!(byte, b'&' | b'=')) {
                    return Err(super::parser::parse_error(
                        target,
                        position,
                        "a raw pair must not carry the & or = that separates pairs",
                    ));
                }
            }
        }
        Ok((key.to_owned(), value.to_owned()))
    }

    /// Spell one pair's text the way the query component carries it.
    fn written<'text>(&self, text: &'text str) -> Cow<'text, str> {
        if self.decoded {
            percent_encode(text, is_parameter_byte)
        } else {
            Cow::Borrowed(text)
        }
    }
}

impl<'view, 'uri> IntoIterator for &'view Parameters<'uri> {
    type Item = (&'view str, &'view str);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'view, (Cow<'uri, str>, Cow<'uri, str>)>,
        fn(&'view (Cow<'uri, str>, Cow<'uri, str>)) -> (&'view str, &'view str),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.pairs
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()))
    }
}

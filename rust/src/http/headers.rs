//! `Headers`: an HTTP field section, RFC 9110 section 5, over the crate's
//! own [`Metadata`].
//!
//! Every header is one `HTTP:<name>` property of the wrapped snapshot, the
//! name folded to lower case and the name and the value validated where
//! every other `HTTP:` metadata entry already is (`rust/src/metadata/`), so
//! no map, no fold and no validation is written a second time. What this
//! file adds is the HTTP contract: a repeated name joins into one field
//! value, `Set-Cookie` excepted, and each header a reader branches on parses
//! once into the type it is - a length, a range, a validator, an instant, a
//! list of links, a pause.

use std::fmt;
use std::ops::Index;
use std::str::FromStr;
use std::time::Duration;

use serde::de::{Error as DeError, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::hashing::stable_hash_display;
use crate::metadata::{MetadataIntoIter, PropertyIter, parse_content_length, write_json_string};
use crate::{Charset, Codec, Error, MediaType, Metadata, MimeType, Result, Scheme};

mod date;
mod etag;
mod link;
mod range;

pub use date::{parse_http_date, render_http_date};
pub use etag::ETag;
pub use link::{Link, parse_links};
pub use range::{ContentRange, range_header};

/// The one scheme every header is filed under; a static so the property
/// iterator borrowing it can be handed out.
static HTTP: Scheme = Scheme::HTTP;

/// The prefix every stored key carries.
const PREFIX: &str = "HTTP:";

/// The one field RFC 9110 section 5.3 forbids joining with a comma: its
/// members hold commas of their own (an `Expires` date), so they are joined
/// with a newline here and split back by [`Headers::get_all`].
const SET_COOKIE: &str = "set-cookie";

/// The separator a repeated field joins with.
const LIST_SEPARATOR: &str = ", ";

/// The separator repeated `Set-Cookie` fields join with.
const COOKIE_SEPARATOR: char = '\n';

/// A stack buffer wide enough for every field name in use; a longer one
/// allocates its key.
const STACK_KEY: usize = 128;

/// An HTTP field section: header names and values, the names folded to
/// lower case, stored as the `HTTP:` properties of one [`Metadata`].
///
/// ```
/// use yggdryl::http::Headers;
///
/// # fn main() -> yggdryl::Result<()> {
/// let headers = Headers::from_entries([
///     ("Content-Type", "application/json; charset=utf-8"),
///     ("Content-Length", "42"),
///     ("Vary", "Accept"),
///     ("Vary", "Accept-Encoding"),
/// ])?;
///
/// assert_eq!(headers.get("content-type"), Some("application/json; charset=utf-8"));
/// assert_eq!(headers.content_length()?, Some(42));
/// assert_eq!(headers.get("vary"), Some("Accept, Accept-Encoding"));
/// assert_eq!(headers.get_all("VARY").collect::<Vec<_>>(), ["Accept", "Accept-Encoding"]);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Headers(Metadata);

impl Headers {
    /// The most codings [`Self::content_encoding`] reads in one chain.
    pub const MAX_CODINGS: usize = 5;

    /// The empty section, sharing the empty metadata map: no allocation.
    pub fn new() -> Self {
        Self(Metadata::new())
    }

    /// Build a section from `(name, value)` pairs, in order.
    ///
    /// A name repeated joins its values with `, ` into one field value, as
    /// RFC 9110 section 5.3 lets a receiver do; `Set-Cookie` repeated joins
    /// with a newline instead, and [`Self::get_all`] splits it back.
    ///
    /// ```
    /// use yggdryl::http::Headers;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let headers = Headers::from_entries([
    ///     ("Accept", "text/html"),
    ///     ("Accept", "application/json"),
    ///     ("Set-Cookie", "session=1; Path=/"),
    ///     ("Set-Cookie", "theme=dark"),
    /// ])?;
    ///
    /// assert_eq!(headers.get("accept"), Some("text/html, application/json"));
    /// assert_eq!(headers.set_cookies(), ["session=1; Path=/", "theme=dark"]);
    /// assert_eq!(headers.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with `target` `http header` for a name that is
    /// not an RFC 9110 token (empty, or holding a space, a colon or a control
    /// byte), a value holding CR, LF, NUL, DEL or a control other than HTAB,
    /// or a `Content-Length` that is not an unsigned decimal integer.
    pub fn from_entries<I, K, V>(pairs: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut headers = Self::new();
        for (name, value) in pairs {
            headers.append(name.as_ref(), value.as_ref())?;
        }
        Ok(headers)
    }

    /// The `HTTP:` properties of `metadata`, and nothing else it holds.
    ///
    /// Every entry was validated when the snapshot took it, so nothing is
    /// checked again; a snapshot holding nothing but headers is kept as it
    /// is, sharing its map.
    pub fn from_metadata(metadata: Metadata) -> Self {
        if metadata.iter().all(|(key, _)| key.starts_with(PREFIX)) {
            return Self(metadata);
        }
        let mut kept = Metadata::new();
        for (name, value) in metadata.property_iter(&HTTP) {
            kept.insert_validated(stored_key(name), value.to_owned());
        }
        Self(kept)
    }

    /// Parse the JSON object [`fmt::Display`] renders: bare names to values.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Json`] when the text is not such an object, or when a
    /// name or value fails what [`Self::from_entries`] refuses.
    pub fn from_json(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(Error::from)
    }

    /// The value of the header `name`, compared ignoring case.
    ///
    /// One exact lookup in the ordered map, the key folded in a stack buffer
    /// and nothing allocated for a name of ordinary length.
    ///
    /// ```
    /// use yggdryl::http::Headers;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let headers = Headers::from_entries([("ETag", "\"v1\"")])?;
    ///
    /// assert_eq!(headers.get("etag"), Some("\"v1\""));
    /// assert_eq!(headers.get("ETAG"), Some("\"v1\""));
    /// assert_eq!(headers.get("last-modified"), None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn get(&self, name: &str) -> Option<&str> {
        with_key(name, |key| self.0.get(key))
    }

    /// The members of the list header `name`: the field value split at the
    /// commas outside quoted strings, each trimmed of surrounding whitespace,
    /// empty members dropped - or, for `Set-Cookie`, one member per stored
    /// cookie. A header that is not a list (a `Date`) is not meant for this
    /// and splits at its own commas.
    pub fn get_all(&self, name: &str) -> impl Iterator<Item = &str> {
        Members {
            rest: self.get(name).unwrap_or_default(),
            cookies: is_set_cookie(name),
        }
    }

    /// Whether a header `name` is present, compared ignoring case.
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The number of distinct header names.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no header is present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The headers as `(name, value)` in lexical order of the folded name,
    /// borrowing and allocating nothing.
    pub fn iter(&self) -> HeadersIter<'_> {
        HeadersIter(self.0.property_iter(&HTTP))
    }

    /// The first header after `after` in lexical order, or the first of all
    /// for `None`: the cursor an owning iterator advances by.
    pub fn next_entry(&self, after: Option<&str>) -> Option<(&str, &str)> {
        self.0.next_property_entry(&HTTP, after)
    }

    /// Set the header `name` to `value`, answering the value it replaced.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with `target` `http header` for what
    /// [`Self::from_entries`] refuses.
    pub fn insert(&mut self, name: &str, value: &str) -> Result<Option<String>> {
        self.insert_owned(name, value.to_owned())
    }

    /// Add `value` to the header `name`: set when absent, joined with `, `
    /// after the present value otherwise - with a newline for `Set-Cookie`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with `target` `http header` for what
    /// [`Self::from_entries`] refuses.
    pub fn append(&mut self, name: &str, value: &str) -> Result<()> {
        let joined = match self.get(name) {
            Some(present) if is_set_cookie(name) => {
                format!("{present}{COOKIE_SEPARATOR}{value}")
            }
            Some(present) => format!("{present}{LIST_SEPARATOR}{value}"),
            None => value.to_owned(),
        };
        self.insert_owned(name, joined).map(drop)
    }

    /// Take the header `name` out, answering its value.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        with_key(name, |key| self.0.remove(key))
    }

    /// The union of this section and `other`, this one winning a name both
    /// carry.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with `target` `http header` when an entry
    /// fails the validation a write takes, which two sections built through
    /// these doors cannot.
    pub fn merge_with(&self, other: &Self) -> Result<Self> {
        let mut merged = self.clone();
        for (name, value) in other {
            if !merged.contains_key(name) {
                merged.insert(name, value)?;
            }
        }
        Ok(merged)
    }

    /// Drop every header.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// The metadata snapshot the headers are stored in, every key `HTTP:`.
    pub fn as_metadata(&self) -> &Metadata {
        &self.0
    }

    /// The metadata snapshot the headers are stored in, handed over.
    pub fn into_metadata(self) -> Metadata {
        self.0
    }

    /// The JSON object [`fmt::Display`] renders, as an owned string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Json`] when serialization fails, which a section of
    /// validated strings does not.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }

    /// The cross-language stable hash of the canonical display.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }

    // The typed readers: each parses one header, `Ok(None)` when it is
    // absent and an error when it is present and malformed.

    /// `Content-Length` as the byte count it states.
    ///
    /// # Errors
    ///
    /// Every write canonicalizes the header, so an error can only come from
    /// a snapshot corrupted after the fact.
    pub fn content_length(&self) -> Result<Option<u64>> {
        self.get("content-length")
            .map(parse_content_length)
            .transpose()
            .map_err(|error| header_error("Content-Length", error))
    }

    /// `Content-Type` as written, parameters included.
    pub fn content_type(&self) -> Option<&str> {
        self.get("content-type")
    }

    /// The base MIME type of `Content-Type`, `application/octet-stream` when
    /// absent.
    ///
    /// # Errors
    ///
    /// Returns an error when a present `Content-Type` is not MIME syntax.
    pub fn mime_type(&self) -> Result<MimeType> {
        self.content_type()
            .map(MimeType::from_content_type)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// `Content-Type`, its `charset` parameter and `Content-Encoding` as one
    /// media value, through [`MediaType::from_content_headers`].
    ///
    /// # Errors
    ///
    /// Returns an error for malformed MIME syntax, an unknown charset or a
    /// content coding no MIME type names.
    pub fn media_type(&self) -> Result<MediaType> {
        MediaType::from_content_headers(self.content_type(), self.get("content-encoding"))
    }

    /// The `charset` parameter of `Content-Type`; `None` is the header
    /// declaring none, not UTF-8.
    ///
    /// # Errors
    ///
    /// Returns an error when the parameter names no known charset.
    pub fn charset(&self) -> Result<Option<Charset>> {
        self.content_type()
            .map_or(Ok(None), Charset::from_content_type)
    }

    /// `Content-Encoding` as the codings this crate decodes, in the order
    /// they were applied; `[Identity]` when the header is absent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with `target` `http header` for a coding this
    /// crate cannot decode (`br`, `compress`), a token no coding has, or more
    /// than [`Self::MAX_CODINGS`] codings: each is a decoder stacked on the
    /// body, and no sender applies that many.
    pub fn content_encoding(&self) -> Result<Vec<Codec>> {
        let Some(value) = self.get("content-encoding") else {
            return Ok(vec![Codec::Identity]);
        };
        let mut codecs = Vec::new();
        for member in list_members(value) {
            if codecs.len() == Self::MAX_CODINGS {
                return Err(refuse(
                    "Content-Encoding",
                    &format!("more than {} codings", Self::MAX_CODINGS),
                    value,
                ));
            }
            if member.eq_ignore_ascii_case(Codec::Identity.as_str()) {
                codecs.push(Codec::Identity);
                continue;
            }
            // The MIME registry knows every coding HTTP names; the codec
            // vocabulary is the ones this crate decodes, and a coding the
            // first has and the second has not is named here.
            let codec = MimeType::from_content_coding(member)
                .ok()
                .map(|mime| Codec::from_mime_type(&mime))
                .filter(|codec| *codec != Codec::Identity)
                .ok_or_else(|| {
                    refuse(
                        "Content-Encoding",
                        &format!("cannot decode the coding {member:?}"),
                        value,
                    )
                })?;
            codecs.push(codec);
        }
        if codecs.is_empty() {
            return Err(refuse(
                "Content-Encoding",
                "expected at least one coding",
                value,
            ));
        }
        Ok(codecs)
    }

    /// `Content-Range` parsed.
    ///
    /// ```
    /// use yggdryl::http::{ContentRange, Headers};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let headers = Headers::from_entries([("Content-Range", "bytes 0-1023/4096")])?;
    /// let range = headers.content_range()?;
    ///
    /// assert_eq!(range, Some(ContentRange::Bytes { start: 0, end: 1023, total: Some(4096) }));
    /// assert_eq!(range.and_then(|range| range.total()), Some(4096));
    ///
    /// let unsatisfied = Headers::from_entries([("Content-Range", "bytes */4096")])?;
    /// assert_eq!(unsatisfied.content_range()?, Some(ContentRange::Unsatisfied { total: 4096 }));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for what [`ContentRange::from_str`] refuses.
    pub fn content_range(&self) -> Result<Option<ContentRange>> {
        self.get("content-range")
            .map(ContentRange::from_str)
            .transpose()
    }

    /// Whether `Accept-Ranges` lists `bytes`.
    pub fn accept_ranges(&self) -> bool {
        self.get_all("accept-ranges")
            .any(|unit| unit.eq_ignore_ascii_case("bytes"))
    }

    /// `ETag` parsed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for what [`ETag::from_str`] refuses.
    pub fn etag(&self) -> Result<Option<ETag>> {
        self.get("etag").map(ETag::from_str).transpose()
    }

    /// `Last-Modified` as UTC nanoseconds since the epoch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for what [`parse_http_date`] refuses.
    pub fn last_modified(&self) -> Result<Option<i64>> {
        self.get("last-modified").map(parse_http_date).transpose()
    }

    /// `Date` as UTC nanoseconds since the epoch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for what [`parse_http_date`] refuses.
    pub fn date(&self) -> Result<Option<i64>> {
        self.get("date").map(parse_http_date).transpose()
    }

    /// `Location` as written: absolute, or relative to the request's URL.
    pub fn location(&self) -> Option<&str> {
        self.get("location")
    }

    /// `Retry-After` as the pause it asks for at `now_ns` (UTC nanoseconds
    /// since the epoch): delta seconds as they are, an HTTP-date as the time
    /// until it, zero when it has passed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the value is neither delta seconds nor
    /// an HTTP-date.
    pub fn retry_after(&self, now_ns: i64) -> Result<Option<Duration>> {
        self.get("retry-after")
            .map(|value| read_retry_after(value, now_ns))
            .transpose()
    }

    /// `Link` parsed into its link values.
    ///
    /// ```
    /// use yggdryl::http::Headers;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let headers = Headers::from_entries([
    ///     ("Link", "</items?page=2>; rel=\"next\""),
    ///     ("Link", "</items?page=9>; rel=\"last\"; title=\"End\""),
    /// ])?;
    /// let links = headers.links()?;
    ///
    /// assert_eq!(links.len(), 2);
    /// assert_eq!(links[0].target, "/items?page=2");
    /// assert!(links[0].has_rel("next"));
    /// assert_eq!(links[1].parameter("title"), Some("End"));
    /// assert_eq!(headers.next_link()?, Some("/items?page=2"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for what [`parse_links`] refuses.
    pub fn links(&self) -> Result<Vec<Link>> {
        self.get("link").map_or_else(|| Ok(Vec::new()), parse_links)
    }

    /// The target of the `Link` whose `rel` holds `next`, borrowed from the
    /// header and building no link.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for what [`parse_links`] refuses.
    pub fn next_link(&self) -> Result<Option<&str>> {
        let Some(value) = self.get("link") else {
            return Ok(None);
        };
        Ok(link::parse_raw_links(value)?
            .into_iter()
            .find(|link| link.has_rel("next"))
            .map(|link| link.target))
    }

    /// Whether `Transfer-Encoding` lists `chunked`.
    pub fn transfer_encoding_chunked(&self) -> bool {
        self.get_all("transfer-encoding")
            .any(|coding| coding.eq_ignore_ascii_case("chunked"))
    }

    /// Whether `Connection` lists `close`.
    pub fn connection_close(&self) -> bool {
        self.get_all("connection")
            .any(|option| option.eq_ignore_ascii_case("close"))
    }

    /// The pause a rate limit asks for at `now_ns` (UTC nanoseconds since
    /// the epoch), read off `RateLimit-Remaining`/`RateLimit-Reset` and then
    /// `X-RateLimit-Remaining`/`X-RateLimit-Reset`: `Some` only when a
    /// remaining count is `0` and its reset is stated. A reset of
    /// 100 000 000 or more is epoch seconds (any instant since 1973), a
    /// smaller one delta seconds; either way a reset already passed is a
    /// zero pause.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when a remaining count or a reset is not a
    /// non-negative decimal number.
    pub fn rate_limit_pause(&self, now_ns: i64) -> Result<Option<Duration>> {
        const PAIRS: [(&str, &str); 2] = [
            ("ratelimit-remaining", "ratelimit-reset"),
            ("x-ratelimit-remaining", "x-ratelimit-reset"),
        ];
        // A reset this large names an instant; a smaller one, a wait.
        const EPOCH_SECONDS_FLOOR: u64 = 100_000_000;
        for (remaining_name, reset_name) in PAIRS {
            let Some(remaining) = self.get(remaining_name) else {
                continue;
            };
            if decimal(remaining_name, remaining)? != 0 {
                return Ok(None);
            }
            let Some(reset) = self.get(reset_name) else {
                return Ok(None);
            };
            let reset = decimal(reset_name, reset)?;
            let pause = if reset >= EPOCH_SECONDS_FLOOR {
                let instant = i64::try_from(reset)
                    .ok()
                    .and_then(|seconds| seconds.checked_mul(1_000_000_000))
                    .unwrap_or(i64::MAX);
                pause_until(instant, now_ns)
            } else {
                Duration::from_secs(reset)
            };
            return Ok(Some(pause));
        }
        Ok(None)
    }

    /// Every `Set-Cookie` field, one per cookie, as written.
    pub fn set_cookies(&self) -> Vec<&str> {
        self.get_all(SET_COOKIE).collect()
    }

    fn insert_owned(&mut self, name: &str, value: String) -> Result<Option<String>> {
        if is_set_cookie(name) {
            // Metadata refuses a newline in a field value, rightly, and it is
            // the one byte the cookie join uses: each member takes that
            // validation on its own, and the joined value lands past it.
            for member in value.split(COOKIE_SEPARATOR) {
                Metadata::new()
                    .insert(stored_key(SET_COOKIE), member.to_owned())
                    .map_err(|error| header_error(name, error))?;
            }
            return Ok(self.0.insert_validated(stored_key(SET_COOKIE), value).0);
        }
        let mut key = String::with_capacity(PREFIX.len() + name.len());
        key.push_str(PREFIX);
        key.push_str(name);
        self.0
            .insert(key, value)
            .map(|(previous, _)| previous)
            .map_err(|error| header_error(name, error))
    }
}

/// The fields whose values are credentials, which `Debug` never prints.
const CREDENTIAL_FIELDS: [&str; 4] = [
    "authorization",
    "cookie",
    "proxy-authorization",
    "set-cookie",
];

/// What `Debug` prints in place of a credential.
struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// A debug rendering: every field, a credential's value `<redacted>` so a
/// logged request or answer leaks no token or session. `Display` and serde
/// are the data forms and carry every value as it is.
impl fmt::Debug for Headers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = formatter.debug_map();
        for (name, value) in self.iter() {
            if CREDENTIAL_FIELDS.contains(&name) {
                map.entry(&name, &Redacted);
            } else {
                map.entry(&name, &value);
            }
        }
        map.finish()
    }
}

impl fmt::Display for Headers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("{")?;
        for (index, (name, value)) in self.iter().enumerate() {
            if index != 0 {
                formatter.write_str(",")?;
            }
            write_json_string(formatter, name)?;
            formatter.write_str(":")?;
            write_json_string(formatter, value)?;
        }
        formatter.write_str("}")
    }
}

impl Serialize for Headers {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (name, value) in self.iter() {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Headers {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct HeadersVisitor;

        impl<'de> Visitor<'de> for HeadersVisitor {
            type Value = Headers;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of header names to values")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut headers = Headers::new();
                while let Some((name, value)) = map.next_entry::<String, String>()? {
                    headers.append(&name, &value).map_err(A::Error::custom)?;
                }
                Ok(headers)
            }
        }

        deserializer.deserialize_map(HeadersVisitor)
    }
}

impl FromStr for Headers {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::from_json(value)
    }
}

impl Index<&str> for Headers {
    type Output = str;

    /// # Panics
    ///
    /// Panics when the header is absent; [`Headers::get`] is the checked
    /// read.
    fn index(&self, name: &str) -> &Self::Output {
        self.get(name)
            .unwrap_or_else(|| panic!("header {name:?} is not present"))
    }
}

impl<'headers> IntoIterator for &'headers Headers {
    type Item = (&'headers str, &'headers str);
    type IntoIter = HeadersIter<'headers>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for Headers {
    type Item = (String, String);
    type IntoIter = HeadersIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        HeadersIntoIter(self.0.into_iter())
    }
}

/// A borrowed iterator over `(name, value)` in lexical order of the folded
/// name.
#[derive(Clone)]
pub struct HeadersIter<'a>(PropertyIter<'a, 'static>);

impl<'a> Iterator for HeadersIter<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl DoubleEndedIterator for HeadersIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}

impl std::iter::FusedIterator for HeadersIter<'_> {}

/// A consuming iterator over `(name, value)` in lexical order of the folded
/// name, each name bare.
pub struct HeadersIntoIter(MetadataIntoIter);

impl Iterator for HeadersIntoIter {
    type Item = (String, String);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(bare)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for HeadersIntoIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(bare)
    }
}

impl ExactSizeIterator for HeadersIntoIter {}
impl std::iter::FusedIterator for HeadersIntoIter {}

/// Strip the stored prefix off one entry's key, in place.
fn bare((mut key, value): (String, String)) -> (String, String) {
    key.drain(..PREFIX.len().min(key.len()));
    (key, value)
}

/// The members of one field value: cookies at newlines, a list at the
/// commas outside quoted strings.
struct Members<'a> {
    rest: &'a str,
    cookies: bool,
}

impl<'a> Iterator for Members<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.rest.is_empty() {
                return None;
            }
            let end = if self.cookies {
                self.rest.find(COOKIE_SEPARATOR)
            } else {
                list_member_end(self.rest)
            };
            let (member, rest) = match end {
                Some(end) => (&self.rest[..end], &self.rest[end + 1..]),
                None => (self.rest, ""),
            };
            self.rest = rest;
            let member = member.trim_matches([' ', '\t']);
            if !member.is_empty() {
                return Some(member);
            }
        }
    }
}

impl std::iter::FusedIterator for Members<'_> {}

/// The members of a list field value that is not `Set-Cookie`.
fn list_members(value: &str) -> Members<'_> {
    Members {
        rest: value,
        cookies: false,
    }
}

/// The byte of the first comma outside a quoted string, RFC 9110 section
/// 5.6.1 over 5.6.4.
fn list_member_end(value: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            _ if escaped => escaped = false,
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b',' if !quoted => return Some(index),
            _ => {}
        }
    }
    None
}

fn is_set_cookie(name: &str) -> bool {
    name.eq_ignore_ascii_case(SET_COOKIE)
}

/// The key a header is stored under: the prefix and the name folded.
fn stored_key(name: &str) -> String {
    let mut key = String::with_capacity(PREFIX.len() + name.len());
    key.push_str(PREFIX);
    key.push_str(name);
    key[PREFIX.len()..].make_ascii_lowercase();
    key
}

/// Hand `read` the stored key of `name`, built in a stack buffer for a name
/// of ordinary length.
fn with_key<R>(name: &str, read: impl FnOnce(&str) -> R) -> R {
    let len = PREFIX.len() + name.len();
    if len <= STACK_KEY {
        let mut key = [0_u8; STACK_KEY];
        key[..PREFIX.len()].copy_from_slice(PREFIX.as_bytes());
        key[PREFIX.len()..len].copy_from_slice(name.as_bytes());
        key[PREFIX.len()..len].make_ascii_lowercase();
        // Two `str`s laid end to end, one folded in ASCII, are one `str`.
        if let Ok(key) = std::str::from_utf8(&key[..len]) {
            return read(key);
        }
    }
    read(&stored_key(name))
}

/// The time from `now_ns` until `instant_ns`, zero when it has passed.
/// One `Retry-After` value as the pause it asks for at `now_ns`: the one
/// reader [`Headers::retry_after`] and every retrying client share.
pub(crate) fn read_retry_after(value: &str, now_ns: i64) -> Result<Duration> {
    let trimmed = value.trim_matches([' ', '\t']);
    if !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        let seconds = trimmed
            .parse()
            .map_err(|_| refuse("Retry-After", "expected delta seconds a u64 holds", value))?;
        return Ok(Duration::from_secs(seconds));
    }
    let instant = parse_http_date(trimmed).map_err(|_| {
        refuse(
            "Retry-After",
            "expected delta seconds or an HTTP-date",
            value,
        )
    })?;
    Ok(pause_until(instant, now_ns))
}

fn pause_until(instant_ns: i64, now_ns: i64) -> Duration {
    Duration::from_nanos(u64::try_from(instant_ns.saturating_sub(now_ns)).unwrap_or(0))
}

/// A non-negative decimal a rate-limit header states, its fraction dropped.
fn decimal(name: &str, value: &str) -> Result<u64> {
    let trimmed = value.trim_matches([' ', '\t']);
    let whole = trimmed.split('.').next().unwrap_or_default();
    if whole.is_empty() || whole.bytes().any(|byte| !byte.is_ascii_digit()) {
        return Err(refuse(
            name,
            "expected a non-negative decimal number",
            value,
        ));
    }
    whole
        .parse()
        .map_err(|_| refuse(name, "expected a number a u64 holds", value))
}

/// The refusal of one header's value, naming the header.
fn refuse(name: &str, reason: &str, value: &str) -> Error {
    Error::Parse {
        target: "http header",
        position: 0,
        reason: format_smolstr!(
            "{name}: {reason}, got {:?}",
            crate::text::elide_to(value, 64)
        ),
    }
}

/// Restate what the metadata layer refused as the header refusal it is.
fn header_error(name: &str, error: Error) -> Error {
    match error {
        Error::InvalidMetadataValue { reason, .. } => Error::Parse {
            target: "http header",
            position: 0,
            reason: format_smolstr!("{name}: {reason}"),
        },
        other => other,
    }
}

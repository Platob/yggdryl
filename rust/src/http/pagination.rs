//! How the next page of a paginated resource is found, and where a page's
//! rows are.
//!
//! An API pages one of a handful of ways - a `Link` header, a header of its
//! own, a URL in the body, a cursor sent back as a query parameter, an
//! offset or a page number counted up - and [`Pagination`] names each, plus
//! [`Pagination::Auto`], which tries every common spelling in a fixed order.
//! Every candidate spelling is one row of a table constant here, so the
//! ladder is read in one place and extended in one place.
//!
//! Detection is written over what it needs - the page's URL, its headers,
//! its parsed body and how many rows it held - so it is pinned without a
//! network; the page walker hands those in from a `Response`.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use super::Headers;
use crate::{Error, FieldPath, FieldSegment, Result, Scalar, Url};

/// What a parse failure names itself as.
const TARGET: &str = "pagination";

/// What a non-URL header value under [`AUTO_HEADERS`] is sent back as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    /// A page number, under the page parameter.
    Page,
    /// A cursor, under the cursor parameter.
    Cursor,
}

/// The headers the `Auto` ladder reads after `Link`: a URL, or a token sent
/// back under the parameter its kind names.
const AUTO_HEADERS: [(&str, Token); 4] = [
    ("X-Next-Page", Token::Page),
    ("X-Next-Cursor", Token::Cursor),
    ("X-Next", Token::Cursor),
    ("Next-Page", Token::Page),
];

/// The body paths the `Auto` ladder reads a next URL from, in order.
const AUTO_URL_PATHS: [&[&str]; 12] = [
    &["next"],
    &["next_url"],
    &["nextUrl"],
    &["next_page_url"],
    &["nextLink"],
    &["@odata.nextLink"],
    &["links", "next"],
    &["links", "next", "href"],
    &["_links", "next", "href"],
    &["paging", "next"],
    &["meta", "next"],
    &["pagination", "next"],
];

/// The body paths the `Auto` ladder reads a cursor from, in order.
const AUTO_CURSOR_PATHS: [&[&str]; 8] = [
    &["next_cursor"],
    &["nextCursor"],
    &["next_page_token"],
    &["nextPageToken"],
    &["cursor"],
    &["after"],
    &["meta", "cursor"],
    &["pagination", "cursor"],
];

/// The query parameters a cursor is sent back under: the one the URL already
/// carries, else the first.
const CURSOR_PARAMETERS: [&str; 5] = ["cursor", "after", "page_token", "pageToken", "next_cursor"];

/// The query parameters a page number is sent back under, the same way.
const PAGE_PARAMETERS: [&str; 3] = ["page", "page_number", "pageNumber"];

/// The body paths whose `false` ends every walk.
const HAS_MORE_PATHS: [&[&str]; 2] = [&["has_more"], &["hasMore"]];

/// The body keys a page's rows are looked for under, in order, after a
/// top-level sequence.
const RECORDS_PATHS: [&[&str]; 10] = [
    &["data"],
    &["items"],
    &["results"],
    &["records"],
    &["value"],
    &["rows"],
    &["entries"],
    &["elements"],
    &["content"],
    &["hits", "hits"],
];

/// How the next page of a resource is named.
///
/// ```
/// use yggdryl::http::Pagination;
///
/// # fn main() -> yggdryl::Result<()> {
/// let cursor = Pagination::from_str("cursor:meta.next_cursor:after")?;
/// assert_eq!(cursor.to_string(), "cursor:meta.next_cursor:after");
/// assert_eq!(Pagination::from_str("Offset:offset:100")?.to_string(), "offset:offset:100");
/// assert!(Pagination::from_str("scroll").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pagination {
    /// Try every common spelling, in the order the module documents.
    #[default]
    Auto,
    /// One page and no more.
    None,
    /// The `Link` header's `rel=next` target.
    Link,
    /// A header holding the next URL.
    Header(SmolStr),
    /// The body holds the next URL, absolute or relative, at this path.
    Url(FieldPath),
    /// The body holds a cursor at `path`, sent back as the query parameter
    /// `parameter`.
    Cursor {
        /// Where the cursor is in the page's document.
        path: FieldPath,
        /// The query parameter the cursor is sent back under.
        parameter: SmolStr,
    },
    /// An offset counted up by `page_size` under `parameter`, ending at a
    /// short page or at `total` when the body states one.
    Offset {
        /// The query parameter carrying the offset.
        parameter: SmolStr,
        /// How many rows one page asks for.
        page_size: u64,
        /// Where the body states the row total, when it does.
        total: Option<FieldPath>,
    },
    /// A page number counted up from `start` under `parameter`, ending at an
    /// empty page.
    Page {
        /// The query parameter carrying the page number.
        parameter: SmolStr,
        /// The number of the first page.
        start: u64,
    },
}

/// Where the next page is: a whole URL, or one query parameter to set on
/// the current one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NextPage {
    /// The next page's URL, already resolved against the current one.
    Url(Url),
    /// The next page is the current URL with this query parameter set.
    Parameter {
        /// The parameter's name.
        name: SmolStr,
        /// The parameter's value, not yet percent-encoded.
        value: String,
    },
}

impl Pagination {
    /// Where the page after the one at `url` is, or `None` when the walk
    /// ends.
    ///
    /// `headers` and `body` are that page's, `page_index` counts pages from
    /// zero and `rows_on_page` is how many rows the page held at its records
    /// path. Every mode ends the walk on an empty page, on a `has_more` or
    /// `hasMore` of `false` in the body, on a next URL equal to the current
    /// one and on a cursor the current URL already carries; a page the caller
    /// already visited is the walker's to notice, since only it knows.
    ///
    /// # Errors
    ///
    /// A header or body value that should have been a URL and is not, a
    /// path holding a range or filter segment, or a malformed `Link` header.
    ///
    /// ```
    /// use yggdryl::http::{Headers, NextPage, Pagination};
    /// use yggdryl::{Url, from_json_scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("https://api.example.com/v1/orders?limit=2")?;
    /// let headers = Headers::new();
    /// let body = from_json_scalar(r#"{"data":[{"id":1},{"id":2}],"next_cursor":"c2"}"#)?;
    ///
    /// let next = Pagination::Auto.next(&url, &headers, Some(&body), 0, 2)?;
    /// assert_eq!(
    ///     next,
    ///     Some(NextPage::Parameter { name: "cursor".into(), value: "c2".to_owned() })
    /// );
    /// assert_eq!(Pagination::None.next(&url, &headers, Some(&body), 0, 2)?, None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn next(
        &self,
        url: &Url,
        headers: &Headers,
        body: Option<&Scalar>,
        page_index: u64,
        rows_on_page: usize,
    ) -> Result<Option<NextPage>> {
        if matches!(self, Self::None) || rows_on_page == 0 || has_more_is_false(body) {
            return Ok(None);
        }
        match self {
            Self::None => Ok(None),
            Self::Auto => auto_next(url, headers, body),
            Self::Link => match headers.next_link()? {
                Some(target) => resolve(url, target),
                None => Ok(None),
            },
            Self::Header(name) => match headers.get(name) {
                Some(value) => resolve(url, value),
                None => Ok(None),
            },
            Self::Url(path) => match body.and_then(|body| text_at_path(body, path).transpose()) {
                Some(text) => resolve(url, &text?),
                None => Ok(None),
            },
            Self::Cursor { path, parameter } => {
                match body.and_then(|body| text_at_path(body, path).transpose()) {
                    Some(text) => parameter_next(url, parameter, &text?),
                    None => Ok(None),
                }
            }
            Self::Offset {
                parameter,
                page_size,
                total,
            } => {
                let current = url_parameter_count(url, parameter)?
                    .unwrap_or_else(|| page_index.saturating_mul(*page_size));
                let next = current.saturating_add(*page_size);
                if u64::try_from(rows_on_page).is_ok_and(|rows| rows < *page_size) {
                    return Ok(None);
                }
                if let Some(total) = total {
                    let stated = body
                        .and_then(|body| text_at_path(body, total).transpose())
                        .transpose()?
                        .and_then(|text| text.parse::<u64>().ok());
                    if stated.is_some_and(|total| next >= total) {
                        return Ok(None);
                    }
                }
                Ok(Some(NextPage::Parameter {
                    name: parameter.clone(),
                    value: next.to_string(),
                }))
            }
            Self::Page { parameter, start } => {
                let current = url_parameter_count(url, parameter)?
                    .unwrap_or_else(|| start.saturating_add(page_index));
                Ok(Some(NextPage::Parameter {
                    name: parameter.clone(),
                    value: current.saturating_add(1).to_string(),
                }))
            }
        }
    }

    /// Where a page's rows are in `body`: `declared` when given; the root
    /// when the body is a sequence; else the first of `data`, `items`,
    /// `results`, `records`, `value`, `rows`, `entries`, `elements`,
    /// `content`, `hits.hits` holding a sequence; else the largest top-level
    /// sequence; else `None`.
    ///
    /// ```
    /// use yggdryl::http::Pagination;
    /// use yggdryl::{FieldPath, from_json_scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let page = from_json_scalar(r#"{"meta":{"n":2},"orders":[{"id":1},{"id":2}]}"#)?;
    /// assert_eq!(Pagination::records_path(&page, None), Some(FieldPath::from_str("orders")?));
    /// let page = from_json_scalar(r#"{"hits":{"hits":[{"id":1}]}}"#)?;
    /// assert_eq!(Pagination::records_path(&page, None), Some(FieldPath::from_str("hits.hits")?));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn records_path(body: &Scalar, declared: Option<&FieldPath>) -> Option<FieldPath> {
        if let Some(declared) = declared {
            return Some(declared.clone());
        }
        if body.sequence_rows().is_some() {
            return Some(FieldPath::root());
        }
        for segments in RECORDS_PATHS {
            if value_at(body, segments).is_some_and(|value| value.sequence_rows().is_some()) {
                return Some(FieldPath::new(
                    segments.iter().map(|segment| FieldSegment::field(*segment)),
                ));
            }
        }
        let mut largest: Option<(&str, usize)> = None;
        for (key, value) in entries_of(body) {
            let Some(rows) = value.sequence_rows() else {
                continue;
            };
            if largest.is_none_or(|(_, len)| rows.len() > len) {
                largest = Some((key, rows.len()));
            }
        }
        largest.map(|(key, _)| FieldPath::new([FieldSegment::field(key)]))
    }

    /// Parse one spelling: `auto`, `none`, `link`, `header:<name>`,
    /// `url:<path>`, `cursor:<path>:<parameter>`,
    /// `offset:<parameter>:<size>` with an optional `:<total path>`, or
    /// `page:<parameter>:<start>`; the mode word in any case.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] naming the byte position and what was expected.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }
}

impl FromStr for Pagination {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        let (mode, rest) = match value.find(':') {
            Some(index) => (&value[..index], Some(&value[index + 1..])),
            None => (value, None),
        };
        let argument_at = mode.len() + 1;
        let lowered = mode.to_ascii_lowercase();
        match (lowered.as_str(), rest) {
            ("auto", None) => Ok(Self::Auto),
            ("none", None) => Ok(Self::None),
            ("link", None) => Ok(Self::Link),
            ("auto" | "none" | "link", Some(_)) => {
                Err(parse_error(argument_at, "this mode takes no argument"))
            }
            ("header", Some(name)) => {
                let name = name.trim();
                if name.is_empty() || name.bytes().any(|byte| !is_token_byte(byte)) {
                    return Err(parse_error(argument_at, "a header name after `header:`"));
                }
                Ok(Self::Header(SmolStr::new(name)))
            }
            ("url", Some(path)) => Ok(Self::Url(body_path(path, argument_at)?)),
            ("cursor", Some(rest)) => {
                let Some((path, parameter)) = rest.rsplit_once(':') else {
                    return Err(parse_error(
                        argument_at,
                        "`<path>:<parameter>` after `cursor:`",
                    ));
                };
                Ok(Self::Cursor {
                    path: body_path(path, argument_at)?,
                    parameter: parameter_name(parameter, argument_at + path.len() + 1)?,
                })
            }
            ("offset", Some(rest)) => {
                let mut parts = rest.splitn(3, ':');
                let parameter = parts.next().unwrap_or_default();
                let Some(size) = parts.next() else {
                    return Err(parse_error(
                        argument_at,
                        "`<parameter>:<size>` after `offset:`",
                    ));
                };
                let size_at = argument_at + parameter.len() + 1;
                let page_size = size.trim().parse::<u64>().map_err(|_| {
                    parse_error(size_at, "a positive page size after the parameter")
                })?;
                if page_size == 0 {
                    return Err(parse_error(
                        size_at,
                        "a positive page size after the parameter",
                    ));
                }
                let total = match parts.next() {
                    Some(total) => Some(body_path(total, size_at + size.len() + 1)?),
                    None => None,
                };
                Ok(Self::Offset {
                    parameter: parameter_name(parameter, argument_at)?,
                    page_size,
                    total,
                })
            }
            ("page", Some(rest)) => {
                let Some((parameter, start)) = rest.split_once(':') else {
                    return Err(parse_error(
                        argument_at,
                        "`<parameter>:<start>` after `page:`",
                    ));
                };
                let start_at = argument_at + parameter.len() + 1;
                let start = start.trim().parse::<u64>().map_err(|_| {
                    parse_error(start_at, "the first page number after the parameter")
                })?;
                Ok(Self::Page {
                    parameter: parameter_name(parameter, argument_at)?,
                    start,
                })
            }
            ("header" | "url" | "cursor" | "offset" | "page", None) => {
                Err(parse_error(mode.len(), "`:` and the mode's argument"))
            }
            _ => Err(parse_error(
                0,
                "one of auto, none, link, header:<name>, url:<path>, cursor:<path>:<parameter>, offset:<parameter>:<size>, page:<parameter>:<start>",
            )),
        }
    }
}

impl fmt::Display for Pagination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::None => formatter.write_str("none"),
            Self::Link => formatter.write_str("link"),
            Self::Header(name) => write!(formatter, "header:{name}"),
            Self::Url(path) => write!(formatter, "url:{path}"),
            Self::Cursor { path, parameter } => write!(formatter, "cursor:{path}:{parameter}"),
            Self::Offset {
                parameter,
                page_size,
                total,
            } => {
                write!(formatter, "offset:{parameter}:{page_size}")?;
                if let Some(total) = total {
                    write!(formatter, ":{total}")?;
                }
                Ok(())
            }
            Self::Page { parameter, start } => write!(formatter, "page:{parameter}:{start}"),
        }
    }
}

impl fmt::Display for NextPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => write!(formatter, "{url}"),
            Self::Parameter { name, value } => write!(formatter, "?{name}={value}"),
        }
    }
}

/// The `Auto` ladder: `Link`, then the headers, then the body URLs, then the
/// body cursors; the first candidate found decides, whatever it decides.
fn auto_next(url: &Url, headers: &Headers, body: Option<&Scalar>) -> Result<Option<NextPage>> {
    if let Some(target) = headers.next_link()? {
        return resolve(url, target);
    }
    for (name, token) in AUTO_HEADERS {
        let Some(value) = headers.get(name).map(str::trim) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        if looks_like_url(value) {
            return resolve(url, value);
        }
        let parameter = match token {
            Token::Page => parameter_carried(url, &PAGE_PARAMETERS)?,
            Token::Cursor => parameter_carried(url, &CURSOR_PARAMETERS)?,
        };
        return parameter_next(url, &parameter, value);
    }
    let Some(body) = body else {
        return Ok(None);
    };
    for segments in AUTO_URL_PATHS {
        if let Some(text) = value_at(body, segments).and_then(text_of) {
            return resolve(url, &text);
        }
    }
    for segments in AUTO_CURSOR_PATHS {
        if let Some(text) = value_at(body, segments).and_then(text_of) {
            let parameter = parameter_carried(url, &CURSOR_PARAMETERS)?;
            return parameter_next(url, &parameter, &text);
        }
    }
    Ok(None)
}

/// Whether `body` states `has_more` or `hasMore` as `false`.
fn has_more_is_false(body: Option<&Scalar>) -> bool {
    let Some(body) = body else {
        return false;
    };
    HAS_MORE_PATHS
        .iter()
        .any(|segments| value_at(body, segments).and_then(Scalar::as_bool) == Some(false))
}

/// `text` resolved against `url`; `None` when empty or the same page.
fn resolve(url: &Url, text: &str) -> Result<Option<NextPage>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let next = url.join_reference(text)?;
    if next == *url {
        return Ok(None);
    }
    Ok(Some(NextPage::Url(next)))
}

/// `value` sent back under `name`; `None` when empty or already carried.
fn parameter_next(url: &Url, name: &str, value: &str) -> Result<Option<NextPage>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if url.parameters(true)?.get(name) == Some(value) {
        return Ok(None);
    }
    Ok(Some(NextPage::Parameter {
        name: SmolStr::new(name),
        value: value.to_owned(),
    }))
}

/// The first of `candidates` the URL's query already carries, else the
/// first candidate.
fn parameter_carried(url: &Url, candidates: &[&str]) -> Result<SmolStr> {
    let parameters = url.parameters(true)?;
    let name = candidates
        .iter()
        .find(|candidate| parameters.contains_key(candidate))
        .or(candidates.first())
        .copied()
        .unwrap_or_default();
    Ok(SmolStr::new(name))
}

/// The count the URL's query carries under `name`, when it does and it is
/// one.
fn url_parameter_count(url: &Url, name: &str) -> Result<Option<u64>> {
    Ok(url
        .parameters(true)?
        .get(name)
        .and_then(|value| value.trim().parse::<u64>().ok()))
}

/// Whether a header value is a URL rather than a token: it names a scheme
/// or opens with `/`.
fn looks_like_url(value: &str) -> bool {
    if value.starts_with('/') || value.starts_with('?') {
        return true;
    }
    value.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    })
}

/// The value of `body` under a run of mapping keys.
fn value_at<'body>(body: &'body Scalar, segments: &[&str]) -> Option<&'body Scalar> {
    let mut current = body;
    for segment in segments {
        current = key_of(current, segment)?;
    }
    Some(current)
}

/// One mapping key of `value`, exactly, else ASCII case-insensitively.
fn key_of<'value>(value: &'value Scalar, key: &str) -> Option<&'value Scalar> {
    if let Some(found) = value.get_key_str(key) {
        return Some(found);
    }
    entries_of(value)
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, found)| found)
}

/// The string-keyed entries of a record or a mapping, in its own order.
fn entries_of(value: &Scalar) -> Box<dyn Iterator<Item = (&str, &Scalar)> + '_> {
    if let Some(record) = value.as_struct() {
        return Box::new(record.iter().map(|(key, value)| (key.as_str(), value)));
    }
    if let Some(entries) = value.as_mapping() {
        return Box::new(
            entries
                .iter()
                .filter_map(|(key, value)| key.as_str().map(|key| (key, value))),
        );
    }
    Box::new(std::iter::empty())
}

/// The text `body` holds at a caller's path, `None` where the path does not
/// resolve or holds null.
///
/// # Errors
///
/// A range or filter segment, which no page document is walked by.
fn text_at_path(body: &Scalar, path: &FieldPath) -> Result<Option<String>> {
    let mut current = Cow::Borrowed(body);
    for segment in path.segments() {
        let stepped = match (segment, &current) {
            (FieldSegment::Field(name), _) => key_of(&current, name).cloned().map(Cow::Owned),
            (FieldSegment::Index(index), _) => {
                let len = i64::try_from(current.len()).unwrap_or(i64::MAX);
                let position = if *index < 0 { len + *index } else { *index };
                usize::try_from(position)
                    .ok()
                    .and_then(|position| current.get(position))
                    .map(|found| Cow::Owned(found.into_owned()))
            }
            (FieldSegment::Key(literal), _) => match literal.value().as_str() {
                Some(key) => key_of(&current, key).cloned().map(Cow::Owned),
                None => literal
                    .value()
                    .as_i128()
                    .and_then(|position| usize::try_from(position).ok())
                    .and_then(|position| current.get(position))
                    .map(|found| Cow::Owned(found.into_owned())),
            },
            (FieldSegment::Range { .. } | FieldSegment::Where(_), _) => {
                return Err(Error::Parse {
                    target: TARGET,
                    position: 0,
                    reason: format_smolstr!(
                        "a pagination path names one value; `{segment}` selects several"
                    ),
                });
            }
        };
        match stepped {
            Some(found) => current = found,
            None => return Ok(None),
        }
    }
    Ok(text_of(&current))
}

/// A scalar as the text a URL or a query parameter carries: a string as it
/// is, an integer rendered; anything else is nothing.
fn text_of(value: &Scalar) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    value.as_i128().map(|count| count.to_string())
}

/// A body path spelled after a mode word, refusing what names several values.
fn body_path(text: &str, at: usize) -> Result<FieldPath> {
    let text = text.trim();
    if text.is_empty() {
        return Err(parse_error(at, "a field path"));
    }
    let path = FieldPath::from_str(text).map_err(|error| match error {
        Error::Parse {
            position, reason, ..
        } => Error::Parse {
            target: TARGET,
            position: at + position,
            reason,
        },
        other => other,
    })?;
    if let Some(segment) = path
        .segments()
        .iter()
        .find(|segment| matches!(segment, FieldSegment::Range { .. } | FieldSegment::Where(_)))
    {
        return Err(parse_error_owned(
            at,
            format_smolstr!("a path naming one value; `{segment}` selects several"),
        ));
    }
    Ok(path)
}

/// A query parameter name spelled after a mode word.
fn parameter_name(text: &str, at: usize) -> Result<SmolStr> {
    let text = text.trim();
    if text.is_empty()
        || text
            .bytes()
            .any(|byte| matches!(byte, b'&' | b'=' | b'#' | b' '))
    {
        return Err(parse_error(at, "a query parameter name"));
    }
    Ok(SmolStr::new(text))
}

/// RFC 9110 token bytes, which a header name is made of.
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

fn parse_error(position: usize, expected: &'static str) -> Error {
    Error::Parse {
        target: TARGET,
        position,
        reason: format_smolstr!("expected {expected}"),
    }
}

fn parse_error_owned(position: usize, reason: SmolStr) -> Error {
    Error::Parse {
        target: TARGET,
        position,
        reason,
    }
}

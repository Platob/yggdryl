//! Serving a mounted [`Holder`]: the HTTP reading of the `IOBase` verbs.
//!
//! `GET` and `HEAD` read a leaf - whole, as one byte range, or as a `304`
//! against its validators - and list a container as JSON; `PUT` writes a
//! leaf whole; `DELETE` removes one; `OPTIONS` names the five. The prefix
//! itself is the mounted holder, and a path below it is
//! [`IOBase::child_by_path`], resolved under the mount's lock and served
//! outside it; a leaf's bytes are never read whole here, they stream when
//! the connection writes the answer.

use std::sync::{Arc, PoisonError, RwLock};

use super::{ALLOW, Answer, AnswerBody, Incoming, Mounted, ServerOptions, Source};
use crate::holder::Holder;
use crate::http::headers::{parse_http_date, render_http_date};
use crate::http::{Headers, Method, Status};
use crate::{DigestAlgorithm, Error, IOBase, IOKind, MediaType, MimeType, Result, Scalar, Url};

/// The answer to `incoming` under the mount `shared` at `prefix`, `rest`
/// being the percent-decoded path below the prefix.
pub(super) fn serve(
    shared: &Arc<RwLock<Mounted>>,
    prefix: &str,
    rest: &str,
    incoming: &Incoming,
    server_url: &Url,
    options: &ServerOptions,
) -> Answer {
    let outcome = match incoming.head.method {
        Method::Get | Method::Head => read(shared, prefix, rest, incoming, server_url, options),
        Method::Put => put(shared, rest, incoming),
        Method::Delete => delete(shared, rest),
        Method::Options => Ok(Answer::status(Status::NO_CONTENT).with_header("allow", ALLOW)),
        _ => Ok(Answer::status(Status::METHOD_NOT_ALLOWED).with_header("allow", ALLOW)),
    };
    outcome.unwrap_or_else(|error| Answer::from_error(&error))
}

/// What a read found under the mount's lock.
enum Found {
    /// Nothing is there.
    Absent,
    /// A container, listed.
    Listing(Answer),
    /// A leaf, described before any byte of it is served.
    Leaf(Leaf),
}

/// What a read learned about a leaf under the mount's lock, before any byte
/// of it is served.
struct Leaf {
    size: u64,
    media_type: MediaType,
    etag: Option<String>,
    last_modified: Option<i64>,
}

/// Run `read` over the holder `rest` names under the mount, with the media
/// type a `PUT` declared for it, and answer the source to stream from.
fn resolve<T>(
    shared: &Arc<RwLock<Mounted>>,
    rest: &str,
    read: impl FnOnce(&Holder, Option<&MediaType>) -> Result<T>,
) -> Result<(T, Source)> {
    let mounted = shared.read().unwrap_or_else(PoisonError::into_inner);
    let declared = mounted.declared.get(rest);
    if rest.is_empty() {
        let answer = read(&mounted.holder, declared)?;
        return Ok((answer, Source::Root(Arc::clone(shared), mounted.version)));
    }
    let child = mounted.holder.child_by_path(rest)?;
    let answer = read(&child, declared)?;
    Ok((answer, Source::Owned(Box::new(child))))
}

/// `GET` or `HEAD`: a listing, a `404`, a `304`, a `206`, a `416` or the
/// whole leaf.
fn read(
    shared: &Arc<RwLock<Mounted>>,
    prefix: &str,
    rest: &str,
    incoming: &Incoming,
    server_url: &Url,
    options: &ServerOptions,
) -> Result<Answer> {
    let (found, source) = resolve(shared, rest, |holder, declared| {
        let kind = holder.kind();
        if kind == IOKind::Unknown {
            return Ok(Found::Absent);
        }
        if kind.is_container() {
            return listing(holder, prefix, rest, server_url).map(Found::Listing);
        }
        let etag = if options.etag {
            let digest = holder.read_digest(DigestAlgorithm::Xxh3)?;
            digest.as_u64().map(|hash| format!("\"{hash:016x}\""))
        } else {
            None
        };
        Ok(Found::Leaf(Leaf {
            size: holder.size(),
            media_type: declared
                .cloned()
                .unwrap_or_else(|| holder.media_type().clone()),
            etag,
            last_modified: holder.mtime(),
        }))
    })?;
    let leaf = match found {
        Found::Absent => return Ok(Answer::status(Status::NOT_FOUND)),
        Found::Listing(listing) => return Ok(listing),
        Found::Leaf(leaf) => leaf,
    };
    let request = &incoming.head.headers;
    let mut answer = Answer::status(Status::OK)
        .with_header("content-type", &content_type(&leaf.media_type))
        .with_header("accept-ranges", "bytes");
    if let Some(coding) = content_encoding(&leaf.media_type) {
        answer = answer.with_header("content-encoding", &coding);
    }
    if let Some(etag) = &leaf.etag {
        answer = answer.with_header("etag", etag);
    }
    let last_modified = leaf.last_modified.map(render_http_date);
    if let Some(date) = &last_modified {
        answer = answer.with_header("last-modified", date);
    }
    if not_modified(request, leaf.etag.as_deref(), leaf.last_modified) {
        answer.status = Status::NOT_MODIFIED;
        return Ok(answer);
    }
    let range = request
        .get("range")
        .and_then(ByteRange::parse)
        .filter(|_| if_range_holds(request, leaf.etag.as_deref(), last_modified.as_deref()));
    let (start, length) = match range {
        None => (0, leaf.size),
        Some(range) => match range.resolve(leaf.size) {
            None => {
                return Ok(answer
                    .with_header("content-range", &format!("bytes */{}", leaf.size))
                    .with_status(Status::RANGE_NOT_SATISFIABLE));
            }
            Some((start, last)) => {
                answer = answer
                    .with_header(
                        "content-range",
                        &format!("bytes {start}-{last}/{}", leaf.size),
                    )
                    .with_status(Status::PARTIAL_CONTENT);
                (start, last - start + 1)
            }
        },
    };
    answer.body = AnswerBody::Stream {
        source,
        start,
        length,
    };
    Ok(answer)
}

/// `PUT`: write the leaf whole; `201` when it was not there, else `204`;
/// `409` on a container.
fn put(shared: &Arc<RwLock<Mounted>>, rest: &str, incoming: &Incoming) -> Result<Answer> {
    let declared = incoming
        .head
        .headers
        .content_type()
        .map(|content_type| {
            MediaType::from_content_headers(
                Some(content_type),
                incoming.head.headers.get("content-encoding"),
            )
        })
        .transpose()?;
    let mut mounted = shared.write().unwrap_or_else(PoisonError::into_inner);
    let mut child;
    let holder = if rest.is_empty() {
        mounted.version += 1;
        &mut mounted.holder
    } else {
        child = mounted.holder.child_by_path(rest)?;
        &mut child
    };
    let kind = holder.kind();
    if kind.is_container() {
        return Ok(Answer::text(
            Status::CONFLICT,
            &format!("{rest:?} is a container and takes no body"),
        ));
    }
    let created = kind == IOKind::Unknown;
    holder.write_all_bytes(&incoming.body)?;
    if let Some(media_type) = declared {
        holder.set_media_type(media_type.clone());
        mounted.declared.insert(rest.to_owned(), media_type);
    }
    Ok(Answer::status(if created {
        Status::CREATED
    } else {
        Status::NO_CONTENT
    }))
}

/// `DELETE`: remove what is there; `404` when nothing was.
fn delete(shared: &Arc<RwLock<Mounted>>, rest: &str) -> Result<Answer> {
    let mut mounted = shared.write().unwrap_or_else(PoisonError::into_inner);
    let mut child;
    let holder = if rest.is_empty() {
        mounted.version += 1;
        &mut mounted.holder
    } else {
        child = mounted.holder.child_by_path(rest)?;
        &mut child
    };
    if holder.kind() == IOKind::Unknown {
        return Ok(Answer::status(Status::NOT_FOUND));
    }
    holder.remove(false)?;
    mounted.declared.remove(rest);
    Ok(Answer::status(Status::NO_CONTENT))
}

/// `path` - decoded segments joined by `/` - as URL path text, each segment
/// escaped, so a reference to a name holding a space, a `?` or a `%` reaches
/// the resource it names.
fn escaped(path: &str) -> String {
    path.split('/')
        .map(crate::uri::percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// A container's direct children as a JSON array of
/// `{"name", "url", "kind", "size", "media_type"}`.
fn listing(holder: &Holder, prefix: &str, rest: &str, server_url: &Url) -> Result<Answer> {
    let base = if rest.is_empty() {
        server_url.join_reference(&escaped(prefix))?
    } else {
        server_url.join_reference(&escaped(&format!(
            "{}/{rest}",
            prefix.trim_end_matches('/')
        )))?
    };
    let mut entries = Vec::new();
    for child in holder.ls(false, false) {
        let child = child?;
        // The URL spells the segment escaped; the listing names it as it is.
        let segment = child
            .url()
            .and_then(|url| url.file_name())
            .map(str::to_owned)
            .ok_or_else(|| Error::absent("a listed child's name", &base))?;
        let name = crate::uri::percent_decode(&segment, "url")?;
        let url = base.join_reference(&format!(
            "{}/{segment}",
            base.path_text(false)?.trim_end_matches('/')
        ))?;
        entries.push(Scalar::from_struct([
            ("name", Scalar::from(name.as_ref())),
            ("url", Scalar::from(url.to_string())),
            ("kind", Scalar::from(child.kind().as_str())),
            ("size", Scalar::from(child.size())),
            ("media_type", Scalar::from(child.media_type().to_string())),
        ])?);
    }
    let body = crate::json::into_json_scalar(&Scalar::from_sequence(entries))?;
    Ok(Answer::status(Status::OK)
        .with_header("content-type", "application/json")
        .with_bytes(body.into()))
}

/// `Content-Type`: the base with its charset when the media type declares
/// one.
fn content_type(media_type: &MediaType) -> String {
    match media_type.charset() {
        Some(charset) => format!("{}; charset={charset}", media_type.base()),
        None => media_type.base().to_string(),
    }
}

/// `Content-Encoding`: the media type's codings as HTTP spells them, in
/// application order; `None` when there is none to state.
fn content_encoding(media_type: &MediaType) -> Option<String> {
    let codings: Vec<&str> = media_type
        .encodings()
        .iter()
        .filter_map(content_coding)
        .collect();
    (!codings.is_empty()).then(|| codings.join(", "))
}

/// The `Content-Encoding` token of one coding, the inverse of
/// [`MimeType::from_content_coding`].
fn content_coding(mime: &MimeType) -> Option<&'static str> {
    if *mime == MimeType::GZIP {
        Some("gzip")
    } else if *mime == MimeType::ZLIB {
        Some("deflate")
    } else if *mime == MimeType::ZSTD {
        Some("zstd")
    } else if *mime == MimeType::BROTLI {
        Some("br")
    } else if *mime == MimeType::COMPRESS {
        Some("compress")
    } else {
        None
    }
}

/// Whether `If-None-Match` names the leaf's `ETag` (or `*`), or, when it is
/// absent, `If-Modified-Since` is at or after the leaf's modification, to
/// the second (RFC 9110 13.1).
fn not_modified(request: &Headers, etag: Option<&str>, last_modified: Option<i64>) -> bool {
    if let Some(candidates) = request.get("if-none-match") {
        return candidates.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate == "*" || etag.is_some_and(|etag| weak_eq(candidate, etag))
        });
    }
    match (request.get("if-modified-since"), last_modified) {
        (Some(since), Some(modified)) => parse_http_date(since)
            .is_ok_and(|since| modified / NANOS_PER_SECOND <= since / NANOS_PER_SECOND),
        _ => false,
    }
}

const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// Two entity tags compared weakly: `W/` stripped from either.
fn weak_eq(left: &str, right: &str) -> bool {
    left.strip_prefix("W/").unwrap_or(left) == right.strip_prefix("W/").unwrap_or(right)
}

/// Whether an `If-Range` precondition holds: absent, or naming the leaf's
/// strong `ETag` exactly or its `Last-Modified` date (RFC 9110 13.1.5).
fn if_range_holds(request: &Headers, etag: Option<&str>, last_modified: Option<&str>) -> bool {
    let Some(validator) = request.get("if-range") else {
        return true;
    };
    let validator = validator.trim();
    if validator.starts_with('"') || validator.starts_with("W/") {
        return etag == Some(validator) && !validator.starts_with("W/");
    }
    match (
        parse_http_date(validator),
        last_modified.map(parse_http_date),
    ) {
        (Ok(asked), Some(Ok(actual))) => asked == actual,
        _ => false,
    }
}

/// One `Range` header's single byte range (RFC 9110 14.1.2).
struct ByteRange {
    /// First byte; `None` for a suffix range.
    start: Option<u64>,
    /// Last byte inclusive, or the suffix length when `start` is `None`.
    end: Option<u64>,
}

impl ByteRange {
    /// `None` for anything but exactly one well-formed byte range: several
    /// ranges, another unit or a malformed spelling are answered whole, as
    /// RFC 9110 allows a server to ignore a `Range` it does not serve.
    fn parse(header: &str) -> Option<Self> {
        let spec = header.trim().strip_prefix("bytes=")?;
        if spec.contains(',') {
            return None;
        }
        let (start, end) = spec.split_once('-')?;
        let number = |text: &str| -> Option<Option<u64>> {
            let text = text.trim();
            if text.is_empty() {
                Some(None)
            } else if text.bytes().all(|byte| byte.is_ascii_digit()) {
                text.parse().ok().map(Some)
            } else {
                None
            }
        };
        let (start, end) = (number(start)?, number(end)?);
        match (start, end) {
            (None, None) => None,
            (Some(start), Some(end)) if end < start => None,
            _ => Some(Self { start, end }),
        }
    }

    /// The inclusive byte bounds inside `size` bytes, or `None` when the
    /// range is unsatisfiable.
    fn resolve(&self, size: u64) -> Option<(u64, u64)> {
        match (self.start, self.end) {
            (Some(start), _) if start >= size => None,
            (Some(start), end) => Some((start, end.map_or(size - 1, |end| end.min(size - 1)))),
            (None, Some(suffix)) if suffix == 0 || size == 0 => None,
            (None, Some(suffix)) => Some((size - suffix.min(size), size - 1)),
            (None, None) => None,
        }
    }
}

impl Answer {
    /// This answer with `status`.
    fn with_status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }
}

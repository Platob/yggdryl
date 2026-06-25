//! Redirect following — resolving a `Location` and deriving the next request.
//!
//! The session drives the hop loop in [`send`](crate::HttpSession::send); this
//! module holds the pure, transport-free decisions: is a status a redirect, how
//! to resolve a (possibly relative) `Location` against the current URL, and how a
//! given status reshapes the next request's method and body.

use yggdryl_core::Url;

use crate::error::HttpError;
use crate::method::Method;
use crate::request::{Body, HttpRequest};

/// The default cap on redirect hops followed before
/// [`HttpError::TooManyRedirects`] is raised.
pub(crate) const DEFAULT_MAX_REDIRECTS: usize = 10;

/// Whether `status` is a redirect this client follows (301/302/303/307/308).
pub(crate) fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Resolves a `Location` value against the request URL `base` into an absolute
/// [`Url`], handling an absolute URL, a network-path reference (`//host/p`), an
/// absolute path (`/p`) and a relative path. The path's `.` / `..` dot-segments
/// are removed (RFC 3986 §5.2.4) via [`Url::join`], and a `#fragment` is split off
/// the path rather than embedded in it. An absolute or scheme-relative `Location`
/// that fails to parse returns [`HttpError::InvalidUrl`]; a path reference is
/// resolved structurally against the base authority.
pub(crate) fn resolve(base: &Url, location: &str) -> Result<Url, HttpError> {
    let location = location.trim();
    if location.is_empty() {
        return Err(HttpError::InvalidUrl("empty Location header".to_string()));
    }
    // An absolute URL (has a scheme) is used as-is.
    if let Ok(url) = Url::from_str(location) {
        if !url.scheme().is_empty() {
            return Ok(url);
        }
    }
    // A scheme-relative reference (`//host/path`) inherits only the scheme.
    if let Some(rest) = location.strip_prefix("//") {
        let candidate = format!("{}://{rest}", base.scheme());
        return Url::from_str(&candidate).map_err(|err| HttpError::InvalidUrl(err.to_string()));
    }
    // A path reference resolved against the base authority. Split off the fragment
    // then the query (so neither lands inside the path), and resolve the path's
    // dot-segments through `Url::join`. `join` drops the base query/fragment, so a
    // reference without one does not inherit the base's — exactly the redirect rule.
    let (location, fragment) = split_off(location, '#');
    let (path, query) = split_off(location, '?');
    let mut next = base.join(path);
    if let Some(query) = query {
        next = next.with_query(query);
    }
    if let Some(fragment) = fragment {
        next = next.with_fragment(fragment);
    }
    Ok(next)
}

/// Splits `input` on the first `sep`, returning the part before and the owned part
/// after (or `None` if `sep` is absent).
fn split_off(input: &str, sep: char) -> (&str, Option<String>) {
    match input.split_once(sep) {
        Some((head, tail)) => (head, Some(tail.to_string())),
        None => (input, None),
    }
}

/// Whether two URLs share an origin (scheme + host + port, the default port
/// folded in). Credentials are kept only on a same-origin redirect; a different
/// host **or port** is treated as cross-origin and strips them.
fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme().eq_ignore_ascii_case(b.scheme())
        && a.host().eq_ignore_ascii_case(b.host())
        && effective_port(a) == effective_port(b)
}

/// The URL's port, defaulting to the well-known port for `http`/`https`.
fn effective_port(url: &Url) -> Option<u16> {
    url.port().or_else(|| match url.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    })
}

/// Builds the next request for a redirect, applying RFC 7231 method/body
/// semantics for `status` and stripping per-host-sensitive headers on a
/// cross-host hop. `replayable` reports whether the *original* request body
/// could be re-sent (it is `false` for a single-shot stream, which `body` has
/// already been downgraded to [`Body::Empty`]). Returns `None` when the redirect
/// cannot be followed safely — a 307/308 whose original body was a non-replayable
/// stream — so the caller returns the 3xx response untouched rather than
/// re-dispatching with a silently emptied body.
pub(crate) fn next_request(
    previous: &HttpRequest,
    target: Url,
    status: u16,
    body: Body,
    replayable: bool,
) -> Option<HttpRequest> {
    let cross_host = !same_origin(&previous.url, &target);
    let (method, body) = match status {
        // 303 See Other: always GET, body dropped.
        303 => (Method::Get, Body::Empty),
        // 301/302: the de-facto browser behaviour downgrades POST to GET; any
        // other method (and its absence of a body) is preserved.
        301 | 302 if previous.method == Method::Post => (Method::Get, Body::Empty),
        301 | 302 => (previous.method, body),
        // 307/308: preserve method and body — but only a replayable body can be
        // re-sent; a consumed stream cannot, so refuse the hop.
        307 | 308 => {
            if !replayable {
                log_event!(
                    warn,
                    "not following {status} redirect: streamed body is single-shot"
                );
                return None;
            }
            (previous.method, body)
        }
        _ => (previous.method, body),
    };

    let mut headers = previous.headers.clone();
    if matches!(body, Body::Empty) {
        // The body was dropped (303, or a 301/302 POST->GET downgrade): the entity
        // headers that described it must not linger on the now-bodyless request.
        headers.remove("content-type");
        headers.remove("content-length");
        headers.remove("content-encoding");
        headers.remove("transfer-encoding");
    }
    if cross_host {
        // RFC: do not leak credentials across hosts. The jar re-derives any
        // Cookie for the new host on its own, so drop a per-request one too.
        headers.remove("authorization");
        headers.remove("cookie");
        log_event!(
            debug,
            "cross-host redirect to {}: stripped Authorization/Cookie",
            target.host()
        );
    }

    Some(HttpRequest {
        method,
        url: target,
        headers,
        body,
        allow_redirect: previous.allow_redirect,
    })
}

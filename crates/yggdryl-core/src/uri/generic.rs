//! [`Uri`] — an RFC 3986 generic URI (or a POSIX-normalized filesystem path).

use core::fmt;
use core::fmt::Write as _;
use std::borrow::Cow;

use super::HashWrite;
use super::{percent, Authority, UriError, UriParts, Url};

/// A generic URI split into its RFC 3986 components, doubling as a filesystem-path
/// abstraction. Any component may be absent; a bare path (no scheme, no authority) is a
/// perfectly good `Uri`.
///
/// DESIGN: the parser is written from scratch against RFC 3986 Appendix B rather than
/// pulling the `url` crate — the core is the minimal-dependency foundation (its only
/// dependency is `arrow-buffer`), and a full URL crate would drag in `idna`,
/// `percent-encoding`, and friends for a component split we do compactly here.
///
/// DESIGN: **paths are standardized POSIX slash-based.** A Windows drive path
/// (`C:\Users\a.txt`), a UNC path (`\\server\share`), or any back-slashed path is
/// detected and every `\` is rewritten to `/` on the way in, so the stored `path` always
/// uses forward slashes. A single letter followed by `:` and a slash is treated as a
/// **drive letter** kept in the path — never a one-letter URI scheme.
///
/// ```
/// use yggdryl_core::uri::Uri;
///
/// let uri = Uri::parse_str("https://user:pw@example.com:8080/a/b.txt?q=1#frag").unwrap();
/// assert_eq!(uri.scheme(), Some("https"));
/// assert_eq!(uri.host(), Some("example.com"));
/// assert_eq!(uri.port(), Some(8080));
/// assert_eq!(uri.path(), "/a/b.txt");
/// assert_eq!(uri.name(), Some("b.txt"));
/// assert_eq!(uri.extension(), Some("txt"));
/// assert_eq!(uri.query(), Some("q=1"));
/// assert_eq!(uri.fragment(), Some("frag"));
///
/// // A Windows drive path is normalized to POSIX slashes, with no scheme.
/// let path = Uri::parse_str(r"C:\Users\x\a.tar.gz").unwrap();
/// assert_eq!(path.scheme(), None);
/// assert_eq!(path.path(), "C:/Users/x/a.tar.gz");
/// assert_eq!(path.extensions(), vec!["tar", "gz"]);
/// ```
#[derive(Debug, Clone, Default)]
pub struct Uri {
    scheme: Option<String>,
    authority: Option<Authority>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Uri {
    /// Parses `s` into its RFC 3986 components, or normalizes a bare filesystem path.
    ///
    /// The split follows RFC 3986 Appendix B: optional `scheme ":"`, optional
    /// `"//" authority`, `path`, optional `"?" query`, optional `"#" fragment`; the
    /// authority is `[ userinfo "@" ] host [ ":" port ]`. A Windows drive/UNC/back-slashed
    /// path is instead routed through [`from_path`](Uri::from_path) and slash-normalized.
    ///
    /// # Errors
    /// [`UriError::EmptyScheme`] / [`UriError::InvalidScheme`] for a malformed scheme,
    /// [`UriError::InvalidPort`] for a non-numeric or out-of-range port.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::parse_str("mailto:a@b.com").unwrap().scheme(), Some("mailto"));
    /// assert_eq!(Uri::parse_str("/a/b/c").unwrap().path(), "/a/b/c");
    /// ```
    pub fn parse_str(s: &str) -> Result<Uri, UriError> {
        // DESIGN: filesystem paths are detected up front and normalized to POSIX slashes.
        // A drive path (`X:\` / `X:/`) or UNC path (`\\…`) is unambiguous; any other
        // scheme-less, back-slashed string is a Windows relative path.
        if is_drive_path(s) || is_unc_path(s) {
            return Ok(Uri::from_path(s));
        }

        let (scheme, rest) = split_scheme(s)?;

        let (authority, after_authority) = if let Some(after) = rest.strip_prefix("//") {
            let end = after.find(['/', '?', '#']).unwrap_or(after.len());
            (Some(parse_authority(&after[..end])?), &after[end..])
        } else {
            (None, rest)
        };

        let (before_fragment, fragment) = match after_authority.split_once('#') {
            Some((head, frag)) => (head, Some(frag.to_string())),
            None => (after_authority, None),
        };
        let (path_str, query) = match before_fragment.split_once('?') {
            Some((head, q)) => (head, Some(q.to_string())),
            None => (before_fragment, None),
        };

        // A scheme-less, authority-less, back-slashed input is a bare Windows path.
        if scheme.is_none() && authority.is_none() && path_str.contains('\\') {
            let mut uri = Uri::from_path(path_str);
            uri.query = query;
            uri.fragment = fragment;
            return Ok(uri);
        }

        Ok(Uri {
            scheme,
            authority,
            path: normalize_slashes(path_str).into_owned(),
            query,
            fragment,
        })
    }

    /// Builds a scheme-less, authority-less `Uri` from a filesystem path, rewriting every
    /// back-slash to a forward slash so the stored path is POSIX slash-based.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_path(r"a\b\c").path(), "a/b/c");
    /// assert_eq!(Uri::from_path("/a/b/c").path(), "/a/b/c");
    /// ```
    pub fn from_path(path: &str) -> Uri {
        let mut uri = Uri::default();
        uri.set_path(path); // the one place path normalization + encoding lives
        uri
    }

    /// A `file://` URL from an **absolute** filesystem path — the addressing form a local
    /// source reports. The path is POSIX-slash-normalized and percent-encoded (as
    /// [`from_path`](Uri::from_path)), rooted with a leading slash (a Windows drive path
    /// `C:/x` becomes `/C:/x`), and given the `file` scheme with an empty host, so it renders
    /// `file:///C:/x` (Windows) or `file:///home/x` (POSIX).
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_file_path("/home/x/a.txt").to_string(), "file:///home/x/a.txt");
    /// assert_eq!(Uri::from_file_path(r"C:\Users\x").to_string(), "file:///C:/Users/x");
    /// ```
    pub fn from_file_path(path: &str) -> Uri {
        let mut uri = Uri::from_path(path); // encodes + POSIX-normalizes the path
        if !uri.path.starts_with('/') {
            uri.path.insert(0, '/'); // root a drive/relative-looking path for a file URL
        }
        uri.scheme = Some("file".to_string());
        uri.authority = Some(Authority::from_host("")); // empty host -> file://…
        uri
    }

    /// The scheme, if any.
    pub fn scheme(&self) -> Option<&str> {
        self.scheme.as_deref()
    }

    /// The authority, if any.
    pub fn authority(&self) -> Option<&Authority> {
        self.authority.as_ref()
    }

    /// The userinfo user, if this URI has an authority carrying one.
    pub fn user(&self) -> Option<&str> {
        self.authority.as_ref().and_then(Authority::user)
    }

    /// The userinfo password, if this URI has an authority carrying one.
    pub fn password(&self) -> Option<&str> {
        self.authority.as_ref().and_then(Authority::password)
    }

    /// The host, if this URI has an authority. An IPv6 literal keeps its brackets (`"[::1]"`);
    /// use [`host_unbracketed`](Uri::host_unbracketed) for the bare address.
    pub fn host(&self) -> Option<&str> {
        self.authority.as_ref().map(Authority::host)
    }

    /// Whether this URI's host is a bracketed IPv6 literal (`false` if it has no authority) —
    /// see [`Authority::host_is_ipv6`].
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert!(Uri::parse_str("http://[::1]:8080/p").unwrap().host_is_ipv6());
    /// assert!(!Uri::parse_str("http://example.com/p").unwrap().host_is_ipv6());
    /// ```
    pub fn host_is_ipv6(&self) -> bool {
        self.authority.as_ref().is_some_and(Authority::host_is_ipv6)
    }

    /// The host with any IPv6 brackets stripped, if this URI has an authority — the bare
    /// address to hand to a socket API. See [`Authority::host_unbracketed`].
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::parse_str("http://[::1]:80/p").unwrap().host_unbracketed(), Some("::1"));
    /// assert_eq!(Uri::parse_str("http://h/p").unwrap().host_unbracketed(), Some("h"));
    /// ```
    pub fn host_unbracketed(&self) -> Option<&str> {
        self.authority.as_ref().map(Authority::host_unbracketed)
    }

    /// The port, if this URI has an authority carrying one. This is the port **as written**;
    /// for the port to actually connect to (falling back to the scheme's default) use
    /// [`port_or_default`](Uri::port_or_default).
    pub fn port(&self) -> Option<u16> {
        self.authority.as_ref().and_then(Authority::port)
    }

    /// The default port registered for this URI's scheme, or `None` when it is scheme-less or
    /// the scheme has no known default — see [`default_port`](super::default_port). A pure
    /// lookup: it does **not** read or need the authority.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::parse_str("https://h/p").unwrap().default_port(), Some(443));
    /// assert_eq!(Uri::parse_str("/just/a/path").unwrap().default_port(), None); // no scheme
    /// ```
    pub fn default_port(&self) -> Option<u16> {
        self.scheme.as_deref().and_then(super::default_port)
    }

    /// The **effective** port to connect to: the explicit [`port`](Uri::port) if the authority
    /// carries one, otherwise the scheme's [`default_port`](Uri::default_port). `None` when
    /// neither is known. This is derived on read — the stored URI is untouched, so its
    /// canonical form still round-trips.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::parse_str("https://h/p").unwrap().port_or_default(), Some(443)); // default
    /// assert_eq!(Uri::parse_str("https://h:8443/p").unwrap().port_or_default(), Some(8443)); // explicit
    /// assert_eq!(Uri::parse_str("//h/p").unwrap().port_or_default(), None); // scheme-less
    /// ```
    pub fn port_or_default(&self) -> Option<u16> {
        self.port().or_else(|| self.default_port())
    }

    /// The path, always POSIX slash-normalized (possibly empty).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// This URI's **filesystem path** — its [`path`](Uri::path) percent-**decoded** to the
    /// native form a filesystem call expects, with a leading-slash-rooted Windows drive path
    /// (`/C:/Users/x`) un-rooted back to `C:/Users/x`. This is exactly what a Python
    /// `os.PathLike.__fspath__` returns; it stays POSIX-slash-based (the project's normalized
    /// form) so it reads identically on every OS.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_file_path("/home/x/a b.txt").fspath(), "/home/x/a b.txt");
    /// assert_eq!(Uri::from_file_path(r"C:\Users\x").fspath(), "C:/Users/x");
    /// ```
    pub fn fspath(&self) -> String {
        let decoded = percent::decode(&self.path).into_owned();
        // A `file://` URL roots a Windows drive path with a leading slash; strip it back off so
        // the result is the plain filesystem path (`C:/…`), never the URL-rooted `/C:/…`.
        let bytes = decoded.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
        {
            decoded[1..].to_string()
        } else {
            decoded
        }
    }

    /// The query, if any (the text after `?`, without the `?`).
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// The fragment, if any (the text after `#`, without the `#`).
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// The last non-empty path segment (the filename), or `None` for an empty or
    /// directory-like path (one ending in `/`).
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_path("/a/b/c.txt").name(), Some("c.txt"));
    /// assert_eq!(Uri::from_path("/a/b/").name(), None);
    /// ```
    pub fn name(&self) -> Option<&str> {
        let seg = self.path.rsplit('/').next().unwrap_or("");
        if seg.is_empty() {
            None
        } else {
            Some(seg)
        }
    }

    /// The filename without its **last** extension. A leading dot marks a hidden file
    /// (`.bashrc`) whose dot is not an extension separator, so its stem is the whole name.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_path("/x/archive.tar.gz").stem(), Some("archive.tar"));
    /// assert_eq!(Uri::from_path("/x/.bashrc").stem(), Some(".bashrc"));
    /// ```
    pub fn stem(&self) -> Option<&str> {
        let name = self.name()?;
        Some(ext_dot(name).map_or(name, |i| &name[..i]))
    }

    /// The **parent URI** — this URI with its last path segment removed — or `None` at a
    /// root (no path segment left to strip). The inverse of [`joinpath`](Uri::joinpath):
    /// `base.joinpath("x").parent()` addresses `base` again (for a rooted or authority-backed
    /// path). Only the path changes; scheme / authority / query / fragment are kept.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let u = Uri::parse_str("https://h/a/b/c.txt?q=1").unwrap();
    /// assert_eq!(u.parent().unwrap().to_string(), "https://h/a/b?q=1");
    /// assert_eq!(u.parent().unwrap().parent().unwrap().path(), "/a");
    /// assert!(Uri::parse_str("https://h").unwrap().parent().is_none());
    /// ```
    pub fn parent(&self) -> Option<Uri> {
        let trimmed = self.path.trim_end_matches('/');
        if trimmed.is_empty() {
            return None; // a root (no path) has no parent
        }
        // `path` is stored percent-encoded; a slice of it is still valid encoding, so assign
        // it directly (never re-encode through `set_path`, which would double-encode).
        let parent_path = match trimmed.rfind('/') {
            Some(cut) => &trimmed[..cut], // keep the leading portion (may be "")
            None => "",                   // a single segment: parent is the empty-path root
        };
        let mut out = self.clone();
        out.path = parent_path.to_string();
        Some(out)
    }

    /// An iterator over this URI's **ancestors**, nearest first: [`parent`](Uri::parent),
    /// then its parent, and so on up to the root, ending when no path segment remains.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let u = Uri::from_path("/a/b/c.txt");
    /// let paths: Vec<String> = u.parents().map(|p| p.path().to_string()).collect();
    /// assert_eq!(paths, vec!["/a/b", "/a", ""]);
    /// ```
    pub fn parents(&self) -> impl Iterator<Item = Uri> {
        std::iter::successors(self.parent(), Uri::parent)
    }

    /// The RFC 3986 top-level components bundled into one owned [`UriParts`] — the
    /// destructuring counterpart of the individual [`scheme`](Uri::scheme) /
    /// [`authority`](Uri::authority) / [`path`](Uri::path) / [`query`](Uri::query) /
    /// [`fragment`](Uri::fragment) accessors.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let parts = Uri::parse_str("https://h/a?q=1").unwrap().parts();
    /// assert_eq!(parts.scheme.as_deref(), Some("https"));
    /// assert_eq!(parts.path, "/a");
    /// assert_eq!(parts.to_string(), "https://h/a?q=1"); // re-renders the URI
    /// ```
    pub fn parts(&self) -> UriParts {
        UriParts {
            scheme: self.scheme.clone(),
            authority: self.authority.as_ref().map(Authority::to_string),
            path: self.path.clone(),
            query: self.query.clone(),
            fragment: self.fragment.clone(),
        }
    }

    /// The **media type** of the resource this URI addresses, inferred from its path
    /// extensions ([`MediaType::from_extensions`](crate::mediatype::MediaType::from_extensions)):
    /// `archive.tar.gz` → `application/x-tar, application/gzip`. Empty when no extension is
    /// recognized. See [`mime_type`](Uri::mime_type) for the single primary type.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let m = Uri::from_path("/data/archive.tar.gz").media_type();
    /// assert_eq!(m.essences(), vec!["application/x-tar", "application/gzip"]);
    /// ```
    pub fn media_type(&self) -> crate::mediatype::MediaType {
        crate::mediatype::MediaType::from_extensions(self.extensions())
    }

    /// The **primary mime type** inferred from this URI's file name — its last extension via
    /// the default catalog, else the `application/octet-stream` fallback (never `None`, so a
    /// caller always has a type).
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_path("/x/report.pdf").mime_type().essence(), "application/pdf");
    /// assert_eq!(Uri::from_path("/x/mystery").mime_type().essence(), "application/octet-stream");
    /// ```
    pub fn mime_type(&self) -> crate::mimetype::MimeType {
        self.name()
            .and_then(crate::mimetype::MimeType::from_name)
            .unwrap_or_else(crate::mimetype::MimeType::octet_stream)
    }

    /// The last extension of the filename (without the dot), or `None` for a name with no
    /// extension, a trailing dot, or a hidden dotfile (`.bashrc`).
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_path("/x/archive.tar.gz").extension(), Some("gz"));
    /// assert_eq!(Uri::from_path("/x/.bashrc").extension(), None);
    /// ```
    pub fn extension(&self) -> Option<&str> {
        let name = self.name()?;
        ext_dot(name)
            .filter(|&i| i + 1 < name.len())
            .map(|i| &name[i + 1..])
    }

    /// Every extension of a multi-dot filename, outermost-last
    /// (`archive.tar.gz` → `["tar", "gz"]`). A hidden dotfile's leading dot is ignored, so
    /// it contributes no extension. Empty for a name with no extension or no filename.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::from_path("/x/a.b.c.d").extensions(), vec!["b", "c", "d"]);
    /// assert!(Uri::from_path("/x/.bashrc").extensions().is_empty());
    /// ```
    pub fn extensions(&self) -> Vec<String> {
        let Some(name) = self.name() else {
            return Vec::new();
        };
        // A trailing dot yields no valid outermost extension — stay coherent with
        // `extension()` (which returns `None` for a trailing dot) so `extension()` always
        // equals `extensions().last()`.
        if name.ends_with('.') {
            return Vec::new();
        }
        // Skip the first character so a leading dot (a hidden file) is not a separator.
        let first = name.chars().next().map(char::len_utf8).unwrap_or(0);
        match name[first..].find('.') {
            Some(rel) => name[first + rel + 1..]
                .split('.')
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect(),
            None => Vec::new(),
        }
    }

    // ---- builder mutators (consume + return Self) ----------------------------------

    /// Returns this URI with the scheme set.
    pub fn with_scheme(mut self, scheme: &str) -> Self {
        self.set_scheme(scheme);
        self
    }

    /// Returns this URI with the whole authority replaced (pass `None` to drop it) — attaches
    /// an [`Authority`] built elsewhere in one call, rather than field by field.
    pub fn with_authority(mut self, authority: Option<Authority>) -> Self {
        self.set_authority(authority);
        self
    }

    /// Returns this URI with the host set (creating an authority if absent).
    pub fn with_host(mut self, host: &str) -> Self {
        self.set_host(host);
        self
    }

    /// Returns this URI with the port set (creating an authority if absent).
    pub fn with_port(mut self, port: u16) -> Self {
        self.set_port(port);
        self
    }

    /// Returns this URI with the userinfo user set (creating an authority if absent).
    pub fn with_user(mut self, user: &str) -> Self {
        self.set_user(user);
        self
    }

    /// Returns this URI with the userinfo password set (creating an authority if absent).
    pub fn with_password(mut self, password: &str) -> Self {
        self.set_password(password);
        self
    }

    /// Returns this URI with the path set, re-normalized to POSIX slashes.
    pub fn with_path(mut self, path: &str) -> Self {
        self.set_path(path);
        self
    }

    /// Returns this URI with the query set.
    pub fn with_query(mut self, query: &str) -> Self {
        self.set_query(query);
        self
    }

    /// Returns this URI with the fragment set.
    pub fn with_fragment(mut self, fragment: &str) -> Self {
        self.set_fragment(fragment);
        self
    }

    // ---- in-place setters ----------------------------------------------------------

    /// Sets the scheme.
    pub fn set_scheme(&mut self, scheme: &str) {
        self.scheme = Some(scheme.to_string());
    }

    /// Replaces the whole authority (pass `None` to drop it).
    pub fn set_authority(&mut self, authority: Option<Authority>) {
        self.authority = authority;
    }

    /// Sets the host, creating an authority if this URI had none.
    pub fn set_host(&mut self, host: &str) {
        self.authority_mut().set_host(host);
    }

    /// Sets the port, creating an authority if this URI had none.
    pub fn set_port(&mut self, port: u16) {
        self.authority_mut().set_port(Some(port));
    }

    /// Sets the userinfo user (percent-encoded for storage), creating an authority if this
    /// URI had none.
    pub fn set_user(&mut self, user: &str) {
        let user = percent::encode(user, percent::is_userinfo_safe);
        self.authority_mut().set_user(Some(&user));
    }

    /// Sets the userinfo password (percent-encoded for storage), creating an authority if
    /// this URI had none.
    pub fn set_password(&mut self, password: &str) {
        let password = percent::encode(password, percent::is_userinfo_safe);
        self.authority_mut().set_password(Some(&password));
    }

    /// Sets the path, re-normalizing back-slashes to forward slashes and percent-encoding
    /// for storage (the `/` separators are preserved).
    pub fn set_path(&mut self, path: &str) {
        self.path =
            percent::encode_owned(normalize_slashes(path).into_owned(), percent::is_path_safe);
    }

    /// Sets the query, percent-encoded for storage (its `&`/`=` structure is preserved).
    pub fn set_query(&mut self, query: &str) {
        self.query = Some(percent::encode(query, percent::is_query_safe).into_owned());
    }

    /// Sets the fragment, percent-encoded for storage.
    pub fn set_fragment(&mut self, fragment: &str) {
        self.fragment = Some(percent::encode(fragment, percent::is_query_safe).into_owned());
    }

    fn authority_mut(&mut self) -> &mut Authority {
        self.authority.get_or_insert_with(Authority::default)
    }

    // ---- combinators (copy / joinpath / merge) -------------------------------------

    /// An explicit copy of this URI — the cross-language name for a clone (Rust already has
    /// [`Clone`], but Python and Node reach the same value through `copy`). Pairs with the
    /// `with_*` builders for a one-line "copy, changing one thing".
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let base = Uri::parse_str("https://h/a").unwrap();
    /// assert_eq!(base.copy(), base);
    /// ```
    pub fn copy(&self) -> Uri {
        self.clone()
    }

    /// Returns this URI with `path` **joined onto its path**, purely lexically (like
    /// `pathlib` — no `.`/`..` resolution), keeping every other component. The argument is
    /// slash-normalized and percent-encoded like [`set_path`](Uri::set_path). Joining is
    /// *correct*: exactly one `/` sits at the seam (a trailing slash on the base and a
    /// segment are never doubled), an **absolute** segment (leading `/`) replaces the path,
    /// an empty segment is a no-op, and when the URI has an authority the result stays rooted
    /// so the segment can't fuse into the host.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let base = Uri::parse_str("https://api.example.com/v1").unwrap();
    /// assert_eq!(base.joinpath("users").to_string(), "https://api.example.com/v1/users");
    /// // A trailing slash on the base is not doubled; multi-segment input is fine.
    /// assert_eq!(base.joinpath("users/").joinpath("42").path(), "/v1/users/42");
    /// // An absolute segment resets the path; the query/fragment are untouched.
    /// let q = Uri::parse_str("https://h/a?x=1#f").unwrap();
    /// assert_eq!(q.joinpath("/b").to_string(), "https://h/b?x=1#f");
    /// // With an authority, a relative segment onto an empty path stays rooted.
    /// assert_eq!(Uri::parse_str("https://h").unwrap().joinpath("p").path(), "/p");
    /// ```
    pub fn joinpath(&self, path: &str) -> Uri {
        let normalized = normalize_slashes(path);
        let segment = percent::encode(&normalized, percent::is_path_safe);
        let mut out = self.clone();
        if segment.is_empty() {
            return out; // joining nothing yields a plain copy
        }
        if segment.starts_with('/') {
            out.path = segment.into_owned(); // absolute segment replaces the path
            return out;
        }
        out.path = if self.path.is_empty() {
            // A relative segment must stay rooted when an authority is present, else it would
            // fuse into the host on render (`//h` + `p` -> `//hp`).
            if self.authority.is_some() {
                let mut joined = String::with_capacity(1 + segment.len());
                joined.push('/');
                joined.push_str(&segment);
                joined
            } else {
                segment.into_owned()
            }
        } else {
            let base = self.path.trim_end_matches('/');
            let mut joined = String::with_capacity(base.len() + 1 + segment.len());
            joined.push_str(base);
            joined.push('/');
            joined.push_str(&segment);
            joined
        };
        out
    }

    /// Returns a copy of this URI **overlaid** by `other`: for each component `other`'s value
    /// wins when it is present (a `Some` scheme/authority/query/fragment, or a non-empty
    /// path), otherwise this URI's is kept. A mechanical component merge — no re-parsing — so
    /// `base.merge_with(&patch)` applies only the fields `patch` actually sets. Merging with a
    /// default (empty) URI returns a copy unchanged.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let base = Uri::parse_str("https://prod.example.com/v1?trace=1").unwrap();
    /// // A patch that only carries an authority swaps the host, keeping scheme/path/query.
    /// let patch = Uri::parse_str("//staging.example.com").unwrap();
    /// assert_eq!(base.merge_with(&patch).to_string(), "https://staging.example.com/v1?trace=1");
    /// ```
    pub fn merge_with(&self, other: &Uri) -> Uri {
        Uri {
            scheme: other.scheme.clone().or_else(|| self.scheme.clone()),
            authority: other.authority.clone().or_else(|| self.authority.clone()),
            path: if other.path.is_empty() {
                self.path.clone()
            } else {
                other.path.clone()
            },
            query: other.query.clone().or_else(|| self.query.clone()),
            fragment: other.fragment.clone().or_else(|| self.fragment.clone()),
        }
    }

    // ---- byte codec + interchange --------------------------------------------------

    /// The canonical URI string as UTF-8 bytes.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("sc://h/p?q#f").unwrap();
    /// assert_eq!(uri.serialize_bytes(), b"sc://h/p?q#f");
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        self.to_canonical().into_bytes()
    }

    /// Decodes a URI from the UTF-8 bytes produced by [`serialize_bytes`](Uri::serialize_bytes)
    /// — the exact inverse.
    ///
    /// # Errors
    /// [`UriError::NonUtf8`] if the bytes are not UTF-8, or any [`parse_str`](Uri::parse_str) error.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("sc://h/p").unwrap();
    /// assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
    /// ```
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Uri, UriError> {
        let text =
            core::str::from_utf8(bytes).map_err(|_| UriError::NonUtf8 { len: bytes.len() })?;
        Uri::parse_str(text)
    }

    /// A **portable, relocatable** string form for pickling / cross-environment transport: a
    /// `file://` URI addressing a path under the **current user's home** directory is emitted
    /// with a leading `~`, and one under the system **temp** directory with a leading `$TMP`, so
    /// [`from_portable_str`](Uri::from_portable_str) rebuilds it against *another* machine's home
    /// / temp roots (a file written to `~/data` on one host resolves to that host's home). The
    /// **most specific** (longest) matching root wins, so on platforms whose temp dir lives under
    /// home (Windows' `%USERPROFILE%\AppData\Local\Temp`) a temp path still relocates as `$TMP`.
    /// Any other URI — a different scheme, or a file path outside both roots — is its exact
    /// [`to_string`](std::string::ToString::to_string), so the round-trip is always lossless.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    /// if home.is_some() {
    ///     let uri = Uri::from_file_path(&format!(
    ///         "{}/notes/today.txt",
    ///         std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap()
    ///     ));
    ///     assert_eq!(uri.to_portable_str(), "~/notes/today.txt");
    ///     assert_eq!(Uri::from_portable_str("~/notes/today.txt").unwrap(), uri);
    /// }
    /// let web = Uri::parse_str("https://h/p").unwrap();
    /// assert_eq!(web.to_portable_str(), "https://h/p"); // other schemes are unchanged
    /// ```
    pub fn to_portable_str(&self) -> String {
        let full = self.to_string();
        if self.scheme() != Some("file") {
            return full; // only local file addresses relocate; everything else is already portable
        }
        // Prefer the longest matching root so a temp dir nested under home relocates as `$TMP`.
        let mut best: Option<(usize, String)> = None;
        for (token, root) in [("$TMP", portable_temp_root()), ("~", portable_home_root())] {
            let Some(root) = root else { continue };
            let root = root.trim_end_matches('/');
            if let Some(rest) = full.strip_prefix(root) {
                if rest.is_empty() || rest.starts_with('/') {
                    let longer = best.as_ref().is_none_or(|(len, _)| root.len() > *len);
                    if longer {
                        best = Some((root.len(), format!("{token}{rest}")));
                    }
                }
            }
        }
        best.map_or(full, |(_, portable)| portable)
    }

    /// Rebuilds a URI from the [`to_portable_str`](Uri::to_portable_str) form, expanding a leading
    /// `~` against **this** environment's home directory and `$TMP` against its temp directory —
    /// the reconstruction half of portable pickling. A string with neither token parses as an
    /// ordinary URI, so it is the exact inverse of `to_portable_str` in every environment.
    ///
    /// # Errors
    /// Any [`parse_str`](Uri::parse_str) error from the reconstructed URI string.
    pub fn from_portable_str(s: &str) -> Result<Uri, UriError> {
        if let Some(rest) = s.strip_prefix('~') {
            if rest.is_empty() || rest.starts_with('/') {
                if let Some(root) = portable_home_root() {
                    return Uri::parse_str(&format!("{}{rest}", root.trim_end_matches('/')));
                }
            }
        }
        if let Some(rest) = s.strip_prefix("$TMP") {
            if rest.is_empty() || rest.starts_with('/') {
                if let Some(root) = portable_temp_root() {
                    return Uri::parse_str(&format!("{}{rest}", root.trim_end_matches('/')));
                }
            }
        }
        Uri::parse_str(s)
    }

    /// Converts into a [`Url`], failing if this URI has no scheme.
    ///
    /// # Errors
    /// [`UriError::MissingScheme`] when the URI is not absolute.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert!(Uri::parse_str("https://h/").unwrap().into_url().is_ok());
    /// assert!(Uri::parse_str("/relative").unwrap().into_url().is_err());
    /// ```
    pub fn into_url(self) -> Result<Url, UriError> {
        Url::try_from(self)
    }

    /// Borrows this URI as a [`Url`] by cloning, failing if it has no scheme.
    ///
    /// # Errors
    /// [`UriError::MissingScheme`] when the URI is not absolute.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// assert_eq!(Uri::parse_str("sc://h").unwrap().to_url().unwrap().scheme(), "sc");
    /// ```
    pub fn to_url(&self) -> Result<Url, UriError> {
        Url::try_from(self.clone())
    }

    // ---- query parameters (map access + CRUD) --------------------------------------

    /// The first value of query parameter `key`, or `None` if absent, as **stored**
    /// (percent-encoded) — use [`param_decoded`](Uri::param_decoded) for the
    /// decoded value. Zero-copy: the value borrows the query string. A bare `key` with no
    /// `=` reads as an empty value. `key` is matched by its encoded form, so pass it decoded.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    /// assert_eq!(uri.param("a"), Some("1")); // first occurrence wins
    /// assert_eq!(uri.param("b"), Some("2"));
    /// assert_eq!(uri.param("z"), None);
    /// ```
    pub fn param(&self, key: &str) -> Option<&str> {
        let query = self.query.as_deref()?;
        let key = percent::encode(key, percent::is_param_safe);
        let key: &str = &key;
        query_pairs(query).find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// Every value of query parameter `key`, in order — for a repeated key such as
    /// `?a=1&a=2`. Empty if the key is absent. Zero-copy: the values borrow the query.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    /// assert_eq!(uri.param_all("a"), vec!["1", "3"]);
    /// assert!(uri.param_all("z").is_empty());
    /// ```
    pub fn param_all(&self, key: &str) -> Vec<&str> {
        let Some(query) = self.query.as_deref() else {
            return Vec::new();
        };
        let key = percent::encode(key, percent::is_param_safe);
        let key: &str = &key;
        // Pre-size to the parameter count (an upper bound on the matches) so the collect
        // allocates once.
        let mut values = Vec::with_capacity(query.bytes().filter(|&b| b == b'&').count() + 1);
        values.extend(
            query_pairs(query)
                .filter(|(k, _)| *k == key)
                .map(|(_, v)| v),
        );
        values
    }

    /// All query parameters as ordered `(key, value)` pairs — the map view, from which a
    /// dict/map is built directly. Empty when there is no query. Zero-copy borrows into the
    /// query; the returned `Vec` is the only allocation, pre-sized to one.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("http://h/p?a=1&b=2").unwrap();
    /// assert_eq!(uri.params(), vec![("a", "1"), ("b", "2")]);
    /// ```
    pub fn params(&self) -> Vec<(&str, &str)> {
        let Some(query) = self.query.as_deref() else {
            return Vec::new();
        };
        // Pre-size to the separator count + 1 (an upper bound), so `collect` allocates once.
        let mut pairs = Vec::with_capacity(query.bytes().filter(|&b| b == b'&').count() + 1);
        pairs.extend(query_pairs(query));
        pairs
    }

    /// All query parameters **grouped by key** — each distinct key (in first-appearance order)
    /// mapped to **all** of its values (in order). This is the `dict[str, tuple[str, …]]` view the
    /// bindings expose, so a repeated key round-trips faithfully: `?a=1&a=3` is the single entry
    /// `("a", ["1", "3"])` rather than two colliding pairs. Empty when there is no query.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    /// assert_eq!(
    ///     uri.params_grouped(),
    ///     vec![("a", vec!["1", "3"]), ("b", vec!["2"])],
    /// );
    /// ```
    pub fn params_grouped(&self) -> Vec<(&str, Vec<&str>)> {
        let Some(query) = self.query.as_deref() else {
            return Vec::new();
        };
        let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
        for (k, v) in query_pairs(query) {
            match groups.iter_mut().find(|(gk, _)| *gk == k) {
                Some((_, values)) => values.push(v),
                None => groups.push((k, vec![v])),
            }
        }
        groups
    }

    /// The first value of query parameter `key`, **percent-decoded** — the value the caller
    /// originally set (or the decoded form of a parsed value). Borrows the query when there
    /// is nothing to decode, otherwise owns the decoded string.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("http://h/p?q=a%20b%26c").unwrap();
    /// assert_eq!(uri.param("q").as_deref(), Some("a%20b%26c")); // stored (encoded)
    /// assert_eq!(uri.param_decoded("q").as_deref(), Some("a b&c")); // decoded
    /// ```
    pub fn param_decoded(&self, key: &str) -> Option<Cow<'_, str>> {
        self.param(key).map(percent::decode)
    }

    /// Every value of query parameter `key`, in order, each **percent-decoded**.
    pub fn param_all_decoded(&self, key: &str) -> Vec<Cow<'_, str>> {
        self.param_all(key)
            .into_iter()
            .map(percent::decode)
            .collect()
    }

    /// All query parameters as ordered `(key, value)` pairs, each **percent-decoded** — the
    /// decoded map view.
    pub fn params_decoded(&self) -> Vec<(Cow<'_, str>, Cow<'_, str>)> {
        self.params()
            .into_iter()
            .map(|(key, value)| (percent::decode(key), percent::decode(value)))
            .collect()
    }

    /// Whether query parameter `key` is present. Zero-copy; `key` is matched by its encoded
    /// form, so pass it decoded.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let uri = Uri::parse_str("http://h/p?a=1").unwrap();
    /// assert!(uri.has_param("a"));
    /// assert!(!uri.has_param("b"));
    /// ```
    pub fn has_param(&self, key: &str) -> bool {
        let key = percent::encode(key, percent::is_param_safe);
        let key: &str = &key;
        self.query
            .as_deref()
            .is_some_and(|query| query_pairs(query).any(|(k, _)| k == key))
    }

    /// Sets query parameter `key` to `value` (map semantics): updates the **first**
    /// occurrence in place, drops any later occurrences, or appends `key=value` if the key
    /// was absent — creating the query if there was none. Both `key` and `value` are
    /// **percent-encoded** for storage, so a value containing `&`, `=`, `#`, or a space is
    /// stored safely. Rebuilds the query with a single pre-sized allocation.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let mut uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    /// uri.set_param("a", "9");
    /// assert_eq!(uri.query(), Some("a=9&b=2")); // first updated, duplicate dropped
    /// uri.set_param("c", "a b&c");
    /// assert_eq!(uri.query(), Some("a=9&b=2&c=a%20b%26c")); // value encoded on store
    /// ```
    pub fn set_param(&mut self, key: &str, value: &str) {
        let key = percent::encode(key, percent::is_param_safe);
        let value = percent::encode(value, percent::is_param_safe);
        let (key, value): (&str, &str) = (&key, &value);
        let existing = self.query.as_deref().unwrap_or("");
        let mut out = String::with_capacity(existing.len() + key.len() + value.len() + 2);
        let mut written = false;
        for token in existing.split('&').filter(|token| !token.is_empty()) {
            if param_key(token) == key {
                if !written {
                    push_param(&mut out, key, value);
                    written = true;
                }
                // later duplicates of `key` are dropped
            } else {
                push_token(&mut out, token);
            }
        }
        if !written {
            push_param(&mut out, key, value);
        }
        self.query = Some(out);
    }

    /// [`set_param`](Uri::set_param) as a builder — returns the updated `Uri`.
    pub fn with_param(mut self, key: &str, value: &str) -> Self {
        self.set_param(key, value);
        self
    }

    /// Removes **every** occurrence of query parameter `key`, returning whether any were
    /// removed. Clears the query entirely if it becomes empty. Allocates only when the key
    /// is actually present (one pre-sized rebuild); an absent key is a no-op.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let mut uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    /// assert!(uri.remove_param("a"));
    /// assert_eq!(uri.query(), Some("b=2"));
    /// assert!(!uri.remove_param("z")); // absent -> no-op
    /// ```
    pub fn remove_param(&mut self, key: &str) -> bool {
        let Some(existing) = self.query.as_deref() else {
            return false;
        };
        let key = percent::encode(key, percent::is_param_safe);
        let key: &str = &key;
        if !query_pairs(existing).any(|(k, _)| k == key) {
            return false; // absent — no rebuild, no allocation
        }
        let mut out = String::with_capacity(existing.len());
        for token in existing.split('&').filter(|token| !token.is_empty()) {
            if param_key(token) != key {
                push_token(&mut out, token);
            }
        }
        self.query = (!out.is_empty()).then_some(out);
        true
    }

    /// [`remove_param`](Uri::remove_param) as a builder — returns the updated `Uri`.
    pub fn without_param(mut self, key: &str) -> Self {
        self.remove_param(key);
        self
    }

    /// **Bulk-updates** the query from `(key, value)` pairs in a single rebuild — each key
    /// is set with the same map semantics as [`set_param`](Uri::set_param)
    /// (updated in place, later existing duplicates dropped, appended if absent), and a key
    /// repeated in `params` takes its **last** value. Cheaper than calling
    /// `set_param` in a loop, which would rebuild the whole query each time.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let mut uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    /// uri.set_params(&[("a", "9"), ("c", "7")]);
    /// assert_eq!(uri.query(), Some("a=9&b=2&c=7")); // a updated (dup dropped), c appended
    /// ```
    pub fn set_params(&mut self, params: &[(&str, &str)]) {
        if params.is_empty() {
            return;
        }
        // Percent-encode (borrowing clean values), then deduplicate to one value per key
        // (last wins), preserving first-appearance order.
        let mut updates: Vec<(Cow<str>, Cow<str>)> = Vec::with_capacity(params.len());
        for &(key, value) in params {
            let key = percent::encode(key, percent::is_param_safe);
            let value = percent::encode(value, percent::is_param_safe);
            match updates.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = value,
                None => updates.push((key, value)),
            }
        }
        let existing = self.query.as_deref().unwrap_or("");
        let extra: usize = updates.iter().map(|(k, v)| k.len() + v.len() + 2).sum();
        let mut out = String::with_capacity(existing.len() + extra);
        let mut written = vec![false; updates.len()];
        for token in existing.split('&').filter(|token| !token.is_empty()) {
            if let Some(idx) = updates
                .iter()
                .position(|(k, _)| k.as_ref() == param_key(token))
            {
                if !written[idx] {
                    push_param(&mut out, updates[idx].0.as_ref(), updates[idx].1.as_ref());
                    written[idx] = true;
                }
                // later existing duplicates of an updated key are dropped
            } else {
                push_token(&mut out, token);
            }
        }
        for (idx, (key, value)) in updates.iter().enumerate() {
            if !written[idx] {
                push_param(&mut out, key.as_ref(), value.as_ref());
            }
        }
        self.query = (!out.is_empty()).then_some(out);
    }

    /// [`set_params`](Uri::set_params) as a builder — returns the updated `Uri`.
    pub fn with_params(mut self, params: &[(&str, &str)]) -> Self {
        self.set_params(params);
        self
    }

    /// Normalizes the query: drops empty tokens (a stray `&`) and **stable-sorts** the
    /// parameters by key, so equal keys keep their relative order. Lossless — repeated keys
    /// are preserved, not merged. Rebuilds in one pre-sized allocation.
    ///
    /// ```
    /// use yggdryl_core::uri::Uri;
    ///
    /// let mut uri = Uri::parse_str("http://h/p?c=3&a=1&b=2&a=0").unwrap();
    /// uri.normalize_params();
    /// assert_eq!(uri.query(), Some("a=1&a=0&b=2&c=3")); // sorted; 'a' order kept
    ///
    /// let mut messy = Uri::parse_str("http://h/p?b=2&&a=1&").unwrap();
    /// messy.normalize_params();
    /// assert_eq!(messy.query(), Some("a=1&b=2")); // empties cleaned, sorted
    /// ```
    pub fn normalize_params(&mut self) {
        let Some(existing) = self.query.as_deref() else {
            return;
        };
        let mut tokens: Vec<&str> =
            Vec::with_capacity(existing.bytes().filter(|&b| b == b'&').count() + 1);
        tokens.extend(existing.split('&').filter(|token| !token.is_empty()));
        tokens.sort_by(|a, b| param_key(a).cmp(param_key(b)));
        let mut out = String::with_capacity(existing.len());
        for token in tokens {
            push_token(&mut out, token);
        }
        self.query = (!out.is_empty()).then_some(out);
    }

    /// [`normalize_params`](Uri::normalize_params) as a builder — returns the normalized `Uri`.
    pub fn with_normalized_params(mut self) -> Self {
        self.normalize_params();
        self
    }

    // ---- canonical encoding (pre-sized: one allocation) ----------------------------

    /// An upper bound on the canonical string's byte length, used to pre-size the codec
    /// buffer so it allocates exactly once. The port digits are over-counted (at most 5),
    /// which only ever over-reserves — it never under-allocates.
    fn encoded_len(&self) -> usize {
        let mut len = self.path.len();
        if let Some(scheme) = &self.scheme {
            len += scheme.len() + 1; // "scheme:"
        }
        if let Some(authority) = &self.authority {
            len += 2 + authority.encoded_len(); // "//authority"
        }
        if let Some(query) = &self.query {
            len += 1 + query.len(); // "?query"
        }
        if let Some(fragment) = &self.fragment {
            len += 1 + fragment.len(); // "#fragment"
        }
        len
    }

    /// The canonical string built into a pre-sized buffer, so it allocates **exactly once**
    /// — `Display` alone starts from an empty `String` and reallocates several times as it
    /// grows for a long URI. This is the single source of both the byte codec and the
    /// value-semantics comparison, so they stay in lock-step with `Display`.
    fn to_canonical(&self) -> String {
        let mut buffer = String::with_capacity(self.encoded_len());
        let _ = write!(buffer, "{self}");
        buffer
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(scheme) = &self.scheme {
            write!(f, "{scheme}:")?;
        }
        if let Some(authority) = &self.authority {
            write!(f, "//{authority}")?;
        }
        f.write_str(&self.path)?;
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}

// Value semantics by canonical string: equal iff `serialize_bytes` (the canonical string's
// bytes) are equal, and equal values hash equal.
impl PartialEq for Uri {
    fn eq(&self, other: &Self) -> bool {
        // Pre-sized canonical strings (one allocation each) rather than `to_string`'s
        // grow-from-empty; the canonical string, not the components, is the identity — a
        // password with no user and `user = Some("")` render alike and must compare equal.
        self.to_canonical() == other.to_canonical()
    }
}

impl Eq for Uri {}

impl core::hash::Hash for Uri {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Stream the canonical string into the hasher without allocating a `String`.
        let _ = write!(HashWrite(&mut *state), "{self}");
        state.write_u8(0xff);
    }
}

// -------------------------------------------------------------------------------------
// RFC 3986 parsing helpers
// -------------------------------------------------------------------------------------

/// The index of the **extension-separator** dot in a filename — the last `.` that is not a
/// leading dot (a leading dot marks a hidden file, `.bashrc`, not an extension). The one
/// place the hidden-dotfile rule shared by [`Uri::stem`] and [`Uri::extension`] lives.
fn ext_dot(name: &str) -> Option<usize> {
    name.rfind('.').filter(|&i| i > 0)
}

/// Rewrites every back-slash to a forward slash (DESIGN: POSIX slash-based paths). Returns
/// the input **borrowed** when it holds no back-slash — the common POSIX case — so a clean
/// path costs no allocation here (the caller then decides whether it needs to own it).
fn normalize_slashes(path: &str) -> Cow<'_, str> {
    if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

/// Splits a raw query into ordered `(key, value)` pairs on `&` then the first `=`; a bare
/// `key` token yields an empty value. Values are verbatim — the query is not percent-decoded,
/// exactly like [`Uri::query`]. Empty tokens (a stray `&`) are skipped.
fn query_pairs(query: &str) -> impl Iterator<Item = (&str, &str)> {
    query
        .split('&')
        .filter(|token| !token.is_empty())
        .map(|token| token.split_once('=').unwrap_or((token, "")))
}

/// The key portion of a `key=value` (or bare `key`) query token.
fn param_key(token: &str) -> &str {
    token.split_once('=').map_or(token, |(key, _)| key)
}

/// Appends `key=value` to a query being rebuilt, inserting the `&` separator when needed.
fn push_param(out: &mut String, key: &str, value: &str) {
    if !out.is_empty() {
        out.push('&');
    }
    out.push_str(key);
    out.push('=');
    out.push_str(value);
}

/// Appends a verbatim token to a query being rebuilt, inserting the `&` separator when needed.
fn push_token(out: &mut String, token: &str) {
    if !out.is_empty() {
        out.push('&');
    }
    out.push_str(token);
}

/// A drive-letter prefix: a single ASCII letter, `:`, then a slash (`C:\` or `C:/`).
fn is_drive_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// A UNC prefix: two leading back-slashes (`\\server\share`).
fn is_unc_path(s: &str) -> bool {
    s.starts_with("\\\\")
}

/// Whether `candidate` is a syntactically valid RFC 3986 scheme.
fn is_valid_scheme(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Splits off a leading `scheme ":"`, if the text before the first `:` (occurring before
/// any `/`, `?`, or `#`) is a valid scheme. Returns `(None, s)` when there is no scheme.
fn split_scheme(s: &str) -> Result<(Option<String>, &str), UriError> {
    for (i, c) in s.char_indices() {
        match c {
            ':' => {
                let candidate = &s[..i];
                if candidate.is_empty() {
                    return Err(UriError::EmptyScheme);
                }
                if !is_valid_scheme(candidate) {
                    return Err(UriError::InvalidScheme {
                        scheme: candidate.to_string(),
                    });
                }
                return Ok((Some(candidate.to_string()), &s[i + 1..]));
            }
            '/' | '?' | '#' => break,
            _ => {}
        }
    }
    Ok((None, s))
}

/// Parses an authority `[ userinfo "@" ] host [ ":" port ]`, with `userinfo` split into
/// `user [ ":" password ]` and an IPv6 literal host kept bracketed.
pub(crate) fn parse_authority(auth: &str) -> Result<Authority, UriError> {
    let (userinfo, hostport) = match auth.split_once('@') {
        Some((ui, hp)) => (Some(ui), hp),
        None => (None, auth),
    };

    let (user, password) = match userinfo {
        Some(ui) => match ui.split_once(':') {
            Some((u, p)) => (Some(u), Some(p)),
            None => (Some(ui), None),
        },
        None => (None, None),
    };

    let (host, port_str) = if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 literal: host runs through the closing bracket; a port may follow `:`.
        match rest.split_once(']') {
            Some((inner, tail)) => {
                let host = format!("[{inner}]");
                // After the closing `]` only a `:port` may follow. A non-empty tail that is not a
                // `:`-prefixed port (`[::1]junk`) would otherwise be silently dropped into a
                // non-round-tripping `Uri`, so reject it with a guided error — mirroring the reg-name
                // branch below, which rejects its own malformed inputs for the same reason. An empty
                // port (`[::1]:`) normalizes to `None`, exactly as `host:` does there.
                let port = if tail.is_empty() {
                    None
                } else if let Some(port) = tail.strip_prefix(':') {
                    (!port.is_empty()).then_some(port)
                } else {
                    return Err(UriError::InvalidPort {
                        port: tail.to_string(),
                    });
                };
                return Ok(Authority::new(user, password, &host, parse_port(port)?));
            }
            // No closing bracket — treat the whole thing as the host, no port.
            None => (hostport, None),
        }
    } else {
        // A reg-name / IPv4 host carries no colon, so the FIRST colon is the port separator.
        // Splitting on the last colon (`rsplit_once`) would leave an inner colon inside the
        // host for a malformed multi-colon authority (`a::`, `:a:`), which `Display` cannot
        // round-trip; splitting on the first makes the leftover a port that `parse_port`
        // then rejects with a guided error, so such inputs never become a non-round-tripping
        // `Uri`.
        match hostport.split_once(':') {
            Some((h, p)) => (h, if p.is_empty() { None } else { Some(p) }),
            None => (hostport, None),
        }
    };

    Ok(Authority::new(user, password, host, parse_port(port_str)?))
}

/// Parses an optional decimal port into `u16`, guiding on a bad value.
fn parse_port(port: Option<&str>) -> Result<Option<u16>, UriError> {
    match port {
        None => Ok(None),
        Some(p) => p
            .parse::<u16>()
            .map(Some)
            .map_err(|_| UriError::InvalidPort {
                port: p.to_string(),
            }),
    }
}

impl crate::io::Serializable for Uri {
    type Error = UriError;

    fn serialize_bytes(&self) -> Vec<u8> {
        Uri::serialize_bytes(self)
    }

    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, UriError> {
        Uri::deserialize_bytes(bytes)
    }
}

/// The current user's **home** directory as a `file://…` URI string, or `None` when neither
/// `$HOME` nor `%USERPROFILE%` is set. Built through [`Uri::from_file_path`] so it carries the
/// exact same POSIX-slash normalization and percent-encoding a rendered `file://` URI does, and
/// therefore prefix-matches one by plain string comparison — the root that
/// [`Uri::to_portable_str`] / [`Uri::from_portable_str`] fold to/from `~`.
fn portable_home_root() -> Option<String> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))?;
    Some(Uri::from_file_path(&home.to_string_lossy()).to_string())
}

/// The system **temp** directory as a `file://…` URI string — the root
/// [`Uri::to_portable_str`] / [`Uri::from_portable_str`] fold to/from `$TMP`.
fn portable_temp_root() -> Option<String> {
    let tmp = std::env::temp_dir();
    if tmp.as_os_str().is_empty() {
        return None;
    }
    Some(Uri::from_file_path(&tmp.to_string_lossy()).to_string())
}

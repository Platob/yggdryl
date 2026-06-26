//! The [`HttpVersion`] negotiated protocol type.

/// The HTTP protocol version negotiated for a request. Returned by
/// [`HttpResponse::protocol`](crate::HttpResponse::protocol) and
/// [`HttpStream::protocol`](crate::HttpStream::protocol).
///
/// ```
/// use yggdryl_http::HttpVersion;
/// assert_eq!(HttpVersion::H1_1.as_str(), "HTTP/1.1");
/// assert_eq!(HttpVersion::H2.to_string(), "HTTP/2");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpVersion {
    /// HTTP/1.1 — one request per connection (or pipelined), cleartext or TLS.
    H1_1,
    /// HTTP/2 — multiplexed streams, header-compressed, TLS ALPN-negotiated.
    H2,
    /// HTTP/3 — multiplexed streams over QUIC/UDP.
    H3,
}

impl HttpVersion {
    /// The canonical string form (`"HTTP/1.1"`, `"HTTP/2"`, `"HTTP/3"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpVersion::H1_1 => "HTTP/1.1",
            HttpVersion::H2 => "HTTP/2",
            HttpVersion::H3 => "HTTP/3",
        }
    }
}

impl std::fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

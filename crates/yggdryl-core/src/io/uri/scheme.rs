//! [`default_port`] — the IANA-registered default port of a well-known URI scheme.
//!
//! RFC 3986 does not itself assign ports to schemes; each scheme's own spec does (HTTP's is
//! RFC 9110, and so on). This is the small curated lookup those specs imply, shared by
//! [`Uri::port_or_default`](super::Uri::port_or_default) so a parsed URI can report the port
//! it would actually connect to when the authority carries none — **without** mutating the
//! stored URI, so its canonical form still round-trips byte-for-byte.

/// The default port registered for a well-known URI `scheme`, or `None` when the scheme has
/// no registered default (or is not one this table knows). Matching is
/// **ASCII-case-insensitive**, since RFC 3986 §3.1 makes the scheme case-insensitive, so
/// `"HTTPS"` and `"https"` agree — and it allocates nothing (a linear scan of a `&str`
/// against a static table).
///
/// ```
/// use yggdryl_core::io::uri::default_port;
///
/// assert_eq!(default_port("https"), Some(443));
/// assert_eq!(default_port("HTTPS"), Some(443)); // scheme is case-insensitive
/// assert_eq!(default_port("ws"), Some(80));
/// assert_eq!(default_port("postgres"), Some(5432));
/// assert_eq!(default_port("s3"), None); // no registered default
/// ```
pub fn default_port(scheme: &str) -> Option<u16> {
    // A curated spread of the schemes a data/networking library actually dials — the WHATWG
    // "special" web schemes plus the common transport, mail, directory, and database ones.
    // Ordered by port so a reviewer can scan for duplicates; the lookup is order-independent.
    // DESIGN: a linear scan (not a `match` on a lowercased `String`) keeps it allocation-free
    // and case-insensitive in one step; the table is short enough that this is not hot.
    const DEFAULTS: &[(&str, u16)] = &[
        ("ftp", 21),
        ("ssh", 22),
        ("sftp", 22),
        ("telnet", 23),
        ("smtp", 25),
        ("dns", 53),
        ("http", 80),
        ("ws", 80),
        ("pop3", 110),
        ("nntp", 119),
        ("imap", 143),
        ("ldap", 389),
        ("https", 443),
        ("wss", 443),
        ("smtps", 465),
        ("ldaps", 636),
        ("ftps", 990),
        ("imaps", 993),
        ("pop3s", 995),
        ("socks5", 1080),
        ("mqtt", 1883),
        ("rdp", 3389),
        ("postgres", 5432),
        ("postgresql", 5432),
        ("amqps", 5671),
        ("amqp", 5672),
        ("coap", 5683),
        ("vnc", 5900),
        ("redis", 6379),
        ("irc", 6667),
        ("ircs", 6697),
        ("git", 9418),
        ("mongodb", 27017),
    ];
    DEFAULTS
        .iter()
        .find(|(name, _)| scheme.eq_ignore_ascii_case(name))
        .map(|&(_, port)| port)
}

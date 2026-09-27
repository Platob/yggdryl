//! What a request was addressed to and who sent it, as the server may
//! believe it behind the proxies in front of it.
//!
//! A server behind a reverse proxy sees the proxy's connection, not the
//! client's: the peer is the proxy, the `Host` is whatever the proxy put
//! there, the scheme is plain HTTP where the client spoke TLS, and the path
//! may have lost the prefix the proxy routes on. What the client used is
//! stated in `Forwarded` (RFC 7239) or in `X-Forwarded-For`, `-Proto`,
//! `-Host`, `-Port` and `-Prefix` - and any client can write those fields
//! too, so they are read only when the TCP peer is one of the networks
//! [`ServerOptions::with_trusted_proxies`] names, an [`IpNetwork`] being that
//! vocabulary, and only the fields [`ServerOptions::with_forwarded_headers`]
//! lists, a [`ForwardedHeader`] each - by default `X-Forwarded-For` and
//! `X-Forwarded-Proto` alone - because a proxy sets some of the six and
//! passes the rest through as the client wrote them. A `for` list is walked
//! from the right, the element nearest this server first, past every address
//! that is itself a trusted proxy; the first address that is not is the
//! client, and the host and scheme it stated are the ones the request was
//! made under. Every other `X-Forwarded-*` list is read the same way, never
//! from its first member, which is the client's own whenever a proxy appends
//! rather than overwrites: where it has as many members as
//! `X-Forwarded-For`, the member at the client's position, written by the
//! proxy that saw the client; else its last, written by the proxy nearest
//! this server. A forwarded value that is no URL part - a host that is no
//! authority, a scheme other than `http` or `https`, a prefix holding a byte
//! the path grammar refuses - is left unread rather than failing the request.
//!
//! The effective base of one request - its scheme, its authority and the
//! public path prefix the routed path sits under - is then, in order: the
//! [`ServerOptions::with_public_url`] the server was given, the forwarded
//! fields of a trusted peer (and the `:scheme` of its HTTP/2 request in the
//! clear), the authority of an absolute-form target, the `Host` field
//! (`https` when the connection itself is TLS), and the bound address with
//! an unspecified one replaced by its loopback. The public path prefix is
//! what a forwarded `X-Forwarded-Prefix` says the proxy stripped, then the
//! [`ServerOptions::with_path_prefix`] this server stripped.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use smol_str::format_smolstr;

use super::{Incoming, ServerOptions};
use crate::http::Headers;
use crate::uri::Authority;
use crate::{Error, Result, Url};

/// An IP network in CIDR form: the address and the leading bits that name
/// it. `10.0.0.5` is `10.0.0.5/32` and `::1` is `::1/128`; an IPv4-mapped
/// IPv6 address is the IPv4 address it maps, so `::ffff:10.0.0.5` lies in
/// `10.0.0.0/8`, and a mapped network's prefix counts the 96 bits of the
/// mapping, so `::ffff:10.0.0.0/104` is `10.0.0.0/8`.
///
/// ```
/// use std::net::IpAddr;
///
/// use yggdryl::http::IpNetwork;
///
/// # fn main() -> yggdryl::Result<()> {
/// let network = IpNetwork::from_str("10.0.0.0/8")?;
/// assert!(network.contains("10.20.30.40".parse::<IpAddr>().unwrap()));
/// assert!(network.contains("::ffff:10.20.30.40".parse::<IpAddr>().unwrap()));
/// assert!(!network.contains("11.0.0.1".parse::<IpAddr>().unwrap()));
/// assert_eq!(IpNetwork::from_str("::1")?.to_string(), "::1/128");
/// assert_eq!(IpNetwork::from_str("::ffff:10.0.0.0/104")?.to_string(), "10.0.0.0/8");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IpNetwork {
    address: IpAddr,
    bits: u8,
}

/// The bits of an IPv4-mapped IPv6 address that are the mapping, before the
/// IPv4 address itself.
const MAPPED_BITS: u8 = 96;

impl IpNetwork {
    /// Parse `address` or `address/bits`; an IPv6 address may be written in
    /// brackets.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with target `ip network` for an address that
    /// is none, a bit count past the family's width, or one below 96 on an
    /// IPv4-mapped address, which names IPv6 space no IPv4 address maps into.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Result<Self> {
        <Self as FromStr>::from_str(text)
    }

    /// The network address, canonical: an IPv4-mapped IPv6 address as the
    /// IPv4 one.
    pub fn address(&self) -> IpAddr {
        self.address
    }

    /// The leading bits that name the network.
    pub fn bits(&self) -> u8 {
        self.bits
    }

    /// Whether `address` lies in this network, an IPv4-mapped IPv6 address
    /// read as the IPv4 one.
    pub fn contains(&self, address: IpAddr) -> bool {
        in_network(address.to_canonical(), self.address, self.bits)
    }
}

impl FromStr for IpNetwork {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let refuse = |position: usize, reason: &str| Error::Parse {
            target: "ip network",
            position,
            reason: format_smolstr!("{reason}, got {text:?}"),
        };
        let trimmed = text.trim();
        let (address, bits) = match trimmed.rsplit_once('/') {
            // `[::1]/128` splits at its one slash; `::1` has none.
            Some((address, bits)) => (address, Some(bits)),
            None => (trimmed, None),
        };
        let bare = address
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or(address);
        let written: IpAddr = bare
            .parse()
            .map_err(|_| refuse(0, "expected an IP address"))?;
        let address = written.to_canonical();
        let mapped = written.is_ipv6() && address.is_ipv4();
        let width = if address.is_ipv4() { 32 } else { 128 };
        let bits = match bits {
            None => width,
            Some(bits) => {
                let position = trimmed.len() - bits.len();
                // A mapped address is written with IPv6's width, and the
                // mapping is the first 96 bits of it.
                let written_width = if mapped { 128 } else { width };
                let bits = bits
                    .parse::<u8>()
                    .ok()
                    .filter(|bits| *bits <= written_width)
                    .ok_or_else(|| {
                        refuse(
                            position,
                            &format!("expected a prefix length of at most {written_width}"),
                        )
                    })?;
                if mapped {
                    bits.checked_sub(MAPPED_BITS).ok_or_else(|| {
                        refuse(
                            position,
                            "expected a prefix length of at least 96 on an IPv4-mapped address",
                        )
                    })?
                } else {
                    bits
                }
            }
        };
        Ok(Self { address, bits })
    }
}

impl fmt::Display for IpNetwork {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.address, self.bits)
    }
}

/// Whether `address` lies in `network/bits`, both of one family.
fn in_network(address: IpAddr, network: IpAddr, bits: u8) -> bool {
    match (address, network) {
        (IpAddr::V4(address), IpAddr::V4(network)) if bits <= 32 => {
            let mask = u32::MAX.checked_shl(32 - u32::from(bits)).unwrap_or(0);
            u32::from(address) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(address), IpAddr::V6(network)) if bits <= 128 => {
            let mask = u128::MAX.checked_shl(128 - u32::from(bits)).unwrap_or(0);
            u128::from(address) & mask == u128::from(network) & mask
        }
        _ => false,
    }
}

/// One field a reverse proxy states what the client used in, believed from a
/// peer [`ServerOptions::with_trusted_proxies`] names only when
/// [`ServerOptions::with_forwarded_headers`] lists it: a proxy sets some of
/// the six on every request it forwards and passes the rest through as the
/// client wrote them, so the server reads exactly the ones its proxy sets.
///
/// ```
/// use yggdryl::http::ForwardedHeader;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(
///     ForwardedHeader::from_str("x-forwarded-host")?,
///     ForwardedHeader::XForwardedHost
/// );
/// assert_eq!(ForwardedHeader::XForwardedHost.as_str(), "X-Forwarded-Host");
/// assert_eq!(
///     ForwardedHeader::DEFAULT,
///     [ForwardedHeader::XForwardedFor, ForwardedHeader::XForwardedProto]
/// );
/// assert!(ForwardedHeader::from_str("X-Real-IP").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ForwardedHeader {
    /// `Forwarded` (RFC 7239): the client, the host and the scheme in one
    /// field, one element per proxy.
    Forwarded,
    /// `X-Forwarded-For`: the client, then each proxy the request crossed.
    XForwardedFor,
    /// `X-Forwarded-Proto`: `http` or `https`, what the client spoke.
    XForwardedProto,
    /// `X-Forwarded-Host`: the host, and port, the client addressed.
    XForwardedHost,
    /// `X-Forwarded-Port`: the port the client addressed.
    XForwardedPort,
    /// `X-Forwarded-Prefix`: the path prefix the proxy stripped before
    /// forwarding.
    XForwardedPrefix,
}

impl ForwardedHeader {
    /// Every field, in the order they are read.
    pub const ALL: [Self; 6] = [
        Self::Forwarded,
        Self::XForwardedFor,
        Self::XForwardedProto,
        Self::XForwardedHost,
        Self::XForwardedPort,
        Self::XForwardedPrefix,
    ];

    /// What every documented proxy sets on every request it forwards -
    /// nginx, Caddy, IIS with Application Request Routing, the cloud load
    /// balancers - and so what a server reads by default.
    pub const DEFAULT: [Self; 2] = [Self::XForwardedFor, Self::XForwardedProto];

    /// The field name, however it was cased.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with target `forwarded header` for a name
    /// that is none of the six.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(name: &str) -> Result<Self> {
        <Self as FromStr>::from_str(name)
    }

    /// The field name as it is written: `X-Forwarded-Host`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Forwarded => "Forwarded",
            Self::XForwardedFor => "X-Forwarded-For",
            Self::XForwardedProto => "X-Forwarded-Proto",
            Self::XForwardedHost => "X-Forwarded-Host",
            Self::XForwardedPort => "X-Forwarded-Port",
            Self::XForwardedPrefix => "X-Forwarded-Prefix",
        }
    }
}

impl FromStr for ForwardedHeader {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self> {
        let trimmed = name.trim();
        Self::ALL
            .into_iter()
            .find(|header| header.as_str().eq_ignore_ascii_case(trimmed))
            .ok_or_else(|| Error::Parse {
                target: "forwarded header",
                position: 0,
                reason: format_smolstr!(
                    "expected Forwarded, X-Forwarded-For, X-Forwarded-Proto, X-Forwarded-Host, X-Forwarded-Port or X-Forwarded-Prefix, got {name:?}"
                ),
            })
    }
}

impl fmt::Display for ForwardedHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What one request was addressed to, and who sent it.
pub(super) struct Addressed {
    /// `http` or `https`.
    pub(super) scheme: &'static str,
    /// The host and, when it is not the scheme's own, the port.
    pub(super) authority: String,
    /// The public path the routed path sits under: `""`, or `/olap` with no
    /// trailing slash.
    pub(super) prefix: String,
    /// The client: the address the trusted proxies forwarded, else the peer.
    pub(super) client: Option<IpAddr>,
    /// Whether the peer is a trusted proxy, so the forwarded fields were
    /// read.
    pub(super) trusted: bool,
}

/// What the trusted forwarded fields of one request state.
#[derive(Default)]
struct Forwarded {
    client: Option<IpAddr>,
    scheme: Option<&'static str>,
    host: Option<String>,
    port: Option<u16>,
    prefix: Option<String>,
}

/// The base of `incoming` under `options`: `absolute` is the target when it
/// was written in absolute form, `socket` the server's own URL, and
/// `stripped` the options' path prefix as the request spelled it, `""` when
/// the path did not lie under it.
pub(super) fn resolve(
    options: &ServerOptions,
    incoming: &Incoming,
    absolute: Option<&Url>,
    socket: &Url,
    stripped: &str,
) -> Addressed {
    let peer = incoming.peer.map(|peer| peer.ip().to_canonical());
    let trusted = peer.is_some_and(|peer| is_trusted(&options.trusted_proxies, peer));
    let forwarded = if trusted {
        forwarded_of(&incoming.head.headers, options)
    } else {
        Forwarded::default()
    };
    let client = forwarded.client.or(peer);
    // A framed request's `:scheme`, sent in the clear, is the statement of
    // the proxy that terminated TLS and speaks HTTP/2 onward, believed from
    // a trusted peer as `X-Forwarded-Proto` is; from any other peer the
    // connection's own TLS is the whole fact.
    let stated = forwarded
        .scheme
        .or(if trusted { incoming.scheme } else { None });
    let fallback_scheme = if incoming.secure { "https" } else { "http" };
    let (scheme, authority) = match &options.public_url {
        Some(public) => (
            if public.scheme().as_str().eq_ignore_ascii_case("https") {
                "https"
            } else {
                "http"
            },
            public.authority().as_str().to_owned(),
        ),
        None => {
            let scheme = stated.unwrap_or(fallback_scheme);
            let authority = match forwarded.host {
                Some(host) => with_port(host, forwarded.port, scheme),
                None => match absolute {
                    Some(target) => target.authority().as_str().to_owned(),
                    None => match incoming.head.headers.get("host").map(str::trim) {
                        // An HTTP/1.1 `Host` that is no authority was
                        // refused at the connection (RFC 9112 3.2); an
                        // HTTP/1.0 one is left unread.
                        Some(host) if is_authority(host) => {
                            with_port(host.to_owned(), forwarded.port, scheme)
                        }
                        _ => socket.authority().as_str().to_owned(),
                    },
                },
            };
            (scheme, authority)
        }
    };
    let prefix = match &options.public_url {
        Some(public) => public
            .path_text(false)
            .map(|path| path.trim_end_matches('/').to_owned())
            .unwrap_or_default(),
        // What the proxy stripped, then what this server stripped: the
        // public path carries both.
        None => {
            let mut prefix = forwarded.prefix.unwrap_or_default();
            prefix.push_str(stripped);
            prefix
        }
    };
    Addressed {
        scheme,
        authority,
        prefix,
        client,
        trusted,
    }
}

/// Whether `address` lies in one of `networks`.
fn is_trusted(networks: &[IpNetwork], address: IpAddr) -> bool {
    networks.iter().any(|network| network.contains(address))
}

/// `host` with `port` appended when the host names none and the port is not
/// the scheme's own.
fn with_port(host: String, port: Option<u16>, scheme: &str) -> String {
    let Some(port) = port else {
        return host;
    };
    let default = if scheme == "https" { 443 } else { 80 };
    let has_port = Authority::from_str(&host).is_ok_and(|authority| authority.port().is_some());
    if port == default || has_port {
        return host;
    }
    format!("{host}:{port}")
}

/// What the forwarded fields the options list say: `Forwarded` read before
/// the `X-Forwarded-*` field stating the same fact, the trusted `for`
/// addresses walked past.
fn forwarded_of(headers: &Headers, options: &ServerOptions) -> Forwarded {
    let trusted = &options.trusted_proxies;
    let reads = |header: ForwardedHeader| options.forwarded_headers.contains(&header);
    let mut forwarded = Forwarded::default();
    if reads(ForwardedHeader::Forwarded) {
        if let Some(value) = headers.get("forwarded") {
            let elements = elements_of(value);
            let client = elements
                .iter()
                .rposition(|element| {
                    !element
                        .get("for")
                        .and_then(node_address)
                        .is_some_and(|address| is_trusted(trusted, address))
                })
                .unwrap_or(0);
            forwarded.client = elements
                .get(client)
                .and_then(|element| element.get("for"))
                .and_then(node_address);
            // The proxy that saw the client states the host and scheme it
            // used; a proxy nearer this server states them when that one
            // did not.
            let stated = |name: &str| {
                elements[client..]
                    .iter()
                    .find_map(|element| element.get(name).map(str::to_owned))
            };
            forwarded.host = stated("host").filter(|host| is_authority(host));
            forwarded.scheme = stated("proto").and_then(|proto| scheme_of(&proto));
        }
    }
    let hops: Vec<Option<IpAddr>> = if reads(ForwardedHeader::XForwardedFor) {
        headers
            .get_all("x-forwarded-for")
            .map(node_address)
            .collect()
    } else {
        Vec::new()
    };
    let client = hops
        .iter()
        .rposition(|hop| !hop.is_some_and(|address| is_trusted(trusted, address)))
        .unwrap_or(0);
    if forwarded.client.is_none() {
        forwarded.client = hops.get(client).copied().flatten();
    }
    // The member of another `X-Forwarded-*` field: where the field lines up
    // with the `for` chain, the one written by the proxy that saw the
    // client; else the one nearest this server, which the trusted proxy
    // wrote - never the first, which is whatever the client sent when a
    // proxy appends rather than overwrites.
    let stated = |name: &str| -> Option<String> {
        let members: Vec<&str> = headers.get_all(name).collect();
        let member = if !hops.is_empty() && members.len() == hops.len() {
            members.get(client)
        } else {
            members.last()
        };
        member.map(|member| member.trim().to_owned())
    };
    if reads(ForwardedHeader::XForwardedHost) && forwarded.host.is_none() {
        forwarded.host = stated("x-forwarded-host").filter(|host| is_authority(host));
    }
    if reads(ForwardedHeader::XForwardedProto) && forwarded.scheme.is_none() {
        forwarded.scheme = stated("x-forwarded-proto").and_then(|proto| scheme_of(&proto));
    }
    if reads(ForwardedHeader::XForwardedPort) {
        forwarded.port = stated("x-forwarded-port").and_then(|port| port.parse().ok());
    }
    if reads(ForwardedHeader::XForwardedPrefix) {
        forwarded.prefix = stated("x-forwarded-prefix").and_then(|prefix| prefix_of(&prefix));
    }
    forwarded
}

/// A forwarded prefix as a URL carries it: `""` for the root, else `/olap`
/// with no trailing slash. One the URL grammar does not read - a space, a
/// quote - is `None`, left unread as a host that is no authority is, rather
/// than failing the request it came with.
fn prefix_of(prefix: &str) -> Option<String> {
    let prefix = super::normalize_path(prefix).ok()?;
    if prefix == "/" {
        return Some(String::new());
    }
    Url::from_str(&format!("http://h{prefix}"))
        .ok()
        .map(|_| prefix)
}

/// `http` or `https`, however it is cased; anything else is not a scheme this
/// server serves under.
pub(super) fn scheme_of(proto: &str) -> Option<&'static str> {
    let proto = proto.trim();
    if proto.eq_ignore_ascii_case("https") {
        Some("https")
    } else if proto.eq_ignore_ascii_case("http") {
        Some("http")
    } else {
        None
    }
}

/// Whether `host` is a host with an optional port and no user information,
/// so a URL can carry it.
pub(super) fn is_authority(host: &str) -> bool {
    !host.is_empty()
        && !host.contains('@')
        && Authority::from_str(host).is_ok_and(|authority| !authority.host().is_empty())
}

/// One `Forwarded` element: its parameters, names folded to lower case,
/// values unquoted, the first spelling of a repeated name kept.
struct Element(Vec<(String, String)>);

impl Element {
    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// The elements of a `Forwarded` field value (RFC 7239 4): comma-separated,
/// each a `;`-separated list of `name=value` pairs, a value a token or a
/// quoted string; commas and semicolons inside a quoted string are its own.
fn elements_of(value: &str) -> Vec<Element> {
    split_outside_quotes(value, b',')
        .into_iter()
        .map(|element| {
            Element(
                split_outside_quotes(element, b';')
                    .into_iter()
                    .filter_map(|pair| {
                        let (name, value) = pair.split_once('=')?;
                        let name = name.trim().to_ascii_lowercase();
                        (!name.is_empty()).then(|| (name, unquoted(value.trim())))
                    })
                    .collect(),
            )
        })
        .collect()
}

/// `value` split at every `delimiter` outside a quoted string, each part
/// trimmed, empty parts dropped.
fn split_outside_quotes(value: &str, delimiter: u8) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            _ if escaped => escaped = false,
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            _ if byte == delimiter && !quoted => {
                parts.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

/// A quoted string's text with its quotes and quoted-pair escapes removed;
/// a token as it is.
fn unquoted(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.to_owned();
    };
    let mut text = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        match character {
            '\\' if !escaped => escaped = true,
            _ => {
                text.push(character);
                escaped = false;
            }
        }
    }
    text
}

/// The address a `for` or `X-Forwarded-For` node names: an IPv4 address, an
/// IPv6 one in brackets, either with a port; `unknown`, an obfuscated
/// `_identifier` and anything else name none.
fn node_address(node: &str) -> Option<IpAddr> {
    let node = node.trim().trim_matches('"');
    if let Some(rest) = node.strip_prefix('[') {
        let (address, _) = rest.split_once(']')?;
        return address
            .parse::<IpAddr>()
            .ok()
            .map(|address| address.to_canonical());
    }
    if let Ok(address) = node.parse::<IpAddr>() {
        return Some(address.to_canonical());
    }
    node.parse::<SocketAddr>()
        .ok()
        .map(|address| address.ip().to_canonical())
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/http/server/forwarded.rs` pins and a caller cannot
    //! reach: the `Forwarded` grammar and the node spellings.
    use std::net::IpAddr;

    /// The elements of a `Forwarded` field value as `(name, value)` pairs,
    /// names folded, values unquoted.
    pub fn elements_of(value: &str) -> Vec<Vec<(String, String)>> {
        super::elements_of(value)
            .into_iter()
            .map(|element| element.0)
            .collect()
    }

    /// The address a `for` node names, or `None` for `unknown`, an
    /// obfuscated identifier or anything that is no address.
    pub fn node_address(node: &str) -> Option<IpAddr> {
        super::node_address(node)
    }
}

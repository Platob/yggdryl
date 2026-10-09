

# ---------------------------------------------------------------------------
# Step 4: the core
# ---------------------------------------------------------------------------


def edit_core_root(root: pathlib.Path) -> None:
    edit(root, "rust/src/lib.rs", ('#[cfg(feature = "s3")]\npub mod s3;\n', ""))
    # No backend is the core's own any more: the register has nothing to seed.
    edit(
        root, "rust/src/holder/backend.rs",
        ("//! states. A scheme a core arm holds is never a backend's to claim. The core\n"
         "//! claims the object stores' backend itself under the `s3` feature, until\n"
         "//! `yggdryl-s3`'s `install()` does; a scheme no claim answers is refused\n"
         "//! naming the crate to install.\n",
         "//! states. A scheme a core arm holds is never a backend's to claim, and no\n"
         "//! backend is the core's own: the object stores' is `yggdryl-s3`'s, claimed\n"
         "//! by its `install()`; a scheme no claim answers is refused naming the crate\n"
         "//! to install.\n"),
        ("use std::sync::{Mutex, OnceLock};\n", "use std::sync::Mutex;\n"),
        ("static SEEDED: OnceLock<()> = OnceLock::new();\n", ""),
        ("/// Claim the core's own backends once, before the register answers\n/// anything.\nfn seed() {\n"
         "    SEEDED.get_or_init(|| {\n        // The core's claims cannot conflict: each scheme is stated once in\n"
         "        // the crate.\n        #[cfg(feature = \"s3\")]\n        claim_unseeded(&crate::s3::S3_BACKEND, CORE)\n"
         "            .expect(\"the core's own storage backends claim cleanly\");\n    });\n}\n\n", ""),
        ("    seed();\n    if by == CORE {\n", "    if by == CORE {\n"),
        ("    claim_unseeded(backend, by)\n}\n\nfn claim_unseeded(backend: &'static dyn StorageBackend, by: &'static str) -> Result<()> {\n",
         ""),
        ("    seed();\n    BACKENDS.get(scheme)\n", "    BACKENDS.get(scheme)\n"),
        ("    seed();\n    let mut backends", "    let mut backends"),
    )
    edit(
        root, "rust/src/holder/mod.rs",
        ("    /// `gs:`, `az:` and their aliases, by the backend the core claims under\n"
         "    /// the `s3` feature until `yggdryl-s3` does, configured by the properties\n",
         "    /// `gs:`, `az:` and their aliases, by the backend `yggdryl-s3`'s\n"
         "    /// `install()` claims, configured by the properties\n"),
    )
    edit(
        root, "rust/src/holder/counted.rs",
        ("`StatsSnapshot` (behind the `s3` feature) counts", "`StatsSnapshot` (`yggdryl-s3`'s) counts"),
    )
    edit(
        root, "rust/src/aws/mod.rs",
        ("//! The module is behind the non-default `aws` feature, which the `s3` feature\n//! implies.\n",
         "//! The module is behind the non-default `aws` feature, which `yggdryl-s3`\n"
         "//! and the `s3tables` feature turn on.\n"),
    )
    edit(
        root, "rust/src/xml/scanner.rs",
        ("//! reads for itself: `s3::aws::xml` reads `ListBucketResult`,\n//! `s3::azure::xml` reads",
         "//! reads for itself: `yggdryl-s3`'s `aws::xml` reads `ListBucketResult`,\n//! its `azure::xml` reads"),
    )


# The 23 `feature = "s3"` sites of `aws/`, `auth/`, `http/` and `xml/` (D36.5):
# each module holding one is already behind `aws` (or `http`), so the gate
# goes; a re-export only the backend read goes with it.
CFG_SITES: dict[str, tuple[int, list[tuple[str, str]]]] = {
    "rust/src/aws/mod.rs": (1, [('#[cfg(feature = "s3")]\npub(crate) mod environment;\n', "pub(crate) mod environment;\n")]),
    "rust/src/aws/session.rs": (11, []),
    "rust/src/aws/sigv4.rs": (3, [
        ('    #[cfg(all(feature = "internals", feature = "s3"))]\n    pub(crate) fn access_key_id(',
         '    #[cfg(feature = "internals")]\n    pub fn access_key_id('),
    ]),
    "rust/src/auth/lease.rs": (4, []),
    "rust/src/auth/mod.rs": (1, [('#[cfg(feature = "s3")]\npub(crate) use lease::Bearer;\n', "")]),
    "rust/src/http/mod.rs": (1, [('#[cfg(feature = "s3")]\npub(crate) use client::record_process;\n', "")]),
    "rust/src/xml/scanner.rs": (2, []),
}


def rekey_cfg_sites(root: pathlib.Path) -> int:
    count = 0
    for rel, (expected, pairs) in CFG_SITES.items():
        path = root / rel
        text = read(path)
        found = len(re.findall(r'feature\s*=\s*"s3"', text))
        if found != expected:
            residue(rel, f"{found} `s3` gates where {expected} were counted (D36.5): re-key them by hand")
        text = edit_text(text, rel, *pairs) if pairs else text
        text, n = re.subn(r'(?m)^[ \t]*#\[cfg\(feature = "s3"\)\]\n', "", text)
        text, m = re.subn(r'(?m)^[ \t]*#\[cfg_attr\(not\(feature = "s3"\), expect\(unused_variables, reason = "s3-only"\)\)\]\n',
                          "", text)
        write(path, text)
        left = len(re.findall(r'feature\s*=\s*"s3"', text))
        if left:
            residue(rel, f"{left} `s3` gate(s) left after the re-key")
        count += found - left
    return count


# Types the backend names, raised to `pub` inside the private module that
# holds them (route R: the module publishes nothing); the methods it calls
# with them; their links to what stays private turned to code.
RAISES: dict[str, list[tuple[str, str]]] = {
    "rust/src/http/retry.rs": [
        ("pub(crate) struct RetryBudget {", "pub struct RetryBudget {"),
        ("    pub(crate) fn withdraw(&self) -> bool {", "    pub fn withdraw(&self) -> bool {"),
        ("    pub(crate) fn refund(&self, tokens: i64) {", "    pub fn refund(&self, tokens: i64) {"),
        ("    pub(crate) fn remaining(&self) -> i64 {", "    pub fn remaining(&self) -> i64 {"),
        ("/// [`RETRY_COST`] tokens, a request that succeeds without one refunds\n/// [`RETRY_REFUND`], and",
         "/// `RETRY_COST` tokens, a request that succeeds without one refunds\n/// `RETRY_REFUND`, and"),
    ],
    "rust/src/aws/sigv4.rs": [
        ("pub(crate) struct Signer {", "pub struct Signer {"),
        ("    pub(crate) fn sign(\n", "    pub fn sign(\n"),
        ("    ///   canonical URI of it by its service's rule ([`Self::canonical_uri`]). \"/\" for the root.\n",
         "    ///   canonical URI of it by its service's rule (`Self::canonical_uri`). \"/\" for the root.\n"),
        ("    /// * `query`: raw (unencoded) name/value pairs; the canonical query string is built with\n"
         "    ///   [`canonical_query`].\n",
         "    /// * `query`: raw (unencoded) name/value pairs; the canonical query string is built with\n"
         "    ///   `canonical_query`.\n"),
        ("    /// * `payload_hash`: `sha256_hex(body)` or [`EMPTY_PAYLOAD_SHA256`]; the S3 family also\n"
         "    ///   accepts [`UNSIGNED_PAYLOAD`], which",
         "    /// * `payload_hash`: `sha256_hex(body)` or `EMPTY_PAYLOAD_SHA256`; the S3 family also\n"
         "    ///   accepts `UNSIGNED_PAYLOAD`, which"),
    ],
    "rust/src/aws/properties.rs": [
        ("pub(crate) struct Identity {", "pub struct Identity {"),
        ("    pub(crate) fn read(&mut self, key: &str, name: &str, value: &str) -> Result<bool> {",
         "    pub fn read(&mut self, key: &str, name: &str, value: &str) -> Result<bool> {"),
        ("    pub(crate) fn apply(&self, session: &Session) -> Result<Session> {",
         "    pub fn apply(&self, session: &Session) -> Result<Session> {"),
        ("pub(crate) enum EndpointName {", "pub enum EndpointName {"),
        ("    pub(crate) fn of(key: &str) -> Option<Self> {", "    pub fn of(key: &str) -> Option<Self> {"),
        ("    /// Any other service, by the service id after [`SERVICE_ENDPOINT`]\n",
         "    /// Any other service, by the service id after `SERVICE_ENDPOINT`\n"),
    ],
    "rust/src/xml/scanner.rs": [
        ("#[derive(Debug, Default)]\npub struct Element {\n",
         "/// One element of a small fixed-shape document: its name without a\n"
         "/// namespace prefix, its own text and its children in document order.\n"
         "#[derive(Debug, Default)]\npub struct Element {\n"),
        ("    /// The text of the first child named `name`, whose absence is an error.\n"
         "    pub fn required(&self, name: &str) -> Result<&str, XmlError> {\n",
         "    /// The text of the first child named `name`, whose absence is an error.\n"
         "    ///\n    /// # Errors\n    ///\n    /// An [`XmlError`] naming both elements where no such child is there.\n"
         "    pub fn required(&self, name: &str) -> Result<&str, XmlError> {\n"),
    ],
}


def raise_items(root: pathlib.Path) -> None:
    for rel, pairs in RAISES.items():
        edit(root, rel, *pairs)


IMPLEMENTER_DOC = [
    (("//! workspace: an item is listed because a crate split off the core - the\n"
      "//! market crate, the FIX crate, the media crates - needs it, and an item no\n"),
     "//! workspace: an item is listed because a crate split off the core - the\n"
     "//! market crate, the FIX crate, the media crates, the object-store crate -\n"
     "//! needs it, and an item no\n"),
]

IMPLEMENTER_SECTION = '''
// ------------------------------------------------------------------------
// Object stores: what the object-store crate reaches - the HTTP client's
// pool and retry rules, Signature Version 4 and the AWS property reader,
// the bearer lease, the XML scanner, the session's own doors, and the
// handle helpers a store's roles share with every backend.
// ------------------------------------------------------------------------

/// `iobase::oversized`, for the object-store crate: the refusal of a value
/// too large for the platform's addressable memory.
#[inline]
#[must_use]
pub fn oversized(size: u64) -> crate::Error {
    crate::iobase::oversized(size)
}

/// `iobase::read_upload`, for the object-store crate: exactly `length` bytes
/// of `source`, what an upload of a stated length sends.
///
/// # Errors
///
/// Returns the reader's failure, or the refusal of a source that ends short
/// of `length`.
#[inline]
pub fn read_upload(source: &mut dyn std::io::Read, length: u64) -> Result<Vec<u8>> {
    crate::iobase::read_upload(source, length)
}

/// `iobase::short_upload`, for the object-store crate: the refusal of an
/// upload whose source ended at `got` bytes of the `expected`.
#[inline]
#[must_use]
pub fn short_upload(expected: u64, got: u64) -> crate::Error {
    crate::iobase::short_upload(expected, got)
}

/// `holder::sibling`, for the object-store crate: the handle beside
/// `handle` that its parent holds under its own name, where it has both.
///
/// # Errors
///
/// Returns the parent's refusal to resolve the child.
#[inline]
pub fn sibling<H: IOBase + ?Sized>(handle: &H) -> Result<Option<crate::holder::Holder>> {
    crate::holder::sibling(handle)
}

/// `holder::system_time_ns`, for the object-store crate: an instant as
/// nanoseconds from the epoch, counting backwards before it.
#[inline]
#[must_use]
pub fn system_time_ns(value: std::time::SystemTime) -> Option<i64> {
    crate::holder::system_time_ns(value)
}

/// `uri::percent_encode_segment`, for the object-store crate: one path
/// segment escaped, its `/` included.
#[inline]
#[must_use]
pub fn percent_encode_segment(value: &str) -> Cow<'_, str> {
    crate::uri::percent_encode_segment(value)
}

/// `integer::BYTE_COUNT_SPELLINGS`, for the object-store crate: what every
/// refusal of a byte count names.
pub const BYTE_COUNT_SPELLINGS: &str = crate::integer::BYTE_COUNT_SPELLINGS;

/// `integer::byte_count_from_text`, for the object-store crate: a byte count
/// with an optional `KiB`, `MiB` or `GiB` suffix.
#[inline]
#[must_use]
pub fn byte_count_from_text(text: &str) -> Option<u64> {
    crate::integer::byte_count_from_text(text)
}

/// `integer::integer_from_scalar_as`, for the object-store crate: an
/// integer a value states, read at one native width.
#[inline]
#[must_use]
pub fn integer_from_scalar_as<T: TryFrom<i128> + TryFrom<u128>>(value: &Scalar) -> Option<T> {
    crate::integer::integer_from_scalar_as(value)
}

/// `xxhash::stream::read_range_digest`, for the object-store crate: the
/// digest of `length` bytes of a handle from `offset`, clamped as a ranged
/// read is.
///
/// # Errors
///
/// Returns the refusal of a container, or the handle's read failure.
#[inline]
pub fn read_range_digest<H: IOBase + ?Sized>(
    handle: &H,
    offset: u64,
    length: usize,
    algorithm: crate::DigestAlgorithm,
) -> Result<crate::Digest> {
    crate::xxhash::stream::read_range_digest(handle, offset, length, algorithm)
}

/// `ByteStream::from_handle`, for the object-store crate: a handle's bytes
/// from `position` in chunks of `batch_size`, read positionally.
///
/// # Errors
///
/// Returns the handle's refusal to be read.
#[inline]
pub fn byte_stream_from_handle<'source, H: IOBase + ?Sized>(
    handle: &'source H,
    position: u64,
    batch_size: usize,
) -> Result<crate::ByteStream<'source>> {
    crate::ByteStream::from_handle(handle, position, batch_size)
}

/// `http::client::agent_for`, for the object-store crate: the one door every
/// client in the build takes its connections through.
///
/// # Errors
///
/// Returns the refusal of a proxy or a TLS setting the options state.
#[cfg(feature = "http")]
#[inline]
pub fn agent_for(
    options: &crate::http::HttpOptions,
    tls: Option<ureq::tls::TlsConfig>,
) -> Result<ureq::Agent> {
    crate::http::client::agent_for(options, tls)
}

/// `http::client::record_process`, for the object-store crate: one request
/// to `host` counted in the process ledger every client records into.
#[cfg(feature = "http")]
#[inline]
pub fn record_process(host: &str, method: &str) {
    crate::http::client::record_process(host, method);
}

/// The retry budget every retrying client spends, for the object-store
/// crate's client.
#[cfg(feature = "http")]
pub use crate::http::retry::RetryBudget;

/// `http::retry::RETRY_BACKOFF`, for the object-store crate: the pause
/// before the first retry.
#[cfg(feature = "http")]
pub const RETRY_BACKOFF: std::time::Duration = crate::http::retry::RETRY_BACKOFF;

/// `http::retry::RETRY_COST`, for the object-store crate: what one retry
/// costs the budget.
#[cfg(feature = "http")]
pub const RETRY_COST: i64 = crate::http::retry::RETRY_COST;

/// `http::retry::RETRY_REFUND`, for the object-store crate: what a
/// first-attempt success refunds.
#[cfg(feature = "http")]
pub const RETRY_REFUND: i64 = crate::http::retry::RETRY_REFUND;

/// `http::retry::backoff`, for the object-store crate: the window attempt
/// `attempt + 1` is drawn from.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn backoff(attempt: u32) -> std::time::Duration {
    crate::http::retry::backoff(attempt)
}

/// `http::retry::delay`, for the object-store crate: the pause before
/// attempt `attempt`, a server's own short ask honoured.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn delay(
    attempt: u32,
    asked: Option<std::time::Duration>,
    jitter: &std::sync::atomic::AtomicU64,
) -> std::time::Duration {
    crate::http::retry::delay(attempt, asked, jitter)
}

/// `http::retry::fresh_jitter`, for the object-store crate: a client's
/// jitter seed.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn fresh_jitter() -> u64 {
    crate::http::retry::fresh_jitter()
}

/// `http::retry::retry_after`, for the object-store crate: the pause a
/// `Retry-After` value asks for.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn retry_after(value: Option<&str>) -> Option<std::time::Duration> {
    crate::http::retry::retry_after(value)
}

/// `http::retry::is_retryable_transport`, for the object-store crate:
/// whether a transport failure may be sent again.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn is_retryable_transport(error: &ureq::Error) -> bool {
    crate::http::retry::is_retryable_transport(error)
}

/// `http::retry::is_unsent`, for the object-store crate: whether a failure
/// happened before any connection took the request.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn is_unsent(error: &ureq::Error) -> bool {
    crate::http::retry::is_unsent(error)
}

/// `http::retry::is_resumable`, for the object-store crate: whether a read
/// failure is the transport's rather than the server's verdict.
#[cfg(feature = "http")]
#[inline]
#[must_use]
pub fn is_resumable(error: &std::io::Error) -> bool {
    crate::http::retry::is_resumable(error)
}

/// The text that renders as `<redacted>`, for the object-store crate: what a
/// bearer token holds.
#[cfg(feature = "http")]
pub use crate::auth::secret::Secret;

/// `auth::variable`, for the object-store crate: a non-empty process
/// environment variable, trimmed.
#[cfg(feature = "http")]
pub use crate::auth::environment::variable;

/// The bearer token, its lease and its expiry, for the object-store crate's
/// Google and Azure dialects; `instant` reads an expiry a tool wrote.
#[cfg(feature = "aws")]
pub use crate::auth::lease::{Bearer, Expiring, Lease, instant};

/// The AWS property reader's collection and its endpoint names, for the
/// object-store crate's property reader.
#[cfg(feature = "aws")]
pub use crate::aws::properties::{EndpointName, Identity};

/// The signer of one credential set bound to a region and a service, for
/// the object-store crate's client.
#[cfg(feature = "aws")]
pub use crate::aws::sigv4::Signer;

/// `aws::sigv4::EMPTY_PAYLOAD_SHA256`, for the object-store crate: the
/// payload hash of an empty body.
#[cfg(feature = "aws")]
pub const EMPTY_PAYLOAD_SHA256: &str = crate::aws::sigv4::EMPTY_PAYLOAD_SHA256;

/// `aws::sigv4::UNSIGNED_PAYLOAD`, for the object-store crate: the payload
/// hash of a body sent unsigned.
#[cfg(feature = "aws")]
pub const UNSIGNED_PAYLOAD: &str = crate::aws::sigv4::UNSIGNED_PAYLOAD;

/// `aws::sigv4::canonical_query`, for the object-store crate: the canonical
/// query string of raw name and value pairs.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn canonical_query(query: &[(String, String)]) -> String {
    crate::aws::sigv4::canonical_query(query)
}

/// `aws::sigv4::encode_key`, for the object-store crate: one raw object key
/// percent-encoded for the request path, its `/` kept.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn encode_key(key: &str) -> String {
    crate::aws::sigv4::encode_key(key)
}

/// `aws::sigv4::encode_query_component`, for the object-store crate: one
/// query name or value percent-encoded.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn encode_query_component(text: &str) -> String {
    crate::aws::sigv4::encode_query_component(text)
}

/// `aws::sigv4::sha256_hex`, for the object-store crate: lowercase hex
/// SHA-256 of `bytes`.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    crate::aws::sigv4::sha256_hex(bytes)
}

/// `aws::sigv4::signed_access_key`, for the object-store crate: the access
/// key an `Authorization` header was signed with.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn signed_access_key(authorization: &str) -> Option<&str> {
    crate::aws::sigv4::signed_access_key(authorization)
}

/// `aws::properties::count`, for the object-store crate: a whole number a
/// property states, refused naming it.
///
/// # Errors
///
/// Returns the refusal naming `name` where `value` is no count.
#[cfg(feature = "aws")]
#[inline]
pub fn count(name: &str, value: &str) -> Result<u32> {
    crate::aws::properties::count(name, value)
}

/// `aws::properties::flag`, for the object-store crate: a boolean a
/// property states, read through the one boolean table.
///
/// # Errors
///
/// Returns the refusal naming `name` where `value` is no boolean.
#[cfg(feature = "aws")]
#[inline]
pub fn flag(name: &str, value: &str) -> Result<bool> {
    crate::aws::properties::flag(name, value)
}

/// `aws::properties::refusal`, for the object-store crate: the error a
/// property reader refuses a value with.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn refusal(message: &str) -> crate::Error {
    crate::aws::properties::refusal(message)
}

/// `aws::properties::seconds`, for the object-store crate: a duration a
/// property states in seconds.
///
/// # Errors
///
/// Returns the refusal naming `name` where `value` is no duration.
#[cfg(feature = "aws")]
#[inline]
pub fn seconds(name: &str, value: &str) -> Result<std::time::Duration> {
    crate::aws::properties::seconds(name, value)
}

/// `aws::environment::is_native`, for the object-store crate: whether a
/// variable is one the AWS tools read for themselves, which the S3 options'
/// sweep leaves to the session.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn is_native(name: &str) -> bool {
    crate::aws::environment::is_native(name)
}

/// The element of a small fixed-shape document and its refusal, for the
/// object-store crate's XML vocabularies.
#[cfg(feature = "aws")]
pub use crate::xml::scanner::{Element as ScannedElement, XmlError};

/// `xml::scanner::parse_document`, for the object-store crate: the root
/// element of `xml`.
///
/// # Errors
///
/// Returns the refusal of malformed XML or anything outside the root.
#[cfg(feature = "aws")]
#[inline]
pub fn parse_document(xml: &[u8]) -> std::result::Result<ScannedElement, XmlError> {
    crate::xml::scanner::parse_document(xml)
}

/// `xml::scanner::parse_root`, for the object-store crate: the root of
/// `xml`, which must be named `expected`.
///
/// # Errors
///
/// Returns the refusal of malformed XML or of another root.
#[cfg(feature = "aws")]
#[inline]
pub fn parse_root(xml: &[u8], expected: &str) -> std::result::Result<ScannedElement, XmlError> {
    crate::xml::scanner::parse_root(xml, expected)
}

/// `ArnPartition::check_region`, for the object-store crate: a region read
/// off a location, a header or a profile refused unless it is one a host can
/// be spelled with.
///
/// # Errors
///
/// Returns the refusal naming `source` and the region.
#[cfg(feature = "aws")]
#[inline]
pub fn arn_partition_check_region(region: &str, source: &str) -> Result<()> {
    crate::ArnPartition::check_region(region, source)
}

/// `Session::signer`, for the object-store crate: the signer of the set in
/// hand at `now` for `service` in `region`; `None` for unsigned requests.
///
/// # Errors
///
/// Returns the credential chain's refusal.
#[cfg(feature = "aws")]
#[inline]
pub fn session_signer(
    session: &crate::aws::Session,
    service: &str,
    region: &str,
    now: std::time::SystemTime,
) -> Result<Option<std::sync::Arc<Signer>>> {
    session.signer(service, region, now)
}

/// `Session::answers_another`, for the object-store crate: whether a store's
/// refusal of the key `signed` leaves the session another set to sign with.
///
/// # Errors
///
/// Returns the session's own refusal once nothing answers any more.
#[cfg(feature = "aws")]
#[inline]
pub fn session_answers_another(
    session: &crate::aws::Session,
    signed: &str,
    code: Option<&str>,
    endpoint: &str,
    region: &str,
    now: std::time::SystemTime,
) -> Result<bool> {
    session.answers_another(signed, code, endpoint, region, now)
}

/// `Session::given_variables`, for the object-store crate: the environment a
/// caller handed the session instead of the process's.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_given_variables(
    session: &crate::aws::Session,
) -> Option<&std::collections::BTreeMap<String, String>> {
    session.given_variables()
}

/// `Session::tls_config`, for the object-store crate: the TLS setup the
/// session's certificate bundle states.
///
/// # Errors
///
/// Returns the refusal of a bundle that cannot be read.
#[cfg(feature = "aws")]
#[inline]
pub fn session_tls_config(session: &crate::aws::Session) -> Result<Option<ureq::tls::TlsConfig>> {
    session.tls_config()
}

/// `Session::bucket_region`, for the object-store crate: the region a
/// redirect found `bucket` in on `partition`'s hosts, when learned.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_bucket_region(session: &crate::aws::Session, partition: &str, bucket: &str) -> Option<String> {
    session.bucket_region(partition, bucket)
}

/// `Session::learn_bucket_region`, for the object-store crate: remember that
/// `bucket` answers in `region`, for every session sharing this one's.
#[cfg(feature = "aws")]
#[inline]
pub fn session_learn_bucket_region(session: &crate::aws::Session, bucket: &str, region: &str) {
    session.learn_bucket_region(bucket, region);
}

/// `Session::stated_region`, for the object-store crate: the region the
/// session was told, nothing resolved.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_stated_region(session: &crate::aws::Session) -> Option<&str> {
    session.stated_region()
}

/// `Session::states_identity`, for the object-store crate: whether the
/// session was told who to sign as.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_states_identity(session: &crate::aws::Session) -> bool {
    session.states_identity()
}

/// `Session::under`, for the object-store crate: `session` with what it
/// leaves unstated taken from `ambient`.
#[cfg(feature = "aws")]
#[inline]
#[must_use]
pub fn session_under(session: &crate::aws::Session, ambient: &crate::aws::Session) -> crate::aws::Session {
    session.under(ambient)
}
'''


def edit_implementer(root: pathlib.Path) -> None:
    rel = "rust/src/implementer.rs"
    edit(root, rel, *IMPLEMENTER_DOC)
    text = read(root / rel)
    if "// Object stores: what the object-store crate reaches" in text:
        return
    for name in ("Cow", "IOBase", "Scalar", "Result"):
        if not re.search(rf"\b{name}\b", text.split("// ----", 1)[0]):
            residue(rel, f"`{name}` is not imported at the top: the object-store section names it")
    write(root / rel, text.rstrip("\n") + "\n" + IMPLEMENTER_SECTION)


def edit_s3tables(root: pathlib.Path) -> list[str]:
    """The core's `s3tables/` (Iceberg's, D15) opens a table's store through
    the backend: re-spelled onto the crate, which the Iceberg crate's
    `s3tables` feature links (D36.6); the core cannot until it moves."""
    touched = []
    for path in sorted((root / "rust/src/s3tables").rglob("*.rs")):
        rel = str(path.relative_to(root))
        text = read(path)
        new = re.sub(r"(?<![\w:])crate::s3::", "yggdryl_s3::", text)
        if new != text:
            write(path, new)
            touched.append(rel)
    return touched


def edit_facade(root: pathlib.Path) -> None:
    rel = "rust/src/logging/facade.rs"
    text = read(root / rel)
    if '("yggdryl_s3", "yggdryl.s3")' not in text:
        m = re.search(r"const CRATES: \[\(&str, &str\); (\d+)\] = \[\n((?:    \([^\n]*\),\n)+)\];", text)
        if not m:
            residue(rel, "no `CRATES` table to add `yggdryl_s3` to")
            return
        rows = m.group(2).splitlines(keepends=True)
        rows.append('    ("yggdryl_s3", "yggdryl.s3"),\n')
        rows.sort(key=lambda row: re.search(r'"([^"]+)"', row).group(1))
        new = f"const CRATES: [(&str, &str); {len(rows)}] = [\n" + "".join(rows) + "];"
        text = text[:m.start()] + new + text[m.end():]
        write(root / rel, text)
    edit(root, "rust/tests/logging/facade.rs",
         ('        ("yggdryl_parquet::reader", "yggdryl.parquet.reader"),\n',
          '        ("yggdryl_parquet::reader", "yggdryl.parquet.reader"),\n'
          '        // The object stores are the `s3` folder they left.\n'
          '        ("yggdryl_s3::client", "yggdryl.s3.client"),\n'))

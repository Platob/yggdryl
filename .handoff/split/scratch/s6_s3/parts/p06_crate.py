

# ---------------------------------------------------------------------------
# Step 3: the crate's own sources
# ---------------------------------------------------------------------------

# Before the rewrite: what the moved sources spell that the rewrite cannot
# re-own - the two core modules the client imported whole, the session's
# crate-private doors called as methods, and links to what the core keeps
# private - in the core's own terms, so the rewrite takes them from there.
SOURCE_EDITS: dict[str, list[tuple[str, str]]] = {
    "client.rs": [
        ("use crate::aws::sigv4::{self, Signer};\n",
         "use crate::aws::sigv4::{\n    EMPTY_PAYLOAD_SHA256, Signer, UNSIGNED_PAYLOAD, canonical_query, encode_key, sha256_hex,\n"
         "    signed_access_key,\n};\n"),
        ("use crate::http::retry::{\n    self, RETRY_COST, RETRY_REFUND, RetryBudget, fresh_jitter, is_resumable,\n"
         "    is_retryable_transport, is_unsent,\n};\n",
         "use crate::http::retry::{\n    RETRY_COST, RETRY_REFUND, RetryBudget, delay, fresh_jitter, is_resumable,\n"
         "    is_retryable_transport, is_unsent, retry_after,\n};\n"),
        ("    retry::retry_after(value).filter(", "    retry_after(value).filter("),
        ("            path.push_str(&sigv4::encode_key(account));\n", "            path.push_str(&encode_key(account));\n"),
        ("            path.push_str(&sigv4::encode_key(container));\n", "            path.push_str(&encode_key(container));\n"),
        ("        let key = sigv4::encode_key(key);\n", "        let key = encode_key(key);\n"),
        ("    /// from the backoff window ([`retry::delay`]).\n", "    /// from the backoff window (`http::retry::delay`).\n"),
        ("        std::thread::sleep(retry::delay(attempt, asked, &self.jitter));\n",
         "        std::thread::sleep(delay(attempt, asked, &self.jitter));\n"),
        (".and_then(sigv4::signed_access_key)", ".and_then(signed_access_key)"),
        ("        let mut query = sigv4::canonical_query(&request.query);\n",
         "        let mut query = canonical_query(&request.query);\n"),
        ("None | Some([]) => sigv4::EMPTY_PAYLOAD_SHA256.to_owned(),", "None | Some([]) => EMPTY_PAYLOAD_SHA256.to_owned(),"),
        ("                        sigv4::sha256_hex(body)\n", "                        sha256_hex(body)\n"),
        ("Some(_) => sigv4::UNSIGNED_PAYLOAD.to_owned(),", "Some(_) => UNSIGNED_PAYLOAD.to_owned(),"),
        ("        self.session.signer(\"s3\", region, now)\n",
         "        crate::implementer::session_signer(&self.session, \"s3\", region, now)\n"),
        ("        let another = self.session.answers_another(\n            signed,\n",
         "        let another = crate::implementer::session_answers_another(\n            &self.session,\n            signed,\n"),
        ("        let tls = session.tls_config()?;\n", "        let tls = crate::implementer::session_tls_config(&session)?;\n"),
        ("        match self.session.bucket_region(partition, bucket) {\n",
         "        match crate::implementer::session_bucket_region(&self.session, partition, bucket) {\n"),
        ("            self.session.learn_bucket_region(bucket, region);\n",
         "            crate::implementer::session_learn_bucket_region(&self.session, bucket, region);\n"),
        ("(options.region(), session.stated_region())", "(options.region(), crate::implementer::session_stated_region(&session))"),
        ("[`crate::http::retry`]'s", "`http::retry`'s"),
        ("rule is `crate::http::retry`'s, re-exported here", "rule is `http::retry`'s, re-exported here"),
        ("[`Session::bucket_region`]", "`Session::bucket_region`"),
        ("[`Session::learn_bucket_region`]", "`Session::learn_bucket_region`"),
    ],
    "properties.rs": [
        ("        match self.session().given_variables() {\n",
         "        match crate::implementer::session_given_variables(self.session()) {\n"),
        ("!self.session().states_identity()", "!crate::implementer::session_states_identity(self.session())"),
        ("        let session = self.session().under(ambient.session());\n",
         "        let session = crate::implementer::session_under(self.session(), ambient.session());\n"),
    ],
    "xml.rs": [
        ("pub(crate) use crate::xml::scanner::{Element, XmlError, parse_document, parse_root};\n",
         "pub(crate) use crate::xml::scanner::{Element as Element, XmlError, parse_document, parse_root};\n"),
        ("[`crate::xml::scanner`]", "`xml::scanner`"),
        ("    //! `yggdryl::internals::xml`.\n", "    //! `yggdryl_s3::internals::xml`.\n"),
    ],
    "aws/xml.rs": [
        ("[`crate::xml::scanner`]", "`xml::scanner`"),
    ],
}
# Every `[`crate::ArnPartition::check_region`]` the client's docs link: the
# rule is the core's own, reached through the implementer.
CHECK_REGION_LINK = "[`crate::ArnPartition::check_region`]"

LIB_DOC_EDITS = [
    ("//! The ten schemes are [`S3_BACKEND`]'s, the\n//! [`StorageBackend`] the core claims under\n"
     "//! them until `yggdryl-s3` does: [`Holder::from_url`] holds a location of any\n"
     "//! of them as the [`S3Path`] it names, [`Holder::Registered`] as every\n"
     "//! claimed backend's handle is, and [`Holder::downcast_ref`] answers it as\n"
     "//! the role it is.\n//!\n",
     "//! The ten schemes are [`S3_BACKEND`]'s, the [`StorageBackend`] [`install`]\n"
     "//! claims under them: [`Holder::from_url`] holds a location of any of them\n"
     "//! as the [`S3Path`] it names, [`Holder::Registered`] as every claimed\n"
     "//! backend's handle is, and [`Holder::downcast_ref`] answers it as the role\n"
     "//! it is. Every binding's init and the `yggdryl` command's `main` call\n"
     "//! [`install`], and so does a Rust program before the core reads a location\n"
     "//! of the ten schemes, which is refused naming the crate to install until\n"
     "//! then; a handle built through this crate's own doors - [`file()`],\n"
     "//! [`folder()`], [`located()`] - needs no claim.\n//!\n"),
    ("/// The core claims it itself, before the register answers anything, until\n"
     "/// `yggdryl-s3`'s `install()` does. A location's query states the store's\n"
     "/// properties in the names [`S3Options::with_properties`] reads, and\n",
     "/// [`install`] claims it, once for the life of the process. A location's\n"
     "/// query states the store's properties in the names\n"
     "/// [`S3Options::with_properties`] reads, and\n"),
    ('.expect("the core claims `gs`");', '.expect("`install()` claimed `gs`");'),
    ("use std::sync::Arc;\n", "use std::sync::{Arc, Mutex, OnceLock, PoisonError};\n"),
]

INSTALL_FN = '''
/// What this crate claims the object stores' backend as.
const CLAIMANT: &str = "yggdryl-s3";

/// Claims the object stores' backend on the core's register of storage
/// backends - [`S3_BACKEND`] under its ten schemes, `s3`, `s3a`, `s3n`,
/// `gs`, `gcs`, `az`, `abfs`, `abfss`, `wasb` and `wasbs` - once for the life
/// of the process; a later call returns at once. Every binding's init and
/// the `yggdryl` command's `main` call it, and so does a Rust caller before
/// [`Holder::from_url`] reads a location of those schemes: until the claim
/// such a location is refused, naming the crate to install.
///
/// ```
/// use yggdryl::Scheme;
/// use yggdryl::holder::backend_for;
///
/// # fn main() -> yggdryl::Result<()> {
/// yggdryl_s3::install()?;
/// yggdryl_s3::install()?;
/// let claimed = backend_for(&Scheme::AZ).expect("`install()` claimed `az`");
/// assert_eq!(claimed.name(), "yggdryl-s3");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the register's refusal where another crate claimed one of the ten
/// schemes first, naming it.
pub fn install() -> Result<()> {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    static INSTALLING: Mutex<()> = Mutex::new(());
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    let _installing = INSTALLING.lock().unwrap_or_else(PoisonError::into_inner);
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    crate::holder::claim_backend(&S3_BACKEND, CLAIMANT)?;
    let _ = INSTALLED.set(());
    Ok(())
}
'''


def build_lib(root: pathlib.Path) -> None:
    """The crate root: the module's own doc and items, `install()` beside the
    backend it claims."""
    rel = f"{CRATE_DIR}/src/lib.rs"
    text = edit_text(read(root / rel), rel, *LIB_DOC_EDITS)
    # `#![deny(unsafe_code)]` after the crate's own documentation.
    head = re.match(r"(?:[ \t]*//![^\n]*\n)+", text)
    if head and "#![deny(unsafe_code)]" not in text:
        text = text[:head.end()] + "\n#![deny(unsafe_code)]\n" + text[head.end():]
    anchor = "use client::Client;\n"
    if text.count(anchor) != 1:
        residue(rel, "no `use client::Client;` to place `install()` after: add it by hand")
    elif "pub fn install()" not in text:
        text = text.replace(anchor, anchor + INSTALL_FN, 1)
    write(root / rel, text)


def edit_crate_sources(root: pathlib.Path) -> None:
    base = f"{CRATE_DIR}/src"
    for rel, pairs in SOURCE_EDITS.items():
        edit(root, f"{base}/{rel}", *pairs)
    rel = f"{base}/client.rs"
    text = read(root / rel)
    if CHECK_REGION_LINK in text:
        text = text.replace(CHECK_REGION_LINK, "`ArnPartition::check_region`")
        write(root / rel, text)
    done("crate: lib.rs and install(), the client's flattened imports, the session's doors through the implementer")


def finish_crate_text(root: pathlib.Path) -> None:
    """After the rewrite: the scanner's element under the name the implementer
    publishes it by, aliased back to what the backend calls it."""
    rel = f"{CRATE_DIR}/src/xml.rs"
    text = read(root / rel)
    new = text.replace("ScannedElement as Element as Element", "ScannedElement as Element")
    if "ScannedElement as Element" not in new:
        residue(rel, "the scanner's element is not imported as `ScannedElement as Element`: fix the `use` by hand")
    if new != text:
        write(root / rel, new)

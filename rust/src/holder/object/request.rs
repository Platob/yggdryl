//! One request, before it is signed and sent.
//!
//! Shared vocabulary between the transport and the three dialects: a dialect
//! says what a request *is* - its method, its path, its query, its headers, its
//! body - and the client says how it goes out and comes back. Neither knows the
//! other's business.

use super::encryption::Encryption;
use super::provider::Provider;

/// One request, before it is signed and sent.
pub(crate) struct Request<'body> {
    pub(crate) method: &'static str,
    /// What the store calls this operation, for the error it may answer with.
    pub(crate) operation: &'static str,
    pub(crate) bucket: String,
    /// The raw object key, unencoded.
    pub(crate) key: String,
    /// Raw query pairs; the canonical form is built once, at signing.
    pub(crate) query: Vec<(String, String)>,
    /// Headers beyond the ones signing always adds.
    pub(crate) headers: Vec<(String, String)>,
    /// The body, which is re-sent verbatim on a retry.
    pub(crate) body: &'body [u8],
    /// The wire path, when the dialect's operation does not live under the
    /// container's own. Already encoded; the endpoint is not consulted for it.
    pub(crate) target: Option<String>,
    /// The whole URL, when the store handed one back and the endpoint has no
    /// say in it - which a resumable upload session is.
    pub(crate) url: Option<String>,
}

impl<'body> Request<'body> {
    pub(crate) fn new(
        method: &'static str,
        operation: &'static str,
        bucket: &str,
        key: &str,
    ) -> Self {
        Self {
            method,
            operation,
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            query: Vec::new(),
            headers: Vec::new(),
            body: &[],
            target: None,
            url: None,
        }
    }

    /// Send this request to `path` rather than to the container's own path.
    pub(crate) fn target(mut self, path: impl Into<String>) -> Self {
        self.target = Some(path.into());
        self
    }

    /// The same, for a path that is not under any container at all.
    pub(crate) fn absolute_path(self, path: impl Into<String>) -> Self {
        self.target(path)
    }

    /// Send this request to `url`, wherever it points.
    ///
    /// A store that hands a location back - a resumable upload session - owns
    /// that location entirely, down to its host and its query, so nothing here
    /// takes it apart or adds to it.
    pub(crate) fn absolute(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub(crate) fn query(mut self, name: &str, value: impl Into<String>) -> Self {
        self.query.push((name.to_owned(), value.into()));
        self
    }

    pub(crate) fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }

    pub(crate) fn body(mut self, body: &'body [u8]) -> Self {
        self.body = body;
        self
    }

    /// Carry the metadata every write of this client's carries.
    ///
    /// A name the request already set wins, so a content type a handle
    /// inferred is never overridden by a default. The name a caller gave is
    /// mapped to the header the store carries it in here rather than when the
    /// options were built, because the three stores prefix user metadata
    /// differently and the options do not yet know which store answers.
    pub(crate) fn with_metadata(
        mut self,
        provider: Provider,
        metadata: &[(String, String)],
    ) -> Self {
        for (name, value) in metadata {
            let name = header_name(provider, name);
            if !self
                .headers
                .iter()
                .any(|(held, _)| held.eq_ignore_ascii_case(&name))
            {
                self.headers.push((name, value.clone()));
            }
        }
        self
    }

    /// Say how the object this request stores is to be encrypted.
    ///
    /// The request that creates an object is the one that decides it, and on
    /// Google that decision is a query parameter rather than a header.
    pub(crate) fn storing(mut self, provider: Provider, encryption: &Encryption) -> Self {
        self.headers.extend(
            encryption
                .write_headers(provider)
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        self.query.extend(encryption.write_query(provider));
        self
    }

    /// Carry the key needed to touch an already-encrypted object's bytes.
    ///
    /// Nothing at all unless the key is the caller's, which is the whole
    /// difference a customer-supplied key makes to a read.
    pub(crate) fn keyed(mut self, provider: Provider, encryption: &Encryption) -> Self {
        self.headers.extend(
            encryption
                .read_headers(provider)
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        self
    }
}

/// The header a metadata name goes over as, on `provider`.
///
/// Every store defines a handful of names itself - the content headers HTTP
/// already has - and treats every other name as user metadata under its own
/// prefix. A caller who spells a store's prefix is taken at their word.
pub(crate) fn header_name(provider: Provider, name: &str) -> String {
    const OWN: [&str; 6] = [
        "cache-control",
        "content-disposition",
        "content-encoding",
        "content-language",
        "content-type",
        "expires",
    ];
    let lowered = name.trim().to_ascii_lowercase();
    if OWN.contains(&lowered.as_str()) || lowered.starts_with(provider.header_prefix()) {
        lowered
    } else {
        format!("{}{lowered}", provider.metadata_prefix())
    }
}

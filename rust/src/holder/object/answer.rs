//! What a store's answer says, in shapes no one store owns.
//!
//! Three stores spell a listing page three ways - `ListBucketResult` on Amazon
//! S3, `EnumerationResults` on Azure Blob Storage, a JSON object on Google
//! Cloud Storage - and one of the three is a different markup language. What
//! each *says* is the same: objects with their sizes, prefixes rolled up under
//! a delimiter, and a token for the next page. These are those shapes, so a
//! dialect parses into them and everything above a dialect is written once.

/// One object's metadata, as a store states it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObjectMeta {
    /// The object's byte length.
    pub(crate) size: u64,
    /// The entity tag, quotes included, when the store gave one.
    pub(crate) etag: Option<String>,
    /// The stored `Content-Type`, when the store gave one.
    pub(crate) content_type: Option<String>,
}

/// One object a listing page named, with what the page said about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObjectSummary {
    /// The object key, percent-decoded when the page says
    /// `<EncodingType>url</EncodingType>`.
    pub(crate) key: String,
    /// The object size in bytes.
    pub(crate) size: u64,
    /// The `ETag` as given, quotes included.
    pub(crate) etag: Option<String>,
    /// The `LastModified` timestamp as given.
    pub(crate) last_modified: Option<String>,
}

/// One page of a listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListPage {
    /// The objects in document order, which is key order.
    pub(crate) objects: Vec<ObjectSummary>,
    /// The `CommonPrefixes`, decoded like keys.
    pub(crate) prefixes: Vec<String>,
    /// Whether another page follows.
    pub(crate) is_truncated: bool,
    /// The token that fetches the next page; absent on the last one.
    pub(crate) next_continuation_token: Option<String>,
}

/// What a store's refusal said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ErrorBody {
    /// The error code (`NoSuchKey`, `AccessDenied`, ...); empty when absent.
    pub(crate) code: String,
    /// The human-readable message; empty when absent.
    pub(crate) message: String,
    /// The `RequestId` S3 stamps for support.
    pub(crate) request_id: Option<String>,
    /// The `Resource` the failure names, when it names one.
    pub(crate) resource: Option<String>,
}

/// One key a bulk delete did not remove, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeleteFailure {
    /// The key that was not deleted.
    pub(crate) key: String,
    /// The error code.
    pub(crate) code: String,
    /// The human-readable message.
    pub(crate) message: String,
}

//! Namespaces: the one level a table bucket groups its tables by.

use super::bucket::{absent_is_done, bucket_label};
use super::client::{Call, Reader, S3Tables, body, check_name, label};
use super::listing::{NamespaceSummaries, Page};
use crate::{Arn, DateTime64, Result, Scalar};

/// What a refusal calls the thing a namespace verb addresses.
pub(crate) const KIND: &str = "namespace";

/// One namespace of a table bucket, as the service describes it - by
/// `GetNamespace` and in a listing alike, the model's `NamespaceSummary`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceSummary {
    name: String,
    created_at: DateTime64,
    created_by: String,
    owner_account_id: String,
}

impl NamespaceSummary {
    /// The namespace's name, unique in its table bucket.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// When the namespace was created.
    pub fn created_at(&self) -> DateTime64 {
        self.created_at
    }

    /// The account that created the namespace.
    pub fn created_by(&self) -> &str {
        &self.created_by
    }

    /// The account that owns the namespace.
    pub fn owner_account_id(&self) -> &str {
        &self.owner_account_id
    }

    /// Read one namespace out of an answer: `GetNamespace`'s document, or
    /// one entry of a listing, which state the same members.
    pub(crate) fn read(reader: Reader<'_>) -> Result<Self> {
        Ok(Self {
            name: reader.namespace()?,
            created_at: reader.instant("createdAt")?,
            created_by: reader.text("createdBy")?.to_owned(),
            owner_account_id: reader.text("ownerAccountId")?.to_owned(),
        })
    }
}

impl S3Tables {
    /// Create the namespace `namespace` in the table bucket `bucket`, with
    /// one `PUT`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN that names
    /// no table bucket or a name that is not 1 to 255 of `0-9`, `a-z` and
    /// `_`, before any request; [`Error::Conflict`](crate::Error::Conflict)
    /// when the bucket already holds a namespace of that name; and the
    /// service's refusal otherwise - a bucket that is not there is its
    /// `404`, since what is missing is not the namespace.
    pub fn create_namespace(&self, bucket: &Arn, namespace: &str) -> Result<()> {
        check_namespace(namespace)?;
        let call = Call::put(
            "CreateNamespace",
            format!("/namespaces/{}", bucket_label(bucket)?),
            addressed(bucket, namespace),
            body([(
                "namespace",
                Scalar::from_sequence([Scalar::from(namespace)]),
            )])?,
        )
        .in_bucket(bucket)
        .conflict_as(KIND);
        self.send(&call).map(drop)
    }

    /// Describe the namespace `namespace` of the table bucket `bucket`,
    /// with one `GET`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN or a name
    /// the model refuses, before any request;
    /// [`Error::Absent`](crate::Error::Absent) when the bucket holds no such
    /// namespace; and the service's refusal otherwise.
    pub fn get_namespace(&self, bucket: &Arn, namespace: &str) -> Result<NamespaceSummary> {
        let call = Call::get(
            "GetNamespace",
            namespace_path(bucket, namespace)?,
            addressed(bucket, namespace),
        )
        .in_bucket(bucket)
        .absent_as(KIND);
        NamespaceSummary::read(self.send(&call)?.reader())
    }

    /// Every namespace of the table bucket `bucket`, one `GET` per page as
    /// the iterator is drained and none before it is asked. An ARN that
    /// names no table bucket is the iterator's one item, a refusal.
    pub fn namespaces(&self, bucket: &Arn) -> NamespaceSummaries {
        let page = bucket_label(bucket).map(|arn| Page {
            operation: "ListNamespaces",
            path: format!("/namespaces/{arn}"),
            query: Vec::new(),
            bucket: Some(bucket.clone()),
            addressed: bucket.to_string(),
            entries: "namespaces",
            limit: "maxNamespaces",
        });
        NamespaceSummaries::new(self.clone(), page)
    }

    /// Remove the namespace `namespace` of the table bucket `bucket`, with
    /// one `DELETE`. A namespace that is not there is already deleted,
    /// which is success.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN or a name
    /// the model refuses, before any request, and the service's refusal
    /// otherwise - a namespace still holding a table is a `409`.
    pub fn remove_namespace(&self, bucket: &Arn, namespace: &str) -> Result<()> {
        let call = Call::delete(
            "DeleteNamespace",
            namespace_path(bucket, namespace)?,
            addressed(bucket, namespace),
        )
        .in_bucket(bucket)
        .absent_as(KIND);
        absent_is_done(self.send(&call))
    }
}

/// Refuse `namespace` unless the model lets a namespace be called it.
pub(crate) fn check_namespace(namespace: &str) -> Result<()> {
    check_name("namespace name", namespace, 1, 255, b'_')
}

/// `/namespaces/{tableBucketARN}/{namespace}`, each label encoded once.
fn namespace_path(bucket: &Arn, namespace: &str) -> Result<String> {
    check_namespace(namespace)?;
    Ok(format!(
        "/namespaces/{}/{}",
        bucket_label(bucket)?,
        label(namespace)
    ))
}

/// What a refusal reports a namespace as: its table bucket's ARN and its
/// name below it.
pub(crate) fn addressed(bucket: &Arn, namespace: &str) -> String {
    format!("{bucket}/{namespace}")
}

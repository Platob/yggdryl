//! Table buckets: the container a region's namespaces and tables live in.

use smol_str::format_smolstr;

use super::client::{Call, Reader, S3Tables, SERVICE, body, check_name, echo, label};
use super::listing::{Page, TableBuckets};
use crate::{Arn, DateTime64, Error, Result, Scalar};

/// What a refusal calls the thing a table bucket verb addresses.
pub(crate) const KIND: &str = "table bucket";

/// One table bucket, as the service describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableBucket {
    arn: Arn,
    name: String,
    owner_account_id: String,
    created_at: DateTime64,
}

impl TableBucket {
    /// The ARN every verb below the bucket addresses it by.
    pub fn arn(&self) -> &Arn {
        &self.arn
    }

    /// The bucket's name, unique in its account and region.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The account that owns the bucket.
    pub fn owner_account_id(&self) -> &str {
        &self.owner_account_id
    }

    /// When the bucket was created.
    pub fn created_at(&self) -> DateTime64 {
        self.created_at
    }

    /// Read one bucket out of an answer: `GetTableBucket`'s document, or
    /// one entry of a listing, which state the same members.
    pub(crate) fn read(reader: Reader<'_>) -> Result<Self> {
        Ok(Self {
            arn: reader.arn("arn")?,
            name: reader.text("name")?.to_owned(),
            owner_account_id: reader.text("ownerAccountId")?.to_owned(),
            created_at: reader.instant("createdAt")?,
        })
    }
}

impl S3Tables {
    /// Create the table bucket `name` in the client's region and answer its
    /// ARN, with one `PUT /buckets`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a name that is not 3 to 63 of `0-9`,
    /// `a-z` and `-`, before any request; [`Error::Conflict`] when the
    /// account already has a bucket of that name in the region; and the
    /// service's refusal otherwise.
    pub fn create_table_bucket(&self, name: &str) -> Result<Arn> {
        check_name("table bucket name", name, 3, 63, b'-')?;
        let call = Call::put(
            "CreateTableBucket",
            "/buckets".to_owned(),
            name.to_owned(),
            body([("name", Scalar::from(name))])?,
        )
        .conflict_as(KIND);
        self.send(&call)?.reader().arn("arn")
    }

    /// Describe the table bucket `bucket`, with one `GET`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for an ARN that names no table bucket,
    /// before any request; [`Error::Absent`] when the service holds no such
    /// bucket; and the service's refusal otherwise.
    pub fn get_table_bucket(&self, bucket: &Arn) -> Result<TableBucket> {
        let call = Call::get("GetTableBucket", bucket_path(bucket)?, bucket.to_string())
            .in_bucket(bucket)
            .absent_as(KIND);
        TableBucket::read(self.send(&call)?.reader())
    }

    /// Every table bucket of the account in the client's region, one `GET`
    /// per page as the iterator is drained and none before it is asked.
    pub fn table_buckets(&self) -> TableBuckets {
        TableBuckets::new(
            self.clone(),
            Ok(Page {
                operation: "ListTableBuckets",
                path: "/buckets".to_owned(),
                query: Vec::new(),
                bucket: None,
                addressed: "table buckets".to_owned(),
                entries: "tableBuckets",
                limit: "maxBuckets",
            }),
        )
    }

    /// Remove the table bucket `bucket`, with one `DELETE`. A bucket that
    /// is not there is already deleted, which is success.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for an ARN that names no table bucket,
    /// before any request, and the service's refusal otherwise - a bucket
    /// still holding a namespace is a `409`.
    pub fn remove_table_bucket(&self, bucket: &Arn) -> Result<()> {
        let call = Call::delete(
            "DeleteTableBucket",
            bucket_path(bucket)?,
            bucket.to_string(),
        )
        .in_bucket(bucket)
        .absent_as(KIND);
        absent_is_done(self.send(&call))
    }
}

/// `/buckets/{tableBucketARN}`, the ARN spelled as one label.
fn bucket_path(bucket: &Arn) -> Result<String> {
    Ok(format!("/buckets/{}", bucket_label(bucket)?))
}

/// The path label of the table bucket `bucket`: its ARN percent-encoded
/// once.
///
/// # Errors
///
/// Returns [`Error::Parse`] when the ARN's service is not `s3tables`, at
/// the service field, or when its resource is not `bucket/<name>`, at the
/// resource: an ARN naming a table, an Amazon S3 bucket or another
/// service's resource addresses no table bucket.
pub(crate) fn bucket_label(bucket: &Arn) -> Result<String> {
    let text = bucket.to_string();
    let refuse = |position: usize| Error::Parse {
        target: "table bucket arn",
        position,
        reason: format_smolstr!(
            "expected a table bucket ARN - arn:<partition>:s3tables:<region>:<account>:bucket/<name> - got {:?}",
            echo(&text)
        ),
    };
    if bucket.service() != SERVICE {
        return Err(refuse("arn:".len() + bucket.partition().len() + 1));
    }
    let resource = bucket.resource();
    let named = resource
        .strip_prefix("bucket/")
        .is_some_and(|name| !name.is_empty() && !name.contains('/'));
    if !named {
        return Err(refuse(text.len() - resource.len()));
    }
    Ok(label(&text))
}

/// A delete's answer, with the service's own absence read as the deletion
/// already done.
///
/// Only [`Error::Absent`] is: a transport failure that happens to spell
/// "not found" - a host that does not resolve - deleted nothing.
pub(crate) fn absent_is_done<T>(sent: Result<T>) -> Result<()> {
    match sent {
        Ok(_) | Err(Error::Absent { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

//! Tables: what a namespace holds, and the metadata location each commit
//! moves under a version token.
//!
//! A table of a table bucket is an Iceberg table whose files live at a
//! warehouse location only the service chooses, and whose current metadata
//! file is named by the service rather than by a version hint beside it. A
//! commit is therefore not a file written but
//! [`S3Tables::update_table_metadata_location`]: the new metadata file's
//! location, under the version token the last reading answered. The token
//! moves on every change to the table, so a commit that lost a race is
//! refused rather than overwriting the one that won it.

use super::bucket::{absent_is_done, bucket_label};
use super::client::{
    Call, Reader, S3Tables, body, check_name, check_version_token, invalid_input, label,
};
use super::listing::{Page, TableSummaries};
use super::namespace::check_namespace;
use crate::iceberg::{PartitionSpec, SortOrder};
use crate::{Arn, DateTime64, Field, Result, Scalar, Url};

/// What a refusal calls the thing a table verb addresses.
const KIND: &str = "table";

/// The one table format the service holds.
const FORMAT: &str = "ICEBERG";

/// One table, as `GetTable` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDescription {
    arn: Arn,
    name: String,
    namespace: String,
    version_token: String,
    metadata_location: Option<Url>,
    warehouse_location: Url,
    created_at: DateTime64,
    modified_at: DateTime64,
    format: String,
}

impl TableDescription {
    /// The table's ARN, which names it by the identifier the service gave
    /// it rather than by its name.
    pub fn arn(&self) -> &Arn {
        &self.arn
    }

    /// The table's name, unique in its namespace.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The namespace the table is in.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The token the next change to the table is made under.
    pub fn version_token(&self) -> &str {
        &self.version_token
    }

    /// Where the table's current metadata file is; `None` for a table
    /// created without a schema that nothing has committed to yet.
    pub fn metadata_location(&self) -> Option<&Url> {
        self.metadata_location.as_ref()
    }

    /// The location every file of the table lives under.
    pub fn warehouse_location(&self) -> &Url {
        &self.warehouse_location
    }

    /// When the table was created.
    pub fn created_at(&self) -> DateTime64 {
        self.created_at
    }

    /// When the table was last changed.
    pub fn modified_at(&self) -> DateTime64 {
        self.modified_at
    }

    /// The table format, `ICEBERG`.
    pub fn format(&self) -> &str {
        &self.format
    }

    fn read(reader: Reader<'_>) -> Result<Self> {
        Ok(Self {
            arn: reader.arn("tableARN")?,
            name: reader.text("name")?.to_owned(),
            namespace: reader.namespace()?,
            version_token: reader.text("versionToken")?.to_owned(),
            metadata_location: reader.optional_url("metadataLocation")?,
            warehouse_location: reader.url("warehouseLocation")?,
            created_at: reader.instant("createdAt")?,
            modified_at: reader.instant("modifiedAt")?,
            format: reader.text("format")?.to_owned(),
        })
    }
}

/// One table as a listing states it: what names it and when it changed,
/// without the version token and the locations only `GetTable` answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSummary {
    arn: Arn,
    name: String,
    namespace: String,
    created_at: DateTime64,
    modified_at: DateTime64,
}

impl TableSummary {
    /// The table's ARN.
    pub fn arn(&self) -> &Arn {
        &self.arn
    }

    /// The table's name, unique in its namespace.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The namespace the table is in.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// When the table was created.
    pub fn created_at(&self) -> DateTime64 {
        self.created_at
    }

    /// When the table was last changed.
    pub fn modified_at(&self) -> DateTime64 {
        self.modified_at
    }

    pub(crate) fn read(reader: Reader<'_>) -> Result<Self> {
        Ok(Self {
            arn: reader.arn("tableARN")?,
            name: reader.text("name")?.to_owned(),
            namespace: reader.namespace()?,
            created_at: reader.instant("createdAt")?,
            modified_at: reader.instant("modifiedAt")?,
        })
    }
}

/// A table's ARN and the token its next change is made under: what a
/// creation and a commit answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableVersion {
    arn: Arn,
    version_token: String,
}

impl TableVersion {
    /// The table's ARN.
    pub fn arn(&self) -> &Arn {
        &self.arn
    }

    /// The token the next change to the table is made under.
    pub fn version_token(&self) -> &str {
        &self.version_token
    }

    fn read(reader: Reader<'_>) -> Result<Self> {
        Ok(Self {
            arn: reader.arn("tableARN")?,
            version_token: reader.text("versionToken")?.to_owned(),
        })
    }
}

/// Where a table's current metadata file is, the token a commit replacing
/// it is made under, and the warehouse the replacement is written into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableMetadataLocation {
    version_token: String,
    metadata_location: Option<Url>,
    warehouse_location: Url,
}

impl TableMetadataLocation {
    /// The token the next change to the table is made under.
    pub fn version_token(&self) -> &str {
        &self.version_token
    }

    /// Where the table's current metadata file is; `None` for a table
    /// created without a schema that nothing has committed to yet.
    pub fn metadata_location(&self) -> Option<&Url> {
        self.metadata_location.as_ref()
    }

    /// The location every file of the table lives under.
    pub fn warehouse_location(&self) -> &Url {
        &self.warehouse_location
    }

    fn read(reader: Reader<'_>) -> Result<Self> {
        Ok(Self {
            version_token: reader.text("versionToken")?.to_owned(),
            metadata_location: reader.optional_url("metadataLocation")?,
            warehouse_location: reader.url("warehouseLocation")?,
        })
    }
}

impl S3Tables {
    /// Create the Iceberg table `name` in the namespace `namespace` of the
    /// table bucket `bucket`, with one `PUT`, and answer its ARN and first
    /// version token.
    ///
    /// A `schema` is sent as the table's Iceberg schema, so the service
    /// writes the first metadata file itself: the schema as Iceberg
    /// expresses it, a column that carries no Iceberg field id yet numbered
    /// above the highest one present, the way a table's own metadata
    /// numbers it. What the schema declares beside its columns goes with
    /// it - its `PARTITION:by` (or its marked partition columns) as the
    /// table's partition spec, its `SORT:by` as the table's write order -
    /// so a declared layout is the table's, never dropped on the way.
    /// Without a schema the table has no metadata location until something
    /// commits one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN or a name
    /// the model refuses, and the Iceberg writers' refusal of a `schema`
    /// that is not a non-null struct, holds a type Iceberg cannot spell or
    /// declares a partition or an order no Iceberg spec holds, before any
    /// request;
    /// [`Error::Conflict`](crate::Error::Conflict) when the namespace
    /// already holds a table of that name; and the service's refusal
    /// otherwise - a namespace that is not there is its `404`, since what
    /// is missing is not the table.
    pub fn create_table(
        &self,
        bucket: &Arn,
        namespace: &str,
        name: &str,
        schema: Option<&Field>,
    ) -> Result<TableVersion> {
        check_namespace(namespace)?;
        check_table(name)?;
        let mut members = vec![
            ("name", Scalar::from(name)),
            ("format", Scalar::from(FORMAT)),
        ];
        if let Some(schema) = schema {
            members.push((
                "metadata",
                Scalar::from_struct([("iceberg", iceberg_metadata(schema)?)])?,
            ));
        }
        let call = Call::put(
            "CreateTable",
            format!("/tables/{}/{}", bucket_label(bucket)?, label(namespace)),
            addressed(bucket, namespace, name),
            crate::json::into_bytes(&Scalar::from_struct(members)?)?,
        )
        .in_bucket(bucket)
        .conflict_as(KIND);
        TableVersion::read(self.send(&call)?.reader())
    }

    /// Describe the table `name` in the namespace `namespace` of the table
    /// bucket `bucket`, with one `GET`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN or a name
    /// the model refuses, before any request;
    /// [`Error::Absent`](crate::Error::Absent) when the namespace holds no
    /// such table; and the service's refusal otherwise.
    pub fn get_table(&self, bucket: &Arn, namespace: &str, name: &str) -> Result<TableDescription> {
        check_namespace(namespace)?;
        check_table(name)?;
        // The one operation that addresses a table by query: the ARN goes
        // in raw, and is encoded where every query pair is.
        bucket_label(bucket)?;
        let call = Call::get(
            "GetTable",
            "/get-table".to_owned(),
            addressed(bucket, namespace, name),
        )
        .with_query("tableBucketARN", &bucket.to_string())
        .with_query("namespace", namespace)
        .with_query("name", name)
        .in_bucket(bucket)
        .absent_as(KIND);
        TableDescription::read(self.send(&call)?.reader())
    }

    /// Every table of the table bucket `bucket` - of its namespace
    /// `namespace` alone when one is named - one `GET` per page as the
    /// iterator is drained and none before it is asked. An ARN or a
    /// namespace name the model refuses is the iterator's one item, a
    /// refusal.
    pub fn tables(&self, bucket: &Arn, namespace: Option<&str>) -> TableSummaries {
        let page = namespace
            .map_or(Ok(()), check_namespace)
            .and_then(|()| bucket_label(bucket))
            .map(|arn| Page {
                operation: "ListTables",
                path: format!("/tables/{arn}"),
                query: namespace
                    .map(|namespace| ("namespace".to_owned(), namespace.to_owned()))
                    .into_iter()
                    .collect(),
                bucket: Some(bucket.clone()),
                addressed: namespace.map_or_else(
                    || bucket.to_string(),
                    |namespace| super::namespace::addressed(bucket, namespace),
                ),
                entries: "tables",
                limit: "maxTables",
            });
        TableSummaries::new(self.clone(), page)
    }

    /// Rename the table `name`, move it to another namespace of its table
    /// bucket, or both, with one `PUT`; under `version_token` when one is
    /// given, so a table changed since that token was read is not renamed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN, a name or
    /// an empty version token the model refuses, and
    /// [`Error::Io`](crate::Error::Io) for a rename that names neither a new
    /// namespace nor a new name, or names the ones the table has - nothing
    /// to change, so nothing is sent - before any request; and the
    /// service's refusal otherwise: a stale token and a name already taken
    /// are both its `409`, a table or a target namespace that is not there
    /// its `404`.
    pub fn rename_table(
        &self,
        bucket: &Arn,
        namespace: &str,
        name: &str,
        new_namespace: Option<&str>,
        new_name: Option<&str>,
        version_token: Option<&str>,
    ) -> Result<()> {
        new_namespace.map_or(Ok(()), check_namespace)?;
        new_name.map_or(Ok(()), check_table)?;
        version_token.map_or(Ok(()), check_version_token)?;
        let path = format!("{}/rename", table_path(bucket, namespace, name)?);
        if new_namespace.is_none_or(|to| to == namespace) && new_name.is_none_or(|to| to == name) {
            return Err(invalid_input(format!(
                "expected a new namespace or a new name to rename {:?} to, got {}",
                addressed(bucket, namespace, name),
                if new_namespace.is_none() && new_name.is_none() {
                    "neither"
                } else {
                    "the ones it has"
                }
            )));
        }
        let members: Vec<(&str, Scalar)> = [
            ("newNamespaceName", new_namespace),
            ("newName", new_name),
            ("versionToken", version_token),
        ]
        .into_iter()
        .filter_map(|(member, value)| value.map(|value| (member, Scalar::from(value))))
        .collect();
        let call = Call::put(
            "RenameTable",
            path,
            addressed(bucket, namespace, name),
            crate::json::into_bytes(&Scalar::from_struct(members)?)?,
        )
        .in_bucket(bucket);
        self.send(&call).map(drop)
    }

    /// Remove the table `name`, with one `DELETE`; under `version_token`
    /// when one is given, so a table changed since that token was read is
    /// kept. A table that is not there is already deleted, which is
    /// success.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN, a name or
    /// an empty version token the model refuses, before any request, and
    /// the service's refusal otherwise - a stale token is a `409`.
    pub fn remove_table(
        &self,
        bucket: &Arn,
        namespace: &str,
        name: &str,
        version_token: Option<&str>,
    ) -> Result<()> {
        let mut call = Call::delete(
            "DeleteTable",
            table_path(bucket, namespace, name)?,
            addressed(bucket, namespace, name),
        )
        .in_bucket(bucket)
        .absent_as(KIND);
        if let Some(token) = version_token {
            check_version_token(token)?;
            call = call.with_query("versionToken", token);
        }
        absent_is_done(self.send(&call))
    }

    /// Where the table `name` keeps its current metadata file, and the
    /// token a commit replacing it is made under, with one `GET`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN or a name
    /// the model refuses, before any request;
    /// [`Error::Absent`](crate::Error::Absent) when the namespace holds no
    /// such table; and the service's refusal otherwise.
    pub fn get_table_metadata_location(
        &self,
        bucket: &Arn,
        namespace: &str,
        name: &str,
    ) -> Result<TableMetadataLocation> {
        let call = Call::get(
            "GetTableMetadataLocation",
            format!("{}/metadata-location", table_path(bucket, namespace, name)?),
            addressed(bucket, namespace, name),
        )
        .in_bucket(bucket)
        .absent_as(KIND);
        TableMetadataLocation::read(self.send(&call)?.reader())
    }

    /// Commit the table `name`: name `metadata_location` as its current
    /// metadata file, under the `version_token` the last reading answered,
    /// with one `PUT` - and answer the token the next commit is made under.
    ///
    /// The request is sent once: a commit the service may have taken is
    /// never sent again, so a failure that leaves it in doubt is the
    /// caller's to resolve by reading the location back.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`](crate::Error::Parse) for an ARN, a name or
    /// an empty version token the model refuses, before any request, and
    /// the service's refusal otherwise: a token the table has moved past is
    /// its `409`, a table that is not there its `404`.
    pub fn update_table_metadata_location(
        &self,
        bucket: &Arn,
        namespace: &str,
        name: &str,
        version_token: &str,
        metadata_location: &Url,
    ) -> Result<TableVersion> {
        check_version_token(version_token)?;
        let call = Call::put(
            "UpdateTableMetadataLocation",
            format!("{}/metadata-location", table_path(bucket, namespace, name)?),
            addressed(bucket, namespace, name),
            body([
                ("versionToken", Scalar::from(version_token)),
                (
                    "metadataLocation",
                    Scalar::from(metadata_location.to_string()),
                ),
            ])?,
        )
        .in_bucket(bucket);
        TableVersion::read(self.send(&call)?.reader())
    }
}

/// The model's `IcebergMetadata` of a table created with `schema`: the
/// schema as Iceberg expresses it and numbered as the folder catalog numbers
/// a table it creates, then the partition spec and the write order the
/// schema declares, each stated only when it holds a field.
fn iceberg_metadata(schema: &Field) -> Result<Scalar> {
    let mut numbered = schema.clone().into_scheme_compat(&crate::Scheme::ICEBERG)?;
    let start = crate::iceberg::last_column_id(&numbered)?.saturating_add(1);
    crate::iceberg::assign_field_ids(&mut numbered, start)?;
    let mut members = vec![("schemaV2", crate::iceberg::schema_into_json(&numbered)?)];
    let spec = PartitionSpec::from_schema(0, &numbered)?;
    if !spec.is_unpartitioned() {
        members.push(("partitionSpec", spec.into_json()?));
    }
    let order = SortOrder::from_schema(1, &numbered)?;
    if !order.fields.is_empty() {
        members.push(("writeOrder", order.into_json()?));
    }
    Scalar::from_struct(members)
}

/// Refuse `name` unless the model lets a table be called it.
fn check_table(name: &str) -> Result<()> {
    check_name("table name", name, 1, 255, b'_')
}

/// `/tables/{tableBucketARN}/{namespace}/{name}`, each label encoded once.
fn table_path(bucket: &Arn, namespace: &str, name: &str) -> Result<String> {
    check_namespace(namespace)?;
    check_table(name)?;
    Ok(format!(
        "/tables/{}/{}/{}",
        bucket_label(bucket)?,
        label(namespace),
        label(name)
    ))
}

/// What a refusal reports a table as: its table bucket's ARN, then its
/// namespace and its name below it.
fn addressed(bucket: &Arn, namespace: &str, name: &str) -> String {
    format!("{bucket}/{namespace}/{name}")
}

//! Fields borrowed as one protocol.
//!
//! A view is a field plus the protocol it is being read through: one pointer
//! and a [`Scheme`], no duplicated state. Property access delegates to
//! [`ProtocolMetadata`], so lookup, iteration, prefix folding and key assembly
//! keep one implementation, and every write routes through [`Field`]'s own
//! cache-aware mutation.
//!
//! The named pairs are minted from the one protocol list [`Metadata`]'s own
//! snapshot accessors come from, so a protocol added there gains its accessor
//! and its two types in the same change. A named view carries the vocabulary
//! of a *foreign* protocol; state a field owns whatever key it is stored
//! under - [`Field::is_init`], [`Field::is_partition`], `alias`, `comment`,
//! `display`, `location` and `PARQUET:field_id` - stays on [`Field`].

use std::borrow::Cow;
use std::cmp;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, Index};
use std::str::FromStr;

use smol_str::SmolStr;

use crate::Field;
use crate::expression::{Ordering, Projection};
use crate::metadata::{
    HTTP_ACCEPT_ENCODING_KEY, HTTP_ACCEPT_KEY, HTTP_ACCEPT_LANGUAGE_KEY, HTTP_ACCEPT_RANGES_KEY,
    HTTP_CACHE_CONTROL_KEY, HTTP_CONTENT_DISPOSITION_KEY, HTTP_CONTENT_ENCODING_KEY,
    HTTP_CONTENT_LANGUAGE_KEY, HTTP_CONTENT_LENGTH_KEY, HTTP_CONTENT_LOCATION_KEY,
    HTTP_CONTENT_RANGE_KEY, HTTP_CONTENT_TYPE_KEY, HTTP_ETAG_KEY, HTTP_EXPIRES_KEY,
    HTTP_LAST_MODIFIED_KEY, HTTP_LOCATION_KEY, HTTP_RANGE_KEY, HTTP_VARY_KEY, PARTITION_BY_KEY,
    PropertyIter, ProtocolMetadata, SORT_BY_KEY, for_each_well_known_protocol,
    parse_content_length, property_key, property_name, protocol_metadata_prefix,
};
use crate::{Charset, Error, MediaType, Metadata, MimeType, Representation, Result, Scheme, Url};

// ------------------------------------------------------------------------
// The `HTTP:` vocabulary, on the field views that own it.
//
// One type reads each header key and one type writes it. The typed pairs -
// `Content-Length`, the MIME and media projections, and `Location` - parse
// and canonicalize here rather than at every caller.
// ------------------------------------------------------------------------

impl<'field> HttpField<'field> {
    /// Returns the raw HTTP `Accept` field value.
    pub fn accept(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_KEY)
    }

    /// Returns the raw HTTP `Accept-Encoding` field value.
    pub fn accept_encoding(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_ENCODING_KEY)
    }

    /// Returns the raw HTTP `Accept-Language` field value.
    pub fn accept_language(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_LANGUAGE_KEY)
    }

    /// Returns the raw HTTP `Accept-Ranges` field value.
    pub fn accept_ranges(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_RANGES_KEY)
    }

    /// Returns the raw HTTP `Cache-Control` field value.
    pub fn cache_control(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CACHE_CONTROL_KEY)
    }

    /// Returns the raw HTTP `Content-Disposition` field value.
    pub fn content_disposition(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_DISPOSITION_KEY)
    }

    /// Returns the raw HTTP `Content-Encoding` field value.
    pub fn content_encoding(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_ENCODING_KEY)
    }

    /// Returns the raw HTTP `Content-Language` field value.
    pub fn content_language(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_LANGUAGE_KEY)
    }

    /// Parses the canonical HTTP `Content-Length` field value.
    ///
    /// # Errors
    ///
    /// Every write canonicalizes this key, so an error can only originate from
    /// externally corrupted serialized state.
    pub fn content_length(&self) -> Result<Option<u64>> {
        self.as_field()
            .get_metadata(HTTP_CONTENT_LENGTH_KEY)
            .map(parse_content_length)
            .transpose()
    }

    /// Returns the raw HTTP `Content-Location` field value.
    pub fn content_location(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_LOCATION_KEY)
    }

    /// Returns the raw HTTP `Content-Range` field value.
    pub fn content_range(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_RANGE_KEY)
    }

    /// Returns the raw HTTP `Content-Type` field value, including parameters.
    pub fn content_type(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_TYPE_KEY)
    }

    /// Parses the base MIME type from HTTP `Content-Type`.
    ///
    /// Parameters are validated but remain available through
    /// [`Self::content_type`]. A missing header defaults to
    /// `application/octet-stream`.
    ///
    /// # Errors
    ///
    /// Returns an error when a present `Content-Type` is not valid MIME
    /// syntax.
    pub fn mime_type(&self) -> Result<MimeType> {
        self.content_type()
            .map(MimeType::from_content_type)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// Parses HTTP `Content-Type` and `Content-Encoding` as one media value.
    ///
    /// A missing content type defaults to `application/octet-stream`.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed present MIME syntax or an unsupported
    /// content coding.
    pub fn media_type(&self) -> Result<MediaType> {
        MediaType::from_content_headers(self.content_type(), self.content_encoding())
    }

    /// Parses the `charset` parameter of the stored HTTP `Content-Type`.
    ///
    /// Answers `None` when the header declares none, which is the header
    /// saying nothing rather than saying UTF-8.
    ///
    /// # Errors
    ///
    /// Returns an error when the parameter names no known charset.
    pub fn charset(&self) -> Result<Option<Charset>> {
        self.content_type()
            .map_or(Ok(None), Charset::from_content_type)
    }

    /// Returns the raw HTTP `ETag` field value.
    pub fn etag(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ETAG_KEY)
    }

    /// Returns the raw HTTP `Expires` field value.
    pub fn expires(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_EXPIRES_KEY)
    }

    /// Returns the raw HTTP `Last-Modified` field value.
    pub fn last_modified(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_LAST_MODIFIED_KEY)
    }

    /// Parses HTTP `Location` as an absolute URL.
    ///
    /// This is `HTTP:location`, a different key from the namespace-free
    /// [`Field::location`](crate::Field::location) the field carries; the
    /// receiver is what says which one is meant.
    ///
    /// # Errors
    ///
    /// Raw `HTTP:location` metadata may be relative or opaque; such a value is
    /// retained by generic access and reported as an error here.
    pub fn location(&self) -> Result<Option<Url>> {
        self.as_field()
            .get_metadata(HTTP_LOCATION_KEY)
            .map(Url::from_str)
            .transpose()
    }

    /// Returns the raw HTTP `Range` field value.
    pub fn range(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_RANGE_KEY)
    }

    /// Returns the raw HTTP `Vary` field value.
    pub fn vary(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_VARY_KEY)
    }
}

impl HttpFieldMut<'_> {
    /// Sets a validated raw HTTP `Accept` field value.
    ///
    /// # Errors
    ///
    /// Returns an error when the value fails the validation its `HTTP:` key
    /// carries, leaving the field unchanged. Every other raw setter here
    /// fails the same way.
    pub fn set_accept(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_ACCEPT_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept` field value.
    pub fn remove_accept(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Encoding` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_accept_encoding(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_ACCEPT_ENCODING_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Encoding` field value.
    pub fn remove_accept_encoding(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_ENCODING_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Language` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_accept_language(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_ACCEPT_LANGUAGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Language` field value.
    pub fn remove_accept_language(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_LANGUAGE_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Ranges` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_accept_ranges(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_ACCEPT_RANGES_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Ranges` field value.
    pub fn remove_accept_ranges(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_RANGES_KEY)
    }

    /// Sets a validated raw HTTP `Cache-Control` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_cache_control(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CACHE_CONTROL_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Cache-Control` field value.
    pub fn remove_cache_control(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CACHE_CONTROL_KEY)
    }

    /// Sets a validated raw HTTP `Content-Disposition` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_disposition(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_DISPOSITION_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Disposition` field value.
    pub fn remove_content_disposition(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_DISPOSITION_KEY)
    }

    /// Sets a validated raw HTTP `Content-Encoding` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_encoding(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_ENCODING_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Encoding` field value.
    pub fn remove_content_encoding(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_ENCODING_KEY)
    }

    /// Sets a validated raw HTTP `Content-Language` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_language(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_LANGUAGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Language` field value.
    pub fn remove_content_language(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_LANGUAGE_KEY)
    }

    /// Sets canonical HTTP `Content-Length` metadata.
    pub fn set_content_length(&mut self, value: u64) {
        let (_, changed) = self
            .0
            .field
            .metadata_mut()
            .insert_validated(HTTP_CONTENT_LENGTH_KEY.to_owned(), value.to_string());
        if changed {
            self.0.field.invalidate_arrow();
        }
    }

    /// Removes and parses the prior HTTP `Content-Length` value.
    ///
    /// # Errors
    ///
    /// [`HttpField::content_length`] carries when the stored value can fail to
    /// parse; this field is left unchanged when it does.
    pub fn remove_content_length(&mut self) -> Result<Option<u64>> {
        let previous = self.as_protocol().content_length()?;
        if previous.is_some() {
            self.0.field.remove_metadata(HTTP_CONTENT_LENGTH_KEY);
        }
        Ok(previous)
    }

    /// Sets a validated raw HTTP `Content-Location` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_location(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_LOCATION_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Location` field value.
    pub fn remove_content_location(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_LOCATION_KEY)
    }

    /// Sets a validated raw HTTP `Content-Range` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_range(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_RANGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Range` field value.
    pub fn remove_content_range(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_RANGE_KEY)
    }

    /// Sets a validated raw HTTP `Content-Type` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_type(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_CONTENT_TYPE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Type` field value.
    pub fn remove_content_type(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_TYPE_KEY)
    }

    /// Sets the bare HTTP `Content-Type` MIME value and preserves encodings.
    pub fn set_mime_type(&mut self, value: MimeType) {
        let (_, changed) = self
            .0
            .field
            .metadata_mut()
            .insert_validated(HTTP_CONTENT_TYPE_KEY.to_owned(), value.to_string());
        if changed {
            self.0.field.invalidate_arrow();
        }
    }

    /// Removes and parses the prior HTTP `Content-Type` MIME value.
    ///
    /// Existing `Content-Encoding` metadata is deliberately preserved.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored `Content-Type` is not valid MIME
    /// syntax, leaving this field unchanged.
    pub fn remove_mime_type(&mut self) -> Result<Option<MimeType>> {
        let Some(content_type) = self.as_protocol().content_type() else {
            return Ok(None);
        };
        let previous = MimeType::from_content_type(content_type)?;
        self.0.field.remove_metadata(HTTP_CONTENT_TYPE_KEY);
        Ok(Some(previous))
    }

    /// Atomically projects a media value to HTTP content headers.
    ///
    /// # Errors
    ///
    /// File encodings without registered HTTP coding tokens are rejected
    /// before either metadata key or the Arrow projection cache is changed.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        // A declared charset is part of what `Content-Type` says, so it is
        // written back with the base rather than dropped; `Self::media_type`
        // reads it out of the same header.
        let content_type = match value.charset() {
            Some(charset) => format!("{}; {}={charset}", value.base(), Charset::PARAMETER),
            None => value.base().to_string(),
        };
        let mut content_encoding = String::new();
        for encoding in value.encodings() {
            let coding = encoding
                .content_coding()
                .ok_or_else(|| Error::InvalidMetadataValue {
                    key: SmolStr::new_static(HTTP_CONTENT_ENCODING_KEY),
                    reason: SmolStr::new_static(
                        "media encoding has no registered HTTP Content-Encoding token",
                    ),
                })?;
            if !content_encoding.is_empty() {
                content_encoding.push_str(", ");
            }
            content_encoding.push_str(coding);
        }

        let mut metadata = self.0.field.as_metadata().clone();
        metadata.insert_validated(HTTP_CONTENT_TYPE_KEY.to_owned(), content_type);
        if content_encoding.is_empty() {
            metadata.remove(HTTP_CONTENT_ENCODING_KEY);
        } else {
            metadata.insert_validated(HTTP_CONTENT_ENCODING_KEY.to_owned(), content_encoding);
        }
        if metadata != *self.0.field.as_metadata() {
            *self.0.field.metadata_mut() = metadata;
            self.0.field.invalidate_arrow();
        }
        Ok(())
    }

    /// Removes both HTTP media header keys after parsing their prior value.
    ///
    /// # Errors
    ///
    /// If either stored header is malformed, this field remains unchanged.
    pub fn remove_media_type(&mut self) -> Result<Option<MediaType>> {
        let view = self.as_protocol();
        if view.content_type().is_none() && view.content_encoding().is_none() {
            return Ok(None);
        }
        let previous = view.media_type()?;
        let mut metadata = self.0.field.as_metadata().clone();
        metadata.remove(HTTP_CONTENT_TYPE_KEY);
        metadata.remove(HTTP_CONTENT_ENCODING_KEY);
        *self.0.field.metadata_mut() = metadata;
        self.0.field.invalidate_arrow();
        Ok(Some(previous))
    }

    /// Sets a validated raw HTTP `ETag` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_etag(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_ETAG_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `ETag` field value.
    pub fn remove_etag(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ETAG_KEY)
    }

    /// Sets a validated raw HTTP `Expires` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_expires(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_EXPIRES_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Expires` field value.
    pub fn remove_expires(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_EXPIRES_KEY)
    }

    /// Sets a validated raw HTTP `Last-Modified` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_last_modified(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_LAST_MODIFIED_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Last-Modified` field value.
    pub fn remove_last_modified(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_LAST_MODIFIED_KEY)
    }

    /// Sets typed absolute HTTP `Location` metadata.
    pub fn set_location(&mut self, value: Url) {
        let (_, changed) = self
            .0
            .field
            .metadata_mut()
            .insert_validated(HTTP_LOCATION_KEY.to_owned(), value.to_string());
        if changed {
            self.0.field.invalidate_arrow();
        }
    }

    /// Removes and parses the prior typed HTTP `Location` URL.
    ///
    /// # Errors
    ///
    /// [`HttpField::location`] carries when the stored value can fail to
    /// parse; this field is left unchanged when it does.
    pub fn remove_location(&mut self) -> Result<Option<Url>> {
        let previous = self.as_protocol().location()?;
        if previous.is_some() {
            self.0.field.remove_metadata(HTTP_LOCATION_KEY);
        }
        Ok(previous)
    }

    /// Sets a validated raw HTTP `Range` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_range(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_RANGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Range` field value.
    pub fn remove_range(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_RANGE_KEY)
    }

    /// Sets a validated raw HTTP `Vary` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_vary(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_VARY_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Vary` field value.
    pub fn remove_vary(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_VARY_KEY)
    }
}

// ------------------------------------------------------------------------
// The `PARTITION:` and `SORT:` declarations: how a struct's rows partition
// and the order they keep, each one `by` list on the struct field.
//
// `PARTITION:by` holds the projections rows partition by, in order - a bare
// column an identity partition, a term a derived one, an alias naming the
// derived column - and `SORT:by` the `order by` keys rows are sorted by.
// Both are stored as JSON arrays of canonical expression texts, read back
// through the grammar that wrote them, and [`Field::with_partition_by`]
// is what turns the first into the marks and the derived columns a layout
// reads.
// ------------------------------------------------------------------------

/// The property holding the ordered list a `PARTITION:` or a `SORT:`
/// declaration is.
const BY: &str = "by";

impl<'field> PartitionField<'field> {
    /// Parses the projections this struct's rows partition by, in order.
    ///
    /// A bare column - `venue` - is an identity partition on that column; a
    /// term - `years(ts)`, `truncate(name, 4) as prefix` - is a derived
    /// partition, published under its alias or the convention
    /// [`Field::with_partition_by`] names. `None` is a struct declaring
    /// nothing here; [`Field::partition_by`] answers the marked columns
    /// then.
    ///
    /// ```
    /// use yggdryl::{DataType, StructType, TimeUnit, Timezone};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from(StructType::from_fields([
    ///     DataType::utf8().required_field("venue"),
    ///     DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }.required_field("ts"),
    /// ])?)
    /// .required_field("row");
    /// row.as_partition_mut().set_by_texts(["venue", "Years(ts)"])?;
    ///
    /// assert_eq!(row.get_metadata("PARTITION:by"), Some(r#"["venue","years(ts)"]"#));
    /// let declared = row.as_partition().by()?.expect("a declaration");
    /// assert_eq!(declared[1].to_string(), "years(ts)");
    /// assert!(row.as_partition().declares_partition());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming `PARTITION:by` when the stored text is not a
    /// JSON array of terms with optional aliases.
    pub fn by(&self) -> Result<Option<Vec<Projection>>> {
        self.get(BY)
            .map(|stored| {
                crate::metadata::parse_by_list(PARTITION_BY_KEY, stored)?
                    .iter()
                    .map(|entry| crate::metadata::parse_by_projection(PARTITION_BY_KEY, entry))
                    .collect()
            })
            .transpose()
    }

    /// Returns whether this struct declares how its rows partition, well
    /// formed or not.
    ///
    /// Answered on the stored property rather than the parsed list, so a
    /// malformed declaration still reports as one and is refused where it is
    /// read.
    #[must_use]
    pub fn declares_partition(&self) -> bool {
        self.contains_key(BY)
    }
}

impl PartitionFieldMut<'_> {
    /// Records the projections this struct's rows partition by, in order.
    ///
    /// The list is stored canonically: each projection as its own text. The
    /// declaration alone marks nothing; [`Field::with_partition_by`] is what
    /// marks the identity columns and materializes the derived ones.
    ///
    /// # Errors
    ///
    /// Returns an error when an entry is repeated, declares a datatype or
    /// metadata, or the property write fails the validation every metadata
    /// write goes through, leaving the field unchanged.
    pub fn set_by(&mut self, by: impl IntoIterator<Item = Projection>) -> Result<()> {
        self.set_by_texts(by.into_iter().map(|projection| projection.to_string()))
    }

    /// Records the projections this struct's rows partition by, each as the
    /// text the grammar reads: `venue`, `years(ts)`, `truncate(name, 4) as prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when an entry does not parse as a term with an
    /// optional alias, or [`Self::set_by`]'s rule refuses it, leaving the
    /// field unchanged.
    pub fn set_by_texts<I, P>(&mut self, by: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        let entries = by
            .into_iter()
            .map(|entry| {
                crate::metadata::parse_by_projection(PARTITION_BY_KEY, entry.as_ref())
                    .map(|projection| projection.to_string())
            })
            .collect::<Result<Vec<String>>>()?;
        self.insert(
            BY,
            crate::metadata::render_by_list(PARTITION_BY_KEY, entries)?,
        )?;
        Ok(())
    }

    /// Removes the declaration, answering the text it held.
    pub fn remove_by(&mut self) -> Option<String> {
        self.remove(BY)
    }
}

impl<'field> SortField<'field> {
    /// Parses the `order by` keys this struct's rows keep, most significant
    /// first.
    ///
    /// Each entry is one key of the plan grammar - `ts`, `price desc`,
    /// `venue desc nulls first` - which is what
    /// [`Plan::from_field`](crate::expression::Plan::from_field) moves into
    /// its `order by` section and an Iceberg table's default sort order is
    /// read from. `None` is a struct declaring no order.
    ///
    /// ```
    /// use yggdryl::{DataType, SortOptions, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from(StructType::from_fields([
    ///     DataType::utf8().required_field("venue"),
    ///     DataType::Float64.nullable_field("price"),
    /// ])?)
    /// .required_field("row");
    /// row.as_sort_mut().set_by_texts(["venue", "price DESC nulls first"])?;
    ///
    /// assert_eq!(row.get_metadata("SORT:by"), Some(r#"["venue","price desc nulls first"]"#));
    /// let keys = row.as_sort().by()?.expect("an order");
    /// assert_eq!(keys[1].term().to_string(), "price");
    /// assert_eq!(keys[1].options(), SortOptions::descending().with_nulls_first(true));
    /// assert!(row.as_sort().declares_order());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming `SORT:by` when the stored text is not a JSON
    /// array of `order by` keys.
    pub fn by(&self) -> Result<Option<Vec<Ordering>>> {
        self.get(BY)
            .map(|stored| {
                crate::metadata::parse_by_list(SORT_BY_KEY, stored)?
                    .iter()
                    .map(|entry| crate::metadata::parse_by_ordering(SORT_BY_KEY, entry))
                    .collect()
            })
            .transpose()
    }

    /// Returns whether this struct declares an order, well formed or not.
    #[must_use]
    pub fn declares_order(&self) -> bool {
        self.contains_key(BY)
    }
}

impl SortFieldMut<'_> {
    /// Records the `order by` keys this struct's rows keep, most significant
    /// first, each stored as the text the grammar writes.
    ///
    /// # Errors
    ///
    /// Returns an error when a key is repeated, or the property write fails
    /// the validation every metadata write goes through, leaving the field
    /// unchanged.
    pub fn set_by(&mut self, by: impl IntoIterator<Item = Ordering>) -> Result<()> {
        self.set_by_texts(by.into_iter().map(|ordering| ordering.to_string()))
    }

    /// Records the `order by` keys this struct's rows keep, each as the text
    /// the grammar reads: `ts`, `price desc`, `venue desc nulls first`.
    ///
    /// # Errors
    ///
    /// Returns an error when an entry does not parse as an `order by` key,
    /// or [`Self::set_by`]'s rule refuses it, leaving the field unchanged.
    pub fn set_by_texts<I, P>(&mut self, by: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        let entries = by
            .into_iter()
            .map(|entry| {
                crate::metadata::parse_by_ordering(SORT_BY_KEY, entry.as_ref())
                    .map(|ordering| ordering.to_string())
            })
            .collect::<Result<Vec<String>>>()?;
        self.insert(BY, crate::metadata::render_by_list(SORT_BY_KEY, entries)?)?;
        Ok(())
    }

    /// Removes the declaration, answering the text it held.
    pub fn remove_by(&mut self) -> Option<String> {
        self.remove(BY)
    }
}

// ------------------------------------------------------------------------
// The `FIELD:` vocabulary a column states about its own values.
// ------------------------------------------------------------------------

/// The bare name `FIELD:representation` is stored under.
const REPRESENTATION: &str = "representation";

impl FieldPropertiesField<'_> {
    /// What crosses when a same-width integer of the other signedness meets
    /// this integer column: [`Representation::Bits`] where the column states
    /// `FIELD:representation=bits` - its integers are bit patterns, an
    /// XXH3-64 digest a table with no unsigned type stores as the `int64` of
    /// its width - and [`Representation::Value`], the default, where it
    /// states nothing. Read by [`Field::scalar`], the cast and
    /// [`Field::into_scheme_compat`], and kept by an Iceberg table; inert on
    /// a column that is no integer.
    ///
    /// ```
    /// use yggdryl::{DataType, Representation, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut digest = DataType::UInt64.required_field("hashcode");
    /// assert_eq!(digest.as_field_properties().representation(), Representation::Value);
    /// assert!(digest.scalar(Scalar::from(-1_i64)).is_err());
    ///
    /// digest.as_field_properties_mut().set_representation(Representation::Bits)?;
    /// assert_eq!(digest.get_metadata("FIELD:representation"), Some("bits"));
    /// assert_eq!(digest.scalar(Scalar::from(-1_i64))?, Scalar::from(u64::MAX));
    /// // The datatype's own door reads no bits: only a column states them.
    /// assert!(DataType::UInt64.scalar(Scalar::from(-1_i64)).is_err());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn representation(&self) -> Representation {
        if self.get(REPRESENTATION) == Some(Representation::Bits.as_str()) {
            Representation::Bits
        } else {
            Representation::Value
        }
    }
}

impl FieldPropertiesFieldMut<'_> {
    /// States what crosses when a same-width integer of the other signedness
    /// meets this integer column; `Value` removes the declaration, its
    /// absence being the value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidMetadataValue`] naming the key and the
    /// datatype when `Bits` is stated on a column that is no integer,
    /// leaving the field unchanged.
    pub fn set_representation(&mut self, representation: Representation) -> Result<()> {
        if !representation.is_bits() {
            self.remove(REPRESENTATION);
            return Ok(());
        }
        if !self.dtype().is_integer() {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new(self.key(REPRESENTATION)),
                reason: crate::text::expected_got(
                    "bits on an integer column",
                    format_args!("a {} column {:?}", self.dtype(), self.name()),
                ),
            });
        }
        self.insert(REPRESENTATION, representation.as_str())?;
        Ok(())
    }
}

// ------------------------------------------------------------------------
// The `PYTHON:` vocabulary, on the field views that carry it.
//
// A Python runtime declares a schema by writing a class: a dataclass, a
// `TypedDict`, a `NamedTuple`, an enumeration, a `NewType`, a type alias, or
// an ordinary class the annotation named. The three properties here are what
// the field remembers of that declaration - where the class lives, what it is
// called there, and which of those forms it is - so a reader can name the
// class again without the runtime that built it.
//
// [`PythonMetadata`] is the whole declaration as one validated value: it is
// parsed once at the boundary, travels typed, and is written back atomically.
// The property names are private to this module, so a caller writes
// `set_class`, never `"PYTHON:qualname"`.
// ------------------------------------------------------------------------

/// Where the declaring class lives, as a dotted module path.
const MODULE: &str = "module";
/// The full key the module is stored under, spelled once.
pub(crate) const PYTHON_MODULE_KEY: &str = "PYTHON:module";
/// What the class is called inside its module, dots and all.
const QUALNAME: &str = "qualname";
/// The full key the qualified name is stored under.
pub(crate) const PYTHON_QUALNAME_KEY: &str = "PYTHON:qualname";
/// Which Python form the declaration takes.
const KIND: &str = "kind";
/// The full key the form is stored under.
pub(crate) const PYTHON_KIND_KEY: &str = "PYTHON:kind";

/// The segment a qualified name carries for a class declared in a function.
///
/// Python spells it exactly this way and it is not an identifier, so it is the
/// one non-identifier segment a qualified name may hold - and the one thing
/// that makes a declaration unimportable.
const LOCALS: &str = "<locals>";

/// What a module path is, spelled once for every refusal.
const MODULE_SHAPE: &str = "a dotted Python module path";

/// What a qualified name is, spelled once for every refusal.
const QUALNAME_SHAPE: &str = "a dotted Python qualified name";

/// What a form is, spelled once for every refusal.
///
/// Held to [`PythonKind::ALL`] by a test rather than built from it: a refusal
/// is a cold path, and one sentence is cheaper to read than a joined list.
const KIND_SHAPE: &str =
    "one of field, dataclass, typed_dict, named_tuple, enum, newtype, type_alias, class";

/// Python's hard keywords, which no identifier may spell.
///
/// The soft keywords - `match`, `case`, `type`, `_` - are deliberately absent:
/// Python itself accepts them as names, and `keyword.iskeyword` answers false
/// for them, so refusing them here would refuse a class the runtime allows.
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// The Python form a declaration takes.
///
/// The distinction is what a reader needs to rebuild the declaration: a
/// dataclass is constructed by keyword, a named tuple positionally, an
/// enumeration by member, a type alias not at all. [`Self::Field`] is the one
/// form this library mints - a class carrying its own native `Field` - and is
/// what tells a reader the schema is authoritative rather than inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PythonKind {
    /// A class decorated by this library, carrying its own native field.
    Field,
    /// A `dataclasses.dataclass`.
    Dataclass,
    /// A `typing.TypedDict`.
    TypedDict,
    /// A `typing.NamedTuple` or a `collections.namedtuple`.
    NamedTuple,
    /// An `enum.Enum` subclass.
    Enum,
    /// A `typing.NewType`.
    NewType,
    /// A `type` statement or a `typing.TypeAliasType`.
    TypeAlias,
    /// An ordinary class, named by an annotation and nothing more.
    Class,
}

impl PythonKind {
    /// Every form, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Field,
        Self::Dataclass,
        Self::TypedDict,
        Self::NamedTuple,
        Self::Enum,
        Self::NewType,
        Self::TypeAlias,
        Self::Class,
    ];

    /// Returns the canonical stored spelling without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Dataclass => "dataclass",
            Self::TypedDict => "typed_dict",
            Self::NamedTuple => "named_tuple",
            Self::Enum => "enum",
            Self::NewType => "newtype",
            Self::TypeAlias => "type_alias",
            Self::Class => "class",
        }
    }

    /// Parses one stored spelling.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `PYTHON:kind` key when the text is not
    /// one of [`Self::ALL`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Returns whether this form constructs its values by keyword.
    ///
    /// A dataclass and a `TypedDict` are filled by name; a named tuple is
    /// filled by position, and the remaining forms wrap a single value.
    pub const fn is_keyword_constructed(self) -> bool {
        matches!(self, Self::Field | Self::Dataclass | Self::TypedDict)
    }
}

impl FromStr for PythonKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| Error::InvalidMetadataValue {
                key: SmolStr::new_static(PYTHON_KIND_KEY),
                reason: crate::text::expected_got(KIND_SHAPE, format_args!("{value:?}")),
            })
    }
}

impl fmt::Display for PythonKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for PythonKind {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// The Python class a field's `PYTHON:` properties name.
///
/// One value carries the whole declaration, so the three properties are
/// validated together, written together, and never read back half-set. The
/// bare class name is derived from the qualified name rather than stored:
/// Python's `__name__` is always the last segment of its `__qualname__`, and
/// storing both would let a hand-edited record disagree with itself.
///
/// ```
/// use yggdryl::{DataType, PythonKind, PythonMetadata};
///
/// # fn main() -> yggdryl::Result<()> {
/// let quote = PythonMetadata::new("trading.book", "Quote", PythonKind::Dataclass)?;
/// assert_eq!(quote.class_name(), "Quote");
/// assert_eq!(quote.import_path(), "trading.book.Quote");
///
/// let mut field = DataType::Int64.required_field("price");
/// field.as_python_mut().set_class(&quote)?;
///
/// assert_eq!(field.as_python().class()?, Some(quote));
/// assert_eq!(field.get_metadata("PYTHON:qualname"), Some("Quote"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PythonMetadata {
    module: SmolStr,
    qualname: SmolStr,
    kind: PythonKind,
}

impl PythonMetadata {
    /// Validates one Python class declaration.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `PYTHON:module` or `PYTHON:qualname`
    /// key when either is not the dotted name it must be.
    pub fn new(
        module: impl Into<SmolStr>,
        qualname: impl Into<SmolStr>,
        kind: PythonKind,
    ) -> Result<Self> {
        let module = module.into();
        let qualname = qualname.into();
        validate_python_module(&module)?;
        validate_python_qualname(&qualname)?;
        Ok(Self {
            module,
            qualname,
            kind,
        })
    }

    /// Assembles a declaration out of properties a field already stores.
    ///
    /// Every write path canonicalizes `PYTHON:module` and `PYTHON:qualname`
    /// through [`validate_python_module`] and [`validate_python_qualname`], so a stored pair
    /// is proven before it is read. Re-validating here would spend the read on
    /// a question the edge already answered.
    fn from_stored(module: &str, qualname: &str, kind: PythonKind) -> Self {
        Self {
            module: SmolStr::new(module),
            qualname: SmolStr::new(qualname),
            kind,
        }
    }

    /// Returns the dotted module path the class is declared in.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Returns the qualified name the class has inside its module.
    pub fn qualname(&self) -> &str {
        &self.qualname
    }

    /// Returns the bare class name, the last segment of the qualified name.
    pub fn class_name(&self) -> &str {
        self.qualname
            .rsplit_once('.')
            .map_or(self.qualname.as_str(), |(_, name)| name)
    }

    /// Returns which Python form the declaration takes.
    pub const fn kind(&self) -> PythonKind {
        self.kind
    }

    /// Returns the dotted path an importing reader would spell.
    ///
    /// This is `module.qualname` whether or not the class can actually be
    /// reached that way; [`Self::is_importable`] is what answers that.
    pub fn import_path(&self) -> String {
        import_path(&self.module, &self.qualname)
    }

    /// Returns whether [`Self::import_path`] resolves to this class.
    ///
    /// A class declared inside a function body carries `<locals>` in its
    /// qualified name and no import reaches it, so a reader must rebuild it
    /// rather than look it up.
    pub fn is_importable(&self) -> bool {
        !self.qualname.split('.').any(|segment| segment == LOCALS)
    }

    /// Returns a deterministic cross-language hash of the whole declaration.
    ///
    /// The form is hashed with the path, not beside it: two declarations of one
    /// class that disagree on what it is are different values, and
    /// [`Self::import_path`] alone cannot tell them apart.
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_display(&Rendered(self))
    }

    /// Returns the three properties in their stored order.
    ///
    /// This is what a protocol write overlays, and what a caller building a
    /// whole metadata snapshot at once inserts.
    pub fn properties(&self) -> [(&'static str, &str); 3] {
        [
            (PYTHON_KIND_KEY, self.kind.as_str()),
            (PYTHON_MODULE_KEY, self.module.as_str()),
            (PYTHON_QUALNAME_KEY, self.qualname.as_str()),
        ]
    }
}

impl fmt::Display for PythonMetadata {
    /// Renders the dotted path an importing reader would spell.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.module, self.qualname)
    }
}

/// A declaration rendered with its form, for [`PythonMetadata::stable_hash`].
struct Rendered<'value>(&'value PythonMetadata);

impl fmt::Display for Rendered<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.0.kind, self.0)
    }
}

impl<'field> PythonField<'field> {
    /// Returns the dotted module path the declaring class lives in.
    pub fn module(&self) -> Option<&'field str> {
        self.get(MODULE)
    }

    /// Returns the qualified name the declaring class has in its module.
    pub fn qualname(&self) -> Option<&'field str> {
        self.get(QUALNAME)
    }

    /// Returns the bare class name, the last segment of the qualified name.
    ///
    /// Derived on every read and never stored, so it cannot disagree with
    /// [`Self::qualname`].
    pub fn class_name(&self) -> Option<&'field str> {
        self.qualname()
            .map(|qualname| qualname.rsplit_once('.').map_or(qualname, |(_, name)| name))
    }

    /// Parses which Python form the declaration takes.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `PYTHON:kind` key when the stored text
    /// is not one of [`PythonKind::ALL`]: every write canonicalizes it, so
    /// this can only come from externally edited state.
    pub fn kind(&self) -> Result<Option<PythonKind>> {
        self.get(KIND).map(PythonKind::from_str).transpose()
    }

    /// Builds the whole declaration, absent unless all three parts are stored.
    ///
    /// A field carrying only some of them has no class to name, so this
    /// answers `None` rather than a value with a part invented for it; the
    /// three accessors above are what read a partial declaration.
    ///
    /// # Errors
    ///
    /// Returns [`Self::kind`]'s failure. The two names are not re-checked:
    /// every write path validated them, so the read spends nothing on them.
    pub fn class(&self) -> Result<Option<PythonMetadata>> {
        let (Some(module), Some(qualname), Some(kind)) =
            (self.module(), self.qualname(), self.kind()?)
        else {
            return Ok(None);
        };
        Ok(Some(PythonMetadata::from_stored(module, qualname, kind)))
    }

    /// Returns the dotted path an importing reader would spell.
    ///
    /// Absent exactly when either half of it is.
    pub fn import_path(&self) -> Option<String> {
        Some(import_path(self.module()?, self.qualname()?))
    }
}

impl PythonFieldMut<'_> {
    /// Records the whole declaration, replacing every part of a prior one.
    ///
    /// The three properties are overlaid in one validated write, so a refusal
    /// leaves the field exactly as it was rather than half-moved to a new
    /// class.
    ///
    /// # Errors
    ///
    /// Returns the property write's refusal, leaving the field unchanged.
    pub fn set_class(&mut self, value: &PythonMetadata) -> Result<()> {
        self.update([
            (KIND, value.kind.as_str()),
            (MODULE, value.module.as_str()),
            (QUALNAME, value.qualname.as_str()),
        ])
    }

    /// Records the dotted module path the declaring class lives in.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `PYTHON:module` key when the text is
    /// not a dotted module path, leaving the field unchanged.
    pub fn set_module(&mut self, value: &str) -> Result<()> {
        self.insert(MODULE, value)?;
        Ok(())
    }

    /// Records the qualified name the declaring class has in its module.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `PYTHON:qualname` key when the text is
    /// not a dotted qualified name, leaving the field unchanged.
    pub fn set_qualname(&mut self, value: &str) -> Result<()> {
        self.insert(QUALNAME, value)?;
        Ok(())
    }

    /// Records which Python form the declaration takes.
    ///
    /// # Errors
    ///
    /// Returns the property write's refusal, leaving the field unchanged.
    pub fn set_kind(&mut self, value: PythonKind) -> Result<()> {
        self.insert(KIND, value.as_str())?;
        Ok(())
    }

    /// Removes the whole declaration and answers what stood there.
    ///
    /// The prior value is answered only when all three parts were present and
    /// valid, which is the same completeness [`PythonField::class`] reports;
    /// every part is removed either way.
    pub fn remove_class(&mut self) -> Option<PythonMetadata> {
        let prior = self.as_protocol().class().ok().flatten();
        self.remove(KIND);
        self.remove(MODULE);
        self.remove(QUALNAME);
        prior
    }
}

/// Join a module and a qualified name into the path an import would spell.
///
/// Sized once rather than grown, so the one value a read hands back costs the
/// one allocation it is.
fn import_path(module: &str, qualname: &str) -> String {
    let mut path = String::with_capacity(module.len() + 1 + qualname.len());
    path.push_str(module);
    path.push('.');
    path.push_str(qualname);
    path
}

/// Return whether one segment is a Python identifier this crate will store.
///
/// Python identifiers are Unicode, and reproducing `XID_Start`/`XID_Continue`
/// here would pin this crate to one Unicode revision to refuse names the
/// runtime accepts. The structural rules are what a stored name is held to
/// instead - non-empty, not digit-led, no separator, no ASCII punctuation
/// beyond `_`, no whitespace or control - so every valid identifier passes and
/// the shapes that would break a dotted path do not.
fn is_identifier(segment: &str) -> bool {
    let Some(first) = segment.chars().next() else {
        return false;
    };
    if first.is_ascii_digit() || KEYWORDS.contains(&segment) {
        return false;
    }
    segment
        .chars()
        .all(|character| character == '_' || !is_refused_in_identifier(character))
}

/// Return whether one character can never appear in a stored identifier.
fn is_refused_in_identifier(character: char) -> bool {
    character.is_ascii_punctuation()
        || character.is_whitespace()
        || character.is_control()
        || !character.is_ascii() && !char::is_alphanumeric(character)
}

/// Validate a dotted module path.
///
/// # Errors
///
/// Returns an error naming the full `PYTHON:module` key.
pub(crate) fn validate_python_module(value: &str) -> Result<()> {
    if !value.is_empty() && value.split('.').all(is_identifier) {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new_static(PYTHON_MODULE_KEY),
        reason: crate::text::expected_got(
            MODULE_SHAPE,
            format_args!("{:?}", crate::text::elide_to(value, 256)),
        ),
    })
}

/// Validate a dotted qualified name.
///
/// `<locals>` is the one non-identifier segment Python itself writes, for a
/// class declared inside a function body, so it is the one this accepts.
///
/// # Errors
///
/// Returns an error naming the full `PYTHON:qualname` key.
pub(crate) fn validate_python_qualname(value: &str) -> Result<()> {
    if !value.is_empty()
        && value
            .split('.')
            .all(|segment| segment == LOCALS || is_identifier(segment))
    {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new_static(PYTHON_QUALNAME_KEY),
        reason: crate::text::expected_got(
            QUALNAME_SHAPE,
            format_args!("{:?}", crate::text::elide_to(value, 256)),
        ),
    })
}

/// Canonicalize a stored form through the one parse that names them.
///
/// # Errors
///
/// [`PythonKind::from_str`] carries the rule.
pub(crate) fn canonicalize_python_kind(value: &str) -> Result<String> {
    PythonKind::from_str(value).map(|kind| kind.as_str().to_owned())
}

/// A field borrowed as one protocol: its properties by bare name, and the
/// field itself.
///
/// A protocol property is stored under a `scheme:name` key, and code that
/// spells that key by hand has to spell it right in every branch it appears
/// in. This view remembers the protocol once, so a caller writes `doc` where
/// it used to write `"ICEBERG:doc"`. Constructing one costs a `Scheme` clone
/// of a known protocol - which allocates nothing - and no map walk, so it is
/// built per call rather than stored.
///
/// `Deref<Target = Field>` puts the whole field surface on the view, but
/// `deref` borrows the *view*, so `field.as_iceberg().name()` is E0716 on a
/// temporary. [`Self::as_field`] answers the field with the view's own
/// lifetime and is the spelling that outlives it; every property read already
/// returns that lifetime and needs no hop.
///
/// Four names shadow [`Field`]'s through that deref, each deliberately:
/// [`Self::comment`] and [`Self::display`] are the protocol forms and strict
/// supersets of [`Field::comment`] and [`Field::display`]; [`Self::merge_with`]
/// takes one argument where [`Field::merge_with`] takes two, so a wrong pick is
/// a compile error rather than a silent one; and `HttpField::location` reads
/// `HTTP:location` where [`Field::location`] reads the namespace-free
/// `location`.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut field = DataType::Int64.required_field("price");
/// field.as_iceberg_mut().insert("doc", "closing price")?;
///
/// // The value outlives the view it was read through.
/// let doc = field.as_iceberg().get("doc");
/// assert_eq!(doc, Some("closing price"));
///
/// assert_eq!(field.as_iceberg().as_field().name(), "price");
/// assert_eq!(&field.as_iceberg()["doc"], "closing price");
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct ProtocolField<'field> {
    field: &'field Field,
    scheme: Scheme,
}

impl<'field> ProtocolField<'field> {
    /// Borrows one protocol's properties on a field.
    pub(crate) fn new(field: &'field Field, scheme: Scheme) -> Self {
        Self { field, scheme }
    }

    /// Borrows the whole field this view reads, with the view's lifetime.
    pub const fn as_field(&self) -> &'field Field {
        self.field
    }

    /// Borrows this protocol's properties on the field's metadata snapshot.
    pub fn as_properties(&self) -> ProtocolMetadata<'field> {
        // Reading the borrow out through a binding copies it; calling on the
        // place `self.field` would reborrow it for `&self` and collapse every
        // `'field` return to the view's own lifetime.
        let field: &'field Field = self.field;
        field.as_metadata().protocol(&self.scheme)
    }

    /// Returns the protocol this view remembers.
    pub const fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Returns the canonical key prefix this view applies.
    ///
    /// [`ProtocolMetadata::prefix`] carries the spelling.
    pub fn prefix(&self) -> Cow<'_, str> {
        protocol_metadata_prefix(&self.scheme)
    }

    /// Returns the full metadata key one property name is stored under.
    pub fn key(&self, name: &str) -> String {
        property_key(&self.scheme, name)
    }

    /// Returns one property value by its bare name.
    pub fn get(&self, name: &str) -> Option<&'field str> {
        self.as_properties().get(name)
    }

    /// Returns whether one property exists.
    pub fn contains_key(&self, name: &str) -> bool {
        self.field.has_property(&self.scheme, name)
    }

    /// Returns the number of properties this protocol holds.
    ///
    /// [`ProtocolMetadata::len`] carries the cost.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Returns whether this protocol holds no properties.
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Iterates this protocol's names and values in lexical order.
    pub fn iter(&self) -> PropertyIter<'field, '_> {
        // Not routed through `as_properties`: the iterator's second lifetime
        // is cut from the `Scheme` it is built over, so a temporary view would
        // hand back an iterator borrowing an already-dead one. This reaches
        // the same single implementation one link further down.
        let field: &'field Field = self.field;
        field.property_iter(&self.scheme)
    }

    /// Returns the first property after `after_name`, or the first for `None`.
    ///
    /// This is the cursor form an owning FFI iterator advances with.
    pub fn next_entry(&self, after_name: Option<&str>) -> Option<(&'field str, &'field str)> {
        self.as_properties().next_entry(after_name)
    }

    /// Returns this protocol's comment, falling back to the straight one.
    ///
    /// [`ProtocolMetadata::comment`] carries the rule.
    pub fn comment(&self) -> Option<&'field str> {
        self.as_properties().comment()
    }

    /// Returns this protocol's display name, falling back to the straight one.
    ///
    /// [`ProtocolMetadata::display`] carries the rule.
    pub fn display(&self) -> Option<&'field str> {
        self.as_properties().display()
    }

    /// Collects this protocol's properties as a standalone snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error only when a property fails the validation it already
    /// passed, which externally corrupted serialized state can produce.
    pub fn into_metadata(self) -> Result<Metadata> {
        self.as_properties().into_metadata()
    }

    /// Returns this protocol's properties merged with another view's.
    ///
    /// [`ProtocolMetadata::merge_with`] carries the direction and the keying.
    ///
    /// # Errors
    ///
    /// Returns an error when a merged property fails the validation every
    /// write goes through.
    pub fn merge_with(&self, other: &ProtocolField<'_>) -> Result<Metadata> {
        self.as_properties().merge_with(&other.as_properties())
    }
}

impl Deref for ProtocolField<'_> {
    type Target = Field;

    fn deref(&self) -> &Field {
        self.field
    }
}

impl AsRef<Field> for ProtocolField<'_> {
    fn as_ref(&self) -> &Field {
        self.field
    }
}

/// Subscripting a protocol view by name reaches one property value.
///
/// The concrete impl is what keeps the operator on properties: [`Field`]'s own
/// `Index<&str>` answers a child field, and the operator autoderefs, so
/// omitting this would silently descend the schema instead.
///
/// # Panics
///
/// Panics when this protocol carries no property of that name.
impl Index<&str> for ProtocolField<'_> {
    type Output = str;

    fn index(&self, name: &str) -> &Self::Output {
        self.get(name)
            .unwrap_or_else(|| missing_property(&self.key(name)))
    }
}

impl fmt::Debug for ProtocolField<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProtocolField")
            .field(&self.prefix())
            .field(&format_args!("{self}"))
            .finish()
    }
}

impl fmt::Display for ProtocolField<'_> {
    /// Renders this protocol's own names as a deterministic JSON object.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_properties(), formatter)
    }
}

impl PartialEq for ProtocolField<'_> {
    /// Compares the properties two views expose, not the fields behind them.
    fn eq(&self, other: &Self) -> bool {
        self.as_properties() == other.as_properties()
    }
}

impl Eq for ProtocolField<'_> {}

impl PartialOrd for ProtocolField<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProtocolField<'_> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.as_properties().cmp(&other.as_properties())
    }
}

impl Hash for ProtocolField<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_properties().hash(state);
    }
}

impl<'view, 'field> IntoIterator for &'view ProtocolField<'field> {
    type Item = (&'field str, &'field str);
    type IntoIter = PropertyIter<'field, 'view>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// A field mutably borrowed as one protocol.
///
/// This is [`ProtocolField`] with the field's mutations added. It is a
/// separate value for the reason every mutable view in Rust is: it borrows the
/// field exclusively, so a read-only view can be handed out freely while this
/// one exists only where a change is actually being made.
///
/// Every write routes through [`Field`]'s own cache-aware mutation, so a
/// protocol write invalidates a populated Arrow projection exactly as a direct
/// metadata write does, and a rejected value leaves the field untouched.
///
/// There is deliberately no `DerefMut<Target = Field>` and no `as_field_mut`:
/// either would put the field's whole mutator surface back on a view named for
/// a foreign protocol, which is the ambiguity these views exist to remove.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut field = DataType::Int64.required_field("price");
///
/// field.as_iceberg_mut().insert("doc", "closing price")?;
/// field.as_iceberg_mut().insert("schema-id", "3")?;
///
/// assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
/// assert_eq!(field.get_metadata("ICEBERG:doc"), Some("closing price"));
///
/// assert_eq!(
///     field.as_iceberg_mut().remove("doc").as_deref(),
///     Some("closing price"),
/// );
/// field.as_iceberg_mut().clear();
/// assert!(field.as_iceberg().is_empty());
/// # Ok(())
/// # }
/// ```
pub struct ProtocolFieldMut<'field> {
    field: &'field mut Field,
    scheme: Scheme,
}

impl<'field> ProtocolFieldMut<'field> {
    /// Borrows one protocol's properties on a field for reading and writing.
    pub(crate) fn new(field: &'field mut Field, scheme: Scheme) -> Self {
        Self { field, scheme }
    }

    /// Borrows the whole field this view writes.
    pub const fn as_field(&self) -> &Field {
        self.field
    }

    /// Borrows the whole field this view writes, mutably.
    ///
    /// A protocol's own properties go through [`Self::set`], which spells the
    /// prefix; this is for the generic state a field owns whatever protocol
    /// is looking at it - its description, its display name - which a
    /// protocol view fills but does not own.
    pub const fn as_field_mut(&mut self) -> &mut Field {
        self.field
    }

    /// Borrows the read-only view of the same protocol.
    pub fn as_protocol(&self) -> ProtocolField<'_> {
        ProtocolField::new(self.field, self.scheme.clone())
    }

    /// Returns the protocol this view remembers.
    pub const fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Returns the canonical key prefix this view applies.
    ///
    /// [`ProtocolMetadata::prefix`] carries the spelling.
    pub fn prefix(&self) -> Cow<'_, str> {
        protocol_metadata_prefix(&self.scheme)
    }

    /// Returns the full metadata key one property name is stored under.
    pub fn key(&self, name: &str) -> String {
        property_key(&self.scheme, name)
    }

    /// Returns one property value by its bare name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.field.get_property(&self.scheme, name)
    }

    /// Returns whether one property exists.
    pub fn contains_key(&self, name: &str) -> bool {
        self.field.has_property(&self.scheme, name)
    }

    /// Returns the number of properties this protocol holds.
    ///
    /// [`ProtocolMetadata::len`] carries the cost.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Returns whether this protocol holds no properties.
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Iterates this protocol's names and values in lexical order.
    pub fn iter(&self) -> PropertyIter<'_, '_> {
        self.field.property_iter(&self.scheme)
    }

    /// Returns the first property after `after_name`, or the first for `None`.
    ///
    /// [`ProtocolField::next_entry`] carries what the cursor form is for.
    pub fn next_entry(&self, after_name: Option<&str>) -> Option<(&str, &str)> {
        self.field.next_property_entry(&self.scheme, after_name)
    }

    /// Returns this protocol's comment, falling back to the straight one.
    ///
    /// [`ProtocolMetadata::comment`] carries the rule.
    pub fn comment(&self) -> Option<&str> {
        self.as_protocol().comment()
    }

    /// Returns this protocol's display name, falling back to the straight one.
    ///
    /// [`ProtocolMetadata::display`] carries the rule.
    pub fn display(&self) -> Option<&str> {
        self.as_protocol().display()
    }

    /// Inserts or replaces one property and returns its prior value.
    ///
    /// # Errors
    ///
    /// Returns an error when the name or value fails the validation the
    /// protocol namespace applies, leaving the field unchanged.
    pub fn insert(&mut self, name: &str, value: impl Into<String>) -> Result<Option<String>> {
        self.field.set_property(&self.scheme, name, value)
    }

    /// Overlays several properties, keeping the ones not named.
    ///
    /// The whole overlay is validated before any of it is applied, so a
    /// rejected entry leaves every other entry unwritten too.
    ///
    /// # Errors
    ///
    /// Returns an error when any name or value fails validation.
    pub fn update<I, N, V>(&mut self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = (N, V)>,
        N: AsRef<str>,
        V: Into<String>,
    {
        let overlay: Vec<(String, String)> = entries
            .into_iter()
            .map(|(name, value)| (self.key(name.as_ref()), value.into()))
            .collect();
        self.field.update_metadata(overlay)
    }

    /// Replaces this protocol's properties with exactly these, atomically.
    ///
    /// Properties of other protocols and every shared key are untouched, which
    /// is what makes this a protocol-scoped `set` rather than a metadata one.
    ///
    /// # Errors
    ///
    /// Returns an error when any name or value fails validation, leaving the
    /// field unchanged.
    pub fn set<I, N, V>(&mut self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = (N, V)>,
        N: AsRef<str>,
        V: Into<String>,
    {
        let prefix = protocol_metadata_prefix(&self.scheme);
        let mut replacement: Vec<(String, String)> = self
            .field
            .metadata_iter()
            .filter(|(key, _)| property_name(key, &prefix).is_none())
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        for (name, value) in entries {
            replacement.push((self.key(name.as_ref()), value.into()));
        }
        self.field.set_metadata(replacement)
    }

    /// Merges another protocol view's properties into this one, in place.
    ///
    /// A name this field already carries keeps its value, so the merge only
    /// ever adds - the same direction [`Metadata::merge_with`] resolves in,
    /// seen from the receiving side. Properties of other protocols are
    /// untouched.
    ///
    /// # Errors
    ///
    /// Returns an error when a merged property fails validation, leaving the
    /// field unchanged.
    pub fn merge_with(&mut self, other: &ProtocolField<'_>) -> Result<()> {
        let held: Vec<String> = self.iter().map(|(name, _)| name.to_owned()).collect();
        let additions: Vec<(String, String)> = other
            .iter()
            .filter(|(name, _)| !held.iter().any(|kept| kept == name))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        self.update(additions)
    }

    /// Removes one property and returns its prior value.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        self.field.remove_property(&self.scheme, name)
    }

    /// Removes every property of this protocol.
    pub fn clear(&mut self) {
        self.field.clear_properties(&self.scheme);
    }
}

impl Deref for ProtocolFieldMut<'_> {
    type Target = Field;

    fn deref(&self) -> &Field {
        self.field
    }
}

impl AsRef<Field> for ProtocolFieldMut<'_> {
    fn as_ref(&self) -> &Field {
        self.field
    }
}

/// Subscripting a mutable protocol view by name reaches one property value.
///
/// [`Index<&str>`] on [`ProtocolField`] carries why the impl is mandatory.
///
/// # Panics
///
/// Panics when this protocol carries no property of that name.
impl Index<&str> for ProtocolFieldMut<'_> {
    type Output = str;

    fn index(&self, name: &str) -> &Self::Output {
        self.get(name)
            .unwrap_or_else(|| missing_property(&self.key(name)))
    }
}

/// Report the full key a subscript found nothing under.
#[cold]
fn missing_property(key: &str) -> ! {
    panic!("metadata property {key:?} is not present")
}

impl fmt::Debug for ProtocolFieldMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProtocolFieldMut")
            .field(&self.prefix())
            .field(&format_args!("{}", self.as_protocol()))
            .finish()
    }
}

impl fmt::Display for ProtocolFieldMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_protocol(), formatter)
    }
}

/// Mints one protocol's named borrowed and mutable field views: the builder
/// a crate invokes for its own scheme - the FIX view's in `fix/field.rs`,
/// with public constructors - beside the core's own views, which the
/// well-known list drives with crate-private ones.
///
/// `$vis` is the constructors' visibility, `$scheme` the [`Scheme`] the
/// views read, `$view` and `$view_mut` the two types it declares and
/// `$label` the protocol's name in their docs. The expansion names only
/// public doors - [`Field::protocol`], [`Field::protocol_mut`] and the
/// views' own `as_field` - so it compiles in any crate depending on this
/// one, and the docs it writes link nothing, since a link would resolve
/// where the views are declared.
#[macro_export]
#[doc(hidden)]
macro_rules! protocol_field_types {
    ($vis:vis, $scheme:expr, $view:ident, $view_mut:ident, $label:expr) => {
        #[doc = concat!("A field borrowed as its ", $label, " protocol.")]
        ///
        /// Dereferences to `ProtocolField` for the property surface and
        /// through it to `Field` for the field surface.
        #[derive(Clone)]
        pub struct $view<'field>($crate::ProtocolField<'field>);

        impl<'field> $view<'field> {
            #[doc = concat!("Borrows a field as its ", $label, " protocol.")]
            $vis fn new(field: &'field $crate::Field) -> Self {
                Self(field.protocol(&$scheme))
            }
        }

        impl<'field> ::core::ops::Deref for $view<'field> {
            type Target = $crate::ProtocolField<'field>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl ::core::convert::AsRef<$crate::Field> for $view<'_> {
            fn as_ref(&self) -> &$crate::Field {
                self.0.as_field()
            }
        }

        impl ::core::fmt::Debug for $view<'_> {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter
                    .debug_tuple(stringify!($view))
                    .field(&format_args!("{}", self.0))
                    .finish()
            }
        }

        impl ::core::fmt::Display for $view<'_> {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, formatter)
            }
        }

        impl ::core::cmp::PartialEq for $view<'_> {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl ::core::cmp::Eq for $view<'_> {}

        impl ::core::cmp::PartialOrd for $view<'_> {
            fn partial_cmp(&self, other: &Self) -> Option<::core::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl ::core::cmp::Ord for $view<'_> {
            fn cmp(&self, other: &Self) -> ::core::cmp::Ordering {
                self.0.cmp(&other.0)
            }
        }

        impl ::core::hash::Hash for $view<'_> {
            fn hash<H: ::core::hash::Hasher>(&self, state: &mut H) {
                ::core::hash::Hash::hash(&self.0, state);
            }
        }

        // `for` selects on the exact type and never derefs, so the base
        // impl alone would not put a named view in a loop.
        impl<'view, 'field> ::core::iter::IntoIterator for &'view $view<'field> {
            type Item = (&'field str, &'field str);
            type IntoIter = $crate::PropertyIter<'field, 'view>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.iter()
            }
        }

        #[doc = concat!("A field mutably borrowed as its ", $label, " protocol.")]
        ///
        /// Dereferences to `ProtocolFieldMut`, which is what carries the
        /// write surface; there is no path from here to `Field`'s own
        /// mutators.
        pub struct $view_mut<'field>($crate::ProtocolFieldMut<'field>);

        impl<'field> $view_mut<'field> {
            #[doc = concat!("Borrows a field mutably as its ", $label, " protocol.")]
            $vis fn new(field: &'field mut $crate::Field) -> Self {
                Self(field.protocol_mut(&$scheme))
            }

            #[doc = concat!("Borrows the read-only ", $label, " view of the same field.")]
            ///
            /// This refines `ProtocolFieldMut::as_protocol` to the named
            /// type, which is what lets a typed remover read its own typed
            /// prior value.
            pub fn as_protocol(&self) -> $view<'_> {
                $view::new(self.0.as_field())
            }
        }

        impl<'field> ::core::ops::Deref for $view_mut<'field> {
            type Target = $crate::ProtocolFieldMut<'field>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<'field> ::core::ops::DerefMut for $view_mut<'field> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl ::core::convert::AsRef<$crate::Field> for $view_mut<'_> {
            fn as_ref(&self) -> &$crate::Field {
                self.0.as_field()
            }
        }

        impl ::core::fmt::Debug for $view_mut<'_> {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter
                    .debug_tuple(stringify!($view_mut))
                    .field(&format_args!("{}", self.0))
                    .finish()
            }
        }

        impl ::core::fmt::Display for $view_mut<'_> {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, formatter)
            }
        }
    };
}

/// The core's own views: every entry of the well-known list through the
/// builder, their constructors crate-private - a caller reaches them through
/// the named accessors the same list puts on [`Field`].
macro_rules! core_protocol_field_types {
    ($name:ident, $mutable:ident, $constant:ident, $view:ident, $view_mut:ident, $label:expr) => {
        protocol_field_types!(pub(crate), Scheme::$constant, $view, $view_mut, $label);
    };
}

for_each_well_known_protocol!(core_protocol_field_types);

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/protocol.rs` pins and a caller cannot reach.
    //!
    //! The three keys a Python declaration is stored under are crate-private:
    //! a caller reads them back only through the borrowed view, so a pin on
    //! the literal key has to name them. The two validators are the same
    //! story. `for_each_well_known_protocol!` is a crate-private macro - the
    //! one place the well-known protocols are named - and a macro cannot be
    //! re-exported outside the crate at all, so the walk over that list lives
    //! here and answers what it measured; every assertion stays in the test.

    use crate::{DataType, Result, Scheme};

    /// The metadata key a declaration's module is stored under.
    pub const PYTHON_MODULE_KEY: &str = super::PYTHON_MODULE_KEY;
    /// The metadata key a declaration's qualified name is stored under.
    pub const PYTHON_QUALNAME_KEY: &str = super::PYTHON_QUALNAME_KEY;
    /// The metadata key a declaration's stored form is stored under.
    pub const PYTHON_KIND_KEY: &str = super::PYTHON_KIND_KEY;
    /// The refusal sentence that names every stored form.
    pub const KIND_SHAPE: &str = super::KIND_SHAPE;

    /// Whether `value` is a name Python could have written as a module.
    pub fn validate_python_module(value: &str) -> Result<()> {
        super::validate_python_module(value)
    }

    /// Whether `value` is a name Python could have written as a qualname.
    pub fn validate_python_qualname(value: &str) -> Result<()> {
        super::validate_python_qualname(value)
    }

    /// What one well-known protocol's pair of views answered on a probe
    /// field, after the mutable view stored `x` as `1`.
    pub struct ProtocolProbe {
        /// The prefix this entry's own [`Scheme`] constant spells.
        pub scheme_prefix: String,
        /// What the field holds under `<scheme_prefix>:x`.
        pub stored: Option<String>,
        /// The prefix the borrowed view answers.
        pub view_prefix: String,
        /// The prefix the mutable view answers.
        pub view_mut_prefix: String,
        /// The key the borrowed view spells for `x`.
        pub key: String,
        /// What iterating the borrowed view yielded.
        pub entries: Vec<(String, String)>,
    }

    /// Probe every well-known protocol through its own pair of accessors.
    ///
    /// One entry per protocol, in the order the crate-private list names
    /// them, so a protocol added to that list arrives here on its own.
    #[must_use]
    pub fn well_known_protocol_probes() -> Vec<ProtocolProbe> {
        let mut probes = Vec::new();
        macro_rules! probe_protocol {
            ($name:ident, $mutable:ident, $constant:ident, $view:ident, $view_mut:ident, $label:literal) => {
                let mut field = DataType::Int64.required_field("probe");
                let scheme_prefix = Scheme::$constant.metadata_prefix().into_owned();
                field
                    .$mutable()
                    .insert("x", "1")
                    .expect("a well-known protocol accepts its own property");
                let key = format!("{scheme_prefix}:x");
                let stored = field.get_metadata(&key).map(str::to_owned);
                let view_mut_prefix = field.$mutable().prefix().into_owned();
                let view = field.$name();
                probes.push(ProtocolProbe {
                    view_prefix: view.prefix().into_owned(),
                    key: view.key("x").to_string(),
                    entries: (&view)
                        .into_iter()
                        .map(|(name, value)| (name.to_owned(), value.to_owned()))
                        .collect(),
                    scheme_prefix,
                    stored,
                    view_mut_prefix,
                });
            };
        }

        crate::metadata::for_each_well_known_protocol!(probe_protocol);
        probes
    }
}

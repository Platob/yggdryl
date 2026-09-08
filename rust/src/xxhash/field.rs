//! Digest roles and effective component selection on fields.

use std::slice;

use smol_str::{SmolStr, format_smolstr};

use crate::metadata::{parse_source_list, render_source_list};
use crate::types::protocol::{DigestField, DigestFieldMut};
use crate::{DataType, DigestAlgorithm, Error, Field, Result};

use crate::txhash::{TIME, UNIT};

const ALGORITHM: &str = "algorithm";
const ROLE: &str = "role";
const SOURCES: &str = "sources";
pub(crate) const DIGEST_ALGORITHM_KEY: &str = "digest:algorithm";
pub(crate) const DIGEST_ROLE_KEY: &str = "digest:role";
pub(crate) const DIGEST_ROLE_HOLDER: &str = "holder";
pub(crate) const DIGEST_SOURCES_KEY: &str = "digest:sources";

/// Return whether a holder's storage carries this algorithm's exact width.
///
/// A holder coupling an instant stores that instant in front of the digest,
/// so its width is the coupled one; `txhash` owns that layout.
pub(crate) fn holder_accepts(field: &Field, algorithm: DigestAlgorithm) -> bool {
    if field.as_digest().time().is_some() {
        return crate::txhash::coupled_holder_accepts(field, algorithm);
    }
    match algorithm {
        DigestAlgorithm::Xxh32 => {
            matches!(field.dtype(), DataType::Int32 | DataType::UInt32)
        }
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => {
            matches!(field.dtype(), DataType::Int64 | DataType::UInt64)
        }
        DigestAlgorithm::Xxh128 => matches!(field.dtype(), DataType::FixedSizeBinary(16)),
    }
}

/// Return the canonical datatype spellings an algorithm's holder accepts.
pub(crate) fn expected_holder_dtypes(field: &Field, algorithm: DigestAlgorithm) -> String {
    if field.as_digest().time().is_some() {
        return crate::txhash::expected_coupled_dtype(algorithm);
    }
    match algorithm {
        DigestAlgorithm::Xxh32 => "int32 or uint32",
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => "int64 or uint64",
        DigestAlgorithm::Xxh128 => "fixed_size_binary[16]",
    }
    .to_owned()
}

/// Parse a stored digest algorithm and return its canonical token.
pub(crate) fn canonicalize_digest_algorithm(value: &str) -> Result<String> {
    parse_digest_algorithm(value).map(|algorithm| algorithm.as_str().to_owned())
}

fn parse_digest_algorithm(value: &str) -> Result<DigestAlgorithm> {
    DigestAlgorithm::from_str(value).map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new_static(DIGEST_ALGORITHM_KEY),
        reason: SmolStr::new(error.to_string()),
    })
}

impl DigestField<'_> {
    /// Returns whether this field holds a digest rather than contributing to it.
    pub fn is_holder(&self) -> bool {
        self.get(ROLE) == Some(DIGEST_ROLE_HOLDER)
    }

    /// Parses the algorithm this holder declares.
    ///
    /// # Errors
    ///
    /// Returns an error naming `digest:algorithm` when externally supplied
    /// metadata is not a [`DigestAlgorithm`] token.
    pub fn algorithm(&self) -> Result<Option<DigestAlgorithm>> {
        self.get(ALGORITHM).map(parse_digest_algorithm).transpose()
    }

    /// Returns whether this root declares a holder a fill would write.
    ///
    /// The answer walks the declared Structs, which is exactly the reach
    /// [`Self::apply_arrow_batch`] has, and reads no rows. A root that answers
    /// `false` cannot be changed by a fill, so a caller applying by default
    /// skips the work rather than casting a batch to prove nothing happens.
    pub fn declares_holder(&self) -> bool {
        fn any_holder(fields: &[Field]) -> bool {
            fields.iter().any(|field| {
                field.as_digest().is_holder() || (field.is_struct() && any_holder(field.fields()))
            })
        }
        any_holder(self.as_field().fields())
    }

    /// Parses the ordered sources this holder selects relative to its Struct.
    ///
    /// The list is answered as it is stored, `"*"` included: that spelling is
    /// the whole selection - every field of the Struct the holder does not
    /// hold, in declaration order - and it is what an absent property means
    /// too. `Some([])` is an explicit empty selection, so absence and an empty
    /// JSON array remain distinct.
    ///
    /// A source states no metadata on the field it names. That is the whole
    /// point of naming it here: a schema declares one holder and the fields it
    /// reads stay ordinary columns.
    ///
    /// # Errors
    ///
    /// Returns an error naming `digest:sources` when stored metadata is not a
    /// JSON array of unique non-empty strings, or names `"*"` beside a path.
    pub fn sources(&self) -> Result<Option<Vec<String>>> {
        self.get(SOURCES)
            .map(|stored| parse_source_list(DIGEST_SOURCES_KEY, stored))
            .transpose()
    }

    /// Fill the digest holders this schema declares in one Arrow batch.
    ///
    /// The view is taken on the Struct root, and every declared Struct beneath
    /// it is walked: nested holders are final before a containing one reads
    /// them, so a row digest can hold a nested row digest. The source is first
    /// cast to that root, which is what materializes a holder column the rows
    /// do not carry at all.
    ///
    /// A holder holding anything but its canonical
    /// [default](crate::Field::default_value) is left alone - the rule
    /// [`PartitionField::apply_arrow_batch`](crate::PartitionField::apply_arrow_batch)
    /// follows for the same reason: recomputing a written value would hide a
    /// mismatch. A holder that is absent, or present holding nothing but that
    /// default, was never written and is filled.
    ///
    /// The algorithm is the one each holder resolves for itself: its own
    /// `digest:algorithm`, else the one its width implies. This entry point
    /// carries no seed or secret, which is what makes its answer the same
    /// [`Scalar::stable_hash`](crate::Scalar::stable_hash) every other reader computes; a
    /// seeded state or a forced recomputation is
    /// [`Digester::apply_arrow_batch`](crate::Digester::apply_arrow_batch).
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array, RecordBatch};
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut stored = DataType::UInt64.nullable_field("row_digest");
    /// stored.as_digest_mut().set_holder()?;
    /// let root = DataType::from_fields([DataType::Int64.required_field("id"), stored])?
    ///     .required_field("row");
    ///
    /// let batch = RecordBatch::try_from_iter([(
    ///     "id",
    ///     Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
    /// )])?;
    ///
    /// let filled = root.as_digest().apply_arrow_batch(&batch)?;
    ///
    /// assert_eq!(filled.num_columns(), 2);
    /// assert_eq!(filled.column(1).null_count(), 0);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this view is not on a usable Struct root, a
    /// holder has the wrong width, a digest path cannot be resolved, or the
    /// batch cannot be cast to that root.
    #[cfg(feature = "arrow")]
    pub fn apply_arrow_batch(
        &self,
        batch: &arrow_array::RecordBatch,
    ) -> Result<arrow_array::RecordBatch> {
        Ok(crate::xxhash::arrow::apply_arrow_batch_with(
            // Seedless, so the holders answer the canonical digest; a holder
            // whose width the default does not fit resolves its own.
            &DigestAlgorithm::Xxh3.digester(),
            self.as_field(),
            batch.clone(),
            false,
        )?)
    }
}

impl DigestFieldMut<'_> {
    /// Marks this field as holding a digest rather than contributing to it.
    pub fn set_holder(&mut self) -> Result<()> {
        self.insert(ROLE, DIGEST_ROLE_HOLDER).map(|_| ())
    }

    /// Records the algorithm this holder carries in canonical spelling.
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a holder or its datatype cannot
    /// store the algorithm's width, leaving it unchanged.
    pub fn set_algorithm(&mut self, algorithm: DigestAlgorithm) -> Result<()> {
        if !self.as_protocol().is_holder() {
            return Err(self.rejected(ALGORITHM, "requires digest:role=holder".into()));
        }
        if !holder_accepts(self.as_field(), algorithm) {
            return Err(self.rejected(
                ALGORITHM,
                format_smolstr!(
                    "algorithm {algorithm} requires {}, got {}",
                    expected_holder_dtypes(self.as_field(), algorithm),
                    self.as_field().dtype()
                ),
            ));
        }
        self.insert(ALGORITHM, algorithm.as_str()).map(|_| ())
    }

    /// Removes this holder's explicit digest algorithm.
    pub fn remove_algorithm(&mut self) -> Option<String> {
        self.remove(ALGORITHM)
    }

    /// Records the ordered sources this holder selects relative to its Struct.
    ///
    /// Order is hash-significant. `["*"]` selects every field of the Struct
    /// this holder does not hold, which is also what storing nothing means.
    /// Empty input stores `[]`, the explicit empty sequence, so it is not the
    /// same as [`Self::remove_sources`].
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a holder, when a source is
    /// empty or repeated, or when `"*"` travels beside a named path, leaving
    /// the field unchanged.
    pub fn set_sources<I, P>(&mut self, sources: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        if !self.as_protocol().is_holder() {
            return Err(self.rejected(SOURCES, "requires digest:role=holder".into()));
        }
        self.insert(SOURCES, render_source_list(DIGEST_SOURCES_KEY, sources)?)
            .map(|_| ())
    }

    /// Removes the explicit source selection, which selects every field again.
    pub fn remove_sources(&mut self) -> Option<String> {
        self.remove(SOURCES)
    }

    /// Removes this field's explicit digest role.
    ///
    /// # Errors
    ///
    /// Returns an error when holder-owned algorithm, source, time, or unit
    /// metadata is present, leaving the field unchanged. Remove those first.
    pub fn remove_role(&mut self) -> Result<Option<String>> {
        if self.has_holder_properties() {
            return Err(self.rejected(
                ROLE,
                "cannot remove holder role while digest:algorithm, digest:sources, digest:time, or digest:unit is present"
                    .into(),
            ));
        }
        Ok(self.remove(ROLE))
    }

    fn has_holder_properties(&self) -> bool {
        self.contains_key(ALGORITHM)
            || self.contains_key(SOURCES)
            || self.contains_key(TIME)
            || self.contains_key(UNIT)
    }

    /// Name the full digest key a typed mutation was refused under.
    pub(crate) fn rejected(&self, name: &str, reason: SmolStr) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason,
        }
    }
}

impl Field {
    /// Returns the struct children a row digest reads by default.
    ///
    /// That is every child except a digest holder, in declaration order, which
    /// is exactly what a holder's `digest:sources` of `["*"]` names and what
    /// storing no sources at all means. A holder naming its own sources
    /// selects from these same children; nothing marks them.
    pub fn digest_fields(&self) -> DigestFields<'_> {
        DigestFields::new(self.fields())
    }

    /// Returns the names of the children a row digest reads by default.
    pub fn digest_field_names(&self) -> DigestFieldNames<'_> {
        DigestFieldNames(self.digest_fields())
    }

    /// Returns the number of children a row digest reads by default.
    pub fn digest_field_len(&self) -> usize {
        self.digest_fields().count()
    }

    /// Returns this struct root holding only the children a digest reads.
    ///
    /// # Errors
    ///
    /// Returns an error when this is not a struct, or when the selected
    /// children do not form a valid datatype.
    pub fn only_digest_fields(&self) -> Result<Self> {
        self.require_struct()?;
        let kept: Vec<Self> = self.digest_fields().cloned().collect();
        Self::from_parts(
            self.name(),
            DataType::from_fields(kept)?,
            self.is_nullable(),
            self.metadata_iter(),
        )
    }
}

/// Return whether a field is one a row digest reads rather than holds.
pub(crate) fn is_digest_source(field: &Field) -> bool {
    !field.as_digest().is_holder()
}

/// A borrowed iterator over the children a row digest reads by default.
#[derive(Clone)]
pub struct DigestFields<'field> {
    fields: slice::Iter<'field, Field>,
}

impl<'field> DigestFields<'field> {
    pub(crate) fn new(fields: &'field [Field]) -> Self {
        Self {
            fields: fields.iter(),
        }
    }
}

impl<'field> Iterator for DigestFields<'field> {
    type Item = &'field Field;

    fn next(&mut self) -> Option<Self::Item> {
        self.fields.find(|field| is_digest_source(field))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.fields.len()))
    }
}

impl DoubleEndedIterator for DigestFields<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.fields.rfind(|field| is_digest_source(field))
    }
}

impl std::iter::FusedIterator for DigestFields<'_> {}

/// A borrowed iterator over the effective row-digest component names.
#[derive(Clone)]
pub struct DigestFieldNames<'field>(DigestFields<'field>);

impl<'field> Iterator for DigestFieldNames<'field> {
    type Item = &'field str;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Field::name)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for DigestFieldNames<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(Field::name)
    }
}

impl std::iter::FusedIterator for DigestFieldNames<'_> {}

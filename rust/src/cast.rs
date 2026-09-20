//! Casting an Arrow array into the exact array a typed field describes.
//!
//! [`FieldValue::cast_arrow_array`](crate::FieldValue::cast_arrow_array)
//! answers "make this array fit that field" for any field, and returns an
//! [`ArrayRef`] because any field could be any datatype. A field leaf already
//! holds its own datatype, so it can answer with the array type itself:
//! [`Int64Field`](crate::Int64Field) casts to an
//! [`Int64Array`](arrow_array::Int64Array), and the caller reads values without
//! a downcast of its own.
//!
//! The field is always the *target*: an incoming array is reconciled to the
//! field's datatype and nullability, never the other way around.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, StringArray};
//! use yggdryl::ArrowCastOptions;
//! use yggdryl::Int64Field;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Int64Field::unit("id", false);
//! let source: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));
//!
//! // The result is an Int64Array, not an ArrayRef needing a downcast.
//! let ids = field.cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))?;
//! assert_eq!(ids.values(), &[1, 2]);
//! # Ok(())
//! # }
//! ```
//!
//! A few datatypes carry a parameter that decides their physical array -
//! a timestamp's unit, a dictionary's key type - so those cast to an
//! [`ArrayRef`]. Every other variant casts to its concrete array.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, RecordBatch, StructArray};
use arrow_buffer::BooleanBuffer;
use arrow_cast::can_cast_types;
use arrow_schema::{DataType as ArrowDataType, FieldRef as ArrowFieldRef};
pub use batch::{preflight_arrow_batch_cast, validate_arrow_batch};
pub(crate) use kernel::arrow_cast_exposed;
pub use options::{ArrowCastOptions, Nullability, Representation};
pub use plan::ArrowCastPlan;
use smol_str::SmolStr;
pub use typed::ArrowFieldType;

use crate::UriType;
use crate::arrow::{Error, Result};
use crate::budget::MaterializationBudget;
use crate::bytes::casts::{bridges_through_binary, ingest_bytes_array};
use crate::cast::columns::{
    cast_dictionary_planned, cast_run_planned, cast_union_planned, contains_struct,
    exposed_logical_null_count, fill_nulls, folded_field_mapping, is_logically_null,
    is_reconcilable_nested, list_child, union_mode_matches,
};
use crate::cast::text::{blank_text_as_null, holds_text, ingest_text_values, keeps_empty_text};
use crate::decimal::casts::holds_decimal;
use crate::enums::EnumType;
use crate::geospatial::casts::{render_wkt_array, validate_wkb_ingest};
use crate::path::{Path, Segment};
use crate::sequence::SequenceType;
use crate::string::casts::{StringSource, ingest_code_array, ingest_string_array};
use crate::string::{is_text_storage, needs_extension};
use crate::temporal::casts::{
    holds_temporal, ingest_temporal_text, is_temporal_arrow, render_temporal_text,
};
use crate::uuid::casts::ingest_uuid_array;
use crate::version::casts::{ingest_version_array, is_text_layout};
use crate::{
    BLOOMBERG_WIDTH, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, CUSIP_WIDTH, ISIN_WIDTH, MIC_WIDTH,
    RecognizedExtension, SEDOL_WIDTH, SIDE_WIDTH, STATE_WIDTH, TIMEINFORCE_WIDTH, code_refusal,
    recognized_arrow_extension,
};
use crate::{DataType, Field, Scalar};

/// Exact and preflight record-batch boundaries.
mod batch {
    use arrow_array::RecordBatch;

    use super::{ArrowCastOptions, ArrowCastPlan};
    use crate::Field;
    use crate::FieldValue as _;
    use crate::arrow::{Error, Result, arrow_schema_from_field};

    /// Validates an exact source batch against one declared Struct root Field.
    ///
    /// This low-level hook is public for native runtime bindings that already own
    /// Arrow arrays but intentionally hidden from their user-facing APIs. It uses
    /// IPC-compatible schema comparison and rejects recursive logical values that
    /// would require default filling or another canonical repair.
    ///
    /// # Errors
    ///
    /// Returns an error unless `field` is a valid non-null Struct root and the
    /// batch has a compatible schema plus valid recursive values.
    #[doc(hidden)]
    pub fn validate_arrow_batch(field: &Field, batch: &RecordBatch) -> Result<()> {
        field.validate_bounded()?;
        let root_field = field.clone();
        root_field.validate_struct_root()?;

        // Valid means "needs no repair": casting an exact batch returns the very
        // arrays it was given, so a changed column is a validation failure.
        let cast = field.cast_arrow_batch(batch.clone(), ArrowCastOptions::new())?;
        for (index, (before, after)) in batch.columns().iter().zip(cast.columns()).enumerate() {
            if !std::sync::Arc::ptr_eq(before, after) {
                let name = batch.schema().field(index).name().clone();
                return Err(Error::IncompatibleSchema(format!(
                    "field {name:?} requires canonical repair and is not valid as stored"
                )));
            }
        }
        Ok(())
    }

    /// Preflights an empty source-to-target batch cast for runtime readers.
    ///
    /// This binding hook validates both Struct roots and constructs the recursive
    /// cast plan from the canonical source schema without materializing arrays. It
    /// lets an empty backend reject an invalid target before a lazy checked read is
    /// attempted, without maintaining another schema table.
    ///
    /// # Errors
    ///
    /// Returns an error when either root Field is invalid/nullable/non-Struct or
    /// the recursive source-to-target cast plan cannot be constructed.
    #[doc(hidden)]
    pub fn preflight_arrow_batch_cast(
        source: &Field,
        target: Option<&Field>,
        options: ArrowCastOptions,
    ) -> Result<()> {
        let schema = arrow_schema_from_field(source)?;
        let target = target.unwrap_or(source);
        ArrowCastPlan::compile(&schema, target, options)?.preflight()
    }
}
/// Bounded execution of Arrow kernel casts.
mod kernel {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, BooleanArray, Scalar as ArrowScalar, UInt32Array};
    use arrow_buffer::BooleanBuffer;
    use arrow_cast::{CastOptions, cast_with_options};
    use arrow_schema::DataType as ArrowDataType;
    use arrow_select::{take::take, zip::zip};

    use crate::arrow::{Error, Result};
    use crate::budget::{
        MaterializationBudget, SourceSelection, reserve_cast_output_payload,
        reserve_new_dictionary_vocabularies, reserve_selected_source_take,
        reserve_source_selection, reserve_vec_bytes,
    };
    use crate::cast::columns::{align_nested_dictionaries, contains_dictionary, default_array};
    use crate::{DataType, Field};

    fn arrow_cast(array: &ArrayRef, expected: &ArrowDataType, safe: bool) -> Result<ArrayRef> {
        let options = CastOptions {
            safe,
            ..CastOptions::default()
        };
        cast_with_options(array.as_ref(), expected, &options).map_err(Into::into)
    }

    pub(crate) fn arrow_cast_exposed(
        array: &ArrayRef,
        expected: &ArrowDataType,
        safe: bool,
        exposure: Option<&BooleanBuffer>,
        target: &Field,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let Some(exposure) = exposure else {
            if array.data_type() == expected {
                return Ok(Arc::clone(array));
            }
            budget.add_array(target.dtype(), array.len())?;
            let source_type = DataType::from_arrow_datatype(array.data_type())?;
            let full = [(0, array.len())];
            reserve_cast_output_payload(
                array.as_ref(),
                &source_type,
                target.dtype(),
                SourceSelection::Ranges(&full),
                budget,
            )?;
            return arrow_cast(array, expected, safe);
        };
        let selected_count = exposure.count_set_bits();
        if selected_count == 0 {
            return default_array(target, array.len(), Some(exposure), budget);
        }
        let phase = budget.mark();
        let output = (|| -> Result<ArrayRef> {
            // The masked kernel retains several arrays at once: selection/scatter
            // indices, a compact source, its compact cast, the scattered target, and
            // (for a required Field) the placeholder plus final zip output. Charge
            // every target-shaped buffer plus the selected source's actual physical
            // layout and copied payload before invoking Arrow's take kernel.
            budget.add_array(&DataType::UInt32, selected_count)?;
            budget.add_array(&DataType::UInt32, array.len())?;
            budget.add_array(target.dtype(), selected_count)?;
            budget.add_array(target.dtype(), array.len())?;
            if !target.is_nullable() {
                budget.add_array(target.dtype(), 1)?;
                budget.add_array(target.dtype(), array.len())?;
            }
            let mut selected = Vec::new();
            selected
                .try_reserve_exact(selected_count)
                .map_err(|error| {
                    Error::IncompatibleSchema(format!(
                        "masked Arrow cast index allocation failed: {error}"
                    ))
                })?;
            for index in 0..array.len() {
                if exposure.value(index) {
                    let source = u32::try_from(index).map_err(|_| {
                        Error::IncompatibleSchema(
                            "masked Arrow cast source index exceeds UInt32".to_owned(),
                        )
                    })?;
                    selected.push(source);
                }
            }
            reserve_selected_source_take(
                array.as_ref(),
                &selected,
                target.dtype(),
                if target.is_nullable() { 2 } else { 3 },
                budget,
            )?;

            reserve_vec_bytes::<Option<u32>>(budget, array.len())?;
            let mut scatter = Vec::new();
            scatter.try_reserve_exact(array.len()).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "masked Arrow cast scatter allocation failed: {error}"
                ))
            })?;
            let mut target_index = 0usize;
            for index in 0..array.len() {
                if exposure.value(index) {
                    scatter.push(Some(u32::try_from(target_index).map_err(|_| {
                        Error::IncompatibleSchema(
                            "masked Arrow cast target index exceeds UInt32".to_owned(),
                        )
                    })?));
                    target_index += 1;
                } else {
                    scatter.push(None);
                }
            }
            let compact = take(array.as_ref(), &UInt32Array::from(selected), None)?;
            let compact = arrow_cast(&compact, expected, safe)?;
            let scattered = take(compact.as_ref(), &UInt32Array::from(scatter), None)?;
            if target.is_nullable() {
                return Ok(scattered);
            }
            let mask = BooleanArray::new(exposure.clone(), None);
            let placeholder = crate::arrow::value::physical_placeholder_for_field(target)?;
            let placeholder = crate::arrow::value::array_from_values(target, &[&placeholder])?;
            let (scattered, placeholder) = if contains_dictionary(target.dtype()) {
                align_nested_dictionaries(
                    target,
                    &scattered,
                    &placeholder,
                    Some(exposure),
                    None,
                    budget,
                )?
            } else {
                (scattered, placeholder)
            };
            let placeholder = ArrowScalar::new(placeholder);
            zip(&mask, &scattered.as_ref(), &placeholder).map_err(Into::into)
        })()?;

        // Compact sources, index arrays, scatter output, and placeholders are
        // phase-local. Retain only the returned target array before sibling
        // columns continue against the shared operation budget.
        budget.restore(phase);
        let full = [(0, output.len())];
        reserve_source_selection(
            output.as_ref(),
            target.dtype(),
            SourceSelection::Ranges(&full),
            budget,
        )?;
        if contains_dictionary(target.dtype()) {
            reserve_new_dictionary_vocabularies(&output, array, target.dtype(), budget)?;
        }
        Ok(output)
    }
}
/// What a cast is allowed to do about failure, absence, and representation.
///
/// Three independent questions travel together through every Arrow cast, and
/// confusing them is what a single `safe` flag invited. `safe` decides whether
/// a *present* value may be converted; [`Nullability`] decides whether a
/// declared value may be *absent* at all; [`Representation`] decides what a
/// same-width pair actually carries. A conversion that fails under `safe`
/// produces a null, and whether that null is then repaired or refused is the
/// nullability policy's answer, not the conversion's.
mod options {
    use std::fmt;
    use std::str::FromStr;

    use smol_str::format_smolstr;

    use crate::{Error, Result};

    /// What a cast does about a non-nullable target field the source cannot fill.
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub enum Nullability {
        /// Repair: a required field absent from the source, or null within it,
        /// takes the target's canonical [default](crate::Field::default_value).
        #[default]
        Default,
        /// Refuse: a required field must be carried by the source and hold a value
        /// in every exposed row, and the error names its full path.
        Strict,
    }

    impl Nullability {
        /// Every policy in canonical order.
        pub const ALL: [Self; 2] = [Self::Default, Self::Strict];

        /// Parse one canonical policy name.
        ///
        /// # Errors
        ///
        /// Returns [`Error::Parse`] naming the complete accepted vocabulary.
        #[allow(clippy::should_implement_trait)]
        pub fn from_str(value: &str) -> Result<Self> {
            <Self as FromStr>::from_str(value)
        }

        /// Return the canonical lowercase spelling.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Default => "default",
                Self::Strict => "strict",
            }
        }

        /// Returns whether absence is refused rather than repaired.
        pub const fn is_strict(self) -> bool {
            matches!(self, Self::Strict)
        }
    }

    impl AsRef<str> for Nullability {
        fn as_ref(&self) -> &str {
            self.as_str()
        }
    }

    impl fmt::Display for Nullability {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.as_str())
        }
    }

    impl FromStr for Nullability {
        type Err = Error;

        fn from_str(value: &str) -> Result<Self> {
            let normalized = value.trim();
            Self::ALL
                .into_iter()
                .find(|policy| normalized.eq_ignore_ascii_case(policy.as_str()))
                .ok_or_else(|| Error::Parse {
                    target: "nullability",
                    position: 0,
                    reason: format_smolstr!(
                        "expected one of {}, got {value:?}",
                        Self::ALL
                            .iter()
                            .map(|policy| policy.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })
        }
    }

    /// What a cast carries across two datatypes of the same physical width.
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub enum Representation {
        /// The value crosses: a number keeps its arithmetic meaning, and one
        /// outside the target's range is refused - or nulled, under `safe`.
        #[default]
        Value,
        /// The bits cross: two datatypes laid out as one fixed-width buffer of the
        /// same byte width are the same bytes under two readings, so every source
        /// bit pattern maps and the buffer is shared rather than rebuilt.
        /// `u64::MAX` reads as `-1_i64`, those eight bytes read as an
        /// `int64`/`uint64`/`float64`/`fixed_binary(8)` alike, and every one
        /// of those round-trips back.
        ///
        /// It is a preference, not a mode: a pair that is not laid out that way
        /// takes the ordinary conversion, so asking for bits never silently
        /// reinterprets something that is not the same bytes.
        Bits,
    }

    impl Representation {
        /// Every reading in canonical order.
        pub const ALL: [Self; 2] = [Self::Value, Self::Bits];

        /// Parse one canonical reading name.
        ///
        /// # Errors
        ///
        /// Returns [`Error::Parse`] naming the complete accepted vocabulary.
        #[allow(clippy::should_implement_trait)]
        pub fn from_str(value: &str) -> Result<Self> {
            <Self as FromStr>::from_str(value)
        }

        /// Return the canonical lowercase spelling.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Value => "value",
                Self::Bits => "bits",
            }
        }

        /// Returns whether a same-width pair is read as the same bytes.
        pub const fn is_bits(self) -> bool {
            matches!(self, Self::Bits)
        }
    }

    impl AsRef<str> for Representation {
        fn as_ref(&self) -> &str {
            self.as_str()
        }
    }

    impl fmt::Display for Representation {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.as_str())
        }
    }

    impl FromStr for Representation {
        type Err = Error;

        fn from_str(value: &str) -> Result<Self> {
            let normalized = value.trim();
            Self::ALL
                .into_iter()
                .find(|reading| normalized.eq_ignore_ascii_case(reading.as_str()))
                .ok_or_else(|| Error::Parse {
                    target: "representation",
                    position: 0,
                    reason: format_smolstr!(
                        "expected one of {}, got {value:?}",
                        Self::ALL
                            .iter()
                            .map(|reading| reading.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })
        }
    }

    /// The three independent decisions every Arrow cast makes.
    ///
    /// ```
    /// use yggdryl::{ArrowCastOptions, Nullability, Representation};
    ///
    /// // The default is today's contract: convert leniently, repair absence, and
    /// // carry the value rather than the bytes under it.
    /// let lenient = ArrowCastOptions::new();
    /// assert!(lenient.is_safe());
    /// assert_eq!(lenient.nullability(), Nullability::Default);
    /// assert_eq!(lenient.representation(), Representation::Value);
    ///
    /// // The three answers move independently.
    /// let strict = ArrowCastOptions::new()
    ///     .with_nullability(Nullability::Strict)
    ///     .with_representation(Representation::Bits);
    /// assert!(strict.is_safe());
    /// assert!(strict.nullability().is_strict());
    /// assert!(strict.representation().is_bits());
    /// ```
    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub struct ArrowCastOptions {
        safe: bool,
        nullability: Nullability,
        representation: Representation,
    }

    impl ArrowCastOptions {
        /// The lenient conversion and the repairing nullability policy.
        pub const fn new() -> Self {
            Self {
                safe: true,
                nullability: Nullability::Default,
                representation: Representation::Value,
            }
        }

        /// Set whether a failed conversion becomes null rather than an error.
        pub const fn with_safe(mut self, safe: bool) -> Self {
            self.safe = safe;
            self
        }

        /// Set what happens to a required field the source cannot fill.
        pub const fn with_nullability(mut self, nullability: Nullability) -> Self {
            self.nullability = nullability;
            self
        }

        /// Returns whether a failed conversion becomes null rather than an error.
        pub const fn is_safe(self) -> bool {
            self.safe
        }

        /// Set what a same-width pair carries: the value, or the bytes under it.
        pub const fn with_representation(mut self, representation: Representation) -> Self {
            self.representation = representation;
            self
        }

        /// Returns what happens to a required field the source cannot fill.
        pub const fn nullability(self) -> Nullability {
            self.nullability
        }

        /// Returns what a same-width pair carries.
        pub const fn representation(self) -> Representation {
            self.representation
        }

        /// Returns this policy with absence repaired rather than refused.
        ///
        /// A materializing protocol fills its own column after the cast, so the
        /// cast may not refuse the hole the protocol is about to close.
        pub(crate) const fn deferred(self) -> Self {
            self.with_nullability(Nullability::Default)
        }
    }

    impl Default for ArrowCastOptions {
        fn default() -> Self {
            Self::new()
        }
    }
}
/// One record-batch cast, compiled once and applied to many batches.
///
/// Everything a cast decides from two schemas - which source column answers
/// which target field, the order the children come back in, the recursive type
/// dispatch, the target schema, and the kernel options - is a function of the
/// schemas alone. A reader whose batches all carry one schema was nonetheless
/// paying for that decision on every batch, because the entry points construct
/// the recursive plan inline from `array.data_type()`. [`ArrowCastPlan`] is
/// that work lifted out: compile once, apply per batch, and let only the
/// masks, offsets, and dictionary reachability a batch actually carries vary.
mod plan {
    use std::fmt;
    use std::sync::Arc;

    use arrow_array::{RecordBatch, RecordBatchOptions, StructArray};
    use arrow_schema::{DataType as ArrowDataType, Schema, SchemaRef};

    use crate::Field;
    use crate::arrow::{Error, Result, arrow_schema_from_field};
    use crate::budget::MaterializationBudget;

    use super::{ArrayCastPlan, ArrowCastOptions, Deferred, downcast};

    /// A compiled Arrow record-batch cast: immutable, shareable, and reusable.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int32Array, RecordBatch};
    /// use yggdryl::{ArrowCastOptions, ArrowCastPlan, DataType, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    ///     .required_field("row");
    /// let source = RecordBatch::try_from_iter([(
    ///     "id",
    ///     Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef,
    /// )])?;
    ///
    /// // The schemas decide everything; the plan is compiled from them alone.
    /// let plan = ArrowCastPlan::compile(source.schema_ref(), &root, ArrowCastOptions::new())?;
    /// assert_eq!(plan.as_schema().field(0).name(), "id");
    ///
    /// // The same plan answers every batch of that schema.
    /// let cast = plan.apply(source)?;
    /// assert_eq!(cast.column(0).data_type(), &arrow_schema::DataType::Int64);
    /// # Ok(())
    /// # }
    /// ```
    pub struct ArrowCastPlan {
        field: Field,
        source: SchemaRef,
        schema: SchemaRef,
        options: ArrowCastOptions,
        root: ArrayCastPlan,
    }

    impl ArrowCastPlan {
        /// Compiles the cast from one source schema to one non-null Struct root.
        ///
        /// Every failure the two schemas alone can produce - an unsupported
        /// conversion, an ambiguous case-insensitive name, a required field no
        /// source column carries under
        /// [`Nullability::Strict`](super::Nullability::Strict) - is raised here,
        /// before a single batch exists.
        ///
        /// # Errors
        ///
        /// Returns an error unless `target` is a bounded, non-nullable Struct
        /// root, or when the recursive cast cannot be planned.
        pub fn compile(source: &Schema, target: &Field, options: ArrowCastOptions) -> Result<Self> {
            Self::compile_deferring(source, target, options, Deferred::default())
        }

        /// Compiles the cast, leaving the columns a named protocol still has to
        /// materialize out of the strict nullability check.
        pub(crate) fn compile_deferring(
            source: &Schema,
            target: &Field,
            options: ArrowCastOptions,
            deferred: Deferred,
        ) -> Result<Self> {
            target.validate_bounded()?;
            if target.is_nullable() {
                return Err(Error::IncompatibleSchema(
                    "record-batch cast target Struct Field must be non-nullable".to_owned(),
                ));
            }
            if target.dtype().as_fields().is_none() {
                return Err(Error::IncompatibleSchema(format!(
                    "record-batch cast target {:?} must have a struct datatype",
                    target.name()
                )));
            }
            let source_type = ArrowDataType::Struct(source.fields().clone());
            let root = ArrayCastPlan::new_root(target, &source_type, options, deferred)?;
            Ok(Self {
                field: target.clone(),
                source: Arc::new(source.clone()),
                schema: arrow_schema_from_field(target)?,
                options,
                root,
            })
        }

        /// The root Field every batch is reconciled to.
        pub const fn as_field(&self) -> &Field {
            &self.field
        }

        /// The schema batches must carry to be applied.
        pub const fn as_source_schema(&self) -> &SchemaRef {
            &self.source
        }

        /// The schema every applied batch carries, target metadata included.
        pub const fn as_schema(&self) -> &SchemaRef {
            &self.schema
        }

        /// The conversion and nullability policy this plan was compiled under.
        pub const fn as_options(&self) -> &ArrowCastOptions {
            &self.options
        }

        /// Runs the compiled cast over no rows.
        ///
        /// A caller planning a read rejects an impossible cast here rather than on
        /// the first batch: an empty batch of the source schema exercises the whole
        /// recursive plan without materializing a value. Row-dependent refusals -
        /// a null in a required column, a value the target cannot hold - have no
        /// rows to find and stay for [`Self::apply`].
        ///
        /// # Errors
        ///
        /// Returns any error the cast raises with no rows to read.
        pub fn preflight(&self) -> Result<()> {
            self.apply(RecordBatch::new_empty(Arc::clone(&self.source)))
                .map(|_| ())
        }

        /// Reconciles one batch of the source schema to the target root.
        ///
        /// An exact batch comes back as itself: the same `RecordBatch`, holding the
        /// same column `Arc`s, not a rebuilt one carrying equal arrays.
        ///
        /// # Errors
        ///
        /// Returns an error when the batch does not carry the schema this plan was
        /// compiled for, when a value cannot be converted, or when a required field
        /// is null under [`Nullability::Strict`](super::Nullability::Strict).
        pub fn apply(&self, batch: RecordBatch) -> Result<RecordBatch> {
            if batch.schema_ref().fields() != self.source.fields() {
                return Err(Error::IncompatibleSchema(format!(
                    "batch schema {:?} differs from the schema this cast plan was compiled for, {:?}",
                    batch.schema_ref().fields(),
                    self.source.fields()
                )));
            }
            let row_count = batch.num_rows();
            let source = super::struct_array_from_batch(batch.clone());
            let mut budget = MaterializationBudget::default();
            let cast = self.root.cast(source, &mut budget)?;
            let columns = downcast::<StructArray>(cast.as_ref())?.columns();

            // Ownership allows handing the caller's own batch back only when every
            // column survived by pointer and the batch already declares the target
            // schema; anything else would be a different batch wearing that claim.
            if batch.schema_ref().as_ref() == self.schema.as_ref()
                && columns.len() == batch.num_columns()
                && columns
                    .iter()
                    .zip(batch.columns())
                    .all(|(cast, source)| Arc::ptr_eq(cast, source))
            {
                return Ok(batch);
            }

            // A batch carries its row count even with no columns, which a struct
            // array cannot, so the count is restored explicitly.
            let options = RecordBatchOptions::new().with_row_count(Some(row_count));
            RecordBatch::try_new_with_options(Arc::clone(&self.schema), columns.to_vec(), &options)
                .map_err(Into::into)
        }

        /// Whether the plan hands every batch of its source schema straight back.
        ///
        /// A reader over such a plan is the reader itself, so nothing wraps it.
        /// Strictness is what makes an equal schema not enough: a non-null Arrow
        /// field can still carry a logical null inside a nested child, and refusing
        /// that is the whole point of asking.
        pub(crate) fn is_identity(&self) -> bool {
            !self.options.nullability().is_strict() && self.source.as_ref() == self.schema.as_ref()
        }
    }

    /// The plan by what decided it, not by the tree that decision produced.
    ///
    /// The recursive dispatch is an implementation detail whose rendering would
    /// bury the three things a reader of a failed plan needs: what it targets,
    /// what it accepts, and under which policy.
    impl fmt::Debug for ArrowCastPlan {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("ArrowCastPlan")
                .field("field", &self.field)
                .field("source", &self.source)
                .field("schema", &self.schema)
                .field("options", &self.options)
                .finish()
        }
    }

    // A compiled plan is schema-derived state with no interior mutation on the
    // cast path, so one plan serves every batch of a reader and every thread of a
    // parallel scan. The assertion is here because nothing else would notice if a
    // future plan field quietly gave that up.
    const _: fn() = || {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ArrowCastPlan>();
    };
}
/// Reading a text column through one of this crate's own value readers.
///
/// A text column entering a temporal or a decimal is read here rather than by
/// Arrow, because both families carry a precision the storage may not hold and
/// this crate refuses what it cannot state exactly where Arrow rounds it. The
/// reader is the same function a row value goes through, so a batch and a row
/// cannot answer differently about a spelling this crate knows. Arrow stays
/// behind the reading for the spellings only it takes, so a column still reads
/// everything it used to.
pub(crate) mod text {
    use crate::enums::EnumType;
    use std::sync::Arc;

    use arrow_array::types::{
        ArrowDictionaryKeyType, Int8Type, Int16Type, Int32Type, Int64Type, RunEndIndexType,
        UInt8Type, UInt16Type, UInt32Type, UInt64Type,
    };
    use arrow_array::{
        Array, ArrayRef, BooleanArray, DictionaryArray, LargeStringArray, PrimitiveArray, RunArray,
        StringArray, StringViewArray,
    };
    use arrow_buffer::{ArrowNativeType, BooleanBuffer, BooleanBufferBuilder};
    use arrow_cast::can_cast_types;
    use arrow_schema::DataType as ArrowDataType;
    use arrow_select::nullif::nullif;
    use arrow_select::zip::zip;

    use crate::arrow::{Error, Result};
    use crate::budget::{MaterializationBudget, reserve_vec_bytes};
    use crate::cast::columns::{is_exposed, run_value_exposure};
    use crate::cast::{arrow_cast_exposed, downcast};
    use crate::sequence::SequenceType;
    use crate::value::dtype_canonical;
    use crate::{DataType, Field, Scalar};

    /// One text cell read into the value a datatype declares.
    ///
    /// [`crate::Error::Parse`] means the reader did not take the spelling at all,
    /// so Arrow is allowed to try it; every other refusal is a spelling this crate
    /// read and then rejected, and it stands.
    pub(crate) type TextReader = fn(&DataType, &str) -> crate::Result<Scalar>;

    /// The value a target holds, past whatever layout encodes it.
    ///
    /// The encoding is a layout: the values read as the value they hold and the
    /// tail encodes them, so a dictionary column reads like a plain one.
    pub(crate) fn encoded_value_of(target: &DataType) -> &DataType {
        match target {
            DataType::Enum(EnumType::Dictionary(dictionary)) => {
                encoded_value_of(dictionary.value())
            }
            DataType::RunEndEncoded(encoded) => encoded_value_of(encoded.values().dtype()),
            other => other,
        }
    }

    /// Whether a source layout holds text values, however it wraps them.
    pub(crate) fn holds_text(source: &ArrowDataType) -> bool {
        match source {
            ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View => true,
            ArrowDataType::Dictionary(_, values) => holds_text(values),
            ArrowDataType::RunEndEncoded(_, values) => holds_text(values.data_type()),
            _ => false,
        }
    }

    /// Whether a target keeps a zero-length text as what it is, rather than
    /// reading it as absence.
    ///
    /// The one owner of the list the empty-cell rule turns on: `""` entering a
    /// datatype this answers `false` for is absence, never a spelling to parse.
    /// A string leaf and a byte leaf hold it as the value it is, past the
    /// layout that encodes them; a list target reads a scalar source into its
    /// item, so the item answers; an interval has no text spelling, so an
    /// empty one is a spelling it refuses, never absence; and a code with a
    /// neutral member - the empty text its own reader accepts, which is also
    /// its canonical default - holds it as that member.
    pub(crate) fn keeps_empty_text(target: &DataType) -> bool {
        match encoded_value_of(target) {
            DataType::String(_) | DataType::Bytes(_) | DataType::Interval(_) => true,
            DataType::Sequence(
                SequenceType::List(item)
                | SequenceType::LargeList(item)
                | SequenceType::ListView(item)
                | SequenceType::LargeListView(item)
                | SequenceType::FixedSizeList(item, _),
            ) => keeps_empty_text(item.dtype()),
            code if code.is_code() => dtype_canonical(code, Scalar::from("")).is_ok(),
            _ => false,
        }
    }

    /// Whether a row value is an empty text cell entering a datatype that reads
    /// it as absence: the empty-cell rule at the scalar door.
    pub(crate) fn is_blank_text(target: &DataType, value: &Scalar) -> bool {
        matches!(value, Scalar::String(text) if text.as_str().is_empty())
            && !keeps_empty_text(target)
    }

    /// The value a scalar door reads before it parses any spelling: an empty
    /// text cell entering a datatype that holds neither text nor bytes *is*
    /// [`Scalar::Null`], and the rest of the door - the bare-null rule, a
    /// field's nullability - answers as it does for one.
    pub(crate) fn blank_text_read(target: &DataType, value: Scalar) -> Scalar {
        if is_blank_text(target, &value) {
            Scalar::Null
        } else {
            value
        }
    }

    /// The same text column with every exposed empty cell turned null: the
    /// empty-cell rule at the Arrow door.
    ///
    /// The array holds text under one of the three plain layouts, a dictionary
    /// or a run-end pair over one. A plain column and a run-end pair's values
    /// get a validity buffer, a dictionary gets it on its keys - the row is
    /// what the rule nulls, and a vocabulary is shared by rows that are not
    /// all empty. The offsets and the payload are shared and never re-read:
    /// the validity is the one buffer built, and only when a cell needs it. A
    /// cell that is not exposed is not read. A run is one value for every row
    /// it covers, so a run holding the empty text is nulled as a whole once
    /// any of its rows is exposed, as the crate already exposes run values.
    pub(crate) fn blank_text_as_null(
        array: &ArrayRef,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        match array.data_type() {
            ArrowDataType::Dictionary(key, _) => match key.as_ref() {
                ArrowDataType::Int8 => blank_dictionary::<Int8Type>(array, exposure, budget),
                ArrowDataType::Int16 => blank_dictionary::<Int16Type>(array, exposure, budget),
                ArrowDataType::Int32 => blank_dictionary::<Int32Type>(array, exposure, budget),
                ArrowDataType::Int64 => blank_dictionary::<Int64Type>(array, exposure, budget),
                ArrowDataType::UInt8 => blank_dictionary::<UInt8Type>(array, exposure, budget),
                ArrowDataType::UInt16 => blank_dictionary::<UInt16Type>(array, exposure, budget),
                ArrowDataType::UInt32 => blank_dictionary::<UInt32Type>(array, exposure, budget),
                ArrowDataType::UInt64 => blank_dictionary::<UInt64Type>(array, exposure, budget),
                other => Err(Error::IncompatibleSchema(format!(
                    "dictionary key type is not a supported integer: {other:?}"
                ))),
            },
            ArrowDataType::RunEndEncoded(run_ends, _) => match run_ends.data_type() {
                ArrowDataType::Int16 => blank_runs::<Int16Type>(array, exposure, budget),
                ArrowDataType::Int32 => blank_runs::<Int32Type>(array, exposure, budget),
                ArrowDataType::Int64 => blank_runs::<Int64Type>(array, exposure, budget),
                _ => Err(Error::IncompatibleSchema(
                    "run-end type is not a supported signed integer".to_owned(),
                )),
            },
            _ => {
                let cells = TextCells::of(array.as_ref())?;
                blank_rows(
                    array,
                    cells.len(),
                    |row| is_exposed(exposure, row) && cells.is_blank(row),
                    budget,
                )
            }
        }
    }

    /// Nulls the keys of a dictionary whose exposed rows point at an empty
    /// value: the pair's validity is its keys', so the vocabulary is untouched.
    fn blank_dictionary<K: ArrowDictionaryKeyType>(
        array: &ArrayRef,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let source = downcast::<DictionaryArray<K>>(array.as_ref())?;
        let cells = TextCells::of(source.values().as_ref())?;
        let keys = source.keys();
        let blank = |row: usize| {
            is_exposed(exposure, row)
                && keys.is_valid(row)
                && cells.is_blank(keys.value(row).as_usize())
        };
        blank_rows(array, keys.len(), blank, budget)
    }

    /// Nulls the values of a run-end pair whose runs reach an exposed row and
    /// hold the empty text; the run ends stay as they are, and so does the
    /// window a slice of the pair looks through.
    fn blank_runs<R: RunEndIndexType>(
        array: &ArrayRef,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let source = downcast::<RunArray<R>>(array.as_ref())?;
        let run_exposure = run_value_exposure(source, exposure, budget)?;
        let values = blank_text_as_null(source.values(), run_exposure.as_ref(), budget)?;
        if Arc::ptr_eq(&values, source.values()) {
            return Ok(Arc::clone(array));
        }
        let run_ends = source.run_ends();
        let whole = RunArray::<R>::try_new(
            &PrimitiveArray::<R>::new(run_ends.inner().clone(), None),
            values.as_ref(),
        )?;
        Ok(Arc::new(whole.slice(run_ends.offset(), run_ends.len())))
    }

    /// The array with the rows a predicate names turned null, or the same
    /// array when it names none.
    ///
    /// The common column has no blank cell and costs nothing: the scan stops
    /// at the first blank, and only then is the mask built and folded into
    /// the validity by [`nullif`], which rewrites the one buffer and never
    /// re-reads the payload under it. Both bitmaps are charged.
    fn blank_rows(
        array: &ArrayRef,
        rows: usize,
        blank: impl Fn(usize) -> bool,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        if !(0..rows).any(&blank) {
            return Ok(Arc::clone(array));
        }
        budget.add_bitmap(rows)?;
        budget.add_bitmap(rows)?;
        let mask = BooleanArray::new(BooleanBuffer::collect_bool(rows, blank), None);
        Ok(nullif(array.as_ref(), &mask)?)
    }

    /// One text column under whichever of the three plain layouts it uses.
    enum TextCells<'a> {
        Utf8(&'a StringArray),
        LargeUtf8(&'a LargeStringArray),
        Utf8View(&'a StringViewArray),
    }

    impl<'a> TextCells<'a> {
        fn of(array: &'a dyn Array) -> Result<Self> {
            Ok(match array.data_type() {
                ArrowDataType::Utf8 => Self::Utf8(downcast(array)?),
                ArrowDataType::LargeUtf8 => Self::LargeUtf8(downcast(array)?),
                ArrowDataType::Utf8View => Self::Utf8View(downcast(array)?),
                other => {
                    return Err(Error::IncompatibleSchema(format!(
                        "expected a text column to blank, got {other:?}"
                    )));
                }
            })
        }

        fn len(&self) -> usize {
            match self {
                Self::Utf8(cells) => cells.len(),
                Self::LargeUtf8(cells) => cells.len(),
                Self::Utf8View(cells) => cells.len(),
            }
        }

        fn is_valid(&self, index: usize) -> bool {
            match self {
                Self::Utf8(cells) => cells.is_valid(index),
                Self::LargeUtf8(cells) => cells.is_valid(index),
                Self::Utf8View(cells) => cells.is_valid(index),
            }
        }

        /// Whether the cell is a present, zero-length text.
        fn is_blank(&self, index: usize) -> bool {
            index < self.len()
                && self.is_valid(index)
                && match self {
                    Self::Utf8(cells) => cells.value(index).is_empty(),
                    Self::LargeUtf8(cells) => cells.value(index).is_empty(),
                    Self::Utf8View(cells) => cells.value(index).is_empty(),
                }
        }
    }

    /// Reads a column of text through one of this crate's own value readers.
    pub(crate) fn ingest_text_values(
        array: &ArrayRef,
        expected: &ArrowDataType,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        reader: TextReader,
    ) -> Result<ArrayRef> {
        // The temporary is nullable text: this leaf owns the failures, and the
        // target's own null policy runs after the reading.
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                safe,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let source = downcast::<StringArray>(text.as_ref())?;
        let rows = source.len();
        let dtype = encoded_value_of(field.dtype());
        let read = Field::new(field.name(), dtype.clone(), true);
        budget.add_array(dtype, rows)?;
        reserve_vec_bytes::<Scalar>(budget, rows)?;
        reserve_vec_bytes::<&Scalar>(budget, rows)?;
        let mut values = Vec::with_capacity(rows);
        let mut ours = BooleanBufferBuilder::new(rows);
        let mut refused = false;
        for index in 0..rows {
            let cell = (is_exposed(exposure, index) && source.is_valid(index))
                .then(|| source.value(index))
                .map(|text| reader(dtype, text));
            match cell {
                // An absent value is this reading's own: nothing else reads it.
                None => {
                    values.push(Scalar::Null);
                    ours.append(true);
                }
                Some(Ok(value)) => {
                    values.push(value);
                    ours.append(true);
                }
                // A spelling this crate read and then refused - a count its unit
                // or width cannot hold exactly, a digit its scale would drop -
                // stays refused: Arrow would round it, and the row tier does not.
                Some(Err(error)) => {
                    let unread = matches!(error, crate::Error::Parse { .. });
                    values.push(Scalar::Null);
                    ours.append(!unread);
                    refused |= unread;
                }
            }
        }
        let mask = BooleanArray::new(ours.finish(), None);
        let read_here =
            crate::arrow::value::array_from_values(&read, &values.iter().collect::<Vec<_>>())?;
        let cast = if refused && can_cast_types(source.data_type(), expected) {
            // Arrow reads what this crate could not at its own risk: a value
            // neither reading takes stays null, and strict mode reports it below.
            // Arrow refuses a whole column whose target zone it cannot name, so
            // its failure leaves this crate's reading standing rather than
            // sinking it.
            match arrow_cast_exposed(&text, expected, true, exposure, &read, budget) {
                Ok(arrow) => zip(&mask, &read_here.as_ref(), &arrow.as_ref())?,
                Err(_) => read_here,
            }
        } else {
            read_here
        };
        if !safe {
            for index in 0..rows {
                let absent = !is_exposed(exposure, index) || source.is_null(index);
                if absent || !cast.is_null(index) {
                    continue;
                }
                let cell = source.value(index);
                let reason = match reader(dtype, cell) {
                    Err(crate::Error::InvalidRecord { reason, .. }) => reason.to_string(),
                    Err(other) => other.to_string(),
                    Ok(_) => String::new(),
                };
                return Err(Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: {cell:?} does not read as {dtype}: {reason}",
                    field.name(),
                )));
            }
        }
        Ok(cast)
    }
}
/// Typed-field Arrow array projections.
mod typed {
    use arrow_array::{Array, ArrayRef, Scalar};

    use crate::FieldValue as _;
    use crate::arrow::{Error, Result};
    use crate::cast::ArrowCastOptions;

    /// The Arrow array a field's values materialize into.
    ///
    /// Implemented for every datatype payload. A variant whose physical array
    /// depends on a datatype parameter reports [`ArrayRef`], because there is no
    /// single concrete type to name.
    pub trait ArrowFieldType: crate::DataTypeValue {
        /// The array produced by casting to this field's datatype.
        type Array: Array + Clone + 'static;

        /// Narrow a cast array to this marker's array type.
        ///
        /// # Errors
        ///
        /// Returns an error when the cast produced a different physical array,
        /// which would mean the cast engine and this table disagree.
        fn downcast_array(array: ArrayRef) -> Result<Self::Array>;
    }

    /// Narrow one cast array, naming both sides when the narrowing fails.
    fn downcast_owned<A: Array + Clone + 'static>(
        array: ArrayRef,
        expected: &'static str,
    ) -> Result<A> {
        array.as_any().downcast_ref::<A>().cloned().ok_or_else(|| {
            Error::IncompatibleSchema(format!(
                "expected a cast to produce an Arrow {expected} array, got {}",
                array.data_type()
            ))
        })
    }

    /// Bind a marker to the concrete Arrow array its datatype materializes into.
    macro_rules! typed_array {
        ($marker:path, $array:ty) => {
            impl ArrowFieldType for $marker {
                type Array = $array;

                fn downcast_array(array: ArrayRef) -> Result<Self::Array> {
                    downcast_owned(array, stringify!($array))
                }
            }
        };
    }

    /// Bind a marker whose physical array depends on a datatype parameter.
    macro_rules! opaque_array {
        ($marker:path) => {
            impl ArrowFieldType for $marker {
                type Array = ArrayRef;

                fn downcast_array(array: ArrayRef) -> Result<Self::Array> {
                    Ok(array)
                }
            }
        };
    }

    typed_array!(crate::NullType, arrow_array::NullArray);
    typed_array!(crate::BooleanType, arrow_array::BooleanArray);
    typed_array!(crate::integer::Int8Type, arrow_array::Int8Array);
    typed_array!(crate::integer::Int16Type, arrow_array::Int16Array);
    typed_array!(crate::integer::Int32Type, arrow_array::Int32Array);
    typed_array!(crate::integer::Int64Type, arrow_array::Int64Array);
    typed_array!(crate::integer::UInt8Type, arrow_array::UInt8Array);
    typed_array!(crate::integer::UInt16Type, arrow_array::UInt16Array);
    typed_array!(crate::integer::UInt32Type, arrow_array::UInt32Array);
    typed_array!(crate::integer::UInt64Type, arrow_array::UInt64Array);
    typed_array!(crate::floating::Float16Type, arrow_array::Float16Array);
    typed_array!(crate::floating::Float32Type, arrow_array::Float32Array);
    typed_array!(crate::floating::Float64Type, arrow_array::Float64Array);
    // A date's width is the leaf, so the family has no single array type: a
    // `date32` column is a `Date32Array` and a `date64` one a `Date64Array`.
    opaque_array!(crate::DateType);
    // A decimal's backing integer is the leaf, so the family has no single
    // array type: a `decimal32` column is a `Decimal32Array` and a
    // `decimal256` one a `Decimal256Array`.
    opaque_array!(crate::DecimalType);
    typed_array!(crate::version::VersionType, arrow_array::StringArray);
    // A registered code stores as the text it is, exactly as a version does.
    typed_array!(crate::CountryType, arrow_array::StringArray);
    typed_array!(crate::CurrencyType, arrow_array::StringArray);
    typed_array!(crate::MicCodeType, arrow_array::StringArray);
    typed_array!(crate::CfiCodeType, arrow_array::StringArray);
    typed_array!(crate::IsinCodeType, arrow_array::StringArray);
    typed_array!(crate::CusipCodeType, arrow_array::StringArray);
    typed_array!(crate::SedolCodeType, arrow_array::StringArray);
    typed_array!(crate::BloombergCodeType, arrow_array::StringArray);
    typed_array!(crate::SideType, arrow_array::StringArray);
    typed_array!(crate::StateType, arrow_array::StringArray);
    typed_array!(crate::TimeInForceType, arrow_array::StringArray);
    // A UUID stores as the fixed binary of its sixteen bytes.
    typed_array!(crate::uuid::UuidType, arrow_array::FixedSizeBinaryArray);
    // The sequence family covers five layouts whose arrays genuinely differ,
    // so it names none of them; a struct is one struct array.
    typed_array!(crate::SequenceType, ArrayRef);
    typed_array!(crate::StructType, arrow_array::StructArray);
    typed_array!(crate::UnionType, arrow_array::UnionArray);
    typed_array!(crate::MappingType, arrow_array::MapArray);
    // A variant's storage is the struct of its two binaries, and a
    // geospatial value is its WKB payload, so their physical arrays are
    // fixed.
    typed_array!(crate::VariantType, arrow_array::StructArray);
    typed_array!(crate::GeometryType, arrow_array::BinaryArray);
    typed_array!(crate::GeographyType, arrow_array::BinaryArray);

    // A leaf and its unit decide the physical width of a temporal value, a key
    // type decides the physical width of a dictionary index, a string's layout
    // and charset decide which text or byte array holds it and a byte layout
    // which binary array, so these have no single array type.
    opaque_array!(crate::string::StringType);
    opaque_array!(crate::bytes::BytesType);
    opaque_array!(crate::DateTimeType);
    opaque_array!(crate::TimeType);
    opaque_array!(crate::DurationType);
    opaque_array!(crate::IntervalType);
    opaque_array!(crate::EnumType);
    opaque_array!(crate::RunEndType);

    impl<D: ArrowFieldType> crate::FieldOf<D> {
        /// Cast an incoming Arrow array to this field, returning its exact array.
        ///
        /// The field is the target: `array` is reconciled to the field's datatype
        /// and nullability. [`ArrowCastOptions`] carries both answers - whether a
        /// failed conversion becomes null, and whether a null a non-nullable field
        /// cannot hold takes its canonical default or is refused by path.
        ///
        /// # Errors
        ///
        /// Returns an error for an unsupported cast, a value that cannot satisfy
        /// the field, or a default that cannot be materialized.
        pub fn cast_arrow_array(
            &self,
            array: ArrayRef,
            options: ArrowCastOptions,
        ) -> Result<D::Array> {
            D::downcast_array(self.to_field().cast_arrow_array(array, options)?)
        }

        /// Cast a one-element Arrow array to this field as a typed scalar.
        ///
        /// # Errors
        ///
        /// Returns an error when `array` does not hold exactly one value, or any
        /// error [`Self::cast_arrow_array`] returns.
        pub fn cast_arrow_scalar(
            &self,
            array: ArrayRef,
            options: ArrowCastOptions,
        ) -> Result<Scalar<D::Array>> {
            if array.len() != 1 {
                return Err(Error::IncompatibleSchema(format!(
                    "expected exactly 1 value to cast as a scalar, got {}",
                    array.len()
                )));
            }
            Ok(Scalar::new(self.cast_arrow_array(array, options)?))
        }
    }
}

/// Casts one Arrow array to a datatype, with no field around it.
///
/// # Errors
///
/// Returns an error for an unsupported cast or a value the datatype cannot
/// hold.
pub(crate) fn cast_dtype_arrow_array(
    dtype: &DataType,
    array: ArrayRef,
    options: ArrowCastOptions,
) -> Result<ArrayRef> {
    let plan = ArrayCastPlan::new_dtype(dtype, array.data_type(), options)?;
    let mut budget = MaterializationBudget::default();
    plan.cast(array, &mut budget)
}

/// Casts one Arrow array to a field: its datatype and its nullability.
///
/// # Errors
///
/// Returns an error for an unsupported cast, a value that cannot satisfy the
/// field, or a default that cannot be materialized.
pub(crate) fn cast_field_arrow_array(
    field: &Field,
    array: ArrayRef,
    options: ArrowCastOptions,
) -> Result<ArrayRef> {
    cast_field_array(field, None, array, options)
}

/// Reconciles one Arrow record batch to a non-null Struct root.
///
/// # Errors
///
/// Returns an error unless the field is a Struct schema, or when a child cast
/// or missing-column default cannot be materialized.
pub(crate) fn cast_field_arrow_batch(
    field: &Field,
    batch: RecordBatch,
    options: ArrowCastOptions,
) -> Result<RecordBatch> {
    ArrowCastPlan::compile(batch.schema_ref(), field, options)?.apply(batch)
}

/// Wraps a reader so every batch it yields is reconciled to one root.
///
/// The streaming form of [`cast_field_arrow_batch`], and a reader for the
/// reason every streaming shape in this crate is one: the plan is compiled
/// once from the reader's own schema, so the returned reader answers the cast
/// schema before the first batch is pulled and no batch is planned for twice.
/// A reader already carrying the declared shape comes back as itself.
///
/// # Errors
///
/// Returns an error when the cast cannot be planned from the reader's schema.
/// A failure on one batch surfaces as that batch's `Err`, and the reader is
/// not fused after it.
pub(crate) fn cast_field_arrow_reader(
    field: &Field,
    reader: crate::arrow::BatchReader,
    options: ArrowCastOptions,
) -> Result<crate::arrow::BatchReader> {
    crate::arrow::cast_reader(reader, field, options)
}

/// Refuses an array that is not the one row a scalar cast takes.
///
/// # Errors
///
/// Returns an error naming the row count.
pub(crate) fn one_row(array: &ArrayRef) -> Result<()> {
    if array.len() == 1 {
        return Ok(());
    }
    Err(Error::IncompatibleSchema(format!(
        "a scalar cast takes exactly one row, got {}",
        array.len()
    )))
}

/// View a batch as one struct array, keeping the row count of an empty batch.
pub(crate) fn struct_array_from_batch(batch: RecordBatch) -> ArrayRef {
    if batch.num_columns() == 0 {
        return Arc::new(StructArray::new_empty_fields(batch.num_rows(), None));
    }
    Arc::new(StructArray::from(batch))
}

#[derive(Clone, Copy)]
enum NullPolicy {
    Field,
    DataType,
    Reject,
}

#[derive(Clone, Copy)]
enum StructPolicy {
    Normal,
    MapEntries,
}

/// The declaring protocols that will materialize a column after a cast.
///
/// A column a protocol fills is allowed to arrive absent or holding its
/// canonical default, because closing that hole is the protocol's job and it
/// has not run yet. Strictness therefore stops at such a field and resumes for
/// every other one; the applied batch is checked again once the protocols are
/// done.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Deferred {
    /// A `TRANSFORM:` or partition declaration derives the column from others.
    pub(crate) transform: bool,
    /// `DIGEST:role=holder` says the column holds the row's hash.
    pub(crate) digest: bool,
}

impl Deferred {
    /// Returns whether an enabled protocol fills this field after the cast.
    fn defers(self, field: &Field) -> Result<bool> {
        if self.digest && field.as_digest().is_holder() {
            return Ok(true);
        }
        Ok(self.transform && field.as_transform().is_derived())
    }
}

/// What a child node inherits from the parent that planned it.
#[derive(Clone, Copy)]
struct PlanRules {
    options: ArrowCastOptions,
    deferred: Deferred,
    null_policy: NullPolicy,
    struct_policy: StructPolicy,
}

impl PlanRules {
    /// The rules a nested child inherits unchanged from its parent.
    const fn nested(options: ArrowCastOptions, deferred: Deferred) -> Self {
        Self {
            options,
            deferred,
            null_policy: NullPolicy::Field,
            struct_policy: StructPolicy::Normal,
        }
    }

    const fn with_null_policy(mut self, null_policy: NullPolicy) -> Self {
        self.null_policy = null_policy;
        self
    }

    const fn with_struct_policy(mut self, struct_policy: StructPolicy) -> Self {
        self.struct_policy = struct_policy;
        self
    }

    /// The rules for one struct child, with strictness dropped where an
    /// enabled protocol is about to fill the column itself.
    fn child(self, field: &Field) -> Result<Self> {
        let options = if self.deferred.defers(field)? {
            self.options.deferred()
        } else {
            self.options
        };
        Ok(Self::nested(options, self.deferred))
    }
}

pub(crate) struct ArrayCastPlan {
    pub(crate) field: Field,
    pub(crate) source_type: ArrowDataType,
    pub(crate) expected: ArrowDataType,
    pub(crate) options: ArrowCastOptions,
    /// The dot/bracket path from the cast root, rendered once at compile time.
    path: SmolStr,
    null_policy: NullPolicy,
    kind: ArrayCastKind,
    /// Whether this node turns an exposed empty text cell null before reading
    /// it, decided once at compile time.
    ///
    /// The empty-cell rule: `""` entering a column that does not keep it is
    /// absence, decided before any spelling is parsed and before `safe` is
    /// asked, so `nullability` alone says what a required column does with
    /// it. The node that reads the values runs it once: a layout node - an
    /// encoding target, a decoded source, a dictionary pair, a run-end pair -
    /// hands the values to a node of its own.
    blanks_text: bool,
}

impl ArrayCastPlan {
    /// Whether a failed conversion becomes null rather than an error.
    pub(crate) const fn safe(&self) -> bool {
        self.options.is_safe()
    }
}

enum ArrayCastKind {
    Exact,
    Kernel,
    /// Two fixed-width layouts of the same byte width read as one another:
    /// the value buffer and the null buffer are handed over unchanged and only
    /// the datatype that reads them differs.
    BitCast,
    /// One byte layout read as another through Arrow's `Binary`: the two
    /// framings hold the same payload, and `Binary` is the framing Arrow
    /// converts both of them to, so the reading is two of its own casts.
    ByteBridge,
    /// Bytes entering a geometry or geography: same bytes out, but every
    /// exposed value is validated as WKB on the way in. A non-Binary binary
    /// layout is first cast to the Binary storage through Arrow's kernel.
    GeospatialIngest,
    /// A recognized geospatial source rendering as WKT text.
    GeospatialWkt,
    /// Values entering a string: every exposed value is read under what the
    /// source declares, restated under the target's layout, charset and
    /// bound, and written into the target's own storage. A recognized string
    /// or code source is read under its own parameters; bare text is read as
    /// text and bare bytes as bytes already in the target's charset.
    StringIngest {
        source: StringSource,
    },
    /// Values entering a bounded byte layout: every exposed value is
    /// measured against the maximum Arrow has nowhere to state, and written
    /// into the target's own layout. An unbounded layout declares nothing
    /// Arrow does not, so it stays with the kernel.
    BytesIngest,
    /// Values entering a registered code: the string rule at the width its
    /// standard fixes, which is a constant, so the validation runs
    /// monomorphized per code rather than reading a width out of the datatype
    /// on every row, and a column that already holds what the code promises
    /// is shared rather than copied.
    CodeIngest,
    /// Values entering a UUID: every exposed value is validated under the one
    /// UUID rule and stored as its sixteen bytes.
    UuidIngest,
    /// Text entering a version is parsed and rewritten to its canonical text.
    VersionIngest,
    /// Text entering a URL is parsed and rewritten to its canonical text.
    UrlIngest,
    /// Text entering a URN is parsed and rewritten to its canonical text.
    UrnIngest,
    TimezoneIngest,
    MimeTypeIngest,
    MediaTypeIngest,
    /// Text entering a decimal: every exposed value is read at the declared
    /// scale, and a digit that scale cannot state stays refused rather than
    /// rounded away - dropping a digit off a price is a value change.
    DecimalIngest,
    /// Text entering a temporal: every exposed value is read through the
    /// crate's own spellings, which are wider than Arrow's. Arrow's kernel
    /// stays behind them for the spellings only it knows, so the reading is
    /// never narrower than it was.
    TemporalIngest,
    /// A temporal rendering as its classic text, so a column spells what a
    /// row spells - a zoned instant included, which Arrow's own formatter
    /// refuses without its timezone database.
    TemporalText,
    DeferredUnsupported {
        reason: String,
    },
    Struct {
        fields: arrow_schema::Fields,
        columns: Vec<StructColumnPlan>,
    },
    List {
        field: ArrowFieldRef,
        child: Box<ArrayCastPlan>,
        kind: ListPlanKind,
    },
    Map {
        source: crate::MappingType,
        field: ArrowFieldRef,
        ordered: bool,
        entries: Box<ArrayCastPlan>,
    },
    Dictionary {
        source_key: ArrowDataType,
        values: Box<ArrayCastPlan>,
    },
    /// A source that is not already this target's encoding: the values are
    /// planned against it under the ordinary rules and the encoding is the
    /// tail, because an encoding is a layout and not a reading.
    Encoded {
        values: Box<ArrayCastPlan>,
    },
    /// The mirror: an encoded source read by a target that is not that
    /// encoding. The values are decoded first, so the cast sees the column the
    /// encoding was hiding - its extension identity included.
    Decoded {
        decoded: ArrowDataType,
        plan: Box<ArrayCastPlan>,
    },
    Union {
        fields: arrow_schema::UnionFields,
        children: Vec<(i8, ArrayCastPlan)>,
    },
    RunEndEncoded {
        source_run_type: ArrowDataType,
        values: Box<ArrayCastPlan>,
    },
}

pub(crate) enum StructColumnPlan {
    Source {
        index: usize,
        cast: ArrayCastPlan,
    },
    /// A target column no source carries, with its canonical one-row default
    /// already materialized.
    ///
    /// That default is a function of the Field alone, but building it runs a
    /// schema preflight and a full `Field::validate` - which the plan already
    /// did for this exact Field - and then materializes a Scalar and a one-row
    /// Arrow array. Paying that per batch is the repeated schema validation the
    /// optimization contract forbids on a record path, so it is paid once here.
    /// A Field whose default cannot be materialized keeps `None` and raises the
    /// same failure from the same place it always did, on the batch.
    Missing {
        field: Field,
        default: Option<ArrayRef>,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum ListPlanKind {
    List,
    LargeList,
    ListView,
    LargeListView,
    FixedSize { size: i32 },
}

impl ArrayCastPlan {
    fn new_dtype(
        dtype: &DataType,
        source_type: &ArrowDataType,
        options: ArrowCastOptions,
    ) -> Result<Self> {
        dtype.validate_bounded()?;
        Self::new_validated_with(
            &Field::new("value", dtype.clone(), false),
            source_type,
            None,
            PlanRules::nested(options, Deferred::default()).with_null_policy(NullPolicy::DataType),
            Path::root(),
        )
    }

    /// Plans the root of a record-batch cast, where the batch's own schema is
    /// the source struct and `$` is the path every child hangs from.
    pub(crate) fn new_root(
        field: &Field,
        source_type: &ArrowDataType,
        options: ArrowCastOptions,
        deferred: Deferred,
    ) -> Result<Self> {
        field.validate_bounded()?;
        Self::new_validated_with(
            field,
            source_type,
            None,
            PlanRules::nested(options, deferred),
            Path::root(),
        )
    }

    fn new_nested_validated(
        field: &Field,
        source_type: &ArrowDataType,
        rules: PlanRules,
        path: Path<'_>,
    ) -> Result<Self> {
        Self::new_validated_with(field, source_type, None, rules, path)
    }

    /// Plans a nested cast whose source is a complete Arrow field, so the
    /// source's extension identity (a variant, a geometry, a geography)
    /// participates in the declared cast rules rather than being erased to
    /// its storage type.
    fn new_nested_from_arrow_field(
        field: &Field,
        source_field: &ArrowFieldRef,
        rules: PlanRules,
        path: Path<'_>,
    ) -> Result<Self> {
        Self::new_validated_with(
            field,
            source_field.data_type(),
            Some(source_field.metadata()),
            rules,
            path,
        )
    }

    fn new_validated_with(
        field: &Field,
        source_type: &ArrowDataType,
        source_metadata: Option<&HashMap<String, String>>,
        rules: PlanRules,
        path: Path<'_>,
    ) -> Result<Self> {
        let PlanRules {
            options,
            null_policy,
            ..
        } = rules;
        let source_extension = match source_metadata {
            Some(metadata) => recognized_arrow_extension(metadata, source_type)?,
            None => None,
        };
        check_extension_source(field, source_extension.as_ref())?;
        let expected = field.clone().into_arrow_field_ref()?.data_type().clone();
        // A geospatial target validates WKB on the way in and a string that
        // declares more than Arrow can say - a charset, a bound - validates
        // text, so an exact storage source must still take the planned path,
        // unless it is a recognized source of exactly this datatype, already
        // validated when it was written. Plain UTF-8 declares nothing Arrow
        // does not, so its storage is its value; the same holds for bytes,
        // where only a maximum says more than the layout.
        let ingest_validated = match field.dtype() {
            DataType::Geometry(_) | DataType::Geography(_) => true,
            DataType::String(parameters) => {
                needs_extension(*parameters)
                    && !matches!(
                        source_extension.as_ref(),
                        Some(RecognizedExtension::String(source)) if source == field.dtype()
                    )
            }
            // A fixed width is not re-read: the storage Arrow declares is the
            // width, so a column already in it holds nothing to check.
            DataType::Bytes(parameters) => {
                crate::bytes::needs_extension(*parameters)
                    && !matches!(
                        source_extension.as_ref(),
                        Some(RecognizedExtension::Bytes(source)) if source == field.dtype()
                    )
            }
            // The same rule for a code, over its own extension: a currency
            // column written as a currency is already validated, and one
            // written as three anonymous bytes is not.
            code if code.is_code() => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::Code(source)) if source == field.dtype()
            ),
            DataType::Uuid => !matches!(source_extension.as_ref(), Some(RecognizedExtension::Uuid)),
            DataType::Version => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::Version)
            ),
            DataType::Uri(UriType::Url) => {
                !matches!(source_extension.as_ref(), Some(RecognizedExtension::Url))
            }
            DataType::Uri(UriType::Urn) => {
                !matches!(source_extension.as_ref(), Some(RecognizedExtension::Urn))
            }
            DataType::Timezone => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::Timezone)
            ),
            DataType::MimeType => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::MimeType)
            ),
            DataType::MediaType => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::MediaType)
            ),
            _ => false,
        };
        let kind = if source_type == &expected
            && !is_reconcilable_nested(field.dtype())
            && !ingest_validated
        {
            ArrayCastKind::Exact
        // Asked for the bytes, and the two layouts really are the same bytes:
        // nothing is converted, so no conversion rule applies. The same guard
        // that forces a validating target onto its own path excludes it here
        // too - a bounded string or a code is a rule about values, and sharing
        // a buffer past it would store bytes the datatype promises are not there.
        } else if options.representation().is_bits()
            && !ingest_validated
            && same_bit_layout(source_type, &expected)
        {
            ArrayCastKind::BitCast
        } else {
            Self::nested_kind(
                field,
                source_type,
                source_metadata,
                source_extension.as_ref(),
                &expected,
                rules,
                path,
            )?
        };
        // An exact source is a column already written as this datatype, so
        // it holds no cell the datatype refuses and is shared unread, as a
        // same-type cast costs nothing.
        let blanks_text = holds_text(source_type)
            && !matches!(
                kind,
                ArrayCastKind::Exact
                    | ArrayCastKind::Encoded { .. }
                    | ArrayCastKind::Decoded { .. }
                    | ArrayCastKind::Dictionary { .. }
                    | ArrayCastKind::RunEndEncoded { .. }
            )
            && !keeps_empty_text(field.dtype());
        Ok(Self {
            field: field.clone(),
            source_type: source_type.clone(),
            expected,
            options,
            path: SmolStr::from(path.render()),
            null_policy,
            kind,
            blanks_text,
        })
    }

    #[allow(clippy::too_many_lines)] // Exhaustive Arrow wrapper dispatch is clearest together.
    fn nested_kind(
        field: &Field,
        source_type: &ArrowDataType,
        source_metadata: Option<&HashMap<String, String>>,
        source_extension: Option<&RecognizedExtension>,
        expected: &ArrowDataType,
        rules: PlanRules,
        path: Path<'_>,
    ) -> Result<ArrayCastKind> {
        let PlanRules {
            options,
            struct_policy,
            ..
        } = rules;
        // Every wrapper below plans exactly one child under the ordinary
        // rules, so the inherited rules and the child paths are named once.
        let nested = PlanRules::nested(options, rules.deferred);
        let item = path.child(Segment::Item);
        let entries = path.child(Segment::MapEntries);
        let dictionary_value = path.child(Segment::DictionaryValue);
        let run_end_values = path.child(Segment::RunEndValues);
        let dtype = field.dtype();
        let kind = match (dtype, source_type) {
            // The extension-typed variants follow declared rules, never the
            // positional kernel: WKB is validated entering a geospatial
            // column, text is validated entering a declared string, WKT needs
            // a parser this workspace deliberately lacks, and a variant's
            // binary encoding lands with the Iceberg v3 layer, so only the
            // identity works until then.
            (DataType::Geometry(_) | DataType::Geography(_), source) => match source {
                ArrowDataType::Binary
                | ArrowDataType::LargeBinary
                | ArrowDataType::BinaryView
                | ArrowDataType::FixedSizeBinary(_) => ArrayCastKind::GeospatialIngest,
                // Deferred rather than refused at compile time: a text
                // column, however it is laid out, holding no visible value -
                // every cell empty, and so absent under the empty-cell rule -
                // is the null policy's to answer, and the refusal waits for
                // the first exposed value.
                source if holds_text(source) => ArrayCastKind::DeferredUnsupported {
                    reason: format!(
                        "casting text to {} would need the WKT parser this workspace \
                             deliberately does not have yet",
                        dtype.name()
                    ),
                },
                other => {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "expected a binary column of WKB payloads to cast into {}, got {other:?}",
                            dtype.name()
                        ),
                    });
                }
            },
            (DataType::Variant, source) => {
                if crate::is_variant_storage(source) {
                    // The identity: the same binary, holding the encoding.
                    ArrayCastKind::Kernel
                } else {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "casting {source:?} to variant: a variant column is the \
                             struct of `metadata` and `value` binaries the variant \
                             encoding writes, which `Variant::encode` fills"
                        ),
                    });
                }
            }
            // A code takes binary directly and everything the kernel renders
            // as text through one Utf8 temporary, at the width its own
            // standard fixes; the rule is checked per value either way, and
            // a column that already holds what the code promises is shared
            // rather than copied.
            (code, source) if code.is_code() => {
                if matches!(source, ArrowDataType::FixedSizeBinary(_))
                    || can_cast_types(source, &ArrowDataType::Utf8)
                {
                    ArrayCastKind::CodeIngest
                } else {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "expected a fixed binary or a column Arrow renders as utf8 to cast \
                             into {}, got {source:?}",
                            dtype.name()
                        ),
                    });
                }
            }
            // The renderings below spell a value as text and write it into
            // the target's text storage as it is, so they take only a string
            // whose storage is text and whose bound has nothing to check.
            (DataType::String(parameters), ArrowDataType::Binary)
                if is_text_storage(*parameters)
                    && !parameters.is_bounded()
                    && matches!(source_extension, Some(RecognizedExtension::Geospatial(_))) =>
            {
                ArrayCastKind::GeospatialWkt
            }
            // A UUID takes its sixteen bytes directly, a slot of any other
            // width as the spelling that slot holds, and every text spelling
            // through one Utf8 temporary; the one UUID rule runs per value
            // whichever it was.
            (DataType::Uuid, source) => {
                if matches!(source, ArrowDataType::FixedSizeBinary(_))
                    || can_cast_types(source, &ArrowDataType::Utf8)
                {
                    ArrayCastKind::UuidIngest
                } else {
                    ArrayCastKind::DeferredUnsupported {
                        reason: format!("casting {source:?} to uuid is not supported"),
                    }
                }
            }
            (DataType::Version, source) if is_text_layout(source) => ArrayCastKind::VersionIngest,
            (DataType::Version, source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to version is not supported"),
            },
            (DataType::Uri(UriType::Url), source) if is_text_layout(source) => {
                ArrayCastKind::UrlIngest
            }
            (DataType::Uri(UriType::Url), source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to url is not supported"),
            },
            (DataType::Uri(UriType::Urn), source) if is_text_layout(source) => {
                ArrayCastKind::UrnIngest
            }
            (DataType::Uri(UriType::Urn), source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to urn is not supported"),
            },
            (DataType::Timezone, source) if is_text_layout(source) => ArrayCastKind::TimezoneIngest,
            (DataType::Timezone, source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to timezone is not supported"),
            },
            (DataType::MimeType, source) if is_text_layout(source) => ArrayCastKind::MimeTypeIngest,
            (DataType::MimeType, source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to mimetype is not supported"),
            },
            (DataType::MediaType, source) if is_text_layout(source) => {
                ArrayCastKind::MediaTypeIngest
            }
            (DataType::MediaType, source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to mediatype is not supported"),
            },
            (DataType::String(parameters), source)
                if is_text_storage(*parameters)
                    && !parameters.is_bounded()
                    && is_temporal_arrow(source) =>
            {
                ArrayCastKind::TemporalText
            }
            // A string reads its values, never its buffers: a recognized
            // string, code or UUID source is read under what it declares -
            // a UUID spelling its sixteen bytes as the identifier they name -
            // bare text as text, and bare bytes as bytes already in the
            // target's charset. Plain UTF-8 from bare storage declares
            // nothing to check, so it stays with Arrow's own kernel below; an
            // encoded source is decoded first, by the arms below, so the
            // reading sees the column the encoding was hiding.
            (DataType::String(parameters), source)
                if !matches!(
                    source,
                    ArrowDataType::Dictionary(..) | ArrowDataType::RunEndEncoded(..)
                ) && (needs_extension(*parameters)
                    || matches!(
                        source_extension,
                        Some(
                            RecognizedExtension::String(_)
                                | RecognizedExtension::Code(_)
                                | RecognizedExtension::Uuid
                        )
                    )) =>
            {
                if !(matches!(
                    source,
                    ArrowDataType::Binary
                        | ArrowDataType::LargeBinary
                        | ArrowDataType::BinaryView
                        | ArrowDataType::FixedSizeBinary(_)
                ) || can_cast_types(source, &ArrowDataType::Utf8))
                {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "expected a binary column or a column Arrow renders as utf8 to \
                             cast into {dtype}, got {source:?}"
                        ),
                    });
                }
                ArrayCastKind::StringIngest {
                    source: match source_extension {
                        Some(RecognizedExtension::String(DataType::String(parameters))) => {
                            StringSource::String(*parameters)
                        }
                        Some(RecognizedExtension::Code(code)) => StringSource::Code(code.clone()),
                        Some(RecognizedExtension::Uuid) => StringSource::Uuid,
                        _ => StringSource::Bare,
                    },
                }
            }
            // Two fixed byte widths are the same refusal a fixed-size list
            // pair gets: the payload a row holds is not the payload the target
            // declares, so it is a value change rather than a framing change,
            // and Arrow's own message names neither datatype.
            (DataType::Bytes(parameters), ArrowDataType::FixedSizeBinary(source_width))
                if parameters
                    .fixed()
                    .is_some_and(|width| u32::try_from(*source_width) != Ok(width)) =>
            {
                return Err(Error::Unsupported {
                    kind: dtype.name(),
                    reason: format!(
                        "a fixed binary of {source_width} bytes holds a different payload than \
                         {dtype}, so it is a value change rather than a framing change"
                    ),
                });
            }
            // A bounded byte layout reads its cells for the same reason: the
            // width a value must fit is a rule about values, and Arrow's own
            // builder refuses a cell that misses it without naming the column
            // or the row. A maximum is the one thing about bytes Arrow cannot
            // check at all; a fixed width it checks, but only with a message
            // that names neither side. An encoded source is decoded first, by
            // the arms below.
            (DataType::Bytes(parameters), source)
                if parameters.is_bounded()
                    && !matches!(
                        source,
                        ArrowDataType::Dictionary(..) | ArrowDataType::RunEndEncoded(..)
                    ) =>
            {
                if !(matches!(source, ArrowDataType::FixedSizeBinary(_))
                    || can_cast_types(source, &ArrowDataType::Binary))
                {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "expected a column Arrow reads as binary to cast into {dtype}, \
                             got {source:?}"
                        ),
                    });
                }
                ArrayCastKind::BytesIngest
            }
            // A temporal reads text with this crate's spellings rather than
            // Arrow's: a comma decimal sign, a grouped fraction, an hour past
            // the end of the day, a bracketed zone name, and a duration in
            // either spelling all read here, and Arrow reads nothing into a
            // duration at all. An
            // encoded column reads its values the same way and is encoded
            // afterwards, because the encoding is a layout, not a reading.
            (target, source) if holds_temporal(target) && holds_text(source) => {
                ArrayCastKind::TemporalIngest
            }
            // A decimal reads text the same way and for the same reason: the
            // spelling carries a precision the declared scale may not hold,
            // and this crate refuses that where Arrow rounds it.
            (target, source) if holds_decimal(target) && holds_text(source) => {
                ArrayCastKind::DecimalIngest
            }
            (DataType::Struct(fields), ArrowDataType::Struct(source_fields)) => {
                let ArrowDataType::Struct(target_fields) = expected else {
                    return Err(internal_target_error("struct"));
                };
                let mapping = folded_field_mapping(source_fields, fields.as_fields())?;
                let mut columns = Vec::with_capacity(fields.len());
                for (target_index, (target, source_index)) in fields.iter().zip(mapping).enumerate()
                {
                    let child_rules = rules.child(target)?;
                    let child_path = path.field(target.name());
                    let column = match source_index {
                        Some(index) => {
                            let child_rules = if matches!(struct_policy, StructPolicy::MapEntries)
                                && target_index == 0
                            {
                                child_rules.with_null_policy(NullPolicy::Reject)
                            } else {
                                child_rules
                            };
                            StructColumnPlan::Source {
                                index,
                                cast: Self::new_validated_with(
                                    target,
                                    source_fields[index].data_type(),
                                    Some(source_fields[index].metadata()),
                                    child_rules,
                                    child_path,
                                )?,
                            }
                        }
                        None if matches!(struct_policy, StructPolicy::MapEntries)
                            && target_index == 0 =>
                        {
                            return Err(Error::IncompatibleSchema(
                                "map key field is missing from the source entries Struct"
                                    .to_owned(),
                            ));
                        }
                        // A required column no source carries is the hole the
                        // default policy fills and strictness refuses: the
                        // schemas alone answer it, so it fails at compile time
                        // rather than on the first batch.
                        None if child_rules.options.nullability().is_strict()
                            && !target.is_nullable() =>
                        {
                            return Err(Error::RequiredField {
                                path: SmolStr::from(child_path.render()),
                                nulls: None,
                            });
                        }
                        // A nullable hole is a null column, which needs no
                        // default; anything else fills with one, so it is built
                        // here rather than on every batch. A failure stays for
                        // the batch that asks, so nothing fails earlier than it
                        // used to.
                        None => StructColumnPlan::Missing {
                            field: target.clone(),
                            default: if target.is_nullable() {
                                None
                            } else {
                                target.default_arrow_array().ok()
                            },
                        },
                    };
                    columns.push(column);
                }
                ArrayCastKind::Struct {
                    fields: target_fields.clone(),
                    columns,
                }
            }
            // Every list layout reads every other one. The child is planned
            // against the source's own child field, so Struct children still
            // reconcile by name, and the array is rebuilt in the source's
            // layout; the layout itself is the tail cast, which Arrow's kernel
            // performs over a child that already matches the target. Only two
            // fixed sizes are a different row shape rather than a layout, and
            // that pair is refused below by name.
            (
                DataType::Sequence(SequenceType::List(child))
                | DataType::Sequence(SequenceType::LargeList(child))
                | DataType::Sequence(SequenceType::ListView(child))
                | DataType::Sequence(SequenceType::LargeListView(child))
                | DataType::Sequence(SequenceType::FixedSizeList(child, _)),
                ArrowDataType::List(source_child)
                | ArrowDataType::LargeList(source_child)
                | ArrowDataType::ListView(source_child)
                | ArrowDataType::LargeListView(source_child)
                | ArrowDataType::FixedSizeList(source_child, _),
            ) => {
                if let (
                    DataType::Sequence(SequenceType::FixedSizeList(_, size)),
                    ArrowDataType::FixedSizeList(_, source),
                ) = (dtype, source_type)
                {
                    if size != source {
                        return Err(Error::Unsupported {
                            kind: dtype.name(),
                            reason: format!(
                                "a fixed-size list of {source} items holds a different row than \
                                 one of {size} items, so it is a value change rather than a \
                                 layout change"
                            ),
                        });
                    }
                }
                ArrayCastKind::List {
                    field: list_child(expected)?,
                    child: Box::new(Self::new_nested_from_arrow_field(
                        child,
                        source_child,
                        nested,
                        item,
                    )?),
                    kind: source_list_kind(source_type)?,
                }
            }
            (DataType::Mapping(map), ArrowDataType::Map(source_entries, _)) => {
                let ArrowDataType::Map(target_entries, ordered) = expected else {
                    return Err(internal_target_error("map"));
                };
                let DataType::Mapping(source) = DataType::from_arrow_datatype(source_type)? else {
                    return Err(Error::IncompatibleSchema(
                        "source Arrow Map did not import as a Map datatype".to_owned(),
                    ));
                };
                ArrayCastKind::Map {
                    source,
                    field: Arc::clone(target_entries),
                    ordered: *ordered,
                    entries: Box::new(Self::new_validated_with(
                        map.entries(),
                        source_entries.data_type(),
                        Some(source_entries.metadata()),
                        nested
                            .with_null_policy(NullPolicy::Reject)
                            .with_struct_policy(StructPolicy::MapEntries),
                        entries,
                    )?),
                }
            }
            (
                DataType::Enum(EnumType::Dictionary(dictionary)),
                ArrowDataType::Dictionary(source_key, source_value),
            ) => ArrayCastKind::Dictionary {
                source_key: source_key.as_ref().clone(),
                values: Box::new(Self::new_nested_validated(
                    &Field::new("values", dictionary.value().clone(), true),
                    source_value,
                    nested,
                    dictionary_value,
                )?),
            },
            (DataType::Union(fields, mode), ArrowDataType::Union(source_fields, source_mode))
                if union_mode_matches(*mode, *source_mode) =>
            {
                if fields.len() != source_fields.len()
                    || source_fields
                        .iter()
                        .any(|(id, _)| !fields.iter().any(|(target_id, _)| target_id == id))
                {
                    return Err(Error::IncompatibleSchema(
                        "source and target union type-ID sets must match exactly".to_owned(),
                    ));
                }
                let ArrowDataType::Union(target_fields, _) = expected else {
                    return Err(internal_target_error("union"));
                };
                let mut children = Vec::with_capacity(fields.len());
                for (type_id, target) in fields.iter() {
                    let source = source_fields
                        .iter()
                        .find_map(|(id, field)| (id == type_id).then_some(field))
                        .ok_or_else(|| {
                            Error::IncompatibleSchema(format!(
                                "source union is missing target type ID {type_id}"
                            ))
                        })?;
                    let branch = path.child(Segment::UnionType(type_id));
                    children.push((
                        type_id,
                        Self::new_nested_from_arrow_field(target, source, nested, branch)?,
                    ));
                }
                ArrayCastKind::Union {
                    fields: target_fields.clone(),
                    children,
                }
            }
            (
                DataType::RunEndEncoded(encoded),
                ArrowDataType::RunEndEncoded(source_runs, source_values),
            ) if source_runs.data_type()
                == encoded
                    .run_ends()
                    .clone()
                    .into_arrow_field_ref()?
                    .data_type() =>
            {
                ArrayCastKind::RunEndEncoded {
                    source_run_type: source_runs.data_type().clone(),
                    values: Box::new(Self::new_nested_from_arrow_field(
                        encoded.values(),
                        source_values,
                        nested,
                        run_end_values,
                    )?),
                }
            }
            // An encoding is a layout, not a reading: a dictionary or a
            // run-end column holds exactly what its value type holds. Planning
            // the values against the source keeps every leaf rule - an ASCII
            // width, a code, a UUID, a version, WKB - which handing the whole
            // wrapper to Arrow's kernel silently skipped. The two arms above
            // stay ahead of this one because a source already in this encoding
            // is re-encoded rather than decoded and rebuilt.
            (DataType::Enum(EnumType::Dictionary(dictionary)), _) => ArrayCastKind::Encoded {
                values: Box::new(Self::new_nested_validated(
                    &Field::new("values", dictionary.value().clone(), true),
                    source_type,
                    nested,
                    dictionary_value,
                )?),
            },
            (DataType::RunEndEncoded(encoded), _) => ArrayCastKind::Encoded {
                values: Box::new(Self::new_nested_validated(
                    encoded.values(),
                    source_type,
                    nested,
                    run_end_values,
                )?),
            },
            // An encoding hides a column: a target that is not that encoding
            // reads the column rather than the index, so the values are decoded
            // and then planned under the ordinary rules. This stays ahead of
            // the Struct guard below because the decoded source is planned by
            // name, which is exactly what that guard exists to protect.
            (_, ArrowDataType::Dictionary(_, values)) => {
                Self::decoded_kind(field, values, source_metadata, rules, path)?
            }
            (_, ArrowDataType::RunEndEncoded(_, values)) => {
                Self::decoded_kind(field, values.data_type(), source_metadata, rules, path)?
            }
            _ if contains_struct(dtype) => {
                return Err(Error::Unsupported {
                    kind: dtype.name(),
                    reason: "a wrapper/layout change around Struct values is not supported because positional Arrow casting would bypass case-insensitive name reconciliation".to_owned(),
                });
            }
            // Anything Arrow's own kernel can cast, it casts - including the
            // wrapper and layout changes around non-Struct values: a list to
            // a view list, a fixed-size list to a variable one, a dictionary
            // encoded or decoded, a run-end array expanded. The Struct guard
            // above already refused the shapes where positional casting would
            // bypass name reconciliation, and the reservation walks nested
            // layouts, so the kernel's materialization stays budgeted.
            _ if can_cast_types(source_type, expected) => ArrayCastKind::Kernel,
            // A fixed binary and text, or a binary view and a fixed binary,
            // carry one payload under two framings that Arrow reads only
            // through its `Binary`. Taking that route is the same reading, so
            // the pair is supported rather than refused.
            _ if bridges_through_binary(source_type, expected) => ArrayCastKind::ByteBridge,
            // A pair neither Arrow nor the byte bridge reads is refused when a
            // value actually arrives under it. A wrapper can hide every row of
            // a child, and a hidden child that no reader would have taken is
            // not a failure, so the refusal waits for the first exposed value.
            _ => ArrayCastKind::DeferredUnsupported {
                reason: format!(
                    "Arrow cannot cast source datatype {source_type:?} to target datatype {expected:?}"
                ),
            },
        };
        Ok(kind)
    }

    /// Plans the column an encoding was hiding, so the cast reads values.
    fn decoded_kind(
        field: &Field,
        decoded: &ArrowDataType,
        source_metadata: Option<&HashMap<String, String>>,
        rules: PlanRules,
        path: Path<'_>,
    ) -> Result<ArrayCastKind> {
        Ok(ArrayCastKind::Decoded {
            plan: Box::new(Self::new_validated_with(
                field,
                decoded,
                source_metadata,
                rules,
                path,
            )?),
            decoded: decoded.clone(),
        })
    }

    fn cast(&self, array: ArrayRef, budget: &mut MaterializationBudget) -> Result<ArrayRef> {
        self.cast_exposed(array, None, budget)
    }

    #[allow(clippy::too_many_lines)] // Recursive Arrow dispatch and final null policy stay aligned.
    pub(crate) fn cast_exposed(
        &self,
        array: ArrayRef,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        if array.data_type() != &self.source_type {
            return Err(Error::IncompatibleSchema(format!(
                "array changed datatype after cast planning: expected {:?}, got {:?}",
                self.source_type,
                array.data_type()
            )));
        }
        if exposure.is_some_and(|exposure| exposure.len() != array.len()) {
            return Err(Error::IncompatibleSchema(
                "nested Arrow exposure mask length differs from its child array".to_owned(),
            ));
        }
        let array = if self.blanks_text {
            blank_text_as_null(&array, exposure, budget)?
        } else {
            array
        };
        let mut cast = match &self.kind {
            ArrayCastKind::Exact => array,
            ArrayCastKind::BitCast => reinterpret(&array, &self.expected)?,
            ArrayCastKind::Kernel => arrow_cast_exposed(
                &array,
                &self.expected,
                self.safe(),
                exposure,
                &self.field,
                budget,
            )?,
            ArrayCastKind::ByteBridge => {
                let bytes = arrow_cast_exposed(
                    &array,
                    &ArrowDataType::Binary,
                    self.safe(),
                    exposure,
                    &self.field,
                    budget,
                )?;
                arrow_cast_exposed(
                    &bytes,
                    &self.expected,
                    self.safe(),
                    exposure,
                    &self.field,
                    budget,
                )?
            }
            ArrayCastKind::GeospatialIngest => {
                let binary = if array.data_type() == &ArrowDataType::Binary {
                    array
                } else {
                    arrow_cast_exposed(
                        &array,
                        &ArrowDataType::Binary,
                        self.safe(),
                        exposure,
                        &self.field,
                        budget,
                    )?
                };
                validate_wkb_ingest(binary.as_ref(), &self.field, exposure)?;
                binary
            }
            ArrayCastKind::GeospatialWkt => {
                render_wkt_array(&array, &self.expected, &self.field, exposure, budget)?
            }
            ArrayCastKind::StringIngest { source } => {
                ingest_string_array(&array, source, self.safe(), &self.field, exposure, budget)?
            }
            ArrayCastKind::BytesIngest => {
                ingest_bytes_array(&array, self.safe(), &self.field, exposure, budget)?
            }
            ArrayCastKind::UuidIngest => ingest_uuid_array(
                &array,
                &self.expected,
                self.safe(),
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::VersionIngest => {
                ingest_version_array(&array, &self.field, exposure, budget)?
            }
            ArrayCastKind::UrlIngest => {
                crate::uri::casts::ingest_url_array(&array, &self.field, exposure, budget)?
            }
            ArrayCastKind::UrnIngest => {
                crate::uri::casts::ingest_urn_array(&array, &self.field, exposure, budget)?
            }
            ArrayCastKind::TimezoneIngest => crate::timezone::casts::ingest_timezone_array(
                &array,
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::MimeTypeIngest => crate::mime_type::casts::ingest_mime_type_array(
                &array,
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::MediaTypeIngest => crate::media_type::casts::ingest_media_type_array(
                &array,
                &self.field,
                exposure,
                budget,
            )?,
            // One match per array selects the code's width; every row after
            // it runs against a constant.
            ArrayCastKind::CodeIngest => match self.field.dtype() {
                DataType::Country => ingest_code_array::<COUNTRY_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Currency => ingest_code_array::<CURRENCY_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::MicCode => ingest_code_array::<MIC_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::CfiCode => ingest_code_array::<CFI_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::IsinCode => ingest_code_array::<ISIN_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::CusipCode => ingest_code_array::<CUSIP_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::SedolCode => ingest_code_array::<SEDOL_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::BloombergCode => ingest_code_array::<BLOOMBERG_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Side => ingest_code_array::<SIDE_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::State => ingest_code_array::<STATE_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::TimeInForce => ingest_code_array::<TIMEINFORCE_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                other => return Err(code_refusal(other).into()),
            },
            ArrayCastKind::TemporalText => {
                render_temporal_text(&array, self.safe(), &self.field, exposure, budget)?
            }
            ArrayCastKind::DecimalIngest => ingest_text_values(
                &array,
                &self.expected,
                self.safe(),
                &self.field,
                exposure,
                budget,
                Scalar::from_decimal_text,
            )?,
            ArrayCastKind::TemporalIngest => ingest_temporal_text(
                &array,
                &self.expected,
                self.safe(),
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::DeferredUnsupported { reason } => {
                let source_type = DataType::from_arrow_datatype(array.data_type())?;
                let exposed = exposure.map_or(array.len(), BooleanBuffer::count_set_bits);
                let logical_nulls =
                    exposed_logical_null_count(array.as_ref(), &source_type, exposure)?;
                let has_visible_value = exposed > logical_nulls;
                if has_visible_value {
                    return Err(Error::Unsupported {
                        kind: self.field.dtype().name(),
                        reason: reason.clone(),
                    });
                }
                // Absence only: a null column, so the one null policy below
                // repairs or refuses it as it does under every other node.
                budget.add_null_array(self.field.dtype(), array.len())?;
                arrow_array::new_null_array(&self.expected, array.len())
            }
            ArrayCastKind::Struct { fields, columns } => {
                self.cast_struct_array(array, fields, columns, exposure, budget)?
            }
            ArrayCastKind::List { field, child, kind } => {
                self.cast_list_array(array, field, child, *kind, exposure, budget)?
            }
            ArrayCastKind::Map {
                source,
                field,
                ordered,
                entries,
            } => self.cast_map_array(array, source, field, *ordered, entries, exposure, budget)?,
            ArrayCastKind::Dictionary { source_key, values } => {
                cast_dictionary_planned(source_key, self, array, values, exposure, budget)?
            }
            ArrayCastKind::Encoded { values } => {
                let read = values.cast_exposed(array, exposure, budget)?;
                let encoded = arrow_cast_exposed(
                    &read,
                    &self.expected,
                    self.safe(),
                    exposure,
                    &self.field,
                    budget,
                )?;
                // Arrow builds the encoding's own child fields, so a values
                // field carrying an extension identity comes back bare. The
                // buffers are the ones asked for either way, so they are read
                // back under the declared fields rather than rebuilt.
                if encoded.data_type() == &self.expected {
                    encoded
                } else {
                    reinterpret(&encoded, &self.expected)?
                }
            }
            ArrayCastKind::Decoded { decoded, plan } => {
                let values = arrow_cast_exposed(
                    &array,
                    decoded,
                    self.safe(),
                    exposure,
                    &self.field,
                    budget,
                )?;
                plan.cast_exposed(values, exposure, budget)?
            }
            ArrayCastKind::Union { fields, children } => {
                cast_union_planned(fields, array, children, exposure, budget)?
            }
            ArrayCastKind::RunEndEncoded {
                source_run_type,
                values,
            } => cast_run_planned(
                source_run_type,
                &self.expected,
                array,
                values,
                exposure,
                budget,
            )?,
        };
        if cast.data_type() != &self.expected {
            cast = arrow_cast_exposed(
                &cast,
                &self.expected,
                self.safe(),
                exposure,
                &self.field,
                budget,
            )?;
        }
        let null_count = exposed_logical_null_count(cast.as_ref(), self.field.dtype(), exposure)?;
        match self.null_policy {
            NullPolicy::Reject if null_count != 0 => {
                return Err(Error::IncompatibleSchema(format!(
                    "field {} contains {null_count} logical null values",
                    self.path
                )));
            }
            // A datatype target is a value of that type, so a null is absence
            // there too - unless null is the datatype's own only value.
            NullPolicy::DataType if null_count != 0 => {
                if self.refuses_absence() && !self.field.dtype().is_default_value(&Scalar::Null)? {
                    return Err(self.required_field(null_count));
                }
                cast = fill_nulls(&self.field, cast, true, exposure, budget)?;
            }
            NullPolicy::Field if !self.field.is_nullable() && null_count != 0 => {
                if self.refuses_absence() {
                    return Err(self.required_field(null_count));
                }
                cast = fill_nulls(&self.field, cast, false, exposure, budget)?;
            }
            NullPolicy::Field | NullPolicy::DataType | NullPolicy::Reject => {}
        }
        if cast.data_type() != &self.expected {
            return Err(Error::IncompatibleSchema(format!(
                "cast result datatype {:?} differs from target {:?}",
                cast.data_type(),
                self.expected
            )));
        }
        // Only a repaired column can still hold a hole, and only the arms
        // above repair one. A column that arrived with no absence at all was
        // handed on untouched, so recounting it would re-answer the count
        // taken a few lines up.
        let remaining_nulls = if null_count != 0
            && matches!(self.null_policy, NullPolicy::Field | NullPolicy::DataType)
            && (matches!(self.null_policy, NullPolicy::DataType) || !self.field.is_nullable())
        {
            exposed_logical_null_count(cast.as_ref(), self.field.dtype(), exposure)?
        } else {
            0
        };
        if remaining_nulls != 0 {
            let default = if matches!(self.null_policy, NullPolicy::DataType) {
                self.field.dtype().default_arrow_array()?
            } else {
                self.field.default_arrow_array()?
            };
            if !is_logically_null(default.as_ref(), 0) {
                return Err(Error::IncompatibleSchema(format!(
                    "required field {} still contains {remaining_nulls} logical null values after default filling",
                    self.path,
                )));
            }
        }
        Ok(cast)
    }

    /// Whether this node refuses absence rather than repairing it.
    const fn refuses_absence(&self) -> bool {
        self.options.nullability().is_strict()
    }

    /// Names this node's declared path and the nulls it was asked to accept.
    fn required_field(&self, nulls: usize) -> Error {
        Error::RequiredField {
            path: self.path.clone(),
            nulls: Some(nulls),
        }
    }
}

/// Casts an array into the shape a field declares.
///
/// `source_metadata` is the Arrow metadata of the field the array came from,
/// when the caller has one: it carries the extension identity, so a
/// recognized string or geospatial column follows the declared rules
/// exactly as a batch column does. A bare array casts as its storage.
pub(crate) fn cast_field_array(
    field: &Field,
    source_metadata: Option<&HashMap<String, String>>,
    array: ArrayRef,
    options: ArrowCastOptions,
) -> Result<ArrayRef> {
    field.validate_bounded()?;
    // A bare array *is* the field, so the field names the root rather than
    // hanging off one: a refusal reads `$.id`, not a bare `$`.
    let root = Path::root();
    let plan = ArrayCastPlan::new_validated_with(
        field,
        array.data_type(),
        source_metadata,
        PlanRules::nested(options, Deferred::default()),
        root.field(field.name()),
    )?;
    let mut budget = MaterializationBudget::default();
    plan.cast(array, &mut budget)
}

/// Enforces the declared rules a recognized extension source adds to a cast.
///
/// A variant crosses only to a variant until the codec lands with the
/// Iceberg v3 layer, and a geospatial source refuses a CRS or an
/// edge-interpretation change by name: both are value transformations, not
/// schema casts. A matching geospatial pair, and every plain-storage target
/// (bytes stay bytes, text renders as WKT), passes through to the planned
/// arms. A string or code source is validated text and crosses to every
/// target: another string re-reads it, text takes its characters, bytes
/// keep what was stored. A bounded byte source crosses the same way.
fn check_extension_source(target: &Field, source: Option<&RecognizedExtension>) -> Result<()> {
    let Some(source) = source else {
        return Ok(());
    };
    match (target.dtype(), source) {
        (DataType::Variant, RecognizedExtension::Variant) => Ok(()),
        (_, RecognizedExtension::Code(_) | RecognizedExtension::String(_)) => Ok(()),
        // A UUID source is sixteen bytes: a UUID target re-validates them,
        // text renders them, and bytes keep them.
        (_, RecognizedExtension::Uuid) => Ok(()),
        // A bounded byte source is its storage with a rule already checked:
        // another byte target re-measures it, and every other target reads
        // the bytes as it reads bare storage.
        (_, RecognizedExtension::Bytes(_)) => Ok(()),
        (DataType::Version, RecognizedExtension::Version) => Ok(()),
        (DataType::String(parameters), RecognizedExtension::Version)
            if is_text_storage(*parameters) =>
        {
            Ok(())
        }
        (other, RecognizedExtension::Version) => Err(Error::Unsupported {
            kind: "version",
            reason: format!("casting version to {} is not supported", other.name()),
        }),
        (DataType::Uri(UriType::Url), RecognizedExtension::Url) => Ok(()),
        (DataType::String(parameters), RecognizedExtension::Url)
            if is_text_storage(*parameters) =>
        {
            Ok(())
        }
        (DataType::Uri(UriType::Urn), RecognizedExtension::Urn) => Ok(()),
        (DataType::String(parameters), RecognizedExtension::Urn)
            if is_text_storage(*parameters) =>
        {
            Ok(())
        }
        (other, RecognizedExtension::Url) => Err(Error::Unsupported {
            kind: "url",
            reason: format!("casting url to {} is not supported", other.name()),
        }),
        (other, RecognizedExtension::Urn) => Err(Error::Unsupported {
            kind: "urn",
            reason: format!("casting urn to {} is not supported", other.name()),
        }),
        // A canonical text source crosses to its own target and to text; every
        // other target would have to re-read a spelling it does not model.
        (DataType::Timezone, RecognizedExtension::Timezone)
        | (DataType::MimeType, RecognizedExtension::MimeType)
        | (DataType::MediaType, RecognizedExtension::MediaType) => Ok(()),
        (
            DataType::String(parameters),
            RecognizedExtension::Timezone
            | RecognizedExtension::MimeType
            | RecognizedExtension::MediaType,
        ) if is_text_storage(*parameters) => Ok(()),
        (other, RecognizedExtension::Timezone) => Err(Error::Unsupported {
            kind: "timezone",
            reason: format!("casting timezone to {} is not supported", other.name()),
        }),
        (other, RecognizedExtension::MimeType) => Err(Error::Unsupported {
            kind: "mimetype",
            reason: format!("casting mimetype to {} is not supported", other.name()),
        }),
        (other, RecognizedExtension::MediaType) => Err(Error::Unsupported {
            kind: "mediatype",
            reason: format!("casting mediatype to {} is not supported", other.name()),
        }),
        (other, RecognizedExtension::Variant) => Err(Error::Unsupported {
            kind: "variant",
            reason: format!(
                "casting variant to {}: a variant column holds the variant encoding of \
                 each value, which `Variant::scalar` reads",
                other.name()
            ),
        }),
        (
            DataType::Geometry(target_geospatial) | DataType::Geography(target_geospatial),
            RecognizedExtension::Geospatial(source_geospatial),
        ) => {
            let target_kind = target.dtype().name();
            match (target_geospatial.algorithm(), source_geospatial.algorithm()) {
                (None, Some(algorithm)) => Err(Error::Unsupported {
                    kind: target_kind,
                    reason: format!(
                        "expected planar geometry edges, got geography {algorithm} edges; \
                         the edge interpretation change is a value transformation, not a \
                         schema cast"
                    ),
                }),
                (Some(algorithm), None) => Err(Error::Unsupported {
                    kind: target_kind,
                    reason: format!(
                        "expected geography {algorithm} edges, got planar geometry edges; \
                         the edge interpretation change is a value transformation, not a \
                         schema cast"
                    ),
                }),
                (Some(target_algorithm), Some(source_algorithm))
                    if target_algorithm != source_algorithm =>
                {
                    Err(Error::Unsupported {
                        kind: target_kind,
                        reason: format!(
                            "expected geography {target_algorithm} edges, got \
                             {source_algorithm} edges; the edge interpretation change is \
                             a value transformation, not a schema cast"
                        ),
                    })
                }
                _ if target_geospatial.crs() != source_geospatial.crs() => {
                    Err(Error::Unsupported {
                        kind: target_kind,
                        reason: format!(
                            "expected CRS {:?}, got CRS {:?}; a CRS change is a coordinate \
                             transformation, not a schema cast",
                            target_geospatial.crs(),
                            source_geospatial.crs()
                        ),
                    })
                }
                _ => Ok(()),
            }
        }
        (_, RecognizedExtension::Geospatial(_)) => Ok(()),
    }
}

/// Names the field and the row on a refused cell.
pub(crate) fn named_cell<T>(field: &Field, index: usize, read: crate::Result<T>) -> Result<T> {
    read.map_err(|error| {
        let reason = match error {
            crate::Error::InvalidRecord { reason, .. } => reason.to_string(),
            other => other.to_string(),
        };
        Error::IncompatibleSchema(format!("field {:?} row {index}: {reason}", field.name()))
    })
}

/// The layout a list source is rebuilt in, before the target layout is read.
fn source_list_kind(source_type: &ArrowDataType) -> Result<ListPlanKind> {
    Ok(match source_type {
        ArrowDataType::List(_) => ListPlanKind::List,
        ArrowDataType::LargeList(_) => ListPlanKind::LargeList,
        ArrowDataType::ListView(_) => ListPlanKind::ListView,
        ArrowDataType::LargeListView(_) => ListPlanKind::LargeListView,
        ArrowDataType::FixedSizeList(_, size) => ListPlanKind::FixedSize { size: *size },
        _ => return Err(internal_target_error("list")),
    })
}

/// The byte width of a datatype laid out as one fixed-width value buffer.
///
/// `None` is everything else - a bitmap, a variable-length payload, an
/// encoding, or anything with children - because those have no single buffer
/// two datatypes could share.
fn bit_layout_width(dtype: &ArrowDataType) -> Option<usize> {
    match dtype {
        // Arrow answers the width for every fixed-width primitive, decimal and
        // temporal; a fixed binary is the same layout under a length it states
        // itself, which is what puts raw bytes on both sides of this cast.
        ArrowDataType::FixedSizeBinary(width) => usize::try_from(*width).ok(),
        other => other.primitive_width(),
    }
}

/// Whether two datatypes are the same bytes under two readings.
fn same_bit_layout(source: &ArrowDataType, target: &ArrowDataType) -> bool {
    match (bit_layout_width(source), bit_layout_width(target)) {
        (Some(source), Some(target)) => source == target,
        _ => false,
    }
}

/// Read one array's buffers as the target datatype, copying nothing.
///
/// The two layouts agree by construction - [`same_bit_layout`] is what selected
/// this path - so the value buffer, the null buffer, the length and the offset
/// all carry over, and Arrow validates the result against the datatype that now
/// reads them.
fn reinterpret(array: &ArrayRef, expected: &ArrowDataType) -> Result<ArrayRef> {
    let data = array.to_data();
    // The null buffer is carried whole rather than as a raw bitmap plus the
    // array's offset: a sliced array's validity keeps an offset of its own,
    // and rebuilding it from the two separately would shift it.
    let rebuilt = arrow_data::ArrayDataBuilder::new(expected.clone())
        .len(data.len())
        .offset(data.offset())
        .buffers(data.buffers().to_vec())
        // A fixed-width pair has no children; an encoding whose declared child
        // fields differ only in their metadata carries the ones it was built
        // with, which is what makes this one function serve both.
        .child_data(data.child_data().to_vec())
        .nulls(data.nulls().cloned())
        .build()?;
    Ok(arrow_array::make_array(rebuilt))
}

pub(crate) fn downcast<T: Array + 'static>(array: &dyn Array) -> Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "Arrow array implementation does not match datatype {:?}",
            array.data_type()
        ))
    })
}

pub(crate) fn internal_target_error(kind: &'static str) -> Error {
    Error::Unsupported {
        kind,
        reason: "validated target projected an unexpected Arrow datatype".to_owned(),
    }
}

/// Arrow column planning, exposure, and logical-null traversal.
///
/// This is array-level machinery, not a datatype layer: it reads and rebuilds
/// Arrow columns for the layouts that carry children, and it lives beside the
/// rest of the cast code rather than beside the datatypes it inspects.
pub(crate) mod columns {

    use std::cmp::Ordering;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use arrow_array::types::{
        ArrowDictionaryKeyType, Int8Type, Int16Type, Int32Type, Int64Type, RunEndIndexType,
        UInt8Type, UInt16Type, UInt32Type, UInt64Type,
    };
    use arrow_array::{
        Array, ArrayRef, BooleanArray, Decimal256Array, DictionaryArray, FixedSizeListArray,
        Float16Array, Float32Array, Float64Array, Int16RunArray, Int32RunArray, Int64RunArray,
        LargeListArray, LargeListViewArray, ListArray, ListViewArray, MapArray, PrimitiveArray,
        RunArray, Scalar as ArrowScalar, StructArray, UInt32Array, UnionArray, make_array,
        new_null_array,
    };
    use arrow_buffer::{ArrowNativeType, BooleanBuffer, BooleanBufferBuilder};
    use arrow_ord::ord::{DynComparator, make_comparator};
    use arrow_schema::{DataType as ArrowDataType, FieldRef as ArrowFieldRef, SortOptions};
    use arrow_select::{concat::concat, take::take, zip::zip};

    use crate::arrow::{Error, Result};
    use crate::budget::{
        MaterializationBudget, SourceSelection, reserve_concat_copy, reserve_field_default_scalar,
        reserve_missing_output, reserve_new_dictionary_vocabularies, reserve_source_selection,
        reserve_to_data_scratch, reserve_vec_bytes, scratch_vec, selected_child_ranges,
    };
    use crate::cast::arrow_cast_exposed;
    use crate::cast::{
        ArrayCastPlan, ListPlanKind, StructColumnPlan, downcast, internal_target_error,
    };
    use crate::decimal::casts::DecimalText;
    use crate::{DataType, Field, Scalar, UnionMode};

    use crate::DecimalType;
    use crate::enums::EnumType;
    use crate::sequence::SequenceType;
    mod dictionary {

        use super::*;

        pub(crate) fn contains_dictionary(dtype: &DataType) -> bool {
            match dtype {
                DataType::Enum(EnumType::Dictionary(_)) => true,
                DataType::Sequence(SequenceType::List(field))
                | DataType::Sequence(SequenceType::ListView(field))
                | DataType::Sequence(SequenceType::FixedSizeList(field, _))
                | DataType::Sequence(SequenceType::LargeList(field))
                | DataType::Sequence(SequenceType::LargeListView(field)) => {
                    contains_dictionary(field.dtype())
                }
                DataType::Struct(fields) => fields
                    .iter()
                    .any(|field| contains_dictionary(field.dtype())),
                DataType::Union(fields, _) => fields
                    .iter()
                    .any(|(_, field)| contains_dictionary(field.dtype())),
                DataType::Mapping(map) => contains_dictionary(map.entries().dtype()),
                DataType::RunEndEncoded(encoded) => contains_dictionary(encoded.values().dtype()),
                _ => false,
            }
        }

        pub(crate) fn dictionary_values_ref<'a>(
            array: &'a dyn Array,
            dictionary: &crate::DictionaryType,
        ) -> Result<&'a ArrayRef> {
            macro_rules! values {
                ($key:ty) => {{ Ok(downcast::<DictionaryArray<$key>>(array)?.values()) }};
            }
            match dictionary.key() {
                DataType::Int8 => values!(Int8Type),
                DataType::Int16 => values!(Int16Type),
                DataType::Int32 => values!(Int32Type),
                DataType::Int64 => values!(Int64Type),
                DataType::UInt8 => values!(UInt8Type),
                DataType::UInt16 => values!(UInt16Type),
                DataType::UInt32 => values!(UInt32Type),
                DataType::UInt64 => values!(UInt64Type),
                key => Err(Error::Unsupported {
                    kind: key.name(),
                    reason: format!(
                        "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
                    ),
                }),
            }
        }

        #[allow(clippy::too_many_lines)]
        pub(crate) fn align_nested_dictionaries(
            field: &Field,
            left: &ArrayRef,
            right: &ArrayRef,
            left_exposure: Option<&BooleanBuffer>,
            right_exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<(ArrayRef, ArrayRef)> {
            if !contains_dictionary(field.dtype()) {
                return Ok((Arc::clone(left), Arc::clone(right)));
            }
            if left.data_type() != right.data_type() {
                return Err(Error::IncompatibleSchema(
                    "dictionary alignment inputs have different physical datatypes".to_owned(),
                ));
            }
            if left_exposure.is_some_and(|exposure| exposure.len() != left.len())
                || right_exposure.is_some_and(|exposure| exposure.len() != right.len())
            {
                return Err(Error::IncompatibleSchema(
                    "dictionary alignment exposure has the wrong length".to_owned(),
                ));
            }

            match field.dtype() {
                DataType::Enum(EnumType::Dictionary(dictionary)) => align_dictionary_arrays(
                    field,
                    dictionary,
                    left,
                    right,
                    left_exposure,
                    right_exposure,
                    budget,
                ),
                DataType::Struct(fields) => {
                    let left_struct = downcast::<StructArray>(left.as_ref())?;
                    let right_struct = downcast::<StructArray>(right.as_ref())?;
                    let left_child_exposure =
                        visible_array_exposure(left.as_ref(), left_exposure, budget)?;
                    let right_child_exposure =
                        visible_array_exposure(right.as_ref(), right_exposure, budget)?;
                    let mut left_children =
                        scratch_vec::<ArrayRef>(budget, fields.len(), "left Struct child arrays")?;
                    let mut right_children =
                        scratch_vec::<ArrayRef>(budget, fields.len(), "right Struct child arrays")?;
                    for (index, child_field) in fields.iter().enumerate() {
                        let (left_child, right_child) = align_nested_dictionaries(
                            child_field,
                            left_struct.column(index),
                            right_struct.column(index),
                            left_child_exposure.as_ref(),
                            right_child_exposure.as_ref(),
                            budget,
                        )?;
                        left_children.push(left_child);
                        right_children.push(right_child);
                    }
                    Ok((
                        replace_array_children(left, left_children, budget)?,
                        replace_array_children(right, right_children, budget)?,
                    ))
                }
                DataType::Sequence(SequenceType::List(child)) => {
                    let left_list = downcast::<ListArray>(left.as_ref())?;
                    let right_list = downcast::<ListArray>(right.as_ref())?;
                    let left_child_exposure = range_exposure(
                        left_list.values().len(),
                        left_list.len(),
                        left_exposure,
                        |row| left_list.is_valid(row),
                        |row| {
                            offset_pair(
                                i64::from(left_list.offsets()[row]),
                                i64::from(left_list.offsets()[row + 1]),
                            )
                        },
                        budget,
                    )?;
                    let right_child_exposure = range_exposure(
                        right_list.values().len(),
                        right_list.len(),
                        right_exposure,
                        |row| right_list.is_valid(row),
                        |row| {
                            offset_pair(
                                i64::from(right_list.offsets()[row]),
                                i64::from(right_list.offsets()[row + 1]),
                            )
                        },
                        budget,
                    )?;
                    let (left_child, right_child) = align_nested_dictionaries(
                        child,
                        left_list.values(),
                        right_list.values(),
                        left_child_exposure.as_ref(),
                        right_child_exposure.as_ref(),
                        budget,
                    )?;
                    Ok((
                        replace_array_children(left, vec![left_child], budget)?,
                        replace_array_children(right, vec![right_child], budget)?,
                    ))
                }
                DataType::Sequence(SequenceType::LargeList(child)) => {
                    let left_list = downcast::<LargeListArray>(left.as_ref())?;
                    let right_list = downcast::<LargeListArray>(right.as_ref())?;
                    let left_child_exposure = range_exposure(
                        left_list.values().len(),
                        left_list.len(),
                        left_exposure,
                        |row| left_list.is_valid(row),
                        |row| offset_pair(left_list.offsets()[row], left_list.offsets()[row + 1]),
                        budget,
                    )?;
                    let right_child_exposure = range_exposure(
                        right_list.values().len(),
                        right_list.len(),
                        right_exposure,
                        |row| right_list.is_valid(row),
                        |row| offset_pair(right_list.offsets()[row], right_list.offsets()[row + 1]),
                        budget,
                    )?;
                    let (left_child, right_child) = align_nested_dictionaries(
                        child,
                        left_list.values(),
                        right_list.values(),
                        left_child_exposure.as_ref(),
                        right_child_exposure.as_ref(),
                        budget,
                    )?;
                    Ok((
                        replace_array_children(left, vec![left_child], budget)?,
                        replace_array_children(right, vec![right_child], budget)?,
                    ))
                }
                DataType::Sequence(SequenceType::ListView(child)) => {
                    let left_list = downcast::<ListViewArray>(left.as_ref())?;
                    let right_list = downcast::<ListViewArray>(right.as_ref())?;
                    let left_child_exposure = range_exposure(
                        left_list.values().len(),
                        left_list.len(),
                        left_exposure,
                        |row| left_list.is_valid(row),
                        |row| {
                            offset_size(
                                i64::from(left_list.offsets()[row]),
                                i64::from(left_list.sizes()[row]),
                            )
                        },
                        budget,
                    )?;
                    let right_child_exposure = range_exposure(
                        right_list.values().len(),
                        right_list.len(),
                        right_exposure,
                        |row| right_list.is_valid(row),
                        |row| {
                            offset_size(
                                i64::from(right_list.offsets()[row]),
                                i64::from(right_list.sizes()[row]),
                            )
                        },
                        budget,
                    )?;
                    let (left_child, right_child) = align_nested_dictionaries(
                        child,
                        left_list.values(),
                        right_list.values(),
                        left_child_exposure.as_ref(),
                        right_child_exposure.as_ref(),
                        budget,
                    )?;
                    Ok((
                        replace_array_children(left, vec![left_child], budget)?,
                        replace_array_children(right, vec![right_child], budget)?,
                    ))
                }
                DataType::Sequence(SequenceType::LargeListView(child)) => {
                    let left_list = downcast::<LargeListViewArray>(left.as_ref())?;
                    let right_list = downcast::<LargeListViewArray>(right.as_ref())?;
                    let left_child_exposure = range_exposure(
                        left_list.values().len(),
                        left_list.len(),
                        left_exposure,
                        |row| left_list.is_valid(row),
                        |row| offset_size(left_list.offsets()[row], left_list.sizes()[row]),
                        budget,
                    )?;
                    let right_child_exposure = range_exposure(
                        right_list.values().len(),
                        right_list.len(),
                        right_exposure,
                        |row| right_list.is_valid(row),
                        |row| offset_size(right_list.offsets()[row], right_list.sizes()[row]),
                        budget,
                    )?;
                    let (left_child, right_child) = align_nested_dictionaries(
                        child,
                        left_list.values(),
                        right_list.values(),
                        left_child_exposure.as_ref(),
                        right_child_exposure.as_ref(),
                        budget,
                    )?;
                    Ok((
                        replace_array_children(left, vec![left_child], budget)?,
                        replace_array_children(right, vec![right_child], budget)?,
                    ))
                }
                DataType::Sequence(SequenceType::FixedSizeList(child, size)) => {
                    let left_list = downcast::<FixedSizeListArray>(left.as_ref())?;
                    let right_list = downcast::<FixedSizeListArray>(right.as_ref())?;
                    let width = usize::try_from(*size).map_err(|_| {
                        Error::IncompatibleSchema("fixed-list size is negative".to_owned())
                    })?;
                    let left_child_exposure = range_exposure(
                        left_list.values().len(),
                        left_list.len(),
                        left_exposure,
                        |row| left_list.is_valid(row),
                        |row| {
                            let start =
                                usize::try_from(left_list.value_offset(row)).map_err(|_| {
                                    Error::IncompatibleSchema(
                                        "fixed-list offset is negative or exceeds usize".to_owned(),
                                    )
                                })?;
                            Ok((start, start + width))
                        },
                        budget,
                    )?;
                    let right_child_exposure = range_exposure(
                        right_list.values().len(),
                        right_list.len(),
                        right_exposure,
                        |row| right_list.is_valid(row),
                        |row| {
                            let start =
                                usize::try_from(right_list.value_offset(row)).map_err(|_| {
                                    Error::IncompatibleSchema(
                                        "fixed-list offset is negative or exceeds usize".to_owned(),
                                    )
                                })?;
                            Ok((start, start + width))
                        },
                        budget,
                    )?;
                    let (left_child, right_child) = align_nested_dictionaries(
                        child,
                        left_list.values(),
                        right_list.values(),
                        left_child_exposure.as_ref(),
                        right_child_exposure.as_ref(),
                        budget,
                    )?;
                    Ok((
                        replace_array_children(left, vec![left_child], budget)?,
                        replace_array_children(right, vec![right_child], budget)?,
                    ))
                }
                DataType::Mapping(map) => {
                    let left_map = downcast::<MapArray>(left.as_ref())?;
                    let right_map = downcast::<MapArray>(right.as_ref())?;
                    let left_entry_exposure = range_exposure(
                        left_map.entries().len(),
                        left_map.len(),
                        left_exposure,
                        |row| left_map.is_valid(row),
                        |row| {
                            offset_pair(
                                i64::from(left_map.offsets()[row]),
                                i64::from(left_map.offsets()[row + 1]),
                            )
                        },
                        budget,
                    )?;
                    let right_entry_exposure = range_exposure(
                        right_map.entries().len(),
                        right_map.len(),
                        right_exposure,
                        |row| right_map.is_valid(row),
                        |row| {
                            offset_pair(
                                i64::from(right_map.offsets()[row]),
                                i64::from(right_map.offsets()[row + 1]),
                            )
                        },
                        budget,
                    )?;
                    let left_entries: ArrayRef = Arc::new(left_map.entries().clone());
                    let right_entries: ArrayRef = Arc::new(right_map.entries().clone());
                    let (left_entries, right_entries) = align_nested_dictionaries(
                        map.entries(),
                        &left_entries,
                        &right_entries,
                        left_entry_exposure.as_ref(),
                        right_entry_exposure.as_ref(),
                        budget,
                    )?;
                    Ok((
                        replace_array_children(left, vec![left_entries], budget)?,
                        replace_array_children(right, vec![right_entries], budget)?,
                    ))
                }
                DataType::Union(fields, _) => {
                    let left_union = downcast::<UnionArray>(left.as_ref())?;
                    let right_union = downcast::<UnionArray>(right.as_ref())?;
                    let mut left_children =
                        scratch_vec::<ArrayRef>(budget, fields.len(), "left Union child arrays")?;
                    let mut right_children =
                        scratch_vec::<ArrayRef>(budget, fields.len(), "right Union child arrays")?;
                    for (type_id, child_field) in fields {
                        let left_child = left_union.child(type_id);
                        let right_child = right_union.child(type_id);
                        let left_child_exposure = selected_index_exposure(
                            left_child.len(),
                            left_union.len(),
                            left_exposure,
                            |row| {
                                (left_union.type_id(row) == type_id)
                                    .then(|| left_union.value_offset(row))
                            },
                            budget,
                        )?;
                        let right_child_exposure = selected_index_exposure(
                            right_child.len(),
                            right_union.len(),
                            right_exposure,
                            |row| {
                                (right_union.type_id(row) == type_id)
                                    .then(|| right_union.value_offset(row))
                            },
                            budget,
                        )?;
                        let (left_child, right_child) = align_nested_dictionaries(
                            child_field,
                            left_child,
                            right_child,
                            left_child_exposure.as_ref(),
                            right_child_exposure.as_ref(),
                            budget,
                        )?;
                        left_children.push(left_child);
                        right_children.push(right_child);
                    }
                    Ok((
                        replace_array_children(left, left_children, budget)?,
                        replace_array_children(right, right_children, budget)?,
                    ))
                }
                DataType::RunEndEncoded(encoded) => {
                    macro_rules! align_run {
                        ($run:ty) => {{
                            let left_run = downcast::<RunArray<$run>>(left.as_ref())?;
                            let right_run = downcast::<RunArray<$run>>(right.as_ref())?;
                            let left_value_exposure = selected_index_exposure(
                                left_run.values().len(),
                                left_run.len(),
                                left_exposure,
                                |row| Some(left_run.run_ends().get_physical_index(row)),
                                budget,
                            )?;
                            let right_value_exposure = selected_index_exposure(
                                right_run.values().len(),
                                right_run.len(),
                                right_exposure,
                                |row| Some(right_run.run_ends().get_physical_index(row)),
                                budget,
                            )?;
                            let (left_values, right_values) = align_nested_dictionaries(
                                encoded.values(),
                                left_run.values(),
                                right_run.values(),
                                left_value_exposure.as_ref(),
                                right_value_exposure.as_ref(),
                                budget,
                            )?;
                            let left_run_ends: ArrayRef = Arc::new(PrimitiveArray::<$run>::new(
                                left_run.run_ends().inner().clone(),
                                None,
                            ));
                            let right_run_ends: ArrayRef = Arc::new(PrimitiveArray::<$run>::new(
                                right_run.run_ends().inner().clone(),
                                None,
                            ));
                            Ok((
                                replace_array_children(
                                    left,
                                    vec![left_run_ends, left_values],
                                    budget,
                                )?,
                                replace_array_children(
                                    right,
                                    vec![right_run_ends, right_values],
                                    budget,
                                )?,
                            ))
                        }};
                    }
                    match encoded.run_ends().dtype() {
                        DataType::Int16 => align_run!(Int16Type),
                        DataType::Int32 => align_run!(Int32Type),
                        DataType::Int64 => align_run!(Int64Type),
                        _ => Err(Error::IncompatibleSchema(
                            "run-end type is not a supported signed integer".to_owned(),
                        )),
                    }
                }
                _ => Ok((Arc::clone(left), Arc::clone(right))),
            }
        }

        pub(crate) fn align_dictionary_arrays(
            field: &Field,
            dictionary: &crate::DictionaryType,
            left: &ArrayRef,
            right: &ArrayRef,
            left_exposure: Option<&BooleanBuffer>,
            right_exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<(ArrayRef, ArrayRef)> {
            macro_rules! align {
                ($key:ty) => {{
                    align_dictionary_arrays_typed::<$key>(
                        field,
                        dictionary,
                        left,
                        right,
                        left_exposure,
                        right_exposure,
                        budget,
                    )
                }};
            }
            match dictionary.key() {
                DataType::Int8 => align!(Int8Type),
                DataType::Int16 => align!(Int16Type),
                DataType::Int32 => align!(Int32Type),
                DataType::Int64 => align!(Int64Type),
                DataType::UInt8 => align!(UInt8Type),
                DataType::UInt16 => align!(UInt16Type),
                DataType::UInt32 => align!(UInt32Type),
                DataType::UInt64 => align!(UInt64Type),
                key => Err(Error::Unsupported {
                    kind: key.name(),
                    reason: format!(
                        "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
                    ),
                }),
            }
        }

        #[derive(Clone, Copy)]
        enum DictionaryCandidate {
            Left(usize),
            Right(usize),
        }

        fn compare_dictionary_candidates(
            left: DictionaryCandidate,
            right: DictionaryCandidate,
            compare_left: &DynComparator,
            compare_right: &DynComparator,
            compare_cross: &DynComparator,
        ) -> Ordering {
            match (left, right) {
                (DictionaryCandidate::Left(left), DictionaryCandidate::Left(right)) => {
                    compare_left(left, right)
                }
                (DictionaryCandidate::Right(left), DictionaryCandidate::Right(right)) => {
                    compare_right(left, right)
                }
                (DictionaryCandidate::Left(left), DictionaryCandidate::Right(right)) => {
                    compare_cross(left, right)
                }
                (DictionaryCandidate::Right(left), DictionaryCandidate::Left(right)) => {
                    compare_cross(right, left).reverse()
                }
            }
        }

        pub(crate) fn dictionary_live_indices<K: ArrowDictionaryKeyType>(
            source: &DictionaryArray<K>,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<Vec<usize>> {
            let mut used =
                scratch_vec::<usize>(budget, source.len(), "dictionary alignment live values")?;
            for row in 0..source.len() {
                if is_exposed(exposure, row) && source.keys().is_valid(row) {
                    let index = source.keys().value(row).as_usize();
                    if index >= source.values().len() {
                        return Err(Error::IncompatibleSchema(
                            "dictionary key points outside its values array".to_owned(),
                        ));
                    }
                    used.push(index);
                }
            }
            used.sort_unstable();
            used.dedup();
            Ok(used)
        }

        pub(crate) fn remap_dictionary_to_values<K>(
            field: &Field,
            source: &DictionaryArray<K>,
            exposure: Option<&BooleanBuffer>,
            mappings: &[(usize, usize)],
            values: ArrayRef,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef>
        where
            K: ArrowDictionaryKeyType,
            K::Native: TryFrom<usize>,
        {
            budget.add_array_layout(field.dtype(), source.len())?;
            let fallback = (!values.is_empty())
                .then(|| K::Native::try_from(0).ok())
                .flatten();
            let mut keys = arrow_array::builder::PrimitiveBuilder::<K>::with_capacity(source.len());
            for row in 0..source.len() {
                if source.keys().is_null(row) {
                    keys.append_null();
                    continue;
                }
                let old = source.keys().value(row).as_usize();
                if is_exposed(exposure, row) {
                    let position = mappings
                        .binary_search_by_key(&old, |(candidate, _)| *candidate)
                        .map_err(|_| {
                            Error::IncompatibleSchema(
                                "live dictionary key is absent from its vocabulary remap"
                                    .to_owned(),
                            )
                        })?;
                    let key = K::Native::try_from(mappings[position].1).map_err(|_| {
                        Error::IncompatibleSchema(
                            "dictionary compact key exceeds its physical key type".to_owned(),
                        )
                    })?;
                    keys.append_value(key);
                } else if let Some(fallback) = fallback {
                    keys.append_value(fallback);
                } else {
                    keys.append_null();
                }
            }
            Ok(Arc::new(DictionaryArray::<K>::try_new(
                keys.finish(),
                values,
            )?))
        }

        pub(crate) fn take_dictionary_candidates(
            values: &ArrayRef,
            value_type: &DataType,
            selected: &[usize],
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            if selected.is_empty() {
                return Ok(arrow_array::new_empty_array(values.data_type()));
            }
            budget.add_array(&DataType::UInt32, selected.len())?;
            reserve_vec_bytes::<u32>(budget, selected.len())?;
            let indices = selected
                .iter()
                .map(|index| {
                    u32::try_from(*index).map_err(|_| {
                        Error::IncompatibleSchema(
                            "dictionary value index exceeds Arrow UInt32 take capacity".to_owned(),
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let indices = UInt32Array::from(indices);
            reserve_source_selection(
                values.as_ref(),
                value_type,
                SourceSelection::Indices(indices.values()),
                budget,
            )?;
            take(values.as_ref(), &indices, None).map_err(Into::into)
        }

        #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
        pub(crate) fn align_dictionary_arrays_typed<K>(
            field: &Field,
            dictionary: &crate::DictionaryType,
            left: &ArrayRef,
            right: &ArrayRef,
            left_exposure: Option<&BooleanBuffer>,
            right_exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<(ArrayRef, ArrayRef)>
        where
            K: ArrowDictionaryKeyType,
            K::Native: TryFrom<usize>,
        {
            let left_source = downcast::<DictionaryArray<K>>(left.as_ref())?;
            let right_source = downcast::<DictionaryArray<K>>(right.as_ref())?;
            if Arc::ptr_eq(left_source.values(), right_source.values()) {
                return Ok((Arc::clone(left), Arc::clone(right)));
            }
            let left_used = dictionary_live_indices(left_source, left_exposure, budget)?;
            let right_used = dictionary_live_indices(right_source, right_exposure, budget)?;
            let value_type = dictionary.value();
            let compare_left =
                make_yggdryl_key_comparator(value_type, left_source.values(), budget)?;
            let compare_right =
                make_yggdryl_key_comparator(value_type, right_source.values(), budget)?;
            let compare_cross = make_yggdryl_comparator(
                value_type,
                left_source.values(),
                right_source.values(),
                budget,
            )?;

            // Reusing either vocabulary is allocation-free for its owner and makes
            // Arrow's recursive zip path share, rather than concatenate, that
            // vocabulary. Compare only reachable entries: a transparent wrapper may
            // have a tiny physical representation but an arbitrarily long hidden
            // logical vocabulary.
            let mut right_to_left = scratch_vec::<(usize, usize)>(
                budget,
                right_used.len(),
                "dictionary right-to-left remap",
            )?;
            if right_used.len() <= HASHED_NAME_INDEX_THRESHOLD {
                for right_index in &right_used {
                    let Some(left_index) = left_used.iter().copied().find(|left_index| {
                        compare_cross(*left_index, *right_index) == Ordering::Equal
                    }) else {
                        right_to_left.clear();
                        break;
                    };
                    right_to_left.push((*right_index, left_index));
                }
            }
            if right_to_left.len() == right_used.len() {
                right_to_left.sort_unstable_by_key(|(old, _)| *old);
                let right = remap_dictionary_to_values(
                    field,
                    right_source,
                    right_exposure,
                    &right_to_left,
                    Arc::clone(left_source.values()),
                    budget,
                )?;
                return Ok((Arc::clone(left), right));
            }

            let mut left_to_right = scratch_vec::<(usize, usize)>(
                budget,
                left_used.len(),
                "dictionary left-to-right remap",
            )?;
            if left_used.len() <= HASHED_NAME_INDEX_THRESHOLD {
                for left_index in &left_used {
                    let Some(right_index) = right_used.iter().copied().find(|right_index| {
                        compare_cross(*left_index, *right_index) == Ordering::Equal
                    }) else {
                        left_to_right.clear();
                        break;
                    };
                    left_to_right.push((*left_index, right_index));
                }
            }
            if left_to_right.len() == left_used.len() {
                left_to_right.sort_unstable_by_key(|(old, _)| *old);
                let left = remap_dictionary_to_values(
                    field,
                    left_source,
                    left_exposure,
                    &left_to_right,
                    Arc::clone(right_source.values()),
                    budget,
                )?;
                return Ok((left, Arc::clone(right)));
            }

            let mut candidates = scratch_vec::<DictionaryCandidate>(
                budget,
                left_used.len().saturating_add(right_used.len()),
                "dictionary reachable vocabulary",
            )?;
            candidates.extend(left_used.iter().copied().map(DictionaryCandidate::Left));
            candidates.extend(right_used.iter().copied().map(DictionaryCandidate::Right));
            candidates.sort_unstable_by(|left, right| {
                compare_dictionary_candidates(
                    *left,
                    *right,
                    &compare_left,
                    &compare_right,
                    &compare_cross,
                )
            });

            let mut representatives = scratch_vec::<DictionaryCandidate>(
                budget,
                candidates.len(),
                "dictionary semantic representatives",
            )?;
            let mut left_groups = scratch_vec::<(usize, usize)>(
                budget,
                left_used.len(),
                "dictionary left compact remap",
            )?;
            let mut right_groups = scratch_vec::<(usize, usize)>(
                budget,
                right_used.len(),
                "dictionary right compact remap",
            )?;
            for candidate in candidates {
                let group = if representatives.last().is_some_and(|prior| {
                    compare_dictionary_candidates(
                        *prior,
                        candidate,
                        &compare_left,
                        &compare_right,
                        &compare_cross,
                    ) == Ordering::Equal
                }) {
                    representatives.len() - 1
                } else {
                    representatives.push(candidate);
                    representatives.len() - 1
                };
                match candidate {
                    DictionaryCandidate::Left(old) => left_groups.push((old, group)),
                    DictionaryCandidate::Right(old) => right_groups.push((old, group)),
                }
            }

            let last = representatives.len().checked_sub(1);
            if last.is_some_and(|last| K::Native::try_from(last).is_err()) {
                return Err(Error::IncompatibleSchema(format!(
                    "dictionary reachable values exceed the {} key capacity",
                    dictionary.key()
                )));
            }

            let mut left_selected = scratch_vec::<usize>(
                budget,
                representatives.len(),
                "dictionary left representative indices",
            )?;
            let mut right_selected = scratch_vec::<usize>(
                budget,
                representatives.len(),
                "dictionary right representative indices",
            )?;
            let mut group_output = scratch_vec::<usize>(
                budget,
                representatives.len(),
                "dictionary representative output mapping",
            )?;
            group_output.resize(representatives.len(), 0);
            for (group, representative) in representatives.iter().enumerate() {
                if let DictionaryCandidate::Left(index) = representative {
                    group_output[group] = left_selected.len();
                    left_selected.push(*index);
                }
            }
            let left_count = left_selected.len();
            for (group, representative) in representatives.iter().enumerate() {
                if let DictionaryCandidate::Right(index) = representative {
                    group_output[group] = left_count + right_selected.len();
                    right_selected.push(*index);
                }
            }
            for (_, group) in &mut left_groups {
                *group = group_output[*group];
            }
            for (_, group) in &mut right_groups {
                *group = group_output[*group];
            }
            left_groups.sort_unstable_by_key(|(old, _)| *old);
            right_groups.sort_unstable_by_key(|(old, _)| *old);

            let left_values = take_dictionary_candidates(
                left_source.values(),
                value_type,
                &left_selected,
                budget,
            )?;
            let right_values = take_dictionary_candidates(
                right_source.values(),
                value_type,
                &right_selected,
                budget,
            )?;
            let value_field = Field::new("dictionary", value_type.clone(), true);
            let (left_values, right_values) = align_nested_dictionaries(
                &value_field,
                &left_values,
                &right_values,
                None,
                None,
                budget,
            )?;
            let values = match (left_values.is_empty(), right_values.is_empty()) {
                (false, false) => {
                    reserve_concat_copy(left_values.as_ref(), value_type, budget)?;
                    reserve_concat_copy(right_values.as_ref(), value_type, budget)?;
                    concat(&[left_values.as_ref(), right_values.as_ref()])?
                }
                (false, true) => left_values,
                (true, false) => right_values,
                (true, true) => arrow_array::new_empty_array(left_source.values().data_type()),
            };
            let left = remap_dictionary_to_values(
                field,
                left_source,
                left_exposure,
                &left_groups,
                Arc::clone(&values),
                budget,
            )?;
            let right = remap_dictionary_to_values(
                field,
                right_source,
                right_exposure,
                &right_groups,
                values,
                budget,
            )?;
            Ok((left, right))
        }
    }
    mod plans {

        use super::*;

        impl ArrayCastPlan {
            pub(crate) fn cast_struct_array(
                &self,
                array: ArrayRef,
                fields: &arrow_schema::Fields,
                columns: &[StructColumnPlan],
                exposure: Option<&BooleanBuffer>,
                budget: &mut MaterializationBudget,
            ) -> Result<ArrayRef> {
                let source = downcast::<StructArray>(&array)?;
                let child_exposure = visible_array_exposure(source, exposure, budget)?;
                let mut output = Vec::with_capacity(columns.len());
                let mut unchanged = self.source_type == self.expected;
                for column in columns {
                    output.push(match column {
                        StructColumnPlan::Source { index, cast } => {
                            let source_column = source.column(*index);
                            let output = cast.cast_exposed(
                                Arc::clone(source_column),
                                child_exposure.as_ref(),
                                budget,
                            )?;
                            unchanged &= output.len() == source_column.len()
                                && Arc::ptr_eq(&output, source_column);
                            output
                        }
                        StructColumnPlan::Missing { field, default } => {
                            unchanged = false;
                            default_array_with(
                                field,
                                default.as_ref(),
                                source.len(),
                                child_exposure.as_ref(),
                                budget,
                            )?
                        }
                    });
                }
                if unchanged {
                    return Ok(array);
                }
                Ok(Arc::new(StructArray::try_new_with_length(
                    fields.clone(),
                    output,
                    source.nulls().cloned(),
                    source.len(),
                )?))
            }

            #[allow(clippy::too_many_lines)] // Keep the five Arrow list layouts behaviorally aligned.
            pub(crate) fn cast_list_array(
                &self,
                array: ArrayRef,
                field: &ArrowFieldRef,
                child: &ArrayCastPlan,
                kind: ListPlanKind,
                exposure: Option<&BooleanBuffer>,
                budget: &mut MaterializationBudget,
            ) -> Result<ArrayRef> {
                Ok(match kind {
                    ListPlanKind::List => {
                        let source = downcast::<ListArray>(&array)?;
                        let child_exposure = range_exposure(
                            source.values().len(),
                            source.len(),
                            exposure,
                            |row| source.is_valid(row),
                            |row| {
                                let offsets = source.value_offsets();
                                offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))
                            },
                            budget,
                        )?;
                        let values = child.cast_exposed(
                            Arc::clone(source.values()),
                            child_exposure.as_ref(),
                            budget,
                        )?;
                        let values = ensure_list_child_physical(&child.field, values, budget)?;
                        if self.source_type == self.expected
                            && Arc::ptr_eq(&values, source.values())
                        {
                            return Ok(array);
                        }
                        Arc::new(ListArray::try_new(
                            Arc::clone(field),
                            source.offsets().clone(),
                            values,
                            source.nulls().cloned(),
                        )?) as ArrayRef
                    }
                    ListPlanKind::LargeList => {
                        let source = downcast::<LargeListArray>(&array)?;
                        let child_exposure = range_exposure(
                            source.values().len(),
                            source.len(),
                            exposure,
                            |row| source.is_valid(row),
                            |row| {
                                let offsets = source.value_offsets();
                                offset_pair(offsets[row], offsets[row + 1])
                            },
                            budget,
                        )?;
                        let values = child.cast_exposed(
                            Arc::clone(source.values()),
                            child_exposure.as_ref(),
                            budget,
                        )?;
                        let values = ensure_list_child_physical(&child.field, values, budget)?;
                        if self.source_type == self.expected
                            && Arc::ptr_eq(&values, source.values())
                        {
                            return Ok(array);
                        }
                        Arc::new(LargeListArray::try_new(
                            Arc::clone(field),
                            source.offsets().clone(),
                            values,
                            source.nulls().cloned(),
                        )?)
                    }
                    ListPlanKind::ListView => {
                        let source = downcast::<ListViewArray>(&array)?;
                        let child_exposure = range_exposure(
                            source.values().len(),
                            source.len(),
                            exposure,
                            |row| source.is_valid(row),
                            |row| {
                                offset_size(
                                    i64::from(source.value_offsets()[row]),
                                    i64::from(source.value_sizes()[row]),
                                )
                            },
                            budget,
                        )?;
                        let values = child.cast_exposed(
                            Arc::clone(source.values()),
                            child_exposure.as_ref(),
                            budget,
                        )?;
                        let values = ensure_list_child_physical(&child.field, values, budget)?;
                        if self.source_type == self.expected
                            && Arc::ptr_eq(&values, source.values())
                        {
                            return Ok(array);
                        }
                        Arc::new(ListViewArray::try_new(
                            Arc::clone(field),
                            source.offsets().clone(),
                            source.sizes().clone(),
                            values,
                            source.nulls().cloned(),
                        )?)
                    }
                    ListPlanKind::LargeListView => {
                        let source = downcast::<LargeListViewArray>(&array)?;
                        let child_exposure = range_exposure(
                            source.values().len(),
                            source.len(),
                            exposure,
                            |row| source.is_valid(row),
                            |row| {
                                offset_size(source.value_offsets()[row], source.value_sizes()[row])
                            },
                            budget,
                        )?;
                        let values = child.cast_exposed(
                            Arc::clone(source.values()),
                            child_exposure.as_ref(),
                            budget,
                        )?;
                        let values = ensure_list_child_physical(&child.field, values, budget)?;
                        if self.source_type == self.expected
                            && Arc::ptr_eq(&values, source.values())
                        {
                            return Ok(array);
                        }
                        Arc::new(LargeListViewArray::try_new(
                            Arc::clone(field),
                            source.offsets().clone(),
                            source.sizes().clone(),
                            values,
                            source.nulls().cloned(),
                        )?)
                    }
                    ListPlanKind::FixedSize { size } => {
                        let source = downcast::<FixedSizeListArray>(&array)?;
                        let child_exposure = range_exposure(
                            source.values().len(),
                            source.len(),
                            exposure,
                            |row| source.is_valid(row),
                            |row| offset_size(i64::from(source.value_offset(row)), i64::from(size)),
                            budget,
                        )?;
                        let values = child.cast_exposed(
                            Arc::clone(source.values()),
                            child_exposure.as_ref(),
                            budget,
                        )?;
                        let values = ensure_list_child_physical(&child.field, values, budget)?;
                        if self.source_type == self.expected
                            && Arc::ptr_eq(&values, source.values())
                        {
                            return Ok(array);
                        }
                        Arc::new(FixedSizeListArray::try_new_with_length(
                            Arc::clone(field),
                            size,
                            values,
                            source.nulls().cloned(),
                            source.len(),
                        )?)
                    }
                })
            }

            #[allow(clippy::too_many_arguments)]
            pub(crate) fn cast_map_array(
                &self,
                array: ArrayRef,
                source_map: &crate::MappingType,
                field: &ArrowFieldRef,
                ordered: bool,
                entries: &ArrayCastPlan,
                exposure: Option<&BooleanBuffer>,
                budget: &mut MaterializationBudget,
            ) -> Result<ArrayRef> {
                let source = downcast::<MapArray>(&array)?;
                validate_map_invariants(source_map, source, exposure, budget)?;
                if self.source_type == self.expected
                    && !(0..source.len())
                        .any(|row| is_exposed(exposure, row) && source.is_valid(row))
                {
                    return Ok(array);
                }
                let entry_exposure = range_exposure(
                    source.entries().len(),
                    source.len(),
                    exposure,
                    |row| source.is_valid(row),
                    |row| {
                        let offsets = source.value_offsets();
                        offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))
                    },
                    budget,
                )?;
                let source_entries = Arc::new(source.entries().clone()) as ArrayRef;
                let entries = entries.cast_exposed(
                    Arc::clone(&source_entries),
                    entry_exposure.as_ref(),
                    budget,
                )?;
                let unchanged =
                    self.source_type == self.expected && Arc::ptr_eq(&entries, &source_entries);
                let output = if unchanged {
                    array
                } else {
                    let entries = downcast::<StructArray>(&entries)?.clone();
                    Arc::new(MapArray::try_new(
                        Arc::clone(field),
                        source.offsets().clone(),
                        entries,
                        source.nulls().cloned(),
                        ordered,
                    )?) as ArrayRef
                };
                let DataType::Mapping(target_map) = self.field.dtype() else {
                    return Err(internal_target_error("map"));
                };
                if !unchanged || source_map != target_map {
                    validate_map_invariants(target_map, output.as_ref(), exposure, budget)?;
                }
                Ok(output)
            }
        }
    }
    mod repair {
        use super::*;

        pub(crate) fn fill_nulls(
            field: &Field,
            array: ArrayRef,
            dtype_semantics: bool,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            if dtype_semantics && field.dtype().is_default_value(&Scalar::Null)? {
                return Ok(array);
            }
            if let DataType::Enum(EnumType::Dictionary(dictionary)) = field.dtype() {
                return fill_dictionary_nulls(field, dictionary, array, exposure, budget);
            }
            let phase = budget.mark();
            let logical = logical_validity_buffer(array.as_ref(), field.dtype(), budget)?;
            let default_count = exposed_null_count(&logical, exposure);
            if default_count == 0 {
                budget.restore(phase);
                return Ok(array);
            }
            let exposed_count = exposure.map_or(array.len(), BooleanBuffer::count_set_bits);
            if default_count == exposed_count && contains_dictionary(field.dtype()) {
                budget.restore(phase);
                return default_array(field, array.len(), None, budget);
            }

            // Reserve the one-row scalar and both parts of the final zip before
            // constructing any default. The source-range walk charges only values the
            // truthy side copies; exposed nulls are charged through canonical defaults.
            budget.add_default_scalar_scratch(field.dtype())?;
            if has_derived_logical_nulls(field.dtype()) {
                budget.add_bitmap(1)?;
            }
            if contains_dictionary(field.dtype()) {
                budget
                    .add_repeated_default_without_dictionary_values(field.dtype(), default_count)?;
            } else {
                budget.add_repeated_default(field.dtype(), default_count)?;
            }
            let full = [(0, array.len())];
            let truthy_ranges = selected_child_ranges(
                SourceSelection::Ranges(&full),
                array.len(),
                array.len(),
                |index| !is_exposed(exposure, index) || logical.is_valid(index),
                |index| Ok((index, index + 1)),
                budget,
            )?;
            let source_type = DataType::from_arrow_datatype(array.data_type())?;
            reserve_source_selection(
                array.as_ref(),
                &source_type,
                SourceSelection::Ranges(&truthy_ranges),
                budget,
            )?;
            if exposure.is_some() {
                budget.add_bitmap(array.len())?;
            }

            let source_for_retention = Arc::clone(&array);
            let default = if dtype_semantics {
                field.dtype().default_arrow_array()?
            } else {
                field.default_arrow_array()?
            };
            if is_logically_null(default.as_ref(), 0) {
                budget.restore(phase);
                return Ok(array);
            }
            let mask = match exposure {
                None => logical.inner().clone(),
                Some(exposure) => BooleanBuffer::collect_bool(array.len(), |index| {
                    !exposure.value(index) || logical.is_valid(index)
                }),
            };
            let mask = BooleanArray::new(mask, None);
            let (array, default) = if contains_dictionary(field.dtype()) {
                budget.add_bitmap(array.len())?;
                let live = BooleanBuffer::collect_bool(array.len(), |index| {
                    is_exposed(exposure, index) && logical.is_valid(index)
                });
                align_nested_dictionaries(field, &array, &default, Some(&live), None, budget)?
            } else {
                (array, default)
            };
            let default = ArrowScalar::new(default);
            let truthy: &dyn Array = array.as_ref();
            let output = zip(&mask, &truthy, &default)?;

            // Only the zip output survives this phase. Release the scalar, mask, and
            // range-planning reservations, then retain the exact two output parts in
            // the operation-wide aggregate for following columns.
            budget.restore(phase);
            if contains_dictionary(field.dtype()) {
                budget
                    .add_repeated_default_without_dictionary_values(field.dtype(), default_count)?;
            } else {
                budget.add_repeated_default(field.dtype(), default_count)?;
            }
            reserve_source_selection(
                source_for_retention.as_ref(),
                &source_type,
                SourceSelection::Ranges(&truthy_ranges),
                budget,
            )?;
            if contains_dictionary(field.dtype()) {
                reserve_new_dictionary_vocabularies(
                    &output,
                    &source_for_retention,
                    field.dtype(),
                    budget,
                )?;
            }
            Ok(output)
        }

        pub(crate) fn fill_dictionary_nulls(
            field: &Field,
            dictionary: &crate::DictionaryType,
            array: ArrayRef,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            macro_rules! fill {
                ($key:ty) => {{ fill_dictionary_nulls_typed::<$key>(field, dictionary, array, exposure, budget) }};
            }
            match dictionary.key() {
                DataType::Int8 => fill!(Int8Type),
                DataType::Int16 => fill!(Int16Type),
                DataType::Int32 => fill!(Int32Type),
                DataType::Int64 => fill!(Int64Type),
                DataType::UInt8 => fill!(UInt8Type),
                DataType::UInt16 => fill!(UInt16Type),
                DataType::UInt32 => fill!(UInt32Type),
                DataType::UInt64 => fill!(UInt64Type),
                key => Err(Error::Unsupported {
                    kind: key.name(),
                    reason: format!(
                        "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
                    ),
                }),
            }
        }

        #[allow(clippy::too_many_lines)]
        pub(crate) fn fill_dictionary_nulls_typed<K>(
            field: &Field,
            dictionary: &crate::DictionaryType,
            array: ArrayRef,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef>
        where
            K: ArrowDictionaryKeyType,
            K::Native: TryFrom<usize>,
        {
            let source = downcast::<DictionaryArray<K>>(&array)?;
            let phase = budget.mark();
            let logical = logical_validity_buffer(source, field.dtype(), budget)?;
            let repair_count = exposed_null_count(&logical, exposure);
            if repair_count == 0 {
                budget.restore(phase);
                return Ok(array);
            }

            let value_type = dictionary.value();
            budget.add_default_scalar_scratch(value_type)?;
            if has_derived_logical_nulls(value_type) {
                budget.add_bitmap(1)?;
            }
            let default = value_type.default_arrow_array()?;
            if is_logically_null(default.as_ref(), 0) {
                budget.restore(phase);
                return Ok(array);
            }

            // Compact to values referenced by rows that survive the repair. Raw
            // dictionary vocabularies are not part of the logical output and may be
            // arbitrarily wider than the key capacity or materialization budget.
            let values = source.values();
            let mut used =
                scratch_vec::<usize>(budget, source.len(), "dictionary live-value indices")?;
            for index in 0..source.len() {
                if is_exposed(exposure, index) && logical.is_null(index) {
                    continue;
                }
                if source.keys().is_valid(index) {
                    used.push(source.keys().value(index).as_usize());
                }
            }
            used.sort_unstable();
            used.dedup();
            if used.iter().any(|index| *index >= values.len()) {
                return Err(Error::IncompatibleSchema(
                    "dictionary key points outside its values array".to_owned(),
                ));
            }

            reserve_vec_bytes::<usize>(budget, used.len())?;
            let mut by_value = used.clone();
            let compare_values = make_yggdryl_key_comparator(value_type, values, budget)?;
            by_value.sort_unstable_by(|left, right| compare_values(*left, *right));
            let mut representatives = scratch_vec::<usize>(
                budget,
                by_value.len().saturating_add(1),
                "dictionary compact vocabulary",
            )?;
            let mut mappings =
                scratch_vec::<(usize, usize)>(budget, by_value.len(), "dictionary key remapping")?;
            for old in by_value {
                let group = if representatives
                    .last()
                    .is_some_and(|prior| compare_values(*prior, old) == Ordering::Equal)
                {
                    representatives.len() - 1
                } else {
                    representatives.push(old);
                    representatives.len() - 1
                };
                mappings.push((old, group));
            }
            mappings.sort_unstable_by_key(|(old, _)| *old);

            // Only reachable representatives may be retained in the output
            // vocabulary. Searching the raw vocabulary here would make a one-row
            // dictionary over a very long run-end encoded value array take work
            // proportional to the hidden logical length.
            let compare_default = make_yggdryl_comparator(value_type, values, &default, budget)?;
            let mut default_index = representatives
                .iter()
                .position(|index| compare_default(*index, 0) == Ordering::Equal);
            let mut appended_default = false;
            if default_index.is_none() {
                default_index = Some(representatives.len());
                appended_default = true;
            }
            let default_index = default_index.ok_or_else(|| {
                Error::IncompatibleSchema("dictionary default index planning failed".to_owned())
            })?;
            let default_key = K::Native::try_from(default_index).map_err(|_| {
                Error::IncompatibleSchema(format!(
                    "dictionary live values plus its default exceed the {} key capacity",
                    dictionary.key()
                ))
            })?;

            budget.add_array_layout(field.dtype(), source.len())?;
            reserve_vec_bytes::<u32>(budget, representatives.len())?;
            let selected = representatives
                .iter()
                .map(|index| {
                    u32::try_from(*index).map_err(|_| {
                        Error::IncompatibleSchema(
                            "dictionary value index exceeds Arrow UInt32 take capacity".to_owned(),
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let selected = UInt32Array::from(selected);
            let compact = if selected.is_empty() {
                None
            } else {
                budget.add_array(&DataType::UInt32, selected.len())?;
                reserve_source_selection(
                    values.as_ref(),
                    value_type,
                    SourceSelection::Indices(selected.values()),
                    budget,
                )?;
                Some(take(values.as_ref(), &selected, None)?)
            };
            let output_values = if appended_default {
                match compact {
                    None => Arc::clone(&default),
                    Some(compact) => {
                        let default_array = Arc::clone(&default);
                        let (compact, default_array) = if contains_dictionary(value_type) {
                            let value_field = Field::new("dictionary", value_type.clone(), true);
                            align_nested_dictionaries(
                                &value_field,
                                &compact,
                                &default_array,
                                None,
                                None,
                                budget,
                            )?
                        } else {
                            (compact, default_array)
                        };
                        reserve_concat_copy(compact.as_ref(), value_type, budget)?;
                        budget.add_repeated_default(value_type, 1)?;
                        concat(&[compact.as_ref(), default_array.as_ref()])?
                    }
                }
            } else {
                compact.ok_or_else(|| {
                    Error::IncompatibleSchema("dictionary compact vocabulary is empty".to_owned())
                })?
            };

            let mut keys = arrow_array::builder::PrimitiveBuilder::<K>::with_capacity(source.len());
            for index in 0..source.len() {
                if is_exposed(exposure, index) && logical.is_null(index) {
                    keys.append_value(default_key);
                } else if source.keys().is_null(index) {
                    keys.append_null();
                } else {
                    let old = source.keys().value(index).as_usize();
                    let position = mappings
                        .binary_search_by_key(&old, |(candidate, _)| *candidate)
                        .map_err(|_| {
                            Error::IncompatibleSchema(
                                "dictionary live key was not present in its compact mapping"
                                    .to_owned(),
                            )
                        })?;
                    let key = K::Native::try_from(mappings[position].1).map_err(|_| {
                        Error::IncompatibleSchema(
                            "dictionary compact key exceeds its physical key type".to_owned(),
                        )
                    })?;
                    keys.append_value(key);
                }
            }
            let keys = keys.finish();
            let output = Arc::new(DictionaryArray::<K>::try_new(keys, output_values)?) as ArrayRef;

            budget.restore(phase);
            budget.add_array_layout(field.dtype(), source.len())?;
            reserve_new_dictionary_vocabularies(&output, &array, field.dtype(), budget)?;
            Ok(output)
        }

        #[allow(clippy::too_many_lines)] // Mirrors Arrow concat's nested layout dispatch.
        pub(crate) fn replace_array_children(
            array: &ArrayRef,
            children: Vec<ArrayRef>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            if array_children_unchanged(array, &children)? {
                return Ok(Arc::clone(array));
            }

            // The old recursive ArrayData tree and every replacement child-data tree
            // coexist until the rebuilt root takes ownership of the new child Vec.
            let phase = budget.mark();
            reserve_to_data_scratch(array, budget)?;
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, children.len())?;
            for child in &children {
                reserve_to_data_scratch(child, budget)?;
            }
            let data = array.to_data();
            if data.child_data().len() != children.len() {
                return Err(Error::IncompatibleSchema(
                    "dictionary alignment produced the wrong child count".to_owned(),
                ));
            }
            let children = children.into_iter().map(|child| child.to_data()).collect();
            let output = make_array(data.into_builder().child_data(children).build()?);
            // ArrayData handle Vecs are phase-local. The returned concrete array owns
            // the already-reserved child arrays/buffers, not these temporary clones.
            budget.restore(phase);
            Ok(output)
        }

        pub(crate) fn array_children_unchanged(
            array: &ArrayRef,
            children: &[ArrayRef],
        ) -> Result<bool> {
            let unchanged = match array.data_type() {
                ArrowDataType::Struct(_) => {
                    let source = downcast::<StructArray>(array.as_ref())?;
                    source.columns().len() == children.len()
                        && source
                            .columns()
                            .iter()
                            .zip(children)
                            .all(|(source, child)| Arc::ptr_eq(source, child))
                }
                ArrowDataType::List(_) => {
                    children.len() == 1
                        && Arc::ptr_eq(
                            downcast::<ListArray>(array.as_ref())?.values(),
                            &children[0],
                        )
                }
                ArrowDataType::LargeList(_) => {
                    children.len() == 1
                        && Arc::ptr_eq(
                            downcast::<LargeListArray>(array.as_ref())?.values(),
                            &children[0],
                        )
                }
                ArrowDataType::ListView(_) => {
                    children.len() == 1
                        && Arc::ptr_eq(
                            downcast::<ListViewArray>(array.as_ref())?.values(),
                            &children[0],
                        )
                }
                ArrowDataType::LargeListView(_) => {
                    children.len() == 1
                        && Arc::ptr_eq(
                            downcast::<LargeListViewArray>(array.as_ref())?.values(),
                            &children[0],
                        )
                }
                ArrowDataType::FixedSizeList(_, _) => {
                    children.len() == 1
                        && Arc::ptr_eq(
                            downcast::<FixedSizeListArray>(array.as_ref())?.values(),
                            &children[0],
                        )
                }
                ArrowDataType::Map(_, _) => {
                    let source = downcast::<MapArray>(array.as_ref())?.entries();
                    let target = children
                        .first()
                        .and_then(|child| child.as_any().downcast_ref::<StructArray>());
                    target.is_some_and(|target| {
                        source.columns().len() == target.columns().len()
                            && source
                                .columns()
                                .iter()
                                .zip(target.columns())
                                .all(|(source, target)| Arc::ptr_eq(source, target))
                            && null_buffers_ptr_eq(source.nulls(), target.nulls())
                    })
                }
                ArrowDataType::Union(fields, _) => {
                    let source = downcast::<UnionArray>(array.as_ref())?;
                    fields.len() == children.len()
                        && fields
                            .iter()
                            .zip(children)
                            .all(|((type_id, _), child)| Arc::ptr_eq(source.child(type_id), child))
                }
                ArrowDataType::RunEndEncoded(run_ends, _) if children.len() == 2 => {
                    match run_ends.data_type() {
                        ArrowDataType::Int16 => Arc::ptr_eq(
                            downcast::<Int16RunArray>(array.as_ref())?.values(),
                            &children[1],
                        ),
                        ArrowDataType::Int32 => Arc::ptr_eq(
                            downcast::<Int32RunArray>(array.as_ref())?.values(),
                            &children[1],
                        ),
                        ArrowDataType::Int64 => Arc::ptr_eq(
                            downcast::<Int64RunArray>(array.as_ref())?.values(),
                            &children[1],
                        ),
                        _ => false,
                    }
                }
                _ => false,
            };
            Ok(unchanged)
        }

        pub(crate) fn null_buffers_ptr_eq(
            left: Option<&arrow_buffer::NullBuffer>,
            right: Option<&arrow_buffer::NullBuffer>,
        ) -> bool {
            match (left, right) {
                (None, None) => true,
                (Some(left), Some(right)) => left.inner().ptr_eq(right.inner()),
                _ => false,
            }
        }

        pub(crate) fn ensure_list_child_physical(
            field: &Field,
            array: ArrayRef,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            if field.is_nullable()
                || exposed_logical_null_count(array.as_ref(), field.dtype(), None)? == 0
            {
                Ok(array)
            } else {
                // Arrow validates a List child Field independently of the parent List
                // validity bitmap. Hidden child slots therefore need a present
                // canonical value even when their parent row is null.
                fill_nulls(field, array, false, None, budget)
            }
        }

        pub(crate) fn default_array(
            field: &Field,
            len: usize,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            default_array_with(field, None, len, exposure, budget)
        }

        /// The same column, over a one-row default a caller already holds.
        ///
        /// `prebuilt` is exactly what `Field::default_arrow_array` would answer
        /// for this Field; a compiled plan materializes it once so a per-batch
        /// fill does not re-run the schema preflight behind it. `None` keeps the
        /// original behaviour, failure timing included.
        pub(crate) fn default_array_with(
            field: &Field,
            prebuilt: Option<&ArrayRef>,
            len: usize,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            // The one-row default, built once per plan or once per call.
            let default_row = || -> Result<ArrayRef> {
                match prebuilt {
                    Some(prebuilt) => Ok(Arc::clone(prebuilt)),
                    None => field.default_arrow_array(),
                }
            };
            let arrow_type = field.clone().into_arrow_field_ref()?.data_type().clone();
            if len == 0 {
                return Ok(arrow_array::new_empty_array(&arrow_type));
            }
            if let Some(exposure) = exposure {
                if exposure.len() != len {
                    return Err(Error::IncompatibleSchema(
                        "missing-field exposure mask has the wrong length".to_owned(),
                    ));
                }
            }
            let exposed = exposure.map_or(len, BooleanBuffer::count_set_bits);
            let hidden = len - exposed;
            if field.is_nullable() {
                budget.add_null_array(field.dtype(), len)?;
                return Ok(new_null_array(&arrow_type, len));
            }
            if exposed != 0 && hidden != 0 {
                if let DataType::Enum(EnumType::Dictionary(dictionary)) = field.dtype() {
                    let exposure = exposure.ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "mixed missing dictionary exposure requires a mask".to_owned(),
                        )
                    })?;
                    return default_dictionary_array(field, dictionary, exposure, budget);
                }
            }

            let phase = budget.mark();
            reserve_missing_output(field, exposed, hidden, budget)?;
            let single_scalar_output = len == 1 && (exposed == 1 || hidden == 1);
            if exposed != 0 && !single_scalar_output {
                reserve_field_default_scalar(field, budget)?;
            }
            if hidden != 0 && !single_scalar_output {
                budget.add_null_scalar_scratch(field.dtype())?;
            }

            let output = match (exposed, hidden) {
                (0, _) => {
                    if len != 1 {
                        budget.add_array(&DataType::UInt32, len)?;
                    }
                    let placeholder = crate::arrow::value::physical_placeholder_for_field(field)?;
                    let placeholder =
                        crate::arrow::value::array_from_values(field, &[&placeholder])?;
                    repeat_scalar(&placeholder, len)?
                }
                (_, 0) => {
                    if len != 1 {
                        budget.add_array(&DataType::UInt32, len)?;
                    }
                    let default = default_row()?;
                    repeat_scalar(&default, len)?
                }
                _ => {
                    let exposure = exposure.ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "mixed missing-field exposure requires a mask".to_owned(),
                        )
                    })?;
                    let default = default_row()?;
                    let placeholder = crate::arrow::value::physical_placeholder_for_field(field)?;
                    let placeholder =
                        crate::arrow::value::array_from_values(field, &[&placeholder])?;
                    let mask = BooleanArray::new(exposure.clone(), None);
                    let (default, placeholder) = if contains_dictionary(field.dtype()) {
                        align_nested_dictionaries(
                            field,
                            &default,
                            &placeholder,
                            None,
                            None,
                            budget,
                        )?
                    } else {
                        (default, placeholder)
                    };
                    let default = ArrowScalar::new(default);
                    let placeholder = ArrowScalar::new(placeholder);
                    zip(&mask, &default, &placeholder)?
                }
            };

            budget.restore(phase);
            reserve_missing_output(field, exposed, hidden, budget)?;
            Ok(output)
        }

        pub(crate) fn default_dictionary_array(
            field: &Field,
            dictionary: &crate::DictionaryType,
            exposure: &BooleanBuffer,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            macro_rules! build {
                ($key:ty) => {{ default_dictionary_array_typed::<$key>(field, dictionary, exposure, budget) }};
            }
            match dictionary.key() {
                DataType::Int8 => build!(Int8Type),
                DataType::Int16 => build!(Int16Type),
                DataType::Int32 => build!(Int32Type),
                DataType::Int64 => build!(Int64Type),
                DataType::UInt8 => build!(UInt8Type),
                DataType::UInt16 => build!(UInt16Type),
                DataType::UInt32 => build!(UInt32Type),
                DataType::UInt64 => build!(UInt64Type),
                key => Err(Error::Unsupported {
                    kind: key.name(),
                    reason: format!(
                        "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
                    ),
                }),
            }
        }

        pub(crate) fn default_dictionary_array_typed<K>(
            field: &Field,
            dictionary: &crate::DictionaryType,
            exposure: &BooleanBuffer,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef>
        where
            K: ArrowDictionaryKeyType,
            K::Native: TryFrom<usize>,
        {
            let phase = budget.mark();
            budget.add_array_layout(field.dtype(), exposure.len())?;
            budget.add_default_scalar_scratch(dictionary.value())?;
            let zero = K::Native::try_from(0).map_err(|_| {
                Error::IncompatibleSchema("dictionary key cannot represent zero".to_owned())
            })?;
            let values = dictionary.value().default_arrow_array()?;
            let mut keys =
                arrow_array::builder::PrimitiveBuilder::<K>::with_capacity(exposure.len());
            for index in 0..exposure.len() {
                if exposure.value(index) {
                    keys.append_value(zero);
                } else {
                    keys.append_null();
                }
            }
            let output =
                Arc::new(DictionaryArray::<K>::try_new(keys.finish(), values)?) as ArrayRef;
            budget.restore(phase);
            budget.add_array_layout(field.dtype(), exposure.len())?;
            budget.add_repeated_default(dictionary.value(), 1)?;
            Ok(output)
        }

        pub(crate) fn repeat_scalar(array: &ArrayRef, len: usize) -> Result<ArrayRef> {
            if len == 1 && array.len() == 1 {
                return Ok(Arc::clone(array));
            }
            let indices = UInt32Array::from_value(0, len);
            take(array.as_ref(), &indices, None).map_err(Into::into)
        }

        pub(crate) fn list_child(expected: &ArrowDataType) -> Result<ArrowFieldRef> {
            match expected {
                ArrowDataType::List(field)
                | ArrowDataType::ListView(field)
                | ArrowDataType::LargeList(field)
                | ArrowDataType::LargeListView(field)
                | ArrowDataType::FixedSizeList(field, _) => Ok(Arc::clone(field)),
                _ => Err(internal_target_error("list")),
            }
        }

        pub(crate) fn ensure_unambiguous_names(fields: &arrow_schema::Fields) -> Result<()> {
            for (index, field) in fields.iter().enumerate() {
                if fields[..index]
                    .iter()
                    .any(|prior| prior.name().eq_ignore_ascii_case(field.name()))
                {
                    return Err(Error::IncompatibleSchema(format!(
                        "ASCII-case-insensitive field name {:?} is ambiguous",
                        field.name()
                    )));
                }
            }
            Ok(())
        }

        pub(crate) fn folded_field_mapping(
            source: &arrow_schema::Fields,
            target: &[Field],
        ) -> Result<Vec<Option<usize>>> {
            if source.len().max(target.len()) <= HASHED_NAME_INDEX_THRESHOLD {
                ensure_unambiguous_names(source)?;
                ensure_unambiguous_target_names(target)?;
                return target
                    .iter()
                    .map(|field| folded_field_index(source, field.name()))
                    .collect();
            }

            let mut source_index = HashMap::with_capacity(source.len());
            for (index, field) in source.iter().enumerate() {
                let folded = field.name().to_ascii_lowercase();
                if source_index.insert(folded, index).is_some() {
                    return Err(Error::IncompatibleSchema(format!(
                        "ASCII-case-insensitive field name {:?} is ambiguous",
                        field.name()
                    )));
                }
            }
            let mut target_names = HashSet::with_capacity(target.len());
            let mut mapping = Vec::with_capacity(target.len());
            for field in target {
                let folded = field.name().to_ascii_lowercase();
                if !target_names.insert(folded.clone()) {
                    return Err(Error::IncompatibleSchema(format!(
                        "ASCII-case-insensitive target field name {:?} is ambiguous",
                        field.name()
                    )));
                }
                mapping.push(source_index.get(&folded).copied());
            }
            Ok(mapping)
        }

        pub(crate) fn ensure_unambiguous_target_names(fields: &[Field]) -> Result<()> {
            for (index, field) in fields.iter().enumerate() {
                if fields[..index]
                    .iter()
                    .any(|prior| prior.name().eq_ignore_ascii_case(field.name()))
                {
                    return Err(Error::IncompatibleSchema(format!(
                        "ASCII-case-insensitive target field name {:?} is ambiguous",
                        field.name()
                    )));
                }
            }
            Ok(())
        }

        pub(crate) fn folded_field_index(
            fields: &arrow_schema::Fields,
            name: &str,
        ) -> Result<Option<usize>> {
            let mut found = None;
            for (index, field) in fields.iter().enumerate() {
                if field.name().eq_ignore_ascii_case(name) {
                    if found.is_some() {
                        return Err(Error::IncompatibleSchema(format!(
                            "ASCII-case-insensitive field name {name:?} matches multiple source columns"
                        )));
                    }
                    found = Some(index);
                }
            }
            Ok(found)
        }
    }

    pub(crate) use dictionary::*;
    pub(crate) use repair::*;

    const HASHED_NAME_INDEX_THRESHOLD: usize = 16;

    pub(crate) fn validate_map_invariants(
        map: &crate::MappingType,
        array: &dyn Array,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<()> {
        let array = downcast::<MapArray>(array)?;
        let phase = budget.mark();
        let keys = array.entries().column(0);
        let Some([key_field, _]) = map.entries().dtype().as_fields() else {
            return Err(Error::IncompatibleSchema(
                "map entries must contain key and value fields".to_owned(),
            ));
        };
        let compare = make_yggdryl_key_comparator(key_field.dtype(), keys, budget)?;
        let offsets = array.value_offsets();

        let mut maximum_row_len = 0usize;
        for row in 0..array.len() {
            if !is_exposed(exposure, row) || array.is_null(row) {
                continue;
            }
            let (start, end) = offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))?;
            maximum_row_len = maximum_row_len.max(end.saturating_sub(start));
        }

        // Small maps are faster with direct comparisons. Wide rows share one
        // allocation across the complete array instead of creating a map and
        // schema for every logical row.
        let mut ordered_indices = Vec::new();
        if maximum_row_len > 16 {
            budget.add_array(&DataType::UInt64, maximum_row_len)?;
            ordered_indices
                .try_reserve_exact(maximum_row_len)
                .map_err(|error| {
                    Error::IncompatibleSchema(format!(
                        "map-key validation scratch allocation failed: {error}"
                    ))
                })?;
        }
        for row in 0..array.len() {
            if !is_exposed(exposure, row) || array.is_null(row) {
                continue;
            }
            let (start, end) = offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))?;
            for index in start..end {
                if logical_null_at(keys.as_ref(), key_field.dtype(), index)? {
                    return Err(Error::IncompatibleSchema(format!(
                        "map row {row} has a null key at entry {}",
                        index - start
                    )));
                }
            }
            if end - start <= 16 {
                for index in (start + 1)..end {
                    if (start..index).any(|previous| compare(previous, index) == Ordering::Equal) {
                        return Err(Error::IncompatibleSchema(format!(
                            "map row {row} has a duplicate key at entry {}",
                            index - start
                        )));
                    }
                }
            } else {
                ordered_indices.clear();
                ordered_indices.extend(start..end);
                ordered_indices.sort_unstable_by(|left, right| compare(*left, *right));
                if ordered_indices
                    .windows(2)
                    .any(|pair| compare(pair[0], pair[1]) == Ordering::Equal)
                {
                    return Err(Error::IncompatibleSchema(format!(
                        "map row {row} has duplicate keys"
                    )));
                }
            }

            if map.keys_sorted()
                && (start..end.saturating_sub(1))
                    .any(|index| compare(index, index + 1) == Ordering::Greater)
            {
                return Err(Error::IncompatibleSchema(format!(
                    "map row {row} declares sorted keys but values are not ordered"
                )));
            }
        }
        budget.restore(phase);
        Ok(())
    }

    pub(crate) fn requires_yggdryl_key_comparator(dtype: &DataType) -> bool {
        match dtype {
            DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Decimal(DecimalType::Decimal256 { .. })
            | DataType::Union(..)
            | DataType::Enum(EnumType::Dictionary(_))
            | DataType::RunEndEncoded(_) => true,
            DataType::Sequence(SequenceType::List(child))
            | DataType::Sequence(SequenceType::ListView(child))
            | DataType::Sequence(SequenceType::FixedSizeList(child, _))
            | DataType::Sequence(SequenceType::LargeList(child))
            | DataType::Sequence(SequenceType::LargeListView(child)) => {
                requires_yggdryl_key_comparator(child.dtype())
            }
            DataType::Struct(fields) => fields
                .iter()
                .any(|field| requires_yggdryl_key_comparator(field.dtype())),
            DataType::Mapping(map) => requires_yggdryl_key_comparator(map.entries().dtype()),
            _ => false,
        }
    }

    pub(crate) fn has_derived_logical_nulls(dtype: &DataType) -> bool {
        matches!(
            dtype,
            DataType::Null
                | DataType::Enum(EnumType::Dictionary(_))
                | DataType::Union(..)
                | DataType::RunEndEncoded(_)
        )
    }

    pub(crate) fn wrap_yggdryl_nulls(
        left: &ArrayRef,
        right: &ArrayRef,
        dtype: &DataType,
        compare: DynComparator,
        budget: &mut MaterializationBudget,
    ) -> Result<DynComparator> {
        if has_derived_logical_nulls(dtype) {
            budget.add_bitmap(left.len())?;
            if !Arc::ptr_eq(left, right) {
                budget.add_bitmap(right.len())?;
            }
        }
        let left_nulls = left.logical_nulls();
        let right_nulls = if Arc::ptr_eq(left, right) {
            left_nulls.clone()
        } else {
            right.logical_nulls()
        };
        if left_nulls.is_none() && right_nulls.is_none() {
            return Ok(compare);
        }
        Ok(Box::new(move |left, right| {
            let left_null = left_nulls.as_ref().is_some_and(|nulls| nulls.is_null(left));
            let right_null = right_nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(right));
            match (left_null, right_null) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => compare(left, right),
            }
        }))
    }

    pub(crate) fn dictionary_key_comparator<K: ArrowDictionaryKeyType>(
        left: &ArrayRef,
        right: &ArrayRef,
        dictionary: &crate::DictionaryType,
        budget: &mut MaterializationBudget,
    ) -> Result<DynComparator> {
        let left_source = downcast::<DictionaryArray<K>>(left.as_ref())?;
        let right_source = downcast::<DictionaryArray<K>>(right.as_ref())?;
        let left_values = Arc::clone(left_source.values());
        let right_values = Arc::clone(right_source.values());
        let value_compare =
            make_yggdryl_comparator(dictionary.value(), &left_values, &right_values, budget)?;
        let left_keys = left_source.keys().values().clone();
        let right_keys = right_source.keys().values().clone();
        let left_nulls = left_source.keys().nulls().cloned();
        let right_nulls = right_source.keys().nulls().cloned();
        Ok(Box::new(move |left, right| {
            let left_null = left_nulls.as_ref().is_some_and(|nulls| nulls.is_null(left));
            let right_null = right_nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(right));
            match (left_null, right_null) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => {
                    value_compare(left_keys[left].as_usize(), right_keys[right].as_usize())
                }
            }
        }))
    }

    pub(crate) fn run_key_comparator<R: RunEndIndexType>(
        left: &ArrayRef,
        right: &ArrayRef,
        encoded: &crate::RunEndEncodedType,
        budget: &mut MaterializationBudget,
    ) -> Result<DynComparator> {
        let left_source = downcast::<RunArray<R>>(left.as_ref())?;
        let right_source = downcast::<RunArray<R>>(right.as_ref())?;
        let left_values = Arc::clone(left_source.values());
        let right_values = Arc::clone(right_source.values());
        let value_compare = make_yggdryl_comparator(
            encoded.values().dtype(),
            &left_values,
            &right_values,
            budget,
        )?;
        let left_run_ends = left_source.run_ends().clone();
        let right_run_ends = right_source.run_ends().clone();
        Ok(Box::new(move |left, right| {
            value_compare(
                left_run_ends.get_physical_index(left),
                right_run_ends.get_physical_index(right),
            )
        }))
    }

    #[allow(clippy::too_many_lines)] // Recurses only where native Scalar ordering differs from Arrow.
    pub(crate) fn make_yggdryl_key_comparator(
        dtype: &DataType,
        array: &ArrayRef,
        budget: &mut MaterializationBudget,
    ) -> Result<DynComparator> {
        make_yggdryl_comparator(dtype, array, array, budget)
    }

    #[allow(clippy::too_many_lines)] // Recurses only where native Scalar ordering differs from Arrow.
    pub(crate) fn make_yggdryl_comparator(
        dtype: &DataType,
        left: &ArrayRef,
        right: &ArrayRef,
        budget: &mut MaterializationBudget,
    ) -> Result<DynComparator> {
        if !requires_yggdryl_key_comparator(dtype) {
            if has_derived_logical_nulls(dtype) {
                budget.add_bitmap(left.len())?;
                if !Arc::ptr_eq(left, right) {
                    budget.add_bitmap(right.len())?;
                }
            }
            return make_comparator(left.as_ref(), right.as_ref(), SortOptions::default())
                .map_err(Into::into);
        }

        let compare: DynComparator = match dtype {
            DataType::Float16 => {
                let left_values = downcast::<Float16Array>(left.as_ref())?.values().clone();
                let right_values = downcast::<Float16Array>(right.as_ref())?.values().clone();
                Box::new(move |left, right| {
                    crate::Float16::from_f16(left_values[left])
                        .cmp(&crate::Float16::from_f16(right_values[right]))
                })
            }
            DataType::Float32 => {
                let left_values = downcast::<Float32Array>(left.as_ref())?.values().clone();
                let right_values = downcast::<Float32Array>(right.as_ref())?.values().clone();
                Box::new(move |left, right| {
                    crate::Float32::from_f32(left_values[left])
                        .cmp(&crate::Float32::from_f32(right_values[right]))
                })
            }
            DataType::Float64 => {
                let left_values = downcast::<Float64Array>(left.as_ref())?.values().clone();
                let right_values = downcast::<Float64Array>(right.as_ref())?.values().clone();
                Box::new(move |left, right| {
                    crate::Float64::from_f64(left_values[left])
                        .cmp(&crate::Float64::from_f64(right_values[right]))
                })
            }
            DataType::Decimal(DecimalType::Decimal256 { .. }) => {
                let left_values = downcast::<Decimal256Array>(left.as_ref())?.values().clone();
                let right_values = downcast::<Decimal256Array>(right.as_ref())?
                    .values()
                    .clone();
                Box::new(move |left, right| {
                    DecimalText::new(left_values[left])
                        .as_bytes()
                        .cmp(DecimalText::new(right_values[right]).as_bytes())
                })
            }
            DataType::Sequence(SequenceType::List(child)) => {
                let left_source = downcast::<ListArray>(left.as_ref())?;
                let right_source = downcast::<ListArray>(right.as_ref())?;
                let left_offsets = left_source.offsets().clone();
                let right_offsets = right_source.offsets().clone();
                let left_values = Arc::clone(left_source.values());
                let right_values = Arc::clone(right_source.values());
                let child_compare =
                    make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
                Box::new(move |left, right| {
                    let left = left_offsets[left].as_usize()..left_offsets[left + 1].as_usize();
                    let right =
                        right_offsets[right].as_usize()..right_offsets[right + 1].as_usize();
                    for (left, right) in left.clone().zip(right.clone()) {
                        let ordering = child_compare(left, right);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    left.len().cmp(&right.len())
                })
            }
            DataType::Sequence(SequenceType::LargeList(child)) => {
                let left_source = downcast::<LargeListArray>(left.as_ref())?;
                let right_source = downcast::<LargeListArray>(right.as_ref())?;
                let left_offsets = left_source.offsets().clone();
                let right_offsets = right_source.offsets().clone();
                let left_values = Arc::clone(left_source.values());
                let right_values = Arc::clone(right_source.values());
                let child_compare =
                    make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
                Box::new(move |left, right| {
                    let left = left_offsets[left].as_usize()..left_offsets[left + 1].as_usize();
                    let right =
                        right_offsets[right].as_usize()..right_offsets[right + 1].as_usize();
                    for (left, right) in left.clone().zip(right.clone()) {
                        let ordering = child_compare(left, right);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    left.len().cmp(&right.len())
                })
            }
            DataType::Sequence(SequenceType::ListView(child)) => {
                let left_source = downcast::<ListViewArray>(left.as_ref())?;
                let right_source = downcast::<ListViewArray>(right.as_ref())?;
                let left_offsets = left_source.offsets().clone();
                let right_offsets = right_source.offsets().clone();
                let left_sizes = left_source.sizes().clone();
                let right_sizes = right_source.sizes().clone();
                let left_values = Arc::clone(left_source.values());
                let right_values = Arc::clone(right_source.values());
                let child_compare =
                    make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
                Box::new(move |left, right| {
                    let left_start = left_offsets[left].as_usize();
                    let right_start = right_offsets[right].as_usize();
                    let left_len = left_sizes[left].as_usize();
                    let right_len = right_sizes[right].as_usize();
                    for offset in 0..left_len.min(right_len) {
                        let ordering = child_compare(left_start + offset, right_start + offset);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    left_len.cmp(&right_len)
                })
            }
            DataType::Sequence(SequenceType::LargeListView(child)) => {
                let left_source = downcast::<LargeListViewArray>(left.as_ref())?;
                let right_source = downcast::<LargeListViewArray>(right.as_ref())?;
                let left_offsets = left_source.offsets().clone();
                let right_offsets = right_source.offsets().clone();
                let left_sizes = left_source.sizes().clone();
                let right_sizes = right_source.sizes().clone();
                let left_values = Arc::clone(left_source.values());
                let right_values = Arc::clone(right_source.values());
                let child_compare =
                    make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
                Box::new(move |left, right| {
                    let left_start = left_offsets[left].as_usize();
                    let right_start = right_offsets[right].as_usize();
                    let left_len = left_sizes[left].as_usize();
                    let right_len = right_sizes[right].as_usize();
                    for offset in 0..left_len.min(right_len) {
                        let ordering = child_compare(left_start + offset, right_start + offset);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    left_len.cmp(&right_len)
                })
            }
            DataType::Sequence(SequenceType::FixedSizeList(child, size)) => {
                let left_values =
                    Arc::clone(downcast::<FixedSizeListArray>(left.as_ref())?.values());
                let right_values =
                    Arc::clone(downcast::<FixedSizeListArray>(right.as_ref())?.values());
                let child_compare =
                    make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
                let size = usize::try_from(*size).map_err(|_| {
                    Error::IncompatibleSchema("map key fixed-list size is negative".to_owned())
                })?;
                Box::new(move |left, right| {
                    let left = left * size;
                    let right = right * size;
                    for offset in 0..size {
                        let ordering = child_compare(left + offset, right + offset);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    Ordering::Equal
                })
            }
            DataType::Struct(fields) => {
                let left_source = downcast::<StructArray>(left.as_ref())?;
                let right_source = downcast::<StructArray>(right.as_ref())?;
                let comparators = fields
                    .iter()
                    .zip(left_source.columns())
                    .zip(right_source.columns())
                    .map(|((field, left), right)| {
                        make_yggdryl_comparator(field.dtype(), left, right, budget)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Box::new(move |left, right| {
                    comparators
                        .iter()
                        .map(|compare| compare(left, right))
                        .find(|ordering| *ordering != Ordering::Equal)
                        .unwrap_or(Ordering::Equal)
                })
            }
            DataType::Mapping(map) => {
                let left_source = downcast::<MapArray>(left.as_ref())?;
                let right_source = downcast::<MapArray>(right.as_ref())?;
                let left_offsets = left_source.offsets().clone();
                let right_offsets = right_source.offsets().clone();
                let left_entries: ArrayRef = Arc::new(left_source.entries().clone());
                let right_entries: ArrayRef = Arc::new(right_source.entries().clone());
                let entry_compare = make_yggdryl_comparator(
                    map.entries().dtype(),
                    &left_entries,
                    &right_entries,
                    budget,
                )?;
                Box::new(move |left, right| {
                    let left = left_offsets[left].as_usize()..left_offsets[left + 1].as_usize();
                    let right =
                        right_offsets[right].as_usize()..right_offsets[right + 1].as_usize();
                    for (left, right) in left.clone().zip(right.clone()) {
                        let ordering = entry_compare(left, right);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    left.len().cmp(&right.len())
                })
            }
            DataType::Enum(EnumType::Dictionary(dictionary)) => {
                return match dictionary.key() {
                    DataType::Int8 => {
                        dictionary_key_comparator::<Int8Type>(left, right, dictionary, budget)
                    }
                    DataType::Int16 => {
                        dictionary_key_comparator::<Int16Type>(left, right, dictionary, budget)
                    }
                    DataType::Int32 => {
                        dictionary_key_comparator::<Int32Type>(left, right, dictionary, budget)
                    }
                    DataType::Int64 => {
                        dictionary_key_comparator::<Int64Type>(left, right, dictionary, budget)
                    }
                    DataType::UInt8 => {
                        dictionary_key_comparator::<UInt8Type>(left, right, dictionary, budget)
                    }
                    DataType::UInt16 => {
                        dictionary_key_comparator::<UInt16Type>(left, right, dictionary, budget)
                    }
                    DataType::UInt32 => {
                        dictionary_key_comparator::<UInt32Type>(left, right, dictionary, budget)
                    }
                    DataType::UInt64 => {
                        dictionary_key_comparator::<UInt64Type>(left, right, dictionary, budget)
                    }
                    _ => Err(Error::IncompatibleSchema(
                        "map key dictionary index is not an integer".to_owned(),
                    )),
                };
            }
            DataType::Union(fields, _) => {
                let left_source = downcast::<UnionArray>(left.as_ref())?.clone();
                let right_source = downcast::<UnionArray>(right.as_ref())?.clone();
                let mut comparators = HashMap::with_capacity(fields.len());
                for (type_id, field) in fields {
                    comparators.insert(
                        type_id,
                        make_yggdryl_comparator(
                            field.dtype(),
                            left_source.child(type_id),
                            right_source.child(type_id),
                            budget,
                        )?,
                    );
                }
                Box::new(move |left, right| {
                    let left_id = left_source.type_id(left);
                    let right_id = right_source.type_id(right);
                    match left_id.cmp(&right_id) {
                        Ordering::Equal => {
                            comparators
                                .get(&left_id)
                                .map_or(Ordering::Equal, |compare| {
                                    compare(
                                        left_source.value_offset(left),
                                        right_source.value_offset(right),
                                    )
                                })
                        }
                        ordering => ordering,
                    }
                })
            }
            DataType::RunEndEncoded(encoded) => {
                return match encoded.run_ends().dtype() {
                    DataType::Int16 => {
                        run_key_comparator::<Int16Type>(left, right, encoded, budget)
                    }
                    DataType::Int32 => {
                        run_key_comparator::<Int32Type>(left, right, encoded, budget)
                    }
                    DataType::Int64 => {
                        run_key_comparator::<Int64Type>(left, right, encoded, budget)
                    }
                    _ => Err(Error::IncompatibleSchema(
                        "map key run-end type is invalid".to_owned(),
                    )),
                };
            }
            _ => {
                return make_comparator(left.as_ref(), right.as_ref(), SortOptions::default())
                    .map_err(Into::into);
            }
        };
        // Union's native representation is always a present `[id, payload]`
        // sequence. Every other sensitive wrapper follows ordinary Scalar nulls.
        if matches!(dtype, DataType::Union(..)) {
            Ok(compare)
        } else {
            wrap_yggdryl_nulls(left, right, dtype, compare, budget)
        }
    }

    pub(crate) fn cast_dictionary_planned(
        source_key: &ArrowDataType,
        plan: &ArrayCastPlan,
        array: ArrayRef,
        values: &ArrayCastPlan,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let expected = &plan.expected;
        macro_rules! rebuild {
            ($key:ty) => {{
                let source = downcast::<DictionaryArray<$key>>(&array)?;
                let source_values = source.values();
                if array.data_type() == expected
                    && !(0..source.len())
                        .any(|row| is_exposed(exposure, row) && source.keys().is_valid(row))
                {
                    // An exact dictionary with no reachable key can retain its
                    // opaque vocabulary. Building a dense value-exposure bitmap
                    // here would expand compact REE or view-backed vocabularies
                    // solely to describe an empty selection.
                    return Ok(array);
                }
                let value_exposure = selected_index_exposure(
                    source_values.len(),
                    source.len(),
                    exposure,
                    |row| {
                        source
                            .keys()
                            .is_valid(row)
                            .then(|| source.keys().value(row).try_into().ok())
                            .flatten()
                    },
                    budget,
                )?;
                let values = values.cast_exposed(
                    Arc::clone(source_values),
                    value_exposure.as_ref(),
                    budget,
                )?;
                if array.data_type() == expected && Arc::ptr_eq(&values, source_values) {
                    return Ok(array);
                }
                Arc::new(DictionaryArray::<$key>::try_new(
                    source.keys().clone(),
                    values,
                )?) as ArrayRef
            }};
        }
        let rebuilt = match source_key {
            ArrowDataType::Int8 => rebuild!(Int8Type),
            ArrowDataType::Int16 => rebuild!(Int16Type),
            ArrowDataType::Int32 => rebuild!(Int32Type),
            ArrowDataType::Int64 => rebuild!(Int64Type),
            ArrowDataType::UInt8 => rebuild!(UInt8Type),
            ArrowDataType::UInt16 => rebuild!(UInt16Type),
            ArrowDataType::UInt32 => rebuild!(UInt32Type),
            ArrowDataType::UInt64 => rebuild!(UInt64Type),
            _ => {
                return arrow_cast_exposed(
                    &array,
                    expected,
                    plan.safe(),
                    exposure,
                    &plan.field,
                    budget,
                );
            }
        };
        if rebuilt.data_type() == expected {
            Ok(rebuilt)
        } else {
            arrow_cast_exposed(
                &rebuilt,
                expected,
                plan.safe(),
                exposure,
                &plan.field,
                budget,
            )
        }
    }

    pub(crate) fn cast_union_planned(
        fields: &arrow_schema::UnionFields,
        array: ArrayRef,
        plans: &[(i8, ArrayCastPlan)],
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let source = downcast::<UnionArray>(&array)?;
        let source_mode = if source.offsets().is_some() {
            arrow_schema::UnionMode::Dense
        } else {
            arrow_schema::UnionMode::Sparse
        };
        let mut unchanged = array.data_type() == &ArrowDataType::Union(fields.clone(), source_mode);
        let mut children = Vec::with_capacity(plans.len());
        for (type_id, plan) in plans {
            let source_child = source.child(*type_id);
            let child_exposure = selected_index_exposure(
                source_child.len(),
                source.len(),
                exposure,
                |row| (source.type_id(row) == *type_id).then(|| source.value_offset(row)),
                budget,
            )?;
            let child =
                plan.cast_exposed(Arc::clone(source_child), child_exposure.as_ref(), budget)?;
            unchanged &= Arc::ptr_eq(&child, source_child);
            children.push(child);
        }
        if unchanged {
            return Ok(array);
        }
        Ok(Arc::new(UnionArray::try_new(
            fields.clone(),
            source.type_ids().clone(),
            source.offsets().cloned(),
            children,
        )?))
    }

    pub(crate) fn run_value_exposure<R: RunEndIndexType>(
        source: &RunArray<R>,
        parent: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<Option<BooleanBuffer>> {
        if parent.is_some_and(|parent| parent.len() != source.len()) {
            return Err(Error::IncompatibleSchema(
                "run-end exposure has the wrong logical length".to_owned(),
            ));
        }
        if source.is_empty() {
            if source.values().is_empty() {
                return Ok(None);
            }
            budget.add_bitmap(source.values().len())?;
            return Ok(Some(BooleanBuffer::new_unset(source.values().len())));
        }
        let first_physical = source.get_start_physical_index();
        let last_physical = source.get_end_physical_index();
        if parent.is_none()
            && first_physical == 0
            && last_physical.checked_add(1) == Some(source.values().len())
        {
            return Ok(None);
        }

        budget.add_bitmap(source.values().len())?;
        let mut builder = BooleanBufferBuilder::new(source.values().len());
        builder.append_n(source.values().len(), false);
        let mut start = 0usize;
        let mut selected = 0usize;
        for (offset, end) in source.run_ends().sliced_values().enumerate() {
            let end = end.as_usize();
            let visible = parent.is_none_or(|parent| {
                end > start && parent.slice(start, end - start).count_set_bits() != 0
            });
            if visible {
                let physical = first_physical + offset;
                if physical >= source.values().len() {
                    return Err(Error::IncompatibleSchema(
                        "run-end value index exceeds its values array".to_owned(),
                    ));
                }
                builder.set_bit(physical, true);
                selected += 1;
            }
            start = end;
        }
        if selected == source.values().len() {
            Ok(None)
        } else {
            Ok(Some(builder.build()))
        }
    }

    pub(crate) fn cast_run_planned(
        source_run_type: &ArrowDataType,
        expected: &ArrowDataType,
        array: ArrayRef,
        values: &ArrayCastPlan,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        macro_rules! rebuild {
            ($array:ty, $key:ty) => {{
                let source = downcast::<$array>(&array)?;
                let source_values = source.values();
                let value_exposure = run_value_exposure(source, exposure, budget)?;
                let values = values.cast_exposed(
                    Arc::clone(source_values),
                    value_exposure.as_ref(),
                    budget,
                )?;
                if array.data_type() == expected && Arc::ptr_eq(&values, source_values) {
                    return Ok(array);
                }
                if array.offset() != 0 {
                    return Err(Error::Unsupported {
                        kind: "run_end_encoded",
                        reason: "nested casting of a sliced run-end encoded array is not supported"
                            .to_owned(),
                    });
                }
                let run_ends = PrimitiveArray::<$key>::new(source.run_ends().inner().clone(), None);
                Arc::new(<$array>::try_new(&run_ends, values.as_ref())?) as ArrayRef
            }};
        }
        let rebuilt = match source_run_type {
            ArrowDataType::Int16 => rebuild!(Int16RunArray, Int16Type),
            ArrowDataType::Int32 => rebuild!(Int32RunArray, Int32Type),
            ArrowDataType::Int64 => rebuild!(Int64RunArray, Int64Type),
            _ => {
                return Err(Error::IncompatibleSchema(
                    "run-end type must be Int16, Int32, or Int64".to_owned(),
                ));
            }
        };
        if rebuilt.data_type() == expected {
            return Ok(rebuilt);
        }
        reserve_to_data_scratch(&rebuilt, budget)?;
        let data = rebuilt
            .to_data()
            .into_builder()
            .data_type(expected.clone())
            .build()?;
        Ok(make_array(data))
    }

    pub(crate) fn contains_struct(dtype: &DataType) -> bool {
        match dtype {
            DataType::Struct(_) | DataType::Mapping(_) => true,
            DataType::Sequence(SequenceType::List(field))
            | DataType::Sequence(SequenceType::ListView(field))
            | DataType::Sequence(SequenceType::FixedSizeList(field, _))
            | DataType::Sequence(SequenceType::LargeList(field))
            | DataType::Sequence(SequenceType::LargeListView(field)) => {
                contains_struct(field.dtype())
            }
            DataType::Union(fields, _) => fields
                .iter()
                .any(|(_, field)| contains_struct(field.dtype())),
            DataType::Enum(EnumType::Dictionary(dictionary)) => contains_struct(dictionary.value()),
            DataType::RunEndEncoded(encoded) => contains_struct(encoded.values().dtype()),
            _ => false,
        }
    }

    pub(crate) fn is_reconcilable_nested(dtype: &DataType) -> bool {
        matches!(
            dtype,
            DataType::Sequence(SequenceType::List(_))
                | DataType::Sequence(SequenceType::ListView(_))
                | DataType::Sequence(SequenceType::FixedSizeList(_, _))
                | DataType::Sequence(SequenceType::LargeList(_))
                | DataType::Sequence(SequenceType::LargeListView(_))
                | DataType::Struct(_)
                | DataType::Union(_, _)
                | DataType::Enum(EnumType::Dictionary(_))
                | DataType::Mapping(_)
                | DataType::RunEndEncoded(_)
        )
    }
    pub(crate) fn is_logically_null(array: &dyn Array, index: usize) -> bool {
        array
            .logical_nulls()
            .is_some_and(|nulls| nulls.is_null(index))
    }

    pub(crate) fn is_exposed(exposure: Option<&BooleanBuffer>, index: usize) -> bool {
        exposure.is_none_or(|exposure| exposure.value(index))
    }

    pub(crate) fn dictionary_logical_null_at<K: ArrowDictionaryKeyType>(
        array: &dyn Array,
        dictionary: &crate::DictionaryType,
        index: usize,
    ) -> Result<bool> {
        let array = downcast::<DictionaryArray<K>>(array)?;
        if array.keys().is_null(index) {
            return Ok(true);
        }
        let value_index = array.keys().value(index).as_usize();
        if value_index >= array.values().len() {
            return Err(Error::IncompatibleSchema(
                "dictionary key points outside its values array".to_owned(),
            ));
        }
        logical_null_at(array.values().as_ref(), dictionary.value(), value_index)
    }

    pub(crate) fn run_logical_null_at<R: RunEndIndexType>(
        array: &dyn Array,
        encoded: &crate::RunEndEncodedType,
        index: usize,
    ) -> Result<bool> {
        let array = downcast::<RunArray<R>>(array)?;
        logical_null_at(
            array.values().as_ref(),
            encoded.values().dtype(),
            array.get_physical_index(index),
        )
    }

    pub(crate) fn logical_null_at(
        array: &dyn Array,
        dtype: &DataType,
        index: usize,
    ) -> Result<bool> {
        if index >= array.len() {
            return Err(Error::IncompatibleSchema(
                "logical-null index exceeds its Arrow array".to_owned(),
            ));
        }
        match dtype {
            DataType::Null => Ok(true),
            DataType::Enum(EnumType::Dictionary(dictionary)) => match dictionary.key() {
                DataType::Int8 => dictionary_logical_null_at::<Int8Type>(array, dictionary, index),
                DataType::Int16 => {
                    dictionary_logical_null_at::<Int16Type>(array, dictionary, index)
                }
                DataType::Int32 => {
                    dictionary_logical_null_at::<Int32Type>(array, dictionary, index)
                }
                DataType::Int64 => {
                    dictionary_logical_null_at::<Int64Type>(array, dictionary, index)
                }
                DataType::UInt8 => {
                    dictionary_logical_null_at::<UInt8Type>(array, dictionary, index)
                }
                DataType::UInt16 => {
                    dictionary_logical_null_at::<UInt16Type>(array, dictionary, index)
                }
                DataType::UInt32 => {
                    dictionary_logical_null_at::<UInt32Type>(array, dictionary, index)
                }
                DataType::UInt64 => {
                    dictionary_logical_null_at::<UInt64Type>(array, dictionary, index)
                }
                key => Err(Error::Unsupported {
                    kind: key.name(),
                    reason: format!(
                        "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
                    ),
                }),
            },
            DataType::Union(fields, _) => {
                let array = downcast::<UnionArray>(array)?;
                let type_id = array.type_id(index);
                let (_, field) = fields
                    .iter()
                    .find(|(candidate, _)| *candidate == type_id)
                    .ok_or_else(|| {
                        Error::IncompatibleSchema(format!("unknown union type id {type_id}"))
                    })?;
                logical_null_at(
                    array.child(type_id).as_ref(),
                    field.dtype(),
                    array.value_offset(index),
                )
            }
            DataType::RunEndEncoded(encoded) => match encoded.run_ends().dtype() {
                DataType::Int16 => run_logical_null_at::<Int16Type>(array, encoded, index),
                DataType::Int32 => run_logical_null_at::<Int32Type>(array, encoded, index),
                DataType::Int64 => run_logical_null_at::<Int64Type>(array, encoded, index),
                _ => Err(Error::IncompatibleSchema(
                    "run-end type is not a supported signed integer".to_owned(),
                )),
            },
            _ => Ok(array.is_null(index)),
        }
    }

    pub(crate) fn run_exposed_logical_null_count<R: RunEndIndexType>(
        array: &dyn Array,
        encoded: &crate::RunEndEncodedType,
        exposure: Option<&BooleanBuffer>,
    ) -> Result<usize> {
        let array = downcast::<RunArray<R>>(array)?;
        if exposure.is_some_and(|exposure| exposure.len() != array.len()) {
            return Err(Error::IncompatibleSchema(
                "run-end null-count exposure has the wrong length".to_owned(),
            ));
        }
        if array.is_empty() {
            return Ok(0);
        }
        let first_physical = array.get_start_physical_index();
        let mut start = 0usize;
        let mut null_count = 0usize;
        for (offset, end) in array.run_ends().sliced_values().enumerate() {
            let end = end.as_usize();
            if logical_null_at(
                array.values().as_ref(),
                encoded.values().dtype(),
                first_physical + offset,
            )? {
                let visible = exposure.map_or(end - start, |exposure| {
                    exposure.slice(start, end - start).count_set_bits()
                });
                null_count = null_count.checked_add(visible).ok_or_else(|| {
                    Error::IncompatibleSchema("logical null count exceeds usize".to_owned())
                })?;
            }
            start = end;
        }
        Ok(null_count)
    }

    pub(crate) fn exposed_logical_null_count(
        array: &dyn Array,
        dtype: &DataType,
        exposure: Option<&BooleanBuffer>,
    ) -> Result<usize> {
        if exposure.is_some_and(|exposure| exposure.len() != array.len()) {
            return Err(Error::IncompatibleSchema(
                "logical-null exposure has the wrong length".to_owned(),
            ));
        }
        if let DataType::RunEndEncoded(encoded) = dtype {
            return match encoded.run_ends().dtype() {
                DataType::Int16 => {
                    run_exposed_logical_null_count::<Int16Type>(array, encoded, exposure)
                }
                DataType::Int32 => {
                    run_exposed_logical_null_count::<Int32Type>(array, encoded, exposure)
                }
                DataType::Int64 => {
                    run_exposed_logical_null_count::<Int64Type>(array, encoded, exposure)
                }
                _ => Err(Error::IncompatibleSchema(
                    "run-end type is not a supported signed integer".to_owned(),
                )),
            };
        }
        // A datatype that does not derive absence from a child reads its nulls
        // straight off the Arrow validity buffer, which already carries the
        // count. Rediscovering it a row at a time is the one step whose cost is
        // the batch height on a cast that is otherwise pointer work, and every
        // node of every batch was paying it.
        if !has_derived_logical_nulls(dtype) {
            return Ok(array
                .nulls()
                .map_or(0, |nulls| exposed_null_count(nulls, exposure)));
        }
        let mut null_count = 0usize;
        for index in 0..array.len() {
            if is_exposed(exposure, index) && logical_null_at(array, dtype, index)? {
                null_count += 1;
            }
        }
        Ok(null_count)
    }

    /// How many exposed rows a validity bitmap marks absent.
    ///
    /// The bitmap already carries its own count, so an unexposed read is a
    /// field read; an exposed one is two bitmaps intersected a machine word at
    /// a time. A mask of the wrong length is left to the row walk, which is
    /// what reports it.
    pub(crate) fn exposed_null_count(
        logical: &arrow_buffer::NullBuffer,
        exposure: Option<&BooleanBuffer>,
    ) -> usize {
        if logical.null_count() == 0 {
            return 0;
        }
        match exposure {
            None => logical.null_count(),
            Some(exposure) if exposure.len() == logical.len() => {
                exposure.count_set_bits() - both_set_bits(exposure, logical.inner())
            }
            Some(exposure) => (0..logical.len())
                .filter(|index| exposure.value(*index) && logical.is_null(*index))
                .count(),
        }
    }

    /// The number of positions set in both buffers.
    ///
    /// Both carry the same bit length, so `iter_padded` yields the same number
    /// of chunks for each and the padding is zero on both sides.
    fn both_set_bits(left: &BooleanBuffer, right: &BooleanBuffer) -> usize {
        left.bit_chunks()
            .iter_padded()
            .zip(right.bit_chunks().iter_padded())
            .map(|(left, right)| (left & right).count_ones() as usize)
            .sum()
    }

    pub(crate) fn logical_validity_buffer(
        array: &dyn Array,
        dtype: &DataType,
        budget: &mut MaterializationBudget,
    ) -> Result<arrow_buffer::NullBuffer> {
        budget.add_bitmap(array.len())?;
        // The same rule as the null count above: unless absence is derived
        // from a child, the array's own validity *is* the logical validity, so
        // it is shared rather than rebuilt one bit at a time.
        if !has_derived_logical_nulls(dtype) {
            return Ok(match array.nulls() {
                Some(nulls) => nulls.clone(),
                None => arrow_buffer::NullBuffer::new_valid(array.len()),
            });
        }
        let mut builder = BooleanBufferBuilder::new(array.len());
        for index in 0..array.len() {
            builder.append(!logical_null_at(array, dtype, index)?);
        }
        Ok(arrow_buffer::NullBuffer::new(builder.build()))
    }

    pub(crate) fn visible_array_exposure(
        array: &dyn Array,
        parent: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<Option<BooleanBuffer>> {
        if array.null_count() == 0
            && parent.is_none_or(|parent| parent.count_set_bits() == parent.len())
        {
            return Ok(None);
        }
        budget.add_bitmap(array.len())?;
        let exposure = BooleanBuffer::collect_bool(array.len(), |index| {
            is_exposed(parent, index) && array.is_valid(index)
        });
        Ok((exposure.count_set_bits() != exposure.len()).then_some(exposure))
    }

    pub(crate) fn offset_pair(start: i64, end: i64) -> Result<(usize, usize)> {
        let start = usize::try_from(start).map_err(|_| {
            Error::IncompatibleSchema("nested Arrow offset is negative or exceeds usize".to_owned())
        })?;
        let end = usize::try_from(end).map_err(|_| {
            Error::IncompatibleSchema("nested Arrow offset is negative or exceeds usize".to_owned())
        })?;
        Ok((start, end))
    }

    pub(crate) fn offset_size(start: i64, size: i64) -> Result<(usize, usize)> {
        let (start, size) = offset_pair(start, size)?;
        let end = start.checked_add(size).ok_or_else(|| {
            Error::IncompatibleSchema("nested Arrow offset plus size exceeds usize".to_owned())
        })?;
        Ok((start, end))
    }

    pub(crate) fn range_exposure<Valid, Range>(
        child_len: usize,
        parent_len: usize,
        parent: Option<&BooleanBuffer>,
        mut is_valid: Valid,
        mut range: Range,
        budget: &mut MaterializationBudget,
    ) -> Result<Option<BooleanBuffer>>
    where
        Valid: FnMut(usize) -> bool,
        Range: FnMut(usize) -> Result<(usize, usize)>,
    {
        if parent.is_some_and(|parent| parent.len() != parent_len) {
            return Err(Error::IncompatibleSchema(
                "nested Arrow range exposure has the wrong parent length".to_owned(),
            ));
        }
        let mut next = 0usize;
        let mut full_coverage = true;
        for row in 0..parent_len {
            if !is_exposed(parent, row) || !is_valid(row) {
                full_coverage = false;
                break;
            }
            let (start, end) = range(row)?;
            if start > end || end > child_len {
                return Err(Error::IncompatibleSchema(
                    "nested Arrow offsets select values outside their child array".to_owned(),
                ));
            }
            if start != next {
                full_coverage = false;
                break;
            }
            next = end;
        }
        if full_coverage && next == child_len {
            return Ok(None);
        }
        budget.add_bitmap(child_len)?;
        let mut builder = BooleanBufferBuilder::new(child_len);
        builder.append_n(child_len, false);
        let mut selected = 0usize;
        for row in 0..parent_len {
            if !is_exposed(parent, row) || !is_valid(row) {
                continue;
            }
            let (start, end) = range(row)?;
            if start > end || end > child_len {
                return Err(Error::IncompatibleSchema(
                    "nested Arrow offsets select values outside their child array".to_owned(),
                ));
            }
            for index in start..end {
                if !builder.get_bit(index) {
                    builder.set_bit(index, true);
                    selected += 1;
                }
            }
        }
        if selected == child_len {
            Ok(None)
        } else {
            Ok(Some(builder.build()))
        }
    }

    pub(crate) fn selected_index_exposure<Index>(
        child_len: usize,
        parent_len: usize,
        parent: Option<&BooleanBuffer>,
        mut index: Index,
        budget: &mut MaterializationBudget,
    ) -> Result<Option<BooleanBuffer>>
    where
        Index: FnMut(usize) -> Option<usize>,
    {
        if parent.is_some_and(|parent| parent.len() != parent_len) {
            return Err(Error::IncompatibleSchema(
                "nested Arrow selection exposure has the wrong parent length".to_owned(),
            ));
        }
        if parent.is_none()
            && parent_len == child_len
            && (0..parent_len).all(|row| index(row) == Some(row))
        {
            return Ok(None);
        }
        budget.add_bitmap(child_len)?;
        let mut builder = BooleanBufferBuilder::new(child_len);
        builder.append_n(child_len, false);
        let mut selected = 0usize;
        for row in 0..parent_len {
            if !is_exposed(parent, row) {
                continue;
            }
            let Some(index) = index(row) else {
                continue;
            };
            if index >= child_len {
                return Err(Error::IncompatibleSchema(
                    "nested Arrow selection points outside its child array".to_owned(),
                ));
            }
            if !builder.get_bit(index) {
                builder.set_bit(index, true);
                selected += 1;
            }
        }
        if selected == child_len {
            Ok(None)
        } else {
            Ok(Some(builder.build()))
        }
    }

    pub(crate) fn union_mode_matches(core: UnionMode, arrow: arrow_schema::UnionMode) -> bool {
        matches!(
            (core, arrow),
            (UnionMode::Sparse, arrow_schema::UnionMode::Sparse)
                | (UnionMode::Dense, arrow_schema::UnionMode::Dense)
        )
    }
}

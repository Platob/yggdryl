//! Casting an Arrow array into the exact array a typed field describes.
//!
//! [`ArrowCast`] answers "make this array fit that field" for any
//! field, and returns an [`ArrayRef`] because any field could be any datatype.
//! A [`TypedField`](crate::TypedField) already knows its variant, so it can answer with the array
//! type itself: [`Int64Field`](crate::types::Int64Field) casts to an
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
//! use yggdryl::types::Int64Field;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Int64Field::new("id", false);
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

use arrow_array::{Array, ArrayRef, RecordBatch, Scalar as ArrowScalar, StructArray};
use arrow_buffer::BooleanBuffer;
use arrow_cast::can_cast_types;
use arrow_schema::{DataType as ArrowDataType, FieldRef as ArrowFieldRef};
pub use batch::{preflight_arrow_batch_cast, validate_arrow_batch};
pub(crate) use kernel::arrow_cast_exposed;
pub use options::{ArrowCastOptions, Nullability, Representation};
pub use plan::ArrowCastPlan;
use smol_str::SmolStr;
pub use typed::ArrowFieldType;

use crate::arrow::{Error, Result};
use crate::path::{Path, Segment};
use crate::types::budget::MaterializationBudget;
use crate::types::bytes::casts::{bridges_through_binary, ingest_bytes_array};
use crate::types::cast::text::{holds_text, ingest_text_values};
use crate::types::decimal::casts::holds_decimal;
use crate::types::geospatial::casts::{render_wkt_array, validate_wkb_ingest};
use crate::types::nested::casts::{cast_dictionary_planned, cast_run_planned, cast_union_planned, contains_struct, default_array, exposed_logical_null_count, fill_nulls, folded_field_mapping, is_logically_null, is_reconcilable_nested, list_child, union_mode_matches};
use crate::types::string::casts::{StringSource, ingest_code_array, ingest_string_array};
use crate::types::string::{is_text_storage, needs_extension};
use crate::types::temporal::casts::{holds_temporal, ingest_temporal_text, is_temporal_arrow, render_temporal_text};
use crate::types::uuid::casts::ingest_uuid_array;
use crate::types::version::casts::{ingest_version_array, is_text_layout};
use crate::types::{BLOOMBERG_WIDTH, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, CUSIP_WIDTH, ISIN_WIDTH, MIC_WIDTH, RecognizedExtension, SEDOL_WIDTH, SIDE_WIDTH, STATE_WIDTH, TIMEINFORCE_WIDTH, code_refusal, recognized_arrow_extension};
use crate::{DataType, Field, Scalar};

/// Exact and preflight record-batch boundaries.
mod batch {
    use arrow_array::RecordBatch;

    use super::{ArrowCast, ArrowCastOptions, ArrowCastPlan};
    use crate::Field;
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
    use crate::types::budget::{
        MaterializationBudget, SourceSelection, reserve_cast_output_payload,
        reserve_new_dictionary_vocabularies, reserve_selected_source_take, reserve_source_selection,
        reserve_vec_bytes,
    };
    use crate::types::nested::casts::{align_nested_dictionaries, contains_dictionary, default_array};
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
            let source_type = DataType::from_arrow(array.data_type())?;
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
        /// `int64`/`uint64`/`float64`/`fixed_size_binary(8)` alike, and every one
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
    use crate::types::budget::MaterializationBudget;

    use super::{ArrayCastPlan, ArrowCastOptions, Deferred, downcast};

    /// A compiled Arrow record-batch cast: immutable, shareable, and reusable.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int32Array, RecordBatch};
    /// use yggdryl::{ArrowCastOptions, ArrowCastPlan, DataType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = DataType::from_fields([DataType::Int64.required_field("id")])?
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
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BooleanArray, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
    use arrow_cast::can_cast_types;
    use arrow_schema::DataType as ArrowDataType;
    use arrow_select::zip::zip;

    use crate::arrow::{Error, Result};
    use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
    use crate::types::cast::{arrow_cast_exposed, downcast};
    use crate::types::nested::casts::is_exposed;
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
            DataType::Dictionary(dictionary) => encoded_value_of(dictionary.value()),
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

    use super::ArrowCast as _;
    use crate::arrow::{Error, Result};
    use crate::types::cast::ArrowCastOptions;
    use crate::types::typed::{FieldType, TypedField, TypedFieldRef};

    /// The Arrow array a field's values materialize into.
    ///
    /// Implemented for every [`FieldType`] marker. A variant whose physical array
    /// depends on a datatype parameter reports [`ArrayRef`], because there is no
    /// single concrete type to name.
    pub trait ArrowFieldType: FieldType {
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
    fn downcast_owned<A: Array + Clone + 'static>(array: ArrayRef, expected: &'static str) -> Result<A> {
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

    typed_array!(crate::types::boolean::NullType, arrow_array::NullArray);
    typed_array!(
        crate::types::boolean::BooleanType,
        arrow_array::BooleanArray
    );
    typed_array!(crate::types::integer::Int8Type, arrow_array::Int8Array);
    typed_array!(crate::types::integer::Int16Type, arrow_array::Int16Array);
    typed_array!(crate::types::integer::Int32Type, arrow_array::Int32Array);
    typed_array!(crate::types::integer::Int64Type, arrow_array::Int64Array);
    typed_array!(crate::types::integer::UInt8Type, arrow_array::UInt8Array);
    typed_array!(crate::types::integer::UInt16Type, arrow_array::UInt16Array);
    typed_array!(crate::types::integer::UInt32Type, arrow_array::UInt32Array);
    typed_array!(crate::types::integer::UInt64Type, arrow_array::UInt64Array);
    typed_array!(
        crate::types::floating::Float16Type,
        arrow_array::Float16Array
    );
    typed_array!(
        crate::types::floating::Float32Type,
        arrow_array::Float32Array
    );
    typed_array!(
        crate::types::floating::Float64Type,
        arrow_array::Float64Array
    );
    typed_array!(crate::types::temporal::Date32Type, arrow_array::Date32Array);
    typed_array!(crate::types::temporal::Date64Type, arrow_array::Date64Array);
    typed_array!(
        crate::types::decimal::Decimal32Type,
        arrow_array::Decimal32Array
    );
    typed_array!(
        crate::types::decimal::Decimal64Type,
        arrow_array::Decimal64Array
    );
    typed_array!(
        crate::types::decimal::Decimal128Type,
        arrow_array::Decimal128Array
    );
    typed_array!(
        crate::types::decimal::Decimal256Type,
        arrow_array::Decimal256Array
    );
    typed_array!(crate::types::version::VersionType, arrow_array::StringArray);
    // A registered code stores as the text it is, exactly as a version does.
    typed_array!(crate::types::CountryType, arrow_array::StringArray);
    typed_array!(crate::types::CurrencyType, arrow_array::StringArray);
    typed_array!(crate::types::MicType, arrow_array::StringArray);
    typed_array!(crate::types::CfiType, arrow_array::StringArray);
    typed_array!(crate::types::IsinType, arrow_array::StringArray);
    typed_array!(crate::types::CusipType, arrow_array::StringArray);
    typed_array!(crate::types::SedolType, arrow_array::StringArray);
    typed_array!(
        crate::types::BloombergType,
        arrow_array::StringArray
    );
    typed_array!(crate::types::SideType, arrow_array::StringArray);
    typed_array!(crate::types::StateType, arrow_array::StringArray);
    typed_array!(
        crate::types::TimeInForceType,
        arrow_array::StringArray
    );
    // A UUID stores as the fixed binary of its sixteen bytes.
    typed_array!(
        crate::types::uuid::UuidType,
        arrow_array::FixedSizeBinaryArray
    );
    typed_array!(crate::types::ListType, arrow_array::ListArray);
    typed_array!(
        crate::types::ListViewType,
        arrow_array::ListViewArray
    );
    typed_array!(
        crate::types::LargeListType,
        arrow_array::LargeListArray
    );
    typed_array!(
        crate::types::nested::LargeListViewType,
        arrow_array::LargeListViewArray
    );
    typed_array!(
        crate::types::nested::FixedSizeListType,
        arrow_array::FixedSizeListArray
    );
    typed_array!(crate::types::StructType, arrow_array::StructArray);
    typed_array!(crate::types::UnionType, arrow_array::UnionArray);
    typed_array!(crate::types::nested::MapTypeMarker, arrow_array::MapArray);
    // A variant's storage is the canonical struct of two required binaries, and a
    // geospatial value is its WKB payload, so their physical arrays are fixed.
    typed_array!(crate::types::nested::VariantType, arrow_array::StructArray);
    typed_array!(
        crate::types::geospatial::GeometryType,
        arrow_array::BinaryArray
    );
    typed_array!(
        crate::types::geospatial::GeographyType,
        arrow_array::BinaryArray
    );

    // A unit decides the physical width of a temporal value, a key type decides
    // the physical width of a dictionary index, a string's layout and charset
    // decide which text or byte array holds it and a byte layout which binary
    // array, so these have no single array type.
    opaque_array!(crate::types::string::StringType);
    opaque_array!(crate::types::bytes::BytesType);
    opaque_array!(crate::types::temporal::DateTime64Type);
    opaque_array!(crate::types::temporal::Time32Type);
    opaque_array!(crate::types::temporal::Time64Type);
    opaque_array!(crate::types::temporal::Duration32Type);
    opaque_array!(crate::types::temporal::Duration64Type);
    opaque_array!(crate::types::temporal::IntervalType);
    opaque_array!(crate::types::nested::DictionaryTypeMarker);
    opaque_array!(crate::types::nested::RunEndEncodedTypeMarker);

    impl<K: ArrowFieldType> TypedField<K> {
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
        pub fn cast_arrow_array(&self, array: ArrayRef, options: ArrowCastOptions) -> Result<K::Array> {
            K::downcast_array(self.as_field().cast_arrow_array(array, options)?)
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
        ) -> Result<Scalar<K::Array>> {
            if array.len() != 1 {
                return Err(Error::IncompatibleSchema(format!(
                    "expected exactly 1 value to cast as a scalar, got {}",
                    array.len()
                )));
            }
            Ok(Scalar::new(self.cast_arrow_array(array, options)?))
        }
    }

    impl<K: ArrowFieldType> TypedFieldRef<'_, K> {
        /// Cast an incoming Arrow array to the borrowed field's exact array.
        ///
        /// # Errors
        ///
        /// Returns any error [`TypedField::cast_arrow_array`] returns.
        pub fn cast_arrow_array(&self, array: ArrayRef, options: ArrowCastOptions) -> Result<K::Array> {
            K::downcast_array(self.as_field().cast_arrow_array(array, options)?)
        }
    }
}

/// Arrow array and record-batch casting owned by a canonical Yggdryl schema.
///
/// This extension trait lives in [`crate::arrow`], keeping recursive runtime
/// casts behind Yggdryl's `arrow` feature while retaining method syntax on
/// [`DataType`] and [`Field`].
pub trait ArrowCast {
    /// Casts an Arrow array to this exact physical datatype.
    ///
    /// [`ArrowCastOptions::is_safe`] is passed to Arrow's
    /// [`CastOptions`](arrow_cast::CastOptions): with Arrow 59 a supported
    /// conversion failure becomes null when it is true and an error when it is
    /// false. [`ArrowCastOptions::nullability`] then decides what happens to a
    /// null a non-nullable target cannot hold - the canonical default under
    /// [`Nullability::Default`], an error naming the path under
    /// [`Nullability::Strict`]. A nullable Field retains its nulls either way.
    ///
    /// Temporals cross a text boundary through this crate's own spellings, so
    /// a column reads and prints what a row reads and prints; Arrow's kernel
    /// keeps the spellings only it knows.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported cast, an ambiguous case-insensitive
    /// Struct match, or a result that cannot satisfy the target Field.
    fn cast_arrow_array(&self, array: ArrayRef, options: ArrowCastOptions) -> Result<ArrayRef>;

    /// Reconciles an Arrow record batch to this Struct schema.
    ///
    /// Struct children are selected in target order by ASCII-case-insensitive
    /// name. Extra source columns are dropped and missing nullable columns are
    /// null-filled. A missing required column uses the canonical default under
    /// [`Nullability::Default`] and is refused by path under
    /// [`Nullability::Strict`]. An already exact batch is returned unchanged -
    /// the same batch object, not a rebuilt one.
    ///
    /// One batch compiles one [`ArrowCastPlan`]; a caller with many batches of
    /// one schema compiles the plan itself and reuses it.
    ///
    /// # Errors
    ///
    /// Returns an error unless this value is a Struct schema, or when a child
    /// cast or missing-column default cannot be materialized.
    fn cast_arrow_batch(
        &self,
        batch: RecordBatch,
        options: ArrowCastOptions,
    ) -> Result<RecordBatch>;

    /// Casts a one-row Arrow array to this exact schema, as a scalar.
    ///
    /// A scalar is a one-row array with the row pinned, so the cast is
    /// [`cast_arrow_array`](Self::cast_arrow_array) plus the length check
    /// that makes the pinning honest.
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row, or any
    /// error the array cast returns.
    fn cast_arrow_scalar(
        &self,
        array: ArrayRef,
        options: ArrowCastOptions,
    ) -> Result<ArrowScalar<ArrayRef>> {
        if array.len() != 1 {
            return Err(Error::IncompatibleSchema(format!(
                "a scalar cast takes exactly one row, got {}",
                array.len()
            )));
        }
        Ok(ArrowScalar::new(self.cast_arrow_array(array, options)?))
    }
}

impl ArrowCast for DataType {
    fn cast_arrow_array(&self, array: ArrayRef, options: ArrowCastOptions) -> Result<ArrayRef> {
        let plan = ArrayCastPlan::new_dtype(self, array.data_type(), options)?;
        let mut budget = MaterializationBudget::default();
        plan.cast(array, &mut budget)
    }

    fn cast_arrow_batch(
        &self,
        batch: RecordBatch,
        options: ArrowCastOptions,
    ) -> Result<RecordBatch> {
        Field::new("record", self.clone(), false).cast_arrow_batch(batch, options)
    }
}

impl ArrowCast for Field {
    fn cast_arrow_array(&self, array: ArrayRef, options: ArrowCastOptions) -> Result<ArrayRef> {
        cast_field_array(self, None, array, options)
    }

    fn cast_arrow_batch(
        &self,
        batch: RecordBatch,
        options: ArrowCastOptions,
    ) -> Result<RecordBatch> {
        ArrowCastPlan::compile(batch.schema_ref(), self, options)?.apply(batch)
    }
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
    /// A `transform:` or partition declaration derives the column from others.
    pub(crate) transform: bool,
    /// `digest:role=holder` says the column holds the row's hash.
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
    Source { index: usize, cast: ArrayCastPlan },
    Missing(Field),
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
        let expected = field.clone().into_arrow_ref()?.data_type().clone();
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
                crate::types::bytes::needs_extension(*parameters)
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
            DataType::Url => !matches!(source_extension.as_ref(), Some(RecognizedExtension::Url)),
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
        Ok(Self {
            field: field.clone(),
            source_type: source_type.clone(),
            expected,
            options,
            path: SmolStr::from(path.render()),
            null_policy,
            kind,
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
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View => {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "casting text to {} would need the WKT parser this workspace \
                             deliberately does not have yet",
                            dtype.name()
                        ),
                    });
                }
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
                if crate::types::is_variant_storage(source) {
                    // The identity: physically the same two required binary
                    // children, reconciled to the canonical child spelling.
                    ArrayCastKind::Kernel
                } else {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "casting {source:?} to variant goes through the variant codec, \
                             which lands with the Iceberg v3 layer"
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
            (DataType::Url, source) if is_text_layout(source) => ArrayCastKind::UrlIngest,
            (DataType::Url, source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to url is not supported"),
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
                let mapping = folded_field_mapping(source_fields, fields)?;
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
                        None => StructColumnPlan::Missing(target.clone()),
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
                DataType::List(child)
                | DataType::LargeList(child)
                | DataType::ListView(child)
                | DataType::LargeListView(child)
                | DataType::FixedSizeList(child, _),
                ArrowDataType::List(source_child)
                | ArrowDataType::LargeList(source_child)
                | ArrowDataType::ListView(source_child)
                | ArrowDataType::LargeListView(source_child)
                | ArrowDataType::FixedSizeList(source_child, _),
            ) => {
                if let (DataType::FixedSizeList(_, size), ArrowDataType::FixedSizeList(_, source)) =
                    (dtype, source_type)
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
                let DataType::Mapping(source) = DataType::from_arrow(source_type)? else {
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
                DataType::Dictionary(dictionary),
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
                == encoded.run_ends().clone().into_arrow_ref()?.data_type() =>
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
            (DataType::Dictionary(dictionary), _) => ArrayCastKind::Encoded {
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
                crate::types::url::casts::ingest_url_array(&array, &self.field, exposure, budget)?
            }
            ArrayCastKind::TimezoneIngest => crate::types::timezone::casts::ingest_timezone_array(
                &array,
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::MimeTypeIngest => {
                crate::types::mime_type::casts::ingest_mime_type_array(
                    &array,
                    &self.field,
                    exposure,
                    budget,
                )?
            }
            ArrayCastKind::MediaTypeIngest => {
                crate::types::media_type::casts::ingest_media_type_array(
                    &array,
                    &self.field,
                    exposure,
                    budget,
                )?
            }
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
                DataType::Mic => ingest_code_array::<MIC_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Cfi => ingest_code_array::<CFI_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Isin => ingest_code_array::<ISIN_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Cusip => ingest_code_array::<CUSIP_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Sedol => ingest_code_array::<SEDOL_WIDTH>(
                    &array,
                    self.safe(),
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Bloomberg => ingest_code_array::<BLOOMBERG_WIDTH>(
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
                let source_type = DataType::from_arrow(array.data_type())?;
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
                default_array(&self.field, array.len(), exposure, budget)?
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
        let remaining_nulls =
            if matches!(self.null_policy, NullPolicy::Field | NullPolicy::DataType)
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
        (DataType::Url, RecognizedExtension::Url) => Ok(()),
        (DataType::String(parameters), RecognizedExtension::Url)
            if is_text_storage(*parameters) =>
        {
            Ok(())
        }
        (other, RecognizedExtension::Url) => Err(Error::Unsupported {
            kind: "url",
            reason: format!("casting url to {} is not supported", other.name()),
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
                "casting variant to {} goes through the variant codec, which lands \
                 with the Iceberg v3 layer",
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

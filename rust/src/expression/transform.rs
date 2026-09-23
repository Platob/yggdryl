//! The `TRANSFORM:` protocol: how a column is computed from the rows around
//! it.
//!
//! A struct [`Field`] says what columns exist. A child carrying
//! `TRANSFORM:expression` also says how it is *derived*: the property holds
//! the canonical text of one [`Term`] over the other columns of the same
//! struct, and applying the field computes it. That is the same declaration a
//! [`Selector`](super::Selector) projection makes - `year(event) as year` -
//! so [`Selector::into_field`](super::Selector::into_field) writes one and
//! [`Selector::from_field`](super::Selector::from_field) reads it back, which
//! is what lets a field carry a plan.
//!
//! # One derivation, two declarations
//!
//! A partition column has declared its derivation since before this protocol
//! existed, as the pair `PARTITION:sources` and `PARTITION:transform` - the
//! shape an Iceberg partition spec takes, one function over one source
//! column. That pair stays what an Iceberg spec is read from, and it is *read
//! here*: [`TransformField::term`] answers the explicit expression where one
//! is declared and the partition pair otherwise, so a column derives one way
//! whichever protocol declared it, and the derivation runs in exactly one
//! place - [`TransformField::apply_arrow_batch`].
//!
//! # Applying
//!
//! A column holding anything but its canonical
//! [default](crate::Field::default_value) is left alone: recomputing a written
//! value would hide a mismatch. A column that is absent, or present holding
//! nothing but that default, was never written and is filled. Every declared
//! struct is walked, descendants before the level that reads them, and each
//! term binds against the columns that exist at its own level.

use smol_str::{SmolStr, format_smolstr};

use super::Function;
use super::term::Term;
use crate::protocol::{TransformField, TransformFieldMut};
use crate::{Error, Field, Result};

/// The property naming the term a column is computed with.
const EXPRESSION: &str = "expression";

/// The property naming the function a column is computed with, qualified.
const FUNCTION: &str = "function";

/// The property listing the columns a function reads, in argument order.
const SOURCES: &str = "sources";

/// The full key of the term a column is computed with.
pub(crate) const TRANSFORM_EXPRESSION_KEY: &str = "TRANSFORM:expression";

/// The full key of the function a column is computed with.
pub(crate) const TRANSFORM_FUNCTION_KEY: &str = "TRANSFORM:function";

/// The full key of the columns a function reads.
pub(crate) const TRANSFORM_SOURCES_KEY: &str = "TRANSFORM:sources";

/// The three properties one derivation may be spelled with.
pub(crate) const TRANSFORM_KEYS: [&str; 3] = [
    TRANSFORM_EXPRESSION_KEY,
    TRANSFORM_FUNCTION_KEY,
    TRANSFORM_SOURCES_KEY,
];

impl<'field> TransformField<'field> {
    /// The term this column is computed with, if it declares one.
    ///
    /// An explicit `TRANSFORM:expression` answers first. Without one, a
    /// partition column's own declaration - its `PARTITION:transform` over its
    /// `PARTITION:sources` - is the term, so a column derived either way reads
    /// the same here. `None` is an ordinary column.
    ///
    /// # Errors
    ///
    /// Returns an error naming the property when a stored declaration does
    /// not parse, or a partition declaration is incomplete.
    pub fn term(&self) -> Result<Option<Term>> {
        if let Some(stored) = self.get(EXPRESSION) {
            return stored
                .parse::<Term>()
                .map(Some)
                .map_err(|error| Error::InvalidMetadataValue {
                    key: SmolStr::new_static(TRANSFORM_EXPRESSION_KEY),
                    reason: format_smolstr!("{error}"),
                });
        }
        if let Some(function) = self.function()? {
            let Some(sources) = self.sources()? else {
                return Err(Error::InvalidMetadataValue {
                    key: SmolStr::new_static(TRANSFORM_SOURCES_KEY),
                    reason: format_smolstr!(
                        "expected the columns {} reads beside {}, got none",
                        function.as_str(),
                        TRANSFORM_FUNCTION_KEY
                    ),
                });
            };
            let arguments = sources.iter().map(|source| {
                let mut segments = source.split('.');
                let root = segments.next().unwrap_or_default();
                segments.fold(Term::column(root), Term::child)
            });
            return Ok(Some(Term::call(function, arguments)));
        }
        self.as_field().as_partition().term()
    }

    /// The function this column is computed with, when it declares one by
    /// name: a grammar function, or a [user-defined](crate::expression::UserFunction) one spelled
    /// `namespace.name`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the property when the stored text names no
    /// function.
    pub fn function(&self) -> Result<Option<Function>> {
        self.get(FUNCTION)
            .map(|stored| parse_transform_function(TRANSFORM_FUNCTION_KEY, stored))
            .transpose()
    }

    /// The columns the declared function reads, in argument order: dotted
    /// paths, as [`Field::get_field_by_path`](crate::Field::get_field_by_path)
    /// spells them.
    ///
    /// # Errors
    ///
    /// Returns an error naming the property when the stored text is not a
    /// JSON array of paths.
    pub fn sources(&self) -> Result<Option<Vec<String>>> {
        self.get(SOURCES)
            .map(|stored| crate::metadata::parse_source_list(TRANSFORM_SOURCES_KEY, stored))
            .transpose()
    }

    /// Return whether this column declares a derivation, however it spells it.
    ///
    /// Answered on the stored properties rather than the parsed term, so a
    /// malformed declaration still reports as one and is refused where it is
    /// read.
    #[must_use]
    pub fn is_derived(&self) -> bool {
        self.contains_key(EXPRESSION)
            || self.contains_key(FUNCTION)
            || self.contains_key(SOURCES)
            || self.as_field().as_partition().is_derived()
    }

    /// Return whether this root declares a derived column anywhere.
    ///
    /// The answer walks the declared structs, which is exactly the reach
    /// [`Self::apply_arrow_batch`] has, and reads no rows.
    #[must_use]
    pub fn declares_derivation(&self) -> bool {
        fn any_derivation(fields: &[Field]) -> bool {
            fields.iter().any(|field| {
                field.as_transform().is_derived()
                    || (field.is_struct() && any_derivation(field.fields()))
            })
        }
        any_derivation(self.as_field().fields())
    }
}

impl TransformFieldMut<'_> {
    /// Record the term this column is computed with.
    ///
    /// A call over plain columns - `year(event)`, `py.double(size)` - is
    /// stored as the function and its sources, the shape a
    /// [signature](super::FunctionSignature) reads and a partition spec
    /// shares; any other term is stored as its canonical text. Either
    /// spelling reads back through [`TransformField::term`], and the one not
    /// written is removed, so a column declares its derivation once.
    ///
    /// # Errors
    ///
    /// Returns an error when the term is past the depth or node budget, or the
    /// property write fails the validation every metadata write goes through,
    /// leaving the field unchanged.
    pub fn set_term(&mut self, term: &Term) -> Result<()> {
        term.check_budget()?;
        if let Term::Function(function, arguments) = term {
            let columns: Option<Vec<String>> = arguments
                .iter()
                .map(|argument| argument.as_column().map(str::to_owned))
                .collect();
            if let Some(columns) = columns.filter(|columns| !columns.is_empty()) {
                return self.set_function(function, columns);
            }
        }
        self.insert(EXPRESSION, term.to_string())?;
        self.remove(FUNCTION);
        self.remove(SOURCES);
        Ok(())
    }

    /// Record the function this column is computed with and the columns it
    /// reads, in argument order.
    ///
    /// # Errors
    ///
    /// Returns an error when a source path is empty or repeated, or a property
    /// write is refused.
    pub fn set_function<I, P>(&mut self, function: &Function, sources: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        let rendered = crate::metadata::render_source_list(TRANSFORM_SOURCES_KEY, sources)?;
        self.insert(FUNCTION, function.as_str())?;
        self.insert(SOURCES, rendered)?;
        self.remove(EXPRESSION);
        Ok(())
    }

    /// Remove the declared derivation, answering the term it spelled.
    pub fn remove_term(&mut self) -> Option<String> {
        let term = self.as_field().as_transform().term().ok().flatten();
        self.remove(EXPRESSION);
        self.remove(FUNCTION);
        self.remove(SOURCES);
        term.map(|term| term.to_string())
    }
}

/// Read the function a `TRANSFORM:function` property names: a grammar
/// function by canonical name or alias, or a user function by qualified name.
///
/// # Errors
///
/// Returns an error naming the key when the text names neither.
pub(crate) fn parse_transform_function(key: &str, value: &str) -> Result<Function> {
    Function::resolve(value).map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: format_smolstr!("{error}"),
    })
}

/// Restate an externally supplied transform function in its one spelling.
///
/// # Errors
///
/// Returns an error naming the key when the text names no function.
pub(crate) fn canonicalize_transform_function(key: &str, value: &str) -> Result<String> {
    Ok(parse_transform_function(key, value)?.as_str().to_owned())
}

/// Restate an externally supplied transform expression in its one spelling.
///
/// # Errors
///
/// Returns an error naming the key when the text is not a term.
pub(crate) fn canonicalize_transform_expression(key: &str, value: &str) -> Result<String> {
    let term: Term = value.parse().map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: format_smolstr!("{error}"),
    })?;
    Ok(term.to_string())
}

pub(crate) use arrow::TransformPlan;

mod arrow {
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};

    use arrow_array::{Array, RecordBatch, StructArray};
    use arrow_schema::{Field as ArrowField, Schema};

    use super::Term;
    use crate::arrow::{field_from_arrow_schema, rebuilt_batch};
    use crate::cast::{ArrowCastOptions, PlanCache};
    use crate::expression::Bound;
    use crate::expression::arrow::ColumnCast;
    use crate::protocol::TransformField;
    use crate::{Error, Field, Result};

    impl TransformField<'_> {
        /// Add the derived columns this schema declares to one batch.
        ///
        /// The view is taken on the struct root: its children are the
        /// declarations, and the batch supplies the rows they are computed
        /// from. Every declared struct is walked, and a term is bound against
        /// the columns that exist at its own level, so a nested declaration
        /// is filled before the level above reads it.
        ///
        /// A column holding anything but its canonical
        /// [default](crate::Field::default_value) is left alone; a column that
        /// is absent, or present holding nothing but that default, is filled.
        /// The filled column keeps its declaration verbatim, extension
        /// identity and all.
        ///
        /// ```
        /// use std::sync::Arc;
        ///
        /// use arrow_array::{ArrayRef, Date32Array, Int32Array, RecordBatch};
        /// use yggdryl::DataType;
        /// use yggdryl::StructType;
        ///
        /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
        /// let mut year = DataType::Int32.nullable_field("year");
        /// year.as_transform_mut().set_term(&"year(event)".parse()?)?;
        /// let root = DataType::from(StructType::from_fields([DataType::date32().required_field("event"), year])?)
        ///     .required_field("row");
        ///
        /// let batch = RecordBatch::try_from_iter([(
        ///     "event",
        ///     Arc::new(Date32Array::from(vec![19_723])) as ArrayRef,
        /// )])?;
        ///
        /// let filled = root.as_transform().apply_arrow_batch(&batch)?;
        ///
        /// assert_eq!(filled.num_columns(), 2);
        /// assert_eq!(filled.schema().field(1).name(), "year");
        /// assert_eq!(
        ///     filled.column(1).as_ref(),
        ///     &Int32Array::from(vec![2024]) as &dyn arrow_array::Array,
        /// );
        /// # Ok(())
        /// # }
        /// ```
        ///
        /// # Errors
        ///
        /// Returns an error when this view is not on a struct root, a
        /// declaration does not parse or bind against the columns at its
        /// level, or a computed value does not fit the type its column
        /// declares.
        pub fn apply_arrow_batch(&self, batch: &RecordBatch) -> Result<RecordBatch> {
            let root = self.as_field();
            root.require_struct()?;
            TransformPlan::new(root).apply(batch)
        }
    }

    /// The derivations one struct root declares, held for every batch they
    /// fill.
    ///
    /// What a batch does not change is settled once and kept: each declared
    /// term is parsed the first time a batch asks for it, bound once per
    /// schema a level's batches carry - again only when that schema changes -
    /// and cast into its column through one held plan. A stream applying a
    /// root holds one; a single batch builds one and drops it.
    pub(crate) struct TransformPlan {
        root: Field,
        level: Level,
    }

    impl TransformPlan {
        /// The plan for one struct root; nothing is parsed or bound yet.
        pub(crate) fn new(root: &Field) -> Self {
            Self {
                root: root.clone(),
                level: Level::new(root),
            }
        }

        /// Add the derived columns the root declares to one batch.
        ///
        /// # Errors
        ///
        /// [`TransformField::apply_arrow_batch`] carries the rule.
        pub(crate) fn apply(&self, batch: &RecordBatch) -> Result<RecordBatch> {
            Ok(filled_struct(&self.root, &self.level, batch)?.unwrap_or_else(|| batch.clone()))
        }
    }

    /// One declared struct level, its children in declared order.
    struct Level {
        children: Vec<Child>,
        /// The columns this level's batches carry, as the root its terms bind
        /// against.
        stored: Mutex<PlanCache<Arc<Stored>>>,
    }

    /// One declared child: the level it declares when it is a struct, the
    /// term it derives with, and the cast into what it declares.
    struct Child {
        nested: Option<Level>,
        term: OnceLock<Option<Term>>,
        cast: ColumnCast,
    }

    /// One batch schema of a level, and each child's term bound against it.
    struct Stored {
        root: Field,
        bound: Vec<OnceLock<Bound>>,
    }

    impl Level {
        fn new(declared: &Field) -> Self {
            Self {
                children: declared
                    .fields()
                    .iter()
                    .map(|child| Child {
                        nested: child.is_struct().then(|| Self::new(child)),
                        term: OnceLock::new(),
                        cast: ColumnCast::default(),
                    })
                    .collect(),
                stored: Mutex::new(PlanCache::new()),
            }
        }

        /// The root this level's terms bind against for one batch schema:
        /// the columns that exist, at the positions they sit at, which is not
        /// the declared level when the declared level is what is missing.
        fn stored(&self, declared: &Field, schema: &Schema) -> Result<Arc<Stored>> {
            let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
            let held = stored.get_or_compile(schema.fields(), || {
                Ok(Arc::new(Stored {
                    root: field_from_arrow_schema(declared.name(), schema)?,
                    bound: self.children.iter().map(|_| OnceLock::new()).collect(),
                }))
            })?;
            Ok(Arc::clone(held))
        }
    }

    impl Child {
        /// The term this child derives with, kept from the first parse that
        /// succeeds.
        fn term(&self, declared: &Field) -> Result<Option<&Term>> {
            if let Some(term) = self.term.get() {
                return Ok(term.as_ref());
            }
            let term = declared.as_transform().term()?;
            Ok(self.term.get_or_init(|| term).as_ref())
        }
    }

    /// Fill one struct level, its own declared structs first.
    ///
    /// `None` is a level nothing was written to, which is what lets an
    /// unchanged batch keep the exact arrays it arrived with.
    fn filled_struct(
        declared: &Field,
        plan: &Level,
        batch: &RecordBatch,
    ) -> Result<Option<RecordBatch>> {
        let nested = filled_children(declared, plan, batch)?;
        let level = nested.as_ref().unwrap_or(batch);
        Ok(filled_columns(declared, plan, level)?.or(nested))
    }

    /// Fill every declared struct column of one level, bottom-up.
    fn filled_children(
        declared: &Field,
        plan: &Level,
        batch: &RecordBatch,
    ) -> Result<Option<RecordBatch>> {
        let rows = batch.num_rows();
        let mut fields: Vec<Arc<ArrowField>> =
            batch.schema().fields().iter().map(Arc::clone).collect();
        let mut columns = batch.columns().to_vec();
        let mut changed = false;

        for (child, planned) in declared.fields().iter().zip(&plan.children) {
            let Some(nested) = &planned.nested else {
                continue;
            };
            let Ok(index) = batch.schema().index_of(child.name()) else {
                continue;
            };
            let Some(held) = columns[index].as_any().downcast_ref::<StructArray>() else {
                continue;
            };
            // A struct's own null mask has no place in a batch, so it stays
            // here and goes back on the array this level rebuilds.
            let inner = rebuilt_batch(
                batch,
                held.fields().iter().map(Arc::clone).collect(),
                held.columns().to_vec(),
            )?;
            let Some(filled) = filled_struct(child, nested, &inner)? else {
                continue;
            };
            let widened = StructArray::try_new_with_length(
                filled.schema().fields().clone(),
                filled.columns().to_vec(),
                held.nulls().cloned(),
                rows,
            )
            .map_err(Error::Arrow)?;
            fields[index] = Arc::new(
                ArrowField::new(
                    fields[index].name(),
                    widened.data_type().clone(),
                    fields[index].is_nullable(),
                )
                .with_metadata(fields[index].metadata().clone()),
            );
            columns[index] = Arc::new(widened);
            changed = true;
        }

        if !changed {
            return Ok(None);
        }
        rebuilt_batch(batch, fields, columns).map(Some)
    }

    /// Fill the derived columns one level declares directly.
    fn filled_columns(
        declared: &Field,
        plan: &Level,
        batch: &RecordBatch,
    ) -> Result<Option<RecordBatch>> {
        let stored = plan.stored(declared, batch.schema_ref())?;
        let rows = batch.num_rows();
        let mut fields: Vec<Arc<ArrowField>> =
            batch.schema().fields().iter().map(Arc::clone).collect();
        let mut columns = batch.columns().to_vec();
        let mut changed = false;

        let children = declared.fields().iter().zip(&plan.children);
        for ((child, derived), bound) in children.zip(&stored.bound) {
            // The declaration is the cheap question and it is asked first: a
            // column that derives nothing is skipped without reading a row,
            // where `is_unwritten` decodes every cell of a column whose
            // default is not null - once per batch, for every ordinary column.
            let Some(term) = derived.term(child)? else {
                continue;
            };
            let held = batch.schema().index_of(child.name()).ok();
            if let Some(index) = held {
                if !is_unwritten(child, columns[index].as_ref(), rows)? {
                    continue;
                }
            }
            let bound = match bound.get() {
                Some(bound) => bound,
                None => {
                    let fresh = term.bind(&stored.root)?;
                    bound.get_or_init(|| fresh)
                }
            };
            // Strict: a declared type the computed value does not fit is an
            // error, not a column of silent nulls.
            let array = derived.cast.reconcile(
                child,
                None,
                bound.evaluate(batch)?,
                ArrowCastOptions::new().with_safe(false),
            )?;
            match held {
                Some(index) => columns[index] = array,
                None => {
                    fields.push(Arc::clone(child.as_arrow_field_ref()?));
                    columns.push(array);
                }
            }
            changed = true;
        }

        if !changed {
            return Ok(None);
        }
        rebuilt_batch(batch, fields, columns).map(Some)
    }

    /// Return whether every row of a column still holds its canonical default.
    ///
    /// This is the one rule a derived column and a
    /// [digest holder](crate::DigestField::apply_arrow_batch) both answer to:
    /// a cell equal to its field's own default was never written, and one
    /// holding anything else was.
    pub(crate) fn is_unwritten(
        field: &Field,
        array: &dyn arrow_array::Array,
        rows: usize,
    ) -> Result<bool> {
        let default = field.default_value()?;
        if default.is_null() {
            // For the ordinary nullable declaration the null mask is the answer.
            return Ok(array.null_count() == rows);
        }
        for row in 0..rows {
            if crate::arrow::value::value_from_array(field.dtype(), array, row)? != default {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

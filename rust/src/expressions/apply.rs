//! One verb over every carrier that holds values - or over a schema alone.
//!
//! An expression is only useful where the values are, and the values are in a
//! dozen shapes: a [`Value`] row, a [`TypedValue`], an Arrow array beside its
//! [`Field`], a `RecordBatch`, a streaming `BatchReader` - and sometimes there
//! are no values at all and the caller only wants the schema the result would
//! have. This module gives all of that one verb, modeled on the precedent this
//! crate already has for exactly this problem:
//! [`ArrowCast`](crate::ArrowCast), one trait with a method per carrier.
//!
//! Two halves, split on the feature boundary: [`Apply`] over the carriers the
//! unconditional core has, and [`ArrowApply`] over the Arrow ones. Both take
//! their *subject* through the same redirection, so a caller writes an
//! [`Expr`], a [`Bound`], a [`Selection`], or a `&str` and never converts by
//! hand - and a subject that is already bound skips binding, which is the whole
//! optimization claim: **binding happens once per apply, never per batch and
//! never per row**.

use smol_str::format_smolstr;

use super::Expr;
use super::bound::{Bound, BoundPredicate};
use super::select::{BoundSelection, Selection};
use crate::{DataType, Error, Field, Result, TypedValue, Value};

/// What a subject lowers to once a schema is known.
///
/// The three cases are what "apply" *means*, defined once and documented once:
/// a boolean expression filters a collection and evaluates a single value, a
/// non-boolean expression computes one column, and a selection projects.
#[derive(Clone, Debug)]
pub enum Program {
    /// A boolean expression: filter a collection, evaluate a single value.
    Predicate(BoundPredicate),
    /// A non-boolean expression: compute one column.
    Compute(Bound),
    /// A selection: project to its root.
    Project(BoundSelection),
}

impl Program {
    /// The struct root a collection carrier produces under this program.
    ///
    /// Filtering does not change a schema, so a predicate answers the schema
    /// it was bound to; a computation answers a one-column root; a projection
    /// answers its own root.
    #[must_use]
    pub fn result_root(&self) -> Field {
        match self {
            Self::Predicate(predicate) => predicate.bound().schema().clone(),
            Self::Compute(bound) => {
                let field = bound.field();
                let name = bound.schema().name().to_owned();
                DataType::from_fields([field])
                    .map(|data_type| data_type.required_field(name))
                    .unwrap_or_else(|_| bound.schema().clone())
            }
            Self::Project(selection) => selection.root().clone(),
        }
    }

    /// The field one *value* carrier produces under this program.
    #[must_use]
    pub fn result_field(&self) -> Field {
        match self {
            Self::Predicate(predicate) => {
                Field::new(predicate.bound().name(), DataType::Boolean, true)
            }
            Self::Compute(bound) => bound.field(),
            Self::Project(selection) => selection.root().clone(),
        }
    }

    /// Evaluate this program over one row.
    ///
    /// # Errors
    ///
    /// Returns whatever evaluating the underlying plan returns.
    pub fn evaluate(&self, row: &Value) -> Result<Value> {
        match self {
            Self::Predicate(predicate) => predicate.bound().evaluate(row),
            Self::Compute(bound) => bound.evaluate(row),
            Self::Project(selection) => selection.evaluate(row),
        }
    }

    /// The struct root this program was bound against.
    #[must_use]
    pub fn schema(&self) -> &Field {
        match self {
            Self::Predicate(predicate) => predicate.bound().schema(),
            Self::Compute(bound) => bound.schema(),
            Self::Project(selection) => selection.schema(),
        }
    }
}

/// Anything that names an expression, a plan, or a projection.
///
/// One required method - lower yourself against this schema - and everything
/// else is provided, so a new subject costs one impl and a new carrier costs
/// one impl, never a cross product.
pub trait Apply {
    /// Lower this subject against a struct root.
    ///
    /// # Errors
    ///
    /// Returns whatever parsing or binding returns.
    fn program(&self, schema: &Field) -> Result<Program>;

    /// The struct root a collection carrier would produce, with nothing read.
    ///
    /// This opens nothing and allocates no data: it is how a read answers
    /// `read_arrow_field` under a selection without touching a file, how a
    /// binding shows a user the output schema before a read, and how a write
    /// validates a projection up front.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::program`] returns.
    fn apply_field(&self, schema: &Field) -> Result<Field> {
        Ok(self.program(schema)?.result_root())
    }

    /// The datatype one value's result would have.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::program`] returns.
    fn apply_data_type(&self, schema: &Field) -> Result<DataType> {
        Ok(self.program(schema)?.result_field().data_type().clone())
    }

    /// Evaluate over one row, given the schema that describes it.
    ///
    /// # Errors
    ///
    /// Returns whatever binding or evaluating returns.
    fn apply_value(&self, schema: &Field, row: &Value) -> Result<Value> {
        self.program(schema)?.evaluate(row)
    }

    /// Evaluate over one typed value, whose datatype is its own schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the pairing is not a struct row.
    fn apply_typed_value(&self, row: &TypedValue) -> Result<TypedValue> {
        let schema = row_schema(row.data_type())?;
        let program = self.program(&schema)?;
        let value = program.evaluate(row.value())?;
        TypedValue::from_parts(program.result_field().data_type().clone(), value)
    }

    /// Keep the rows of an iterator that match, or compute over each of them.
    ///
    /// # Errors
    ///
    /// Returns whatever binding or evaluating returns.
    fn apply_rows(&self, schema: &Field, rows: &[Value]) -> Result<Vec<Value>> {
        let program = self.program(schema)?;
        let mut kept = Vec::new();
        for row in rows {
            match &program {
                Program::Predicate(predicate) => {
                    if predicate.matches(row)? {
                        kept.push(row.clone());
                    }
                }
                other => kept.push(other.evaluate(row)?),
            }
        }
        Ok(kept)
    }

    /// Apply to any carrier that holds values, or to a schema alone.
    ///
    /// This monomorphizes to the same call the explicit method makes: there is
    /// no `dyn`, no boxing on the value path, and the only trait object
    /// anywhere is the `BatchReader` this project already boxes.
    ///
    /// # Errors
    ///
    /// Returns whatever the carrier's own application returns.
    fn apply<C: Applicable>(&self, carrier: C) -> Result<C::Output>
    where
        Self: Sized,
    {
        let schema = carrier.carrier_schema()?;
        let program = self.program(&schema)?;
        carrier.run(&program)
    }
}

/// The struct root a row's datatype describes.
fn row_schema(data_type: &DataType) -> Result<Field> {
    let field = data_type.clone().required_field("row");
    field.require_struct()?;
    Ok(field)
}

impl Apply for Expr {
    fn program(&self, schema: &Field) -> Result<Program> {
        program_of(self.bind(schema)?)
    }
}

impl Apply for &Expr {
    fn program(&self, schema: &Field) -> Result<Program> {
        (*self).program(schema)
    }
}

impl Apply for Bound {
    /// A subject that is already bound skips binding entirely.
    ///
    /// A plan bound against a different schema is a typed error naming both,
    /// never a silent mismatch - which is what makes hoisting a `Bound` out of
    /// a loop safe as well as fast.
    fn program(&self, schema: &Field) -> Result<Program> {
        if !self.schema().equals(schema, false) {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected data matching the schema this plan was bound to ({}), got ({})",
                    crate::text::elide_display(&columns_of(self.schema())),
                    crate::text::elide_display(&columns_of(schema)),
                ),
            });
        }
        program_of(self.clone())
    }
}

impl Apply for BoundPredicate {
    fn program(&self, schema: &Field) -> Result<Program> {
        self.bound().program(schema)
    }
}

impl Apply for Selection {
    fn program(&self, schema: &Field) -> Result<Program> {
        Ok(Program::Project(self.bind(schema)?))
    }
}

impl Apply for BoundSelection {
    fn program(&self, _schema: &Field) -> Result<Program> {
        Ok(Program::Project(self.clone()))
    }
}

/// Text is parsed by the **core** parser, never by the caller.
///
/// A `&str` subject parses and binds per call, which is why the documentation
/// tells a reader to hoist a [`Bound`] out of a loop and why the benchmark
/// puts a number on the difference rather than leaving it as advice.
impl Apply for str {
    fn program(&self, schema: &Field) -> Result<Program> {
        self.parse::<Expr>()?.program(schema)
    }
}

impl Apply for &str {
    fn program(&self, schema: &Field) -> Result<Program> {
        <str as Apply>::program(self, schema)
    }
}

impl Apply for String {
    fn program(&self, schema: &Field) -> Result<Program> {
        <str as Apply>::program(self, schema)
    }
}

/// Read a bound plan as the program its result type names.
fn program_of(bound: Bound) -> Result<Program> {
    if matches!(bound.data_type(), DataType::Boolean) {
        return Ok(Program::Predicate(bound.into_predicate()?));
    }
    Ok(Program::Compute(bound))
}

/// The column names of a struct root, for an error message.
fn columns_of(schema: &Field) -> String {
    schema
        .fields()
        .iter()
        .map(Field::name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Anything an expression can be applied to in the unconditional core.
pub trait Applicable {
    /// What applying produces.
    type Output;

    /// The struct root that describes this carrier's rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the carrier does not describe a row at all.
    fn carrier_schema(&self) -> Result<Field>;

    /// Apply an already-lowered program.
    ///
    /// # Errors
    ///
    /// Returns whatever evaluating returns.
    fn run(self, program: &Program) -> Result<Self::Output>;
}

/// A schema alone: no data, nothing opened, just the shape of the answer.
impl Applicable for &Field {
    type Output = Field;

    fn carrier_schema(&self) -> Result<Field> {
        Ok((*self).clone())
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        Ok(program.result_root())
    }
}

/// One typed row: the value and the datatype that describes it, together.
impl Applicable for &TypedValue {
    type Output = TypedValue;

    fn carrier_schema(&self) -> Result<Field> {
        row_schema(self.data_type())
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        let value = program.evaluate(self.value())?;
        TypedValue::from_parts(program.result_field().data_type().clone(), value)
    }
}

/// One row that carries its own schema.
///
/// A [`Value::Record`] names its datatype, so it is self-describing; a bare
/// sequence or mapping is not, and says so rather than guessing a schema.
impl Applicable for &Value {
    type Output = Value;

    fn carrier_schema(&self) -> Result<Field> {
        match self {
            Value::Record(data_type, _) => row_schema(data_type),
            other => Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    "a record row, which names its own schema; pass a schema to apply_value for anything else",
                    other.kind(),
                ),
            }),
        }
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        program.evaluate(self)
    }
}

//! The statement vocabulary, and the three primitives every verb lowers to.
//!
//! A statement says what to *do* with the rows a handle holds, and every one of
//! them is a **selection, a filter, and a write mode** - all three of which the
//! record surface already has. That is what makes this a complete surface
//! rather than a second engine: nothing here decodes or encodes anything, and
//! every verb reaches the bytes through the same three record methods.
//!
//! The lowering is a real function rather than a shape hidden inside an
//! executor, so a wrong `UPDATE` is caught as a wrong `CASE` expression rather
//! than as wrong bytes.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use super::select::{Selection, SelectionItem};
use super::{Expr, write_identifier};
use crate::{DataType, Error, Field, Result, Value};

/// What a lowered statement does to the resource it names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    /// Nothing is written; the rows are handed back.
    Read,
    /// Every row is replaced by what the read produced.
    Overwrite,
    /// The rows are added after what is stored.
    Append,
}

/// One statement, or a chain of them.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Statement {
    /// Read the projection of the matching rows.
    Select {
        /// What to produce.
        selection: Selection,
        /// Which rows, or every row when absent.
        filter: Option<Expr>,
    },
    /// Append literal rows.
    Insert {
        /// The columns the values are for, or positional when empty.
        columns: Arc<[SmolStr]>,
        /// One list of values per row.
        rows: Arc<[Arc<[Expr]>]>,
    },
    /// Rewrite the named columns of the matching rows.
    Update {
        /// Column and the value it takes.
        assignments: Arc<[(SmolStr, Expr)]>,
        /// Which rows, or every row when absent.
        filter: Option<Expr>,
    },
    /// Remove the matching rows; without a filter, all of them.
    Delete {
        /// Which rows, or every row when absent.
        filter: Option<Expr>,
    },
    /// Add a column, valued by its default or computed from the others.
    AddColumn {
        /// The new column's name.
        name: SmolStr,
        /// Its type.
        data_type: DataType,
        /// The constant it reads as, when there is one.
        default: Option<Expr>,
        /// What computes it, when it is computed.
        computed: Option<Expr>,
    },
    /// Remove a column.
    DropColumn {
        /// The column to remove.
        name: SmolStr,
    },
    /// Rename a column, keeping its field id.
    RenameColumn {
        /// The current name.
        from: SmolStr,
        /// The new name.
        to: SmolStr,
    },
    /// Change a column's type.
    AlterColumnType {
        /// The column to convert.
        name: SmolStr,
        /// Its new type.
        data_type: DataType,
    },
    /// A chain: each step typed against the last, run as one pass.
    Chain(Arc<[Statement]>),
}

impl Statement {
    /// Read every row.
    #[must_use]
    pub fn select_all() -> Self {
        Self::Select {
            selection: Selection::everything(),
            filter: None,
        }
    }

    /// Read a projection of the rows a predicate selects.
    #[must_use]
    pub fn select(selection: Selection, filter: Option<Expr>) -> Self {
        Self::Select { selection, filter }
    }

    /// Remove the rows a predicate selects.
    #[must_use]
    pub fn delete(filter: Option<Expr>) -> Self {
        Self::Delete { filter }
    }

    /// Rewrite columns of the rows a predicate selects.
    #[must_use]
    pub fn update(
        assignments: impl IntoIterator<Item = (SmolStr, Expr)>,
        filter: Option<Expr>,
    ) -> Self {
        Self::Update {
            assignments: assignments.into_iter().collect(),
            filter,
        }
    }

    /// Chain statements, flattening a chain of chains.
    #[must_use]
    pub fn chain(steps: impl IntoIterator<Item = Self>) -> Self {
        let mut flat = Vec::new();
        for step in steps {
            match step {
                Self::Chain(inner) => flat.extend(inner.iter().cloned()),
                other => flat.push(other),
            }
        }
        match flat.len() {
            1 => flat.swap_remove(0),
            _ => Self::Chain(Arc::from(flat)),
        }
    }

    /// Return this statement followed by another, as one chain.
    #[must_use]
    pub fn then(self, next: Self) -> Self {
        Self::chain([self, next])
    }

    /// The steps this statement runs, which is itself when it is not a chain.
    #[must_use]
    pub fn steps(&self) -> Vec<Self> {
        match self {
            Self::Chain(steps) => steps.iter().flat_map(Self::steps).collect(),
            other => vec![other.clone()],
        }
    }

    /// Return whether this statement only reads.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        match self {
            Self::Select { .. } => true,
            Self::Chain(steps) => steps.iter().all(Self::is_read_only),
            _ => false,
        }
    }

    /// Return whether this statement changes the schema rather than the rows.
    ///
    /// A table format can commit one of these as a metadata document and never
    /// rewrite a byte; a leaf and a folder have to rewrite, because a Parquet
    /// footer carries its own schema.
    #[must_use]
    pub fn is_schema_only(&self) -> bool {
        match self {
            Self::DropColumn { .. } | Self::RenameColumn { .. } | Self::AlterColumnType { .. } => {
                true
            }
            Self::AddColumn { computed, .. } => computed.is_none(),
            Self::Chain(steps) => steps.iter().all(Self::is_schema_only),
            _ => false,
        }
    }

    /// Lower this statement against the schema it runs on.
    ///
    /// # Errors
    ///
    /// Returns an error naming the columns the schema has when this statement
    /// names one it does not, or naming the guard a statement tripped.
    pub fn lower(&self, schema: &Field) -> Result<Lowered> {
        schema.require_struct()?;
        match self {
            Self::Chain(steps) => lower_chain(steps, schema),
            other => lower_one(other, schema),
        }
    }
}

/// A statement reduced to a selection, a filter, and a write mode.
#[derive(Clone, Debug)]
pub struct Lowered {
    /// What each surviving row becomes.
    pub selection: Selection,
    /// Which rows survive, or every row when absent.
    pub filter: Option<Expr>,
    /// What is done with the result.
    pub mode: WriteMode,
    /// Literal rows to append, for the one verb that has them.
    pub rows: Vec<Value>,
    /// The struct root the result has.
    pub root: Field,
}

impl Lowered {
    /// Return whether this lowering neither filters nor projects.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.selection.is_empty() && self.filter.is_none() && self.rows.is_empty()
    }
}

/// Lower one non-chain statement.
#[allow(clippy::too_many_lines)]
fn lower_one(statement: &Statement, schema: &Field) -> Result<Lowered> {
    let plain = |selection: Selection, filter: Option<Expr>, mode: WriteMode| -> Result<Lowered> {
        let root = selection.bind(schema)?.root().clone();
        Ok(Lowered {
            selection,
            filter,
            mode,
            rows: Vec::new(),
            root,
        })
    };
    match statement {
        Statement::Select { selection, filter } => {
            guard_filter(filter.as_ref(), schema, "SELECT")?;
            plain(selection.clone(), filter.clone(), WriteMode::Read)
        }
        Statement::Delete { filter } => {
            let Some(filter) = filter else {
                // A `DELETE` with no `WHERE` at all is a truncate, and it is
                // allowed: omitting the clause is a deliberate act rather than
                // a slip, which is exactly why a `WHERE` that binds to TRUE is
                // not.
                return plain(Selection::everything(), None, WriteMode::Overwrite).map(
                    |mut lowered| {
                        lowered.filter = Some(Expr::always_false());
                        lowered
                    },
                );
            };
            guard_filter(Some(filter), schema, "DELETE")?;
            // The complement of a three-valued predicate keeps the rows it did
            // *not* match - including the ones it answered unknown for. So
            // `DELETE WHERE price > 10` must not remove a row whose price is
            // null, and `NOT p` alone would, because `NOT unknown` is unknown.
            let kept = Expr::any([filter.clone().not(), Expr::IsNull(Arc::new(filter.clone()))]);
            plain(Selection::everything(), Some(kept), WriteMode::Overwrite)
        }
        Statement::Update {
            assignments,
            filter,
        } => {
            guard_filter(filter.as_ref(), schema, "UPDATE")?;
            let mut items = Vec::with_capacity(schema.field_len());
            for field in schema.fields() {
                let assigned = assignments
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(field.name()));
                let expr = match (assigned, filter) {
                    // Only the matching rows take the new value; every other
                    // row keeps what it had, which is what a conditional
                    // rewrite *is*.
                    (Some((_, value)), Some(filter)) => Expr::case(
                        [(filter.clone(), value.clone())],
                        Some(Expr::column(field.name())),
                    ),
                    (Some((_, value)), None) => value.clone(),
                    (None, _) => Expr::column(field.name()),
                };
                items.push(SelectionItem::aliased(expr, field.name()));
            }
            for (name, _) in assignments.iter() {
                if resolve(schema, name).is_none() {
                    return Err(unknown(schema, name, "UPDATE cannot assign to"));
                }
            }
            plain(Selection::new(items), None, WriteMode::Overwrite)
        }
        Statement::Insert { columns, rows } => {
            let root = if columns.is_empty() {
                schema.clone()
            } else {
                let mut fields = Vec::with_capacity(columns.len());
                for name in columns.iter() {
                    let field =
                        resolve(schema, name).ok_or_else(|| unknown(schema, name, "INSERT"))?;
                    fields.push(field);
                }
                DataType::from_fields(fields)?.required_field(schema.name())
            };
            let mut literal = Vec::with_capacity(rows.len());
            for values in rows.iter() {
                literal.push(literal_row(&root, values)?);
            }
            Ok(Lowered {
                selection: Selection::everything(),
                filter: None,
                mode: WriteMode::Append,
                rows: literal,
                root,
            })
        }
        Statement::AddColumn {
            name,
            data_type,
            default,
            computed,
        } => {
            if resolve(schema, name).is_some() {
                return Err(guard(format_smolstr!(
                    "expected a column the schema does not have, got {name:?}; \
                     ALTER COLUMN changes one that exists"
                )));
            }
            let value = match (computed, default) {
                (Some(computed), _) => computed.clone(),
                (None, Some(default)) => default.clone(),
                (None, None) => Expr::literal(Value::Null),
            };
            let mut items = kept_columns(schema, |_| true);
            items.push(SelectionItem::aliased(
                value.try_cast_to(data_type.clone()),
                name.clone(),
            ));
            plain(Selection::new(items), None, WriteMode::Overwrite)
        }
        Statement::DropColumn { name } => {
            if resolve(schema, name).is_none() {
                return Err(unknown(schema, name, "DROP COLUMN"));
            }
            let items = kept_columns(schema, |field| !field.name().eq_ignore_ascii_case(name));
            plain(Selection::new(items), None, WriteMode::Overwrite)
        }
        Statement::RenameColumn { from, to } => {
            if resolve(schema, from).is_none() {
                return Err(unknown(schema, from, "RENAME COLUMN"));
            }
            let items = schema
                .fields()
                .iter()
                .map(|field| {
                    let name = if field.name().eq_ignore_ascii_case(from) {
                        to.clone()
                    } else {
                        SmolStr::new(field.name())
                    };
                    SelectionItem::aliased(Expr::column(field.name()), name)
                })
                .collect::<Vec<_>>();
            plain(Selection::new(items), None, WriteMode::Overwrite)
        }
        Statement::AlterColumnType { name, data_type } => {
            if resolve(schema, name).is_none() {
                return Err(unknown(schema, name, "ALTER COLUMN"));
            }
            let items = schema
                .fields()
                .iter()
                .map(|field| {
                    let expr = if field.name().eq_ignore_ascii_case(name) {
                        Expr::column(field.name()).try_cast_to(data_type.clone())
                    } else {
                        Expr::column(field.name())
                    };
                    SelectionItem::aliased(expr, field.name())
                })
                .collect::<Vec<_>>();
            plain(Selection::new(items), None, WriteMode::Overwrite)
        }
        Statement::Chain(steps) => lower_chain(steps, schema),
    }
}

/// Lower a chain, typing each step against the one before it.
///
/// The steps are *fused* rather than run one after another: adjacent
/// projections compose into one, adjacent filters conjoin, and the whole chain
/// becomes one selection, one filter, and one write mode - which is what lets
/// four statements cost one read and at most one write, with nothing
/// materialized between them.
fn lower_chain(steps: &[Statement], schema: &Field) -> Result<Lowered> {
    let mut root = schema.clone();
    let mut selection = Selection::everything();
    let mut filter: Option<Expr> = None;
    let mut mode = WriteMode::Read;
    let mut rows = Vec::new();

    for (position, step) in steps.iter().enumerate() {
        let step_number = position + 1;
        let lowered = lower_one(step, &root)
            .map_err(|error| step_error(step_number, steps.len(), &root, error))?;
        // A filter from a later step is pushed onto the accumulated one only
        // when every column it reads survives the projection so far unchanged;
        // otherwise it would be asking about a column that no longer means what
        // it did, so it is composed *through* the projection instead.
        filter = match (filter.take(), lowered.filter) {
            (Some(held), Some(next)) => Some(compose_filter(held, next, &selection)?),
            (Some(held), None) => Some(held),
            (None, Some(next)) => Some(rewrite_through(&next, &selection)?),
            (None, None) => None,
        };
        selection = compose_selection(&selection, &lowered.selection)?;
        rows.extend(lowered.rows);
        mode = match (mode, lowered.mode) {
            (WriteMode::Read, other) => other,
            // A chain that both rewrites and appends is two writes, and the
            // contract is at most one - so the later mode wins and the earlier
            // rewrite is folded into it.
            (held, WriteMode::Read) => held,
            (_, later) => later,
        };
        root = lowered.root;
    }
    Ok(Lowered {
        selection,
        filter,
        mode,
        rows,
        root,
    })
}

/// Name the step a chain failed at, by position and by what it could read.
fn step_error(step: usize, total: usize, root: &Field, error: Error) -> Error {
    let columns = root
        .fields()
        .iter()
        .map(Field::name)
        .collect::<Vec<_>>()
        .join(", ");
    Error::InvalidRecord {
        path: SmolStr::new(format_smolstr!("step {step}")),
        reason: format_smolstr!(
            "step {step} of {total}: {error}; the previous step produces {}",
            crate::text::elide_display(&columns)
        ),
    }
}

/// Compose two projections into the one a single pass applies.
fn compose_selection(held: &Selection, next: &Selection) -> Result<Selection> {
    if next.is_empty() {
        return Ok(held.clone());
    }
    if held.is_empty() {
        return Ok(next.clone());
    }
    // Every item of the later projection is rewritten so its column references
    // name what the earlier one produced, which is what makes the two one.
    let items = next
        .items()
        .iter()
        .map(|item| {
            let expr = substitute(item.expr(), held);
            SelectionItem::aliased(expr, item.name())
        })
        .collect::<Vec<_>>();
    Ok(Selection::new(items))
}

/// Conjoin a later filter onto an earlier one, through the projection between.
fn compose_filter(held: Expr, next: Expr, through: &Selection) -> Result<Expr> {
    Ok(Expr::all([held, rewrite_through(&next, through)?]))
}

/// Rewrite one expression so its columns name what a projection produced.
fn rewrite_through(expr: &Expr, through: &Selection) -> Result<Expr> {
    if through.is_empty() {
        return Ok(expr.clone());
    }
    Ok(substitute(expr, through))
}

/// Replace every column reference with what a projection computes for it.
///
/// A name the projection does not produce is left alone: the fusion is a
/// rewrite of what a later step *reads*, and binding is what refuses a name
/// that exists nowhere - which is where that error belongs.
fn substitute(expr: &Expr, through: &Selection) -> Expr {
    match expr {
        Expr::Column(column) if column.path().is_empty() => through
            .items()
            .iter()
            .find(|item| item.name().eq_ignore_ascii_case(column.name()))
            .map_or_else(|| expr.clone(), |item| item.expr().clone()),
        Expr::Column(_) | Expr::Literal(_) => expr.clone(),
        Expr::Cast {
            expr: inner,
            data_type,
            safe,
        } => Expr::Cast {
            expr: Arc::new(substitute(inner, through)),
            data_type: data_type.clone(),
            safe: *safe,
        },
        Expr::Compare { op, left, right } => Expr::Compare {
            op: *op,
            left: Arc::new(substitute(left, through)),
            right: Arc::new(substitute(right, through)),
        },
        Expr::And(operands) => {
            Expr::all(operands.iter().map(|operand| substitute(operand, through)))
        }
        Expr::Or(operands) => {
            Expr::any(operands.iter().map(|operand| substitute(operand, through)))
        }
        Expr::Not(inner) => Expr::Not(Arc::new(substitute(inner, through))),
        Expr::IsNull(inner) => Expr::IsNull(Arc::new(substitute(inner, through))),
        Expr::IsNotNull(inner) => Expr::IsNotNull(Arc::new(substitute(inner, through))),
        Expr::In {
            expr: inner,
            list,
            negated,
        } => Expr::In {
            expr: Arc::new(substitute(inner, through)),
            list: list.iter().map(|item| substitute(item, through)).collect(),
            negated: *negated,
        },
        Expr::Between {
            expr: inner,
            low,
            high,
            negated,
        } => Expr::Between {
            expr: Arc::new(substitute(inner, through)),
            low: Arc::new(substitute(low, through)),
            high: Arc::new(substitute(high, through)),
            negated: *negated,
        },
        Expr::Like {
            expr: inner,
            pattern,
            escape,
            negated,
            case_insensitive,
        } => Expr::Like {
            expr: Arc::new(substitute(inner, through)),
            pattern: Arc::new(substitute(pattern, through)),
            escape: *escape,
            negated: *negated,
            case_insensitive: *case_insensitive,
        },
        Expr::StartsWith {
            expr: inner,
            prefix,
        } => Expr::StartsWith {
            expr: Arc::new(substitute(inner, through)),
            prefix: prefix.clone(),
        },
        Expr::Arithmetic { op, left, right } => Expr::Arithmetic {
            op: *op,
            left: Arc::new(substitute(left, through)),
            right: Arc::new(substitute(right, through)),
        },
        Expr::Neg(inner) => Expr::Neg(Arc::new(substitute(inner, through))),
        Expr::Function { name, args } => Expr::Function {
            name: *name,
            args: args.iter().map(|arg| substitute(arg, through)).collect(),
        },
        Expr::Case {
            branches,
            otherwise,
        } => Expr::Case {
            branches: branches
                .iter()
                .map(|(when, then)| (substitute(when, through), substitute(then, through)))
                .collect(),
            otherwise: otherwise
                .as_ref()
                .map(|otherwise| Arc::new(substitute(otherwise, through))),
        },
        Expr::Alias { expr: inner, name } => Expr::Alias {
            expr: Arc::new(substitute(inner, through)),
            name: name.clone(),
        },
    }
}

/// Every column of a schema, kept under its own name, filtered by `keep`.
fn kept_columns(schema: &Field, keep: impl Fn(&Field) -> bool) -> Vec<SelectionItem> {
    schema
        .fields()
        .iter()
        .filter(|field| keep(field))
        .map(|field| SelectionItem::aliased(Expr::column(field.name()), field.name()))
        .collect()
}

/// Resolve one column, ASCII case-insensitively.
fn resolve(schema: &Field, name: &str) -> Option<Field> {
    schema
        .fields()
        .iter()
        .find(|field| field.name().eq_ignore_ascii_case(name))
        .cloned()
}

/// The failure that names the columns a schema does have.
fn unknown(schema: &Field, name: &str, verb: &str) -> Error {
    let columns = schema
        .fields()
        .iter()
        .map(Field::name)
        .collect::<Vec<_>>()
        .join(", ");
    Error::InvalidRecord {
        path: SmolStr::new(name),
        reason: format_smolstr!(
            "expected a column the schema declares, got {verb} {name:?}; it has {}",
            crate::text::elide_display(&columns)
        ),
    }
}

/// A statement refused by a guard rather than by a missing name.
fn guard(reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason,
    }
}

/// Refuse a `WHERE` that binds to `TRUE` unless it was spelled `WHERE TRUE`.
///
/// This is the typo guard, and it earns its place on `DELETE` above all: the
/// one thing worse than a mistyped filter is a mistyped filter that deletes
/// everything. Spelling `WHERE TRUE` is a deliberate act and passes.
fn guard_filter(filter: Option<&Expr>, schema: &Field, verb: &str) -> Result<()> {
    let Some(filter) = filter else {
        return Ok(());
    };
    if filter.is_always_true() {
        // Written as the constant, which is the deliberate spelling.
        return Ok(());
    }
    let bound = filter.bind(schema)?;
    // A predicate that reads no column at all has one answer for every row, and
    // the row it is asked over is irrelevant - which is what makes `1 = 1`
    // catchable without depending on whether the optimizer folded it.
    let constant =
        bound.columns().is_empty() && matches!(bound.evaluate(&Value::Null), Ok(Value::Bool(true)));
    if bound.is_always_true() || constant {
        return Err(guard(format_smolstr!(
            "expected a WHERE that selects some rows, got {} which is true for every row; \
             write WHERE TRUE to mean every row of a {verb}",
            crate::text::elide_display(filter)
        )));
    }
    Ok(())
}

/// Build one literal row from a `VALUES` list.
fn literal_row(root: &Field, values: &[Expr]) -> Result<Value> {
    if root.field_len() == 0 {
        // A resource that holds nothing and declares nothing has no columns to
        // put a row into, and inventing them from the values would be guessing
        // a schema - which this project does not do anywhere.
        return Err(guard(SmolStr::new_static(
            "expected a schema to INSERT into, got a resource that holds and declares no columns;              declare one with with_schema, or write the first rows through the record surface",
        )));
    }
    if values.len() != root.field_len() {
        return Err(guard(format_smolstr!(
            "expected {} value(s) per row, got {}",
            root.field_len(),
            values.len()
        )));
    }
    let mut row = Vec::with_capacity(values.len());
    for (field, value) in root.fields().iter().zip(values) {
        let Expr::Literal(literal) = value else {
            return Err(guard(format_smolstr!(
                "expected a literal in VALUES, got {}",
                crate::text::elide_display(value)
            )));
        };
        let converted = super::coerce_value(literal, field.data_type()).ok_or_else(|| {
            Error::InvalidRecord {
                path: SmolStr::new(field.name()),
                reason: crate::text::expected_got(
                    format_smolstr!("a value a {} column can hold", field.data_type()),
                    crate::text::elide_display(&super::Literal(literal)),
                ),
            }
        })?;
        row.push(converted);
    }
    Value::record(root.data_type().clone(), row)
}

impl fmt::Display for Statement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Select { selection, filter } => {
                write!(formatter, "SELECT {selection}")?;
                write_where(formatter, filter.as_ref())
            }
            Self::Insert { columns, rows } => {
                formatter.write_str("INSERT INTO .")?;
                if !columns.is_empty() {
                    formatter.write_str(" (")?;
                    for (position, name) in columns.iter().enumerate() {
                        if position > 0 {
                            formatter.write_str(", ")?;
                        }
                        write_identifier(formatter, name)?;
                    }
                    formatter.write_str(")")?;
                }
                formatter.write_str(" VALUES ")?;
                for (position, row) in rows.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    formatter.write_str("(")?;
                    for (index, value) in row.iter().enumerate() {
                        if index > 0 {
                            formatter.write_str(", ")?;
                        }
                        write!(formatter, "{value}")?;
                    }
                    formatter.write_str(")")?;
                }
                Ok(())
            }
            Self::Update {
                assignments,
                filter,
            } => {
                formatter.write_str("UPDATE . SET ")?;
                for (position, (name, value)) in assignments.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write_identifier(formatter, name)?;
                    write!(formatter, " = {value}")?;
                }
                write_where(formatter, filter.as_ref())
            }
            Self::Delete { filter } => {
                formatter.write_str("DELETE FROM .")?;
                write_where(formatter, filter.as_ref())
            }
            Self::AddColumn {
                name,
                data_type,
                default,
                computed,
            } => {
                formatter.write_str("ALTER TABLE . ADD COLUMN ")?;
                write_identifier(formatter, name)?;
                write!(formatter, " {data_type}")?;
                if let Some(default) = default {
                    write!(formatter, " DEFAULT {default}")?;
                }
                if let Some(computed) = computed {
                    write!(formatter, " AS {computed}")?;
                }
                Ok(())
            }
            Self::DropColumn { name } => {
                formatter.write_str("ALTER TABLE . DROP COLUMN ")?;
                write_identifier(formatter, name)
            }
            Self::RenameColumn { from, to } => {
                formatter.write_str("ALTER TABLE . RENAME COLUMN ")?;
                write_identifier(formatter, from)?;
                formatter.write_str(" TO ")?;
                write_identifier(formatter, to)
            }
            Self::AlterColumnType { name, data_type } => {
                formatter.write_str("ALTER TABLE . ALTER COLUMN ")?;
                write_identifier(formatter, name)?;
                write!(formatter, " TYPE {data_type}")
            }
            Self::Chain(steps) => {
                for (position, step) in steps.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str("; ")?;
                    }
                    write!(formatter, "{step}")?;
                }
                Ok(())
            }
        }
    }
}

/// Write a `WHERE` clause, or nothing when there is none.
fn write_where(formatter: &mut fmt::Formatter<'_>, filter: Option<&Expr>) -> fmt::Result {
    match filter {
        Some(filter) => write!(formatter, " WHERE {filter}"),
        None => Ok(()),
    }
}

impl FromStr for Statement {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        super::parser::parse_statement(text)
    }
}

impl TryFrom<&str> for Statement {
    type Error = Error;

    fn try_from(text: &str) -> Result<Self> {
        text.parse()
    }
}

/// A total order over statements, consistent with structural equality.
impl Ord for Statement {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Statements are compared rarely and never in a hot path, so the
        // canonical text - which round-trips - is the whole ordering.
        self.to_string().cmp(&other.to_string())
    }
}

impl PartialOrd for Statement {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// What carrying out a statement produced.
///
/// [`Debug`] is deliberately absent: a reader is a stream and printing one
/// would mean draining it, which is the one thing a caller holding it must be
/// able to rely on not happening.
#[cfg(feature = "arrow")]
pub enum Applied {
    /// A statement that reads hands back the rows it selected.
    Rows(crate::arrow::BatchReader),
    /// A statement that changes something reports what it changed.
    Changed(StatementReport),
}

#[cfg(feature = "arrow")]
impl Applied {
    /// The rows, when this statement read them.
    #[must_use]
    pub fn into_rows(self) -> Option<crate::arrow::BatchReader> {
        match self {
            Self::Rows(reader) => Some(reader),
            Self::Changed(_) => None,
        }
    }

    /// The report, when this statement changed something.
    #[must_use]
    pub const fn report(&self) -> Option<&StatementReport> {
        match self {
            Self::Changed(report) => Some(report),
            Self::Rows(_) => None,
        }
    }
}

/// What one statement did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct StatementReport {
    /// Rows the statement read from the resource.
    pub rows_read: u64,
    /// Rows the statement wrote back.
    pub rows_written: u64,
    /// Rows the statement removed.
    pub rows_deleted: u64,
    /// Columns the statement added.
    pub columns_added: u64,
    /// Columns the statement removed.
    pub columns_dropped: u64,
    /// Whether the resource was rewritten at all.
    pub rewritten: bool,
}

impl fmt::Display for StatementReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "read {} row(s), wrote {}, deleted {}; +{} column(s), -{}; {}",
            self.rows_read,
            self.rows_written,
            self.rows_deleted,
            self.columns_added,
            self.columns_dropped,
            if self.rewritten {
                "rewritten"
            } else {
                "untouched"
            }
        )
    }
}

/// Carry out one statement over one handle, through the three record methods.
///
/// Nothing here decodes or encodes anything: the read is
/// [`read_arrow_batch_reader`](crate::io::IOBase::read_arrow_batch_reader) under
/// options carrying the lowered selection and filter, and the write is
/// [`write_arrow_batch_reader`](crate::io::IOBase::write_arrow_batch_reader) or
/// [`append_arrow_batch_reader`](crate::io::IOBase::append_arrow_batch_reader).
///
/// # Errors
///
/// Returns whatever lowering, reading, or writing returns.
#[cfg(feature = "arrow")]
pub fn apply_statement(
    handle: &mut (impl crate::io::IOBase + ?Sized),
    statement: &Statement,
    options: &crate::generic::RecordOptions,
) -> Result<Applied> {
    use crate::generic::IORecordOptions;

    let base = options.clone();
    let schema = match base.schema() {
        Some(schema) => schema.clone(),
        None => handle.read_arrow_field(&base)?,
    };
    let lowered = statement.lower(&schema)?;

    let mut options = base.clone();
    if !lowered.selection.is_empty() {
        options.set_selection(lowered.selection.clone());
    }
    if let Some(filter) = &lowered.filter {
        options.set_filter(filter.clone());
    }

    match lowered.mode {
        WriteMode::Read => Ok(Applied::Rows(handle.read_arrow_batch_reader(&options)?)),
        WriteMode::Append => {
            let rows = literal_batch(&lowered.root, &lowered.rows)?;
            let written = u64::try_from(rows.num_rows()).unwrap_or(u64::MAX);
            let reader = crate::arrow::batch_reader(rows.schema(), [rows]);
            let mut append = base.clone();
            append.set_schema(lowered.root.clone());
            handle.append_arrow_batch_reader(reader, &append)?;
            Ok(Applied::Changed(StatementReport {
                rows_written: written,
                rewritten: true,
                ..StatementReport::default()
            }))
        }
        WriteMode::Overwrite => {
            let before = count_rows(handle, &base)?;
            // The surviving rows are held before the resource is rewritten,
            // and this is why: an in-place rewrite reads and writes the same
            // bytes, so streaming one into the other would be reading a file
            // while truncating it. This is the one collection this path makes,
            // and it is bounded by what the statement kept rather than by what
            // the resource holds.
            let mut kept = Vec::new();
            let mut rows_written = 0_u64;
            for batch in handle.read_arrow_batch_reader(&options)? {
                let batch = batch.map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("{error}"),
                })?;
                rows_written += u64::try_from(batch.num_rows()).unwrap_or(0);
                kept.push(batch);
            }
            let root = lowered.root.clone();
            let arrow = crate::arrow::schema_from_field(&root)?;
            let reader = crate::arrow::batch_reader(arrow, kept);
            let mut write = base.clone();
            write.set_schema(root.clone());
            write.set_merge_by_names(Vec::new());
            // An ordinary overwrite replaces *rows* and casts them onto the
            // shape the resource already stores, which is what stops a write
            // from silently redefining a column for every other reader. A
            // statement that changes the schema is the one caller that really
            // does mean to redefine it, so it clears the resource first -
            // exactly what the write contract says such a caller must do.
            if !same_columns(&schema, &root) {
                handle.clear()?;
            }
            handle.write_arrow_batch_reader(reader, &write)?;
            let dropped = schema
                .fields()
                .iter()
                .filter(|field| {
                    !lowered
                        .root
                        .fields()
                        .iter()
                        .any(|held| held.name().eq_ignore_ascii_case(field.name()))
                })
                .count();
            let added = lowered
                .root
                .fields()
                .iter()
                .filter(|field| {
                    !schema
                        .fields()
                        .iter()
                        .any(|held| held.name().eq_ignore_ascii_case(field.name()))
                })
                .count();
            Ok(Applied::Changed(StatementReport {
                rows_read: before,
                rows_written,
                rows_deleted: before.saturating_sub(rows_written),
                columns_added: u64::try_from(added).unwrap_or(0),
                columns_dropped: u64::try_from(dropped).unwrap_or(0),
                rewritten: true,
            }))
        }
    }
}

/// Count the rows a resource currently holds.
///
/// A resource that holds nothing is skipped rather than decoded, per the
/// laziness contract, so a statement against a missing one reports zeros.
#[cfg(feature = "arrow")]
fn count_rows(
    handle: &(impl crate::io::IOBase + ?Sized),
    options: &crate::generic::RecordOptions,
) -> Result<u64> {
    if handle.is_empty() && !handle.is_container() {
        return Ok(0);
    }
    let mut rows = 0_u64;
    for batch in handle.read_arrow_batch_reader(options)? {
        let batch = batch.map_err(|error| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("{error}"),
        })?;
        rows += u64::try_from(batch.num_rows()).unwrap_or(0);
    }
    Ok(rows)
}

/// Materialize the literal rows of a `VALUES` list as one batch.
///
/// One column at a time through the crate's own scalar boundary, so the values
/// land in exactly the physical shape the root declares - nullability,
/// dictionary options, and extension identity included.
#[cfg(feature = "arrow")]
fn literal_batch(root: &Field, rows: &[Value]) -> Result<arrow_array::RecordBatch> {
    use arrow_array::Array;

    let schema = crate::arrow::schema_from_field(root)?;
    let mut columns: Vec<arrow_array::ArrayRef> = Vec::with_capacity(root.field_len());
    for (index, field) in root.fields().iter().enumerate() {
        let mut parts: Vec<arrow_array::ArrayRef> = Vec::with_capacity(rows.len());
        for row in rows {
            let value = row
                .as_record()
                .map(|(_, values)| values)
                .or_else(|| row.as_sequence())
                .and_then(|values| values.get(index))
                .cloned()
                .unwrap_or(Value::Null);
            // A required column cannot hold the absence a short row would
            // leave, and the scalar boundary is what says so by name.
            parts.push(crate::arrow::scalar_array(field, &value)?);
        }
        let refs: Vec<&dyn Array> = parts.iter().map(AsRef::as_ref).collect();
        columns.push(if refs.is_empty() {
            crate::arrow::scalar_array(field, &Value::Null)?.slice(0, 0)
        } else {
            arrow_select::concat::concat(&refs).map_err(crate::Error::Arrow)?
        });
    }
    arrow_array::RecordBatch::try_new(schema, columns).map_err(crate::Error::Arrow)
}

/// Return whether two struct roots declare the same columns, in the same order.
#[cfg(feature = "arrow")]
fn same_columns(held: &Field, next: &Field) -> bool {
    held.field_len() == next.field_len()
        && held
            .fields()
            .iter()
            .zip(next.fields())
            .all(|(left, right)| {
                left.name().eq_ignore_ascii_case(right.name())
                    && left.data_type() == right.data_type()
            })
}

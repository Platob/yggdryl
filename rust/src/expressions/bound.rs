//! Binding: where a name becomes a position and a text becomes a typed value.
//!
//! Binding happens **once per read** - never per batch and never per row. It
//! resolves every column to an ordinal slot chain, computes each node's result
//! datatype, folds every literal into the column's own type, and runs the
//! optimizer with the schema in hand. What comes out is a [`Bound`]: a shared,
//! cheap-to-clone plan that the row evaluator, the vectorized evaluator, and
//! the statistics evaluator all read, which is what makes the three agree.
//!
//! # Why folding is a correctness rule, not an optimization
//!
//! [`Value`]'s [`Ord`] is a *total* order over every kind of value, which is
//! what an Arrow dictionary and a sorting caller need - so `Decimal` sorts
//! after every integer regardless of magnitude. That is right for sorting and
//! useless for `price > 10` against a `decimal(10, 2)` column. Folding the
//! literal into the column's own type is what turns the one comparator this
//! crate has into the right answer, and it is why the engine never grows a
//! second one.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::graph::{Node, NodeId, Plan};
use super::optimize::{self, Explanation};
use super::stats::{Certainty, StatsSource};
use super::{Accessor, Column, Expr};
use crate::{DataType, Error, Field, Result, Value};

/// What binding refuses and what it absorbs.
///
/// Strictness belongs to the *caller's vocabulary*, not to the expression, and
/// the two halves are genuinely independent: a folder route has always
/// tolerated a filter naming a column its rows do not carry, while a table
/// route refuses one - and *both* have always tolerated a value the column's
/// type cannot read, because a filter value arrives as text and text that
/// names nothing simply matches nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Strictness {
    /// Whether a column the schema lacks is an error rather than a drop-out.
    columns: bool,
    /// Whether a literal the type cannot hold is an error rather than a null.
    literals: bool,
}

impl Strictness {
    /// Everything is a claim: an absent column and an unreadable value both
    /// fail. This is what an expression written against a known schema gets.
    pub(super) const STRICT: Self = Self {
        columns: true,
        literals: true,
    };

    /// Nothing is a claim, which is how the `(column, value)` pair vocabulary
    /// has always behaved on a folder route.
    pub(super) const TOLERANT: Self = Self {
        columns: false,
        literals: false,
    };

    /// A column is a claim and a value is text - the table route's reading.
    pub(super) const DECLARED: Self = Self {
        columns: true,
        literals: false,
    };
}

/// One resolved step of a column path.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Step {
    /// A struct child at a fixed ordinal.
    Child {
        /// The child's position in its parent struct.
        index: usize,
        /// The child's name, as the schema spells it.
        name: SmolStr,
    },
    /// A map entry, keyed by a value already read through the key type.
    Key(Value),
    /// One item by position; negative counts back from the end.
    Index(i64),
    /// A half-open range of items.
    Range {
        /// The inclusive lower bound, or the start when absent.
        start: Option<i64>,
        /// The exclusive upper bound, or the end when absent.
        end: Option<i64>,
    },
}

/// A column resolved against a schema: a slot chain and the field it reaches.
#[derive(Clone, Debug)]
pub struct BoundColumn {
    /// The column as it was written.
    column: Column,
    /// The root column's ordinal in the struct root.
    root_index: usize,
    /// The root column's own field, before any accessor.
    root_field: Field,
    /// What reaching inside that column costs, already resolved.
    steps: Vec<Step>,
    /// The field the whole chain reaches.
    leaf: Field,
}

impl BoundColumn {
    /// The column as the caller wrote it.
    #[must_use]
    #[inline]
    pub const fn column(&self) -> &Column {
        &self.column
    }

    /// The root column's name, as the schema spells it.
    #[must_use]
    #[inline]
    pub fn name(&self) -> &str {
        self.root_field.name()
    }

    /// The root column's ordinal in the struct root.
    #[must_use]
    #[inline]
    pub const fn root_index(&self) -> usize {
        self.root_index
    }

    /// The root column's own field.
    #[must_use]
    #[inline]
    pub const fn root_field(&self) -> &Field {
        &self.root_field
    }

    /// The resolved accessor chain, empty for a bare column.
    #[must_use]
    #[inline]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The field the whole chain reaches.
    #[must_use]
    #[inline]
    pub const fn field(&self) -> &Field {
        &self.leaf
    }

    /// The datatype the whole chain reaches.
    #[must_use]
    #[inline]
    pub fn data_type(&self) -> &DataType {
        self.leaf.data_type()
    }

    /// The `PARQUET:field_id` statistics are keyed by, when the leaf has one.
    #[must_use]
    pub fn field_id(&self) -> Option<i32> {
        self.leaf.parquet_field_id().ok().flatten()
    }

    /// Return whether statistics can bound this column at all.
    ///
    /// Both Parquet and Iceberg key bounds per *leaf*, so a struct child has
    /// its own statistics and can prune. A list element, a map entry, and every
    /// range have no statistic that bounds them, so they always answer
    /// [`Certainty::Maybe`] - getting this wrong loses rows, which is why the
    /// rule lives here beside the resolution rather than in a pruning caller.
    #[must_use]
    pub fn is_prunable(&self) -> bool {
        self.steps
            .iter()
            .all(|step| matches!(step, Step::Child { .. }))
    }

    /// The dotted path this column names, for an error message.
    #[must_use]
    pub fn path(&self) -> String {
        self.column.to_string()
    }
}

/// A column node: what was written, and its resolution when there is one.
///
/// Equality and hashing read the *written* column alone, which is what lets the
/// plan intern two references to the same column as one node: within one plan
/// every binding runs against the same schema, so equal spellings resolve
/// identically by construction.
#[derive(Clone, Debug)]
pub struct ColumnRef {
    column: Column,
    bound: Option<BoundColumn>,
}

impl ColumnRef {
    /// A column that no schema has resolved yet.
    #[must_use]
    pub(super) const fn unresolved(column: Column) -> Self {
        Self {
            column,
            bound: None,
        }
    }

    /// A resolved column.
    #[must_use]
    pub(super) fn resolved(bound: BoundColumn) -> Self {
        Self {
            column: bound.column.clone(),
            bound: Some(bound),
        }
    }

    /// The column as it was written.
    #[must_use]
    #[inline]
    pub const fn column(&self) -> &Column {
        &self.column
    }

    /// The root column name as it was written.
    #[must_use]
    #[inline]
    pub fn name(&self) -> &str {
        self.column.name()
    }

    /// The resolution, when this plan was bound.
    #[must_use]
    #[inline]
    pub const fn bound(&self) -> Option<&BoundColumn> {
        self.bound.as_ref()
    }
}

impl PartialEq for ColumnRef {
    fn eq(&self, other: &Self) -> bool {
        self.column == other.column
    }
}

impl Eq for ColumnRef {}

impl std::hash::Hash for ColumnRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.column.hash(state);
    }
}

/// A resolved, typed, optimized plan - the one thing every evaluator reads.
#[derive(Clone, Debug)]
pub struct Bound {
    plan: Arc<Plan>,
    root: NodeId,
    schema: Field,
    explanation: Explanation,
}

impl Bound {
    /// Bind `expr` against a struct root.
    ///
    /// # Errors
    ///
    /// Returns an error naming the columns the schema does have when a name is
    /// absent, or naming both types when a comparison has no common type.
    pub(super) fn new(expr: &Expr, schema: &Field, strictness: Strictness) -> Result<Self> {
        expr.check_depth()?;
        schema.require_struct()?;
        let mut plan = Plan::new();
        let mut binder = Binder {
            schema,
            strictness,
            failure: None,
        };
        let root = plan.insert_expr_with(expr, &mut |column| match binder.resolve(column) {
            Ok(bound) => Some(Arc::new(ColumnRef::resolved(bound))),
            Err(error) => {
                if binder.strictness.columns {
                    binder.failure.get_or_insert(error);
                }
                None
            }
        });
        if let Some(failure) = binder.failure.take() {
            return Err(failure);
        }
        let root = type_and_fold(&mut plan, root, schema, strictness)?;
        let (root, explanation) = optimize::run_explained(&mut plan, root, Some(schema));
        Ok(Self {
            plan: Arc::new(plan),
            root,
            schema: schema.clone(),
            explanation,
        })
    }

    /// Borrow the plan graph.
    #[must_use]
    #[inline]
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The plan's root node.
    #[must_use]
    #[inline]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    /// The struct root this plan was bound against.
    ///
    /// An evaluator handed data that disagrees with it is a typed error naming
    /// the differing column, never a silent mismatch or a null column.
    #[must_use]
    #[inline]
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// The datatype this plan evaluates to.
    #[must_use]
    pub fn data_type(&self) -> DataType {
        self.plan
            .data_type(self.root)
            .cloned()
            .unwrap_or(DataType::Null)
    }

    /// The field this plan's result would be, named by its alias.
    #[must_use]
    pub fn field(&self) -> Field {
        Field::new(self.name(), self.data_type(), true)
    }

    /// The name this plan's result column carries.
    ///
    /// An alias when one was written, and the expression's own canonical
    /// spelling otherwise - which is what SQL does and what makes a computed
    /// column addressable without the caller having to name it.
    #[must_use]
    pub fn name(&self) -> String {
        match self.plan.get(self.root) {
            Some(Node::Alias { name, .. }) => name.to_string(),
            _ => self.to_expr().to_string(),
        }
    }

    /// Read this plan back out as the expression value it names.
    #[must_use]
    pub fn to_expr(&self) -> Expr {
        self.plan.to_expr(self.root)
    }

    /// Every column this plan reads, deduplicated in first-seen order.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for id in self.plan.reachable(self.root) {
            let Some(Node::Column(column)) = self.plan.get(id) else {
                continue;
            };
            let name = column
                .bound()
                .map_or_else(|| column.name().to_owned(), |bound| bound.name().to_owned());
            if !names.iter().any(|held| held.eq_ignore_ascii_case(&name)) {
                names.push(name);
            }
        }
        names
    }

    /// Every resolved column this plan reads.
    #[must_use]
    pub fn bound_columns(&self) -> Vec<BoundColumn> {
        let mut columns = Vec::new();
        for id in self.plan.reachable(self.root) {
            if let Some(Node::Column(column)) = self.plan.get(id) {
                if let Some(bound) = column.bound() {
                    if !columns
                        .iter()
                        .any(|held: &BoundColumn| held.column() == bound.column())
                    {
                        columns.push(bound.clone());
                    }
                }
            }
        }
        columns
    }

    /// The top-level `AND` operands, each as a plan of its own.
    ///
    /// The plan graph is shared, so splitting a predicate into the conjuncts a
    /// read pushes down one layer at a time costs reference counts rather than
    /// a rebuild.
    #[must_use]
    pub fn conjuncts(&self) -> Vec<Self> {
        let roots = match self.plan.get(self.root) {
            Some(Node::And(operands)) => operands.clone(),
            _ if self.is_always_true() => Vec::new(),
            _ => vec![self.root],
        };
        roots
            .into_iter()
            .map(|root| Self {
                plan: Arc::clone(&self.plan),
                root,
                schema: self.schema.clone(),
                explanation: Explanation::default(),
            })
            .collect()
    }

    /// Return whether this plan is the constant `TRUE`.
    #[must_use]
    pub fn is_always_true(&self) -> bool {
        matches!(
            self.plan.get(self.root),
            Some(Node::Literal(Value::Bool(true)))
        ) || matches!(self.plan.get(self.root), Some(Node::And(operands)) if operands.is_empty())
    }

    /// Return whether this plan is the constant `FALSE`.
    #[must_use]
    pub fn is_always_false(&self) -> bool {
        matches!(
            self.plan.get(self.root),
            Some(Node::Literal(Value::Bool(false)))
        ) || matches!(self.plan.get(self.root), Some(Node::Or(operands)) if operands.is_empty())
    }

    /// What the optimizer did, rule by rule, with the plan it produced.
    #[must_use]
    pub fn explain(&self) -> String {
        let mut text = self.explanation.to_string();
        text.push_str(&self.plan.explain_from(self.root));
        text
    }

    /// Narrow to a predicate, refusing an expression that is not boolean.
    ///
    /// Every surface that needs a predicate - a mask, a filtered batch, a
    /// statistics decision - takes [`BoundPredicate`] rather than a bare plan,
    /// so "somebody passed a non-boolean expression as a filter" stops being a
    /// run-time error class.
    ///
    /// # Errors
    ///
    /// Returns an error naming the datatype the expression evaluates to.
    pub fn into_predicate(self) -> Result<BoundPredicate> {
        let data_type = self.data_type();
        // A plan that reads no column at all can still be a predicate - a
        // literal `TRUE` is the commonest one - and a null-typed plan is the
        // `NULL` literal, which is a perfectly good three-valued predicate.
        if !matches!(data_type, DataType::Boolean | DataType::Null) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    "a boolean expression to filter with",
                    format_smolstr!(
                        "{} of {data_type}",
                        crate::text::elide_display(&self.to_expr())
                    ),
                ),
            });
        }
        Ok(BoundPredicate(self))
    }
}

impl std::fmt::Display for Bound {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.to_expr())
    }
}

/// A [`Bound`] proven to evaluate to a boolean.
///
/// The narrowing is the crate's typed-marker philosophy applied to a plan: the
/// same one [`TypedField`](crate::field::TypedField) applies to a field. It is
/// built exactly once, by [`Bound::into_predicate`].
#[derive(Clone, Debug)]
pub struct BoundPredicate(pub(super) Bound);

impl BoundPredicate {
    /// Borrow the plan underneath.
    #[must_use]
    #[inline]
    pub const fn bound(&self) -> &Bound {
        &self.0
    }

    /// Consume this predicate and return the plan underneath.
    #[must_use]
    #[inline]
    pub fn into_bound(self) -> Bound {
        self.0
    }

    /// The top-level conjuncts, each already narrowed.
    #[must_use]
    pub fn conjuncts(&self) -> Vec<Self> {
        self.0.conjuncts().into_iter().map(Self).collect()
    }

    /// Return whether this predicate keeps every row.
    #[must_use]
    pub fn is_always_true(&self) -> bool {
        self.0.is_always_true()
    }

    /// Return whether this predicate keeps no row.
    #[must_use]
    pub fn is_always_false(&self) -> bool {
        self.0.is_always_false()
    }

    /// Every column this predicate reads.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        self.0.columns()
    }

    /// Decide what a source of column statistics can prove about this
    /// predicate, without reading a single row.
    ///
    /// [`Certainty::Maybe`] is always safe; [`Certainty::AlwaysFalse`] must be
    /// provable, because a wrong one loses rows silently.
    #[must_use]
    pub fn evaluate_stats(&self, source: &dyn StatsSource) -> Certainty {
        super::stats::evaluate(&self.0.plan, self.0.root, source)
    }

    /// The conjuncts a statistics source did not settle, or nothing at all.
    ///
    /// A conjunct the source proves always true is dropped, because no row can
    /// fail it; the rest are carried forward for the rows themselves to answer.
    /// `None` is the third answer and it is the valuable one: one conjunct was
    /// proved false for every row, so the file, manifest, or directory holds
    /// nothing and is never opened.
    ///
    /// An empty `Some` therefore means "every row matches" and `None` means
    /// "no row matches" - two answers a bare list could not tell apart, which
    /// is exactly the confusion that would silently read every file.
    #[must_use]
    pub fn residual(&self, source: &dyn StatsSource) -> Option<Vec<Self>> {
        let mut residual = Vec::new();
        for conjunct in self.conjuncts() {
            match conjunct.evaluate_stats(source) {
                Certainty::AlwaysTrue => {}
                Certainty::AlwaysFalse => return None,
                Certainty::Maybe => residual.push(conjunct),
            }
        }
        Some(residual)
    }
}

impl std::fmt::Display for BoundPredicate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// The resolution half of binding: names to slots.
struct Binder<'schema> {
    schema: &'schema Field,
    strictness: Strictness,
    failure: Option<Error>,
}

impl Binder<'_> {
    /// Resolve one written column against the struct root.
    fn resolve(&self, column: &Column) -> Result<BoundColumn> {
        let (root_index, root_field) = resolve_child(self.schema, column.name())?;
        let mut leaf = root_field.clone();
        let mut steps = Vec::with_capacity(column.path().len());
        for accessor in column.path() {
            let (step, next) = resolve_accessor(&leaf, accessor, column)?;
            steps.push(step);
            leaf = next;
        }
        Ok(BoundColumn {
            column: column.clone(),
            root_index,
            root_field,
            steps,
            leaf,
        })
    }
}

/// Resolve one child of a struct, ASCII case-insensitively.
///
/// Exact spelling wins; otherwise a single case-insensitive match is taken and
/// two are refused naming both, exactly as struct reconciliation already
/// refuses an ambiguous fold - never a silent first-wins.
pub(super) fn resolve_child(parent: &Field, name: &str) -> Result<(usize, Field)> {
    if let Some(index) = parent.index_of(name) {
        let field = parent
            .get_field(index)
            .cloned()
            .ok_or_else(|| unknown(parent, name))?;
        return Ok((index, field));
    }
    let mut found: Option<usize> = None;
    for (index, field) in parent.fields().iter().enumerate() {
        if !field.name().eq_ignore_ascii_case(name) {
            continue;
        }
        if let Some(first) = found {
            let other = parent.fields()[first].name();
            return Err(Error::InvalidRecord {
                path: SmolStr::new(name),
                reason: format_smolstr!(
                    "expected one column matching {name:?}, got both {other:?} and {:?}",
                    field.name()
                ),
            });
        }
        found = Some(index);
    }
    let Some(index) = found else {
        return Err(unknown(parent, name));
    };
    let field = parent
        .get_field(index)
        .cloned()
        .ok_or_else(|| unknown(parent, name))?;
    Ok((index, field))
}

/// The failure that names the columns a schema does have.
fn unknown(parent: &Field, name: &str) -> Error {
    let columns = parent
        .fields()
        .iter()
        .map(Field::name)
        .collect::<Vec<_>>()
        .join(", ");
    Error::InvalidRecord {
        path: SmolStr::new(name),
        reason: format_smolstr!(
            "expected a column the schema declares, got {name:?}; it has {}",
            crate::text::elide_display(&columns)
        ),
    }
}

/// Resolve one accessor against the datatype it reaches into.
fn resolve_accessor(parent: &Field, accessor: &Accessor, column: &Column) -> Result<(Step, Field)> {
    let refuse = |what: &str| {
        Err(Error::InvalidRecord {
            path: SmolStr::new(column.to_string()),
            reason: format_smolstr!(
                "expected {what}, got {} at {accessor} of {}",
                parent.data_type(),
                column
            ),
        })
    };
    match accessor {
        Accessor::Child(name) => match unwrap_logical(parent.data_type()) {
            DataType::Struct(_) => {
                let (index, field) = resolve_child(parent, name)?;
                let name = SmolStr::new(field.name());
                Ok((Step::Child { index, name }, nullable(field)))
            }
            // A map with text keys reads `a.b` as `a['b']`, which is the
            // sugar every dialect spells and costs nothing to allow.
            DataType::Map(map) => {
                let (key_field, value_field) = map_halves(map)?;
                let key = coerce_value(&Value::String(name.clone()), key_field.data_type())
                    .ok_or_else(|| Error::InvalidRecord {
                        path: SmolStr::new(column.to_string()),
                        reason: format_smolstr!(
                            "expected a key the map's {} key type can hold, got {name:?}",
                            key_field.data_type()
                        ),
                    })?;
                Ok((Step::Key(key), nullable(value_field)))
            }
            _ => refuse("a struct or a map to read a child of"),
        },
        Accessor::Key(key) => match unwrap_logical(parent.data_type()) {
            DataType::Map(map) => {
                let (key_field, value_field) = map_halves(map)?;
                let coerced = coerce_value(key, key_field.data_type()).ok_or_else(|| {
                    Error::InvalidRecord {
                        path: SmolStr::new(column.to_string()),
                        reason: format_smolstr!(
                            "expected a key the map's {} key type can hold, got {}",
                            key_field.data_type(),
                            crate::text::elide_display(&super::Literal(key))
                        ),
                    }
                })?;
                Ok((Step::Key(coerced), nullable(value_field)))
            }
            DataType::Struct(_) => {
                let Value::String(name) = key else {
                    return refuse("a text key to read a struct child by");
                };
                let (index, field) = resolve_child(parent, name)?;
                let name = SmolStr::new(field.name());
                Ok((Step::Child { index, name }, nullable(field)))
            }
            _ => refuse("a map or a struct to read a key of"),
        },
        Accessor::Index(index) => match item_field(parent) {
            Some(item) => Ok((Step::Index(*index), item)),
            None => refuse("a list, string, or binary value to index"),
        },
        Accessor::Range { start, end } => {
            // A range of a list is a list and a range of text is text, so the
            // datatype is the container's own rather than its item's.
            if item_field(parent).is_none() {
                return refuse("a list, string, or binary value to take a range of");
            }
            Ok((
                Step::Range {
                    start: *start,
                    end: *end,
                },
                nullable(parent.clone()),
            ))
        }
    }
}

/// The key and value fields a map's entries struct declares.
///
/// A map is an entries struct of exactly two children, so the halves are read
/// off it rather than remembered separately - the schema is the authority.
fn map_halves(map: &crate::MapType) -> Result<(Field, Field)> {
    let entries = map.entries();
    match (entries.get_field(0), entries.get_field(1)) {
        (Some(key), Some(value)) => Ok((key.clone(), value.clone())),
        _ => Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "a map whose entries declare a key and a value",
                entries.field_len(),
            ),
        }),
    }
}

/// The field one item of a container has, when the container is indexable.
fn item_field(parent: &Field) -> Option<Field> {
    let field = match unwrap_logical(parent.data_type()) {
        DataType::List(item)
        | DataType::LargeList(item)
        | DataType::ListView(item)
        | DataType::LargeListView(item)
        | DataType::FixedSizeList(item, _) => (**item).clone(),
        // One character of text is text, and one byte of binary is binary, so
        // the two families slice into themselves rather than into a scalar.
        text @ (DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
            Field::new("item", text.clone(), true)
        }
        bytes @ (DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_)) => Field::new("item", bytes.clone(), true),
        _ => return None,
    };
    Some(nullable(field))
}

/// Reaching inside a value can always come up empty, so the result is nullable.
fn nullable(field: Field) -> Field {
    if field.is_nullable() {
        field
    } else {
        field.with_nullable(true)
    }
}

/// See through the wrappers that do not change what a value *is*.
///
/// A dictionary and a run-end encoding are storage layouts over their value
/// type, so an accessor and a comparison read straight through them rather
/// than learning that the layout exists.
pub(super) fn unwrap_logical(data_type: &DataType) -> &DataType {
    match data_type {
        DataType::Dictionary(dictionary) => unwrap_logical(dictionary.value()),
        DataType::RunEndEncoded(encoded) => unwrap_logical(encoded.values().data_type()),
        other => other,
    }
}

/// The family a datatype's values compare within.
///
/// [`Value`]'s one total order is exact inside a family and rank-based across
/// families, so this is what decides whether a comparison needs a coercion
/// before it can mean anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Family {
    Null,
    Boolean,
    Integer,
    Float,
    Decimal,
    Text,
    Binary,
    Date,
    Time,
    Timestamp,
    NaiveTimestamp,
    Duration,
    Interval,
    Nested,
}

/// The family a datatype's values compare within.
#[must_use]
pub(super) fn family(data_type: &DataType) -> Family {
    use DataType as D;
    match unwrap_logical(data_type) {
        D::Null => Family::Null,
        D::Boolean => Family::Boolean,
        D::Int8 | D::Int16 | D::Int32 | D::Int64 | D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => {
            Family::Integer
        }
        D::Float16 | D::Float32 | D::Float64 => Family::Float,
        D::Decimal32 { .. } | D::Decimal64 { .. } | D::Decimal128 { .. } | D::Decimal256 { .. } => {
            Family::Decimal
        }
        D::Utf8 | D::LargeUtf8 | D::Utf8View => Family::Text,
        D::Binary | D::LargeBinary | D::BinaryView | D::FixedSizeBinary(_) => Family::Binary,
        D::Date32 | D::Date64 => Family::Date,
        D::Time32(_) | D::Time64(_) => Family::Time,
        D::Timestamp(_, Some(_)) => Family::Timestamp,
        D::Timestamp(_, None) => Family::NaiveTimestamp,
        D::Duration(_) => Family::Duration,
        D::Interval(_) => Family::Interval,
        _ => Family::Nested,
    }
}

/// The type two operands must both be read as before they can be compared.
///
/// `None` means the pair has no common comparison type at all, which is a bind
/// error naming both sides rather than a run-time surprise.
#[must_use]
pub(super) fn common_comparison_type(left: &DataType, right: &DataType) -> Option<DataType> {
    let (left_family, right_family) = (family(left), family(right));
    if left_family == right_family {
        // One family is one exact order across widths, scales, and units, so
        // there is nothing to convert.
        return Some(left.clone());
    }
    Some(match (left_family, right_family) {
        // A null literal takes whatever it is compared against.
        (Family::Null, _) => right.clone(),
        (_, Family::Null) => left.clone(),
        // An integer is an exact decimal with scale zero, so the decimal side
        // is the one that keeps every digit.
        (Family::Integer, Family::Decimal) => right.clone(),
        (Family::Decimal, Family::Integer) => left.clone(),
        // A float cannot hold every decimal exactly, and a decimal cannot hold
        // every float at all, so SQL's own answer - compare as double - is
        // what this does, and the documentation says it is inexact.
        (Family::Float, Family::Integer | Family::Decimal)
        | (Family::Integer | Family::Decimal, Family::Float) => DataType::Float64,
        // A date is a timestamp at midnight, which is the reading every
        // dialect gives `ts >= DATE '2024-01-01'`.
        (Family::Date, Family::Timestamp | Family::NaiveTimestamp) => right.clone(),
        (Family::Timestamp | Family::NaiveTimestamp, Family::Date) => left.clone(),
        _ => return None,
    })
}

/// Read one value as the datatype names it, or answer that it cannot be.
///
/// This is the value-level sibling of [`ArrowCast`](crate::ArrowCast): the row
/// evaluator, the statistics evaluator, and literal folding all run in builds
/// with no Arrow at all, so the conversion they need cannot be an array cast.
/// The two agree on what converts; where they could differ - a lossy narrowing
/// - both refuse.
#[must_use]
#[allow(clippy::too_many_lines)]
pub(super) fn coerce_value(value: &Value, target: &DataType) -> Option<Value> {
    use DataType as D;

    if matches!(value, Value::Null) {
        return Some(Value::Null);
    }
    match unwrap_logical(target) {
        D::Null => None,
        D::Boolean => match value {
            Value::Bool(_) => Some(value.clone()),
            Value::String(text) => match text.to_ascii_lowercase().as_str() {
                "true" | "t" | "yes" | "y" | "1" => Some(Value::Bool(true)),
                "false" | "f" | "no" | "n" | "0" => Some(Value::Bool(false)),
                _ => None,
            },
            other => other.as_i128().map(|integer| Value::Bool(integer != 0)),
        },
        D::Int8 => bounded_signed(value, i128::from(i8::MIN), i128::from(i8::MAX)),
        D::Int16 => bounded_signed(value, i128::from(i16::MIN), i128::from(i16::MAX)),
        D::Int32 => bounded_signed(value, i128::from(i32::MIN), i128::from(i32::MAX)),
        D::Int64 => bounded_signed(value, i128::from(i64::MIN), i128::from(i64::MAX)),
        D::UInt8 => bounded_unsigned(value, u128::from(u8::MAX)),
        D::UInt16 => bounded_unsigned(value, u128::from(u16::MAX)),
        D::UInt32 => bounded_unsigned(value, u128::from(u32::MAX)),
        D::UInt64 => bounded_unsigned(value, u128::from(u64::MAX)),
        D::Float16 | D::Float32 => as_f64(value).map(|number| Value::from(number as f32)),
        D::Float64 => as_f64(value).map(Value::from),
        D::Decimal32 { scale, .. }
        | D::Decimal64 { scale, .. }
        | D::Decimal128 { scale, .. }
        | D::Decimal256 { scale, .. } => as_decimal(value, *scale),
        D::Utf8 | D::LargeUtf8 | D::Utf8View => Some(match value {
            Value::String(_) => value.clone(),
            other => Value::String(SmolStr::new(super::Literal(other).to_string())),
        }),
        D::Binary | D::LargeBinary | D::BinaryView | D::FixedSizeBinary(_) => match value {
            Value::Bytes(_) => Some(value.clone()),
            Value::String(text) => Some(Value::Bytes(Arc::from(text.as_bytes()))),
            _ => None,
        },
        D::Date32 | D::Date64 => match value {
            Value::Date(_) => Some(value.clone()),
            Value::String(text) => crate::generic::iso::parse_date(text).ok().map(Value::Date),
            Value::DateTime(count, unit) => days_of(*count, *unit).map(Value::Date),
            Value::Timestamp(count, unit, _) => days_of(*count, *unit).map(Value::Date),
            _ => None,
        },
        D::Time32(unit) | D::Time64(unit) => match value {
            Value::Time(count, held) => {
                restate(*count, *held, *unit).map(|count| Value::Time(count, *unit))
            }
            Value::String(text) => crate::generic::iso::parse_time(text)
                .ok()
                .and_then(|(count, held)| restate(count, held, *unit))
                .map(|count| Value::Time(count, *unit)),
            _ => None,
        },
        D::Timestamp(unit, zone) => {
            let naive = |count: i64, held: crate::TimeUnit| restate(count, held, *unit);
            let counted = match value {
                Value::Timestamp(count, held, _) | Value::DateTime(count, held) => {
                    naive(*count, *held)
                }
                Value::Date(days) => i64::from(*days)
                    .checked_mul(86_400)
                    .and_then(|seconds| restate(seconds, crate::TimeUnit::Second, *unit)),
                Value::String(text) => crate::generic::iso::parse_timestamp(text)
                    .ok()
                    .and_then(|(count, held, _)| naive(count, held))
                    .or_else(|| {
                        crate::generic::iso::parse_datetime(text)
                            .ok()
                            .and_then(|(count, held)| naive(count, held))
                    })
                    .or_else(|| {
                        crate::generic::iso::parse_date(text).ok().and_then(|days| {
                            i64::from(days).checked_mul(86_400).and_then(|seconds| {
                                restate(seconds, crate::TimeUnit::Second, *unit)
                            })
                        })
                    }),
                _ => None,
            }?;
            Some(match zone {
                Some(zone) => Value::Timestamp(counted, *unit, zone.clone()),
                None => Value::DateTime(counted, *unit),
            })
        }
        D::Duration(unit) => match value {
            Value::Duration(count, held) => {
                restate(*count, *held, *unit).map(|count| Value::Duration(count, *unit))
            }
            Value::String(text) => crate::generic::iso::parse_duration(text)
                .ok()
                .and_then(|(count, held)| restate(count, held, *unit))
                .map(|count| Value::Duration(count, *unit)),
            _ => None,
        },
        // A calendar interval is a tuple, not a count, so nothing converts
        // into one; a nested value is compared as the value it already is.
        D::Interval(_) => None,
        _ => Some(value.clone()),
    }
}

/// Read any numeric or textual value as an `f64`.
fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::F32(_) | Value::F64(_) => value.as_f64(),
        Value::Decimal(unscaled, scale) => {
            let divisor = 10_f64.powi(i32::from(*scale));
            #[allow(clippy::cast_precision_loss)]
            Some(*unscaled as f64 / divisor)
        }
        Value::String(text) => text.parse::<f64>().ok(),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        #[allow(clippy::cast_precision_loss)]
        other => other.as_i128().map(|integer| integer as f64),
    }
}

/// Read any value as an exact decimal at the target scale.
fn as_decimal(value: &Value, scale: i8) -> Option<Value> {
    if let Some(unscaled) = value.decimal_unscaled_at(scale) {
        return Some(Value::Decimal(unscaled, scale));
    }
    let scaled = |integer: i128| {
        let factor = 10_i128.checked_pow(u32::try_from(scale.max(0)).ok()?)?;
        if scale >= 0 {
            integer
                .checked_mul(factor)
                .map(|unscaled| Value::Decimal(unscaled, scale))
        } else {
            // A negative scale multiplies rather than divides, so the value
            // has to be an exact multiple or the restatement drops digits.
            let divisor = 10_i128.checked_pow(u32::try_from(-i32::from(scale)).ok()?)?;
            (integer % divisor == 0).then(|| Value::Decimal(integer / divisor, scale))
        }
    };
    match value {
        Value::String(text) => parse_decimal(text, scale),
        Value::F32(_) | Value::F64(_) => None,
        other => other.as_i128().and_then(scaled),
    }
}

/// Read decimal text at an exact scale, refusing to drop a digit.
fn parse_decimal(text: &str, scale: i8) -> Option<Value> {
    let trimmed = text.trim();
    let (whole, fraction) = trimmed.split_once('.').unwrap_or((trimmed, ""));
    let negative = whole.starts_with('-');
    let whole_digits = whole.trim_start_matches(['-', '+']);
    if whole_digits.is_empty() && fraction.is_empty() {
        return None;
    }
    if !whole_digits.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let written = i8::try_from(fraction.len()).ok()?;
    let mut unscaled = whole_digits.parse::<i128>().unwrap_or(0);
    unscaled = unscaled.checked_mul(10_i128.checked_pow(u32::try_from(fraction.len()).ok()?)?)?;
    if !fraction.is_empty() {
        unscaled = unscaled.checked_add(fraction.parse::<i128>().ok()?)?;
    }
    if negative {
        unscaled = unscaled.checked_neg()?;
    }
    Value::Decimal(unscaled, written)
        .decimal_unscaled_at(scale)
        .map(|unscaled| Value::Decimal(unscaled, scale))
}

/// Read any integral value inside a signed width's range.
fn bounded_signed(value: &Value, least: i128, most: i128) -> Option<Value> {
    let integer = match value {
        Value::String(text) => text.trim().parse::<i128>().ok()?,
        Value::Bool(flag) => i128::from(*flag),
        Value::Decimal(..) => value.decimal_unscaled_at(0)?,
        other => other.as_i128()?,
    };
    (integer >= least && integer <= most).then(|| Value::I64(integer as i64))
}

/// Read any integral value inside an unsigned width's range.
fn bounded_unsigned(value: &Value, most: u128) -> Option<Value> {
    let integer = match value {
        Value::String(text) => text.trim().parse::<u128>().ok()?,
        Value::Bool(flag) => u128::from(*flag),
        Value::Decimal(..) => u128::try_from(value.decimal_unscaled_at(0)?).ok()?,
        other => other.as_u128()?,
    };
    (integer <= most).then(|| Value::U64(integer as u64))
}

/// Restate a temporal count from one resolution to another, exactly.
fn restate(count: i64, held: crate::TimeUnit, wanted: crate::TimeUnit) -> Option<i64> {
    Value::Duration(count, held).temporal_count_at(wanted)
}

/// The whole days a temporal count covers, refusing a partial day.
fn days_of(count: i64, unit: crate::TimeUnit) -> Option<i32> {
    let seconds = restate(count, unit, crate::TimeUnit::Second)?;
    let days = seconds.div_euclid(86_400);
    i32::try_from(days).ok()
}

/// Type every node, folding each literal into the type it is compared against.
///
/// Typing and folding are one pass because they need each other: a literal
/// cannot be folded until the column beside it has a type, and a comparison
/// cannot be typed until both its operands are.
fn type_and_fold(
    plan: &mut Plan,
    root: NodeId,
    schema: &Field,
    strictness: Strictness,
) -> Result<NodeId> {
    // The schema is already baked into every resolved column, so typing needs
    // only the strictness the caller chose.
    let _ = schema;
    let mut typer = Typer {
        strictness,
        rebuilt: std::collections::HashMap::new(),
    };
    typer.visit(plan, root)
}

/// The typing half of binding.
struct Typer {
    strictness: Strictness,
    rebuilt: std::collections::HashMap<NodeId, NodeId>,
}

impl Typer {
    /// Type one node and everything below it, returning its rebuilt id.
    fn visit(&mut self, plan: &mut Plan, id: NodeId) -> Result<NodeId> {
        if let Some(done) = self.rebuilt.get(&id) {
            return Ok(*done);
        }
        let node = plan.get(id).cloned().unwrap_or(Node::Literal(Value::Null));
        let mut children = Vec::new();
        node.for_each_child(|child| children.push(child));
        let mut mapped = std::collections::HashMap::new();
        for child in children {
            let rebuilt = self.visit(plan, child)?;
            mapped.insert(child, rebuilt);
        }
        let rebuilt = node.map_children(|child| mapped.get(&child).copied().unwrap_or(child));
        let rebuilt = self.reshape(plan, rebuilt)?;
        let new_id = plan.insert(rebuilt);
        let data_type = self.data_type_of(plan, new_id)?;
        plan.set_data_type(new_id, data_type);
        self.rebuilt.insert(id, new_id);
        Ok(new_id)
    }

    /// Fold the literals of one node into the types beside them.
    fn reshape(&mut self, plan: &mut Plan, node: Node) -> Result<Node> {
        match node {
            Node::Compare { op, left, right } => {
                let (left, right) = self.align(plan, left, right)?;
                Ok(Node::Compare { op, left, right })
            }
            Node::In {
                child,
                list,
                negated,
            } => {
                let target = plan.data_type(child).cloned();
                let list = list
                    .into_iter()
                    .map(|item| self.fold_into(plan, item, target.as_ref()))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Node::In {
                    child,
                    list,
                    negated,
                })
            }
            Node::Between {
                child,
                low,
                high,
                negated,
            } => {
                let target = plan.data_type(child).cloned();
                Ok(Node::Between {
                    child,
                    low: self.fold_into(plan, low, target.as_ref())?,
                    high: self.fold_into(plan, high, target.as_ref())?,
                    negated,
                })
            }
            other => Ok(other),
        }
    }

    /// Bring two operands into one comparison type.
    fn align(&mut self, plan: &mut Plan, left: NodeId, right: NodeId) -> Result<(NodeId, NodeId)> {
        let left_type = plan.data_type(left).cloned();
        let right_type = plan.data_type(right).cloned();
        let (Some(left_type), Some(right_type)) = (left_type, right_type) else {
            return Ok((left, right));
        };
        if family(&left_type) == family(&right_type) {
            return Ok((left, right));
        }
        // A literal beside a column is folded into the column's own type,
        // which is the case worth optimizing: it is what makes
        // `price > '10.5'` compare two decimals rather than a string and one.
        if plan.get(right).and_then(Node::as_literal).is_some() {
            let folded = self.fold_into(plan, right, Some(&left_type))?;
            if folded != right {
                return Ok((left, folded));
            }
        }
        if plan.get(left).and_then(Node::as_literal).is_some() {
            let folded = self.fold_into(plan, left, Some(&right_type))?;
            if folded != left {
                return Ok((folded, right));
            }
        }
        let Some(common) = common_comparison_type(&left_type, &right_type) else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected two operands with a common comparison type, got {left_type} and {right_type}"
                ),
            });
        };
        Ok((
            self.coerce_to(plan, left, &left_type, &common),
            self.coerce_to(plan, right, &right_type, &common),
        ))
    }

    /// Wrap one operand in the cast that brings it to the common type.
    fn coerce_to(
        &mut self,
        plan: &mut Plan,
        id: NodeId,
        held: &DataType,
        common: &DataType,
    ) -> NodeId {
        if family(held) == family(common) {
            return id;
        }
        let cast = plan.insert(Node::Cast {
            child: id,
            data_type: common.clone(),
            safe: true,
        });
        plan.set_data_type(cast, common.clone());
        cast
    }

    /// Fold one literal into a target type, once, at bind time.
    ///
    /// A literal the type cannot hold is a bind error under
    /// [`Strictness::Strict`], and folds to `NULL` under
    /// [`Strictness::Tolerant`] - which makes every comparison it appears in
    /// unknown, so the predicate matches nothing, exactly as the tolerant pair
    /// vocabulary has always behaved.
    fn fold_into(
        &mut self,
        plan: &mut Plan,
        id: NodeId,
        target: Option<&DataType>,
    ) -> Result<NodeId> {
        let Some(target) = target else { return Ok(id) };
        let Some(value) = plan.get(id).and_then(Node::as_literal).cloned() else {
            return Ok(id);
        };
        if matches!(value, Value::Null) {
            return Ok(id);
        }
        let held = plan.data_type(id).cloned();
        if held
            .as_ref()
            .is_some_and(|held| family(held) == family(target))
        {
            return Ok(id);
        }
        match coerce_value(&value, target) {
            Some(folded) => {
                let new_id = plan.insert(Node::Literal(folded));
                plan.set_data_type(new_id, target.clone());
                Ok(new_id)
            }
            None if !self.strictness.literals => {
                let new_id = plan.insert(Node::Literal(Value::Null));
                plan.set_data_type(new_id, target.clone());
                Ok(new_id)
            }
            None => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    format_smolstr!("a literal a {target} column can hold"),
                    crate::text::elide_display(&super::Literal(&value)),
                ),
            }),
        }
    }

    /// The datatype one already-rebuilt node evaluates to.
    #[allow(clippy::too_many_lines)]
    fn data_type_of(&self, plan: &Plan, id: NodeId) -> Result<DataType> {
        let node = plan.get(id).cloned().unwrap_or(Node::Literal(Value::Null));
        let child_type = |child: NodeId| plan.data_type(child).cloned().unwrap_or(DataType::Null);
        Ok(match node {
            Node::Column(column) => match column.bound() {
                Some(bound) => bound.data_type().clone(),
                // A column the tolerant mode could not resolve reads as
                // nothing, which makes every comparison on it unknown.
                None => DataType::Null,
            },
            Node::Literal(value) => value.data_type().unwrap_or(DataType::Null),
            Node::Cast { data_type, .. } => data_type,
            Node::Compare { .. }
            | Node::And(_)
            | Node::Or(_)
            | Node::Not(_)
            | Node::IsNull(_)
            | Node::IsNotNull(_)
            | Node::In { .. }
            | Node::Between { .. }
            | Node::Like { .. }
            | Node::StartsWith { .. } => DataType::Boolean,
            Node::Arithmetic { op, left, right } => {
                arithmetic_type(op, &child_type(left), &child_type(right))?
            }
            Node::Neg(child) => child_type(child),
            Node::Function { name, args } => function_type(
                name,
                &args.iter().map(|arg| child_type(*arg)).collect::<Vec<_>>(),
            )?,
            Node::Case {
                branches,
                otherwise,
            } => {
                let mut result = otherwise.map_or(DataType::Null, child_type);
                for (_, then) in &branches {
                    let then_type = child_type(*then);
                    result = if matches!(result, DataType::Null) {
                        then_type
                    } else if matches!(then_type, DataType::Null)
                        || family(&then_type) == family(&result)
                    {
                        result
                    } else {
                        common_comparison_type(&result, &then_type).ok_or_else(|| {
                            Error::InvalidRecord {
                                path: SmolStr::new_static("$"),
                                reason: format_smolstr!(
                                    "expected every CASE branch to share a type, got {result} and {then_type}"
                                ),
                            }
                        })?
                    };
                }
                result
            }
            Node::Alias { child, .. } => child_type(child),
        })
    }
}

/// The datatype an arithmetic node evaluates to.
fn arithmetic_type(op: super::ArithOp, left: &DataType, right: &DataType) -> Result<DataType> {
    use super::ArithOp;

    let (left_family, right_family) = (family(left), family(right));
    if matches!(left_family, Family::Null) || matches!(right_family, Family::Null) {
        return Ok(DataType::Null);
    }
    // A timestamp minus a timestamp is an elapsed count, which is the one
    // arithmetic pairing whose result is a different family from its operands.
    if op == ArithOp::Sub
        && left_family == right_family
        && matches!(
            left_family,
            Family::Timestamp | Family::NaiveTimestamp | Family::Date | Family::Time
        )
    {
        return Ok(DataType::Duration(crate::TimeUnit::Millisecond));
    }
    let numeric = |held: Family| matches!(held, Family::Integer | Family::Float | Family::Decimal);
    if !numeric(left_family) || !numeric(right_family) {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected two numbers to apply {op} to, got {left} and {right}"
            ),
        });
    }
    Ok(match (left_family, right_family) {
        (Family::Float, _) | (_, Family::Float) => DataType::Float64,
        // A division is not exact in general, so an exact pair still answers
        // as a double rather than pretending to a scale it did not derive.
        _ if op == ArithOp::Div => DataType::Float64,
        (Family::Decimal, _) => left.clone(),
        (_, Family::Decimal) => right.clone(),
        _ => DataType::Int64,
    })
}

/// The datatype a function call evaluates to.
fn function_type(name: super::Function, args: &[DataType]) -> Result<DataType> {
    use super::Function as F;

    let first = args.first().cloned().unwrap_or(DataType::Null);
    Ok(match name {
        F::Coalesce => args
            .iter()
            .find(|held| !matches!(family(held), Family::Null))
            .cloned()
            .unwrap_or(DataType::Null),
        F::Length => DataType::Int64,
        F::Lower | F::Upper | F::Trim | F::Substring => DataType::Utf8,
        F::Abs | F::Truncate => first,
        F::Year | F::Month | F::Day | F::Hour | F::Minute | F::Second => DataType::Int32,
    })
}

/// Answer a comparison over two already-aligned values, three-valued.
///
/// `None` is SQL's *unknown*: either operand was null, or the two ended up in
/// families with no common reading, and neither answer is `false`.
#[must_use]
pub(super) fn compare(op: super::CompareOp, left: &Value, right: &Value) -> Option<bool> {
    if matches!(left, Value::Null) || matches!(right, Value::Null) {
        return None;
    }
    let left_family = value_family(left);
    let right_family = value_family(right);
    if left_family == right_family {
        return Some(op.answers(left.cmp(right)));
    }
    // Binding aligns the families it can, so an unaligned pair here is a
    // comparison the caller wrote between two columns of different kinds -
    // and the safe answer to a comparison with no reading is unknown.
    let aligned = common_value_type(left_family, right_family)?;
    let left = coerce_value(left, &aligned)?;
    let right = coerce_value(right, &aligned)?;
    Some(op.answers(left.cmp(&right)))
}

/// The family a value belongs to, mirroring [`family`] over datatypes.
fn value_family(value: &Value) -> Family {
    match value {
        Value::Null => Family::Null,
        Value::Bool(_) => Family::Boolean,
        Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::I128(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::U64(_)
        | Value::U128(_) => Family::Integer,
        Value::F32(_) | Value::F64(_) => Family::Float,
        Value::Decimal(..) => Family::Decimal,
        Value::String(_) => Family::Text,
        Value::Bytes(_) => Family::Binary,
        Value::Date(_) => Family::Date,
        Value::Time(..) => Family::Time,
        Value::Timestamp(..) => Family::Timestamp,
        Value::DateTime(..) => Family::NaiveTimestamp,
        Value::Duration(..) => Family::Duration,
        _ => Family::Nested,
    }
}

/// A concrete datatype two value families can both be read as.
fn common_value_type(left: Family, right: Family) -> Option<DataType> {
    Some(match (left, right) {
        (Family::Integer, Family::Decimal) | (Family::Decimal, Family::Integer) => {
            // Every integer is exact at scale zero, and a decimal restates to
            // the widest scale either side carries; picking the larger keeps
            // both readings exact.
            DataType::Decimal128 {
                precision: 38,
                scale: 18,
            }
        }
        (Family::Float, Family::Integer | Family::Decimal)
        | (Family::Integer | Family::Decimal, Family::Float) => DataType::Float64,
        (Family::Date, Family::Timestamp | Family::NaiveTimestamp)
        | (Family::Timestamp | Family::NaiveTimestamp, Family::Date) => {
            DataType::Timestamp(crate::TimeUnit::Millisecond, None)
        }
        _ => return None,
    })
}

/// Read a certainty as a boolean where one is already known.
impl From<bool> for Certainty {
    fn from(known: bool) -> Self {
        if known {
            Self::AlwaysTrue
        } else {
            Self::AlwaysFalse
        }
    }
}

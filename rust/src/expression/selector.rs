//! The `select` block: which columns a read or write publishes, computed how,
//! typed as what.
//!
//! A [`Selector`] is an optional `*` - every column, less the ones it
//! excludes - followed by a list of [`Projection`]s, and a projection is one
//! output column: a [`Term`] that computes it, the name it is published under,
//! and - the way a `create table` column definition says it - the datatype and
//! nullability it is published as. `select *` publishes the rows unchanged,
//! and `select * exclude (a), upper(b) as c` every column but `a` with `c`
//! appended. One selector therefore spells everything from `id, price` through
//! `cast(price as float64) * size as notional` to the column list a table is
//! created with, `id int64 not null, price decimal(9,2)`.
//!
//! # One projection may multiply rows
//!
//! `unnest(<serie>) [as name]` is the one select-list form that does: each
//! parent row becomes one row per element of its serie, the other columns
//! repeated beside it, and a null or empty serie drops the row. A struct item
//! publishes one column per child, named `<name>.<child>`; any other item one
//! column named `name`. It stands only as the whole term of a projection, at
//! most once per selector, and a filter or an ordering that names what it
//! publishes runs after it.
//!
//! # One application, four targets
//!
//! A selector applies to a schema, a row, an Arrow array, and an Arrow batch,
//! and the four cannot disagree: every one binds each projection's term once
//! through the one [`bind`](Term::bind) and types it through the one
//! [`Term::field`], so the schema [`Selector::apply_field`] answers is the
//! schema the batches [`Selector::apply_arrow_batch`] produces carry.
//!
//! # A field holds a plan
//!
//! A struct [`Field`] already says what columns exist, of what type, nullable
//! or not. With the [`TRANSFORM:`](super::TransformField) protocol it also
//! says how a column is computed, so [`Selector::into_field`] writes a
//! selector into a field and [`Selector::from_field`] reads it back - which is
//! what lets a declared field on record options carry a whole projection, and
//! a derived partition column be the projection it always was.

use std::str::FromStr;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::Safety;
use super::arrow::ColumnCast;
use super::bind::Bound;
use super::eval::convert;
use super::path::FieldPath;
use super::term::Term;
use crate::{DataType, Error, Field, Metadata, Result, Scalar, StructType};

/// One output column: the term that computes it, its name, and what it is
/// published as.
///
/// ```
/// use yggdryl::expression::Projection;
///
/// # fn main() -> yggdryl::Result<()> {
/// let typed: Projection = "price * size as notional float64 not null".parse()?;
/// assert_eq!(typed.name(), "notional");
/// assert_eq!(typed.dtype().map(ToString::to_string).as_deref(), Some("float64"));
/// assert_eq!(typed.nullable(), Some(false));
/// assert_eq!(typed.to_string(), "price * size as notional float64 not null");
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct Projection {
    term: Term,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<SmolStr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dtype: Option<DataType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nullable: Option<bool>,
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    metadata: Metadata,
}

impl Projection {
    /// Publish a term under the name it derives.
    #[must_use]
    pub fn new(term: Term) -> Self {
        Self {
            term,
            alias: None,
            dtype: None,
            nullable: None,
            metadata: Metadata::new(),
        }
    }

    /// Publish a term under an explicit name.
    #[must_use]
    pub fn aliased(term: Term, alias: impl Into<SmolStr>) -> Self {
        Self::new(term).with_alias(alias)
    }

    /// Publish one column unchanged.
    #[must_use]
    pub fn column(name: impl Into<SmolStr>) -> Self {
        Self::new(Term::column(name))
    }

    /// Return this projection published under a name.
    #[must_use]
    pub fn with_alias(mut self, alias: impl Into<SmolStr>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    /// Return this projection published as a datatype.
    ///
    /// The term is cast strictly into it when the projection is applied, so a
    /// value the datatype cannot hold is an error naming the column rather
    /// than a silent null.
    #[must_use]
    pub fn with_dtype(mut self, dtype: DataType) -> Self {
        self.dtype = Some(dtype);
        self
    }

    /// Return this projection published required or nullable.
    ///
    /// A required projection is verified when it is applied: a null the term
    /// computed is an error naming the column.
    #[must_use]
    pub const fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = Some(nullable);
        self
    }

    /// Return this projection published with metadata on its column.
    ///
    /// This is the `with (...)` clause of a column declaration: the
    /// properties the published field carries, protocol views included, so
    /// a schema round-trips through a plan with its declarations intact.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// The term computed.
    #[must_use]
    pub const fn term(&self) -> &Term {
        &self.term
    }

    /// The metadata the published column carries.
    #[must_use]
    pub const fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// The explicit name, when one was given.
    #[must_use]
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    /// The declared datatype, when one was given.
    #[must_use]
    pub const fn dtype(&self) -> Option<&DataType> {
        self.dtype.as_ref()
    }

    /// The declared nullability, when one was given.
    #[must_use]
    pub const fn nullable(&self) -> Option<bool> {
        self.nullable
    }

    /// The name this projection publishes.
    ///
    /// An aliased projection uses its alias; a path keeps the name of what it
    /// reaches, and so does an `unnest` of one; anything else is named by its
    /// canonical text, so a selector never produces two columns that are
    /// impossible to tell apart.
    #[must_use]
    pub fn name(&self) -> SmolStr {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        let named = match self.term.as_unnest() {
            Some([argument]) => argument,
            _ => &self.term,
        };
        match named {
            Term::Path(steps) => FieldPath::from_shared(Arc::clone(steps), None)
                .column_name()
                .map_or_else(|| SmolStr::new(self.term.to_string()), SmolStr::new),
            _ => SmolStr::new(self.term.to_string()),
        }
    }

    /// Return whether this projection publishes one column as it is stored.
    ///
    /// A bare column is what a projection pushdown can hand to an encoding and
    /// what an Arrow batch can answer without touching a buffer.
    #[must_use]
    pub fn is_column(&self) -> bool {
        self.term.as_column().is_some_and(|column| {
            self.dtype.is_none()
                && self.metadata.is_empty()
                && self.alias.as_deref().is_none_or(|alias| alias == column)
        })
    }

    /// The field this projection publishes against a struct root schema.
    ///
    /// The term is typed by [`Term::field`], named by [`Self::name`], and then
    /// restated as whatever the projection declares. An `unnest` publishes
    /// its serie's item, declared the same way: a struct item is the
    /// children a [`Selector`] expands it into, each `<name>.<child>`.
    ///
    /// # Errors
    ///
    /// Returns an error when the term cannot be typed against the schema, or
    /// an `unnest` is not of one serie.
    pub fn field(&self, schema: &Field) -> Result<Field> {
        let typed = match self.term.as_unnest() {
            Some(arguments) => self.unnested_item(arguments, schema)?,
            None => self.term.field(schema)?,
        };
        let mut field = typed.with_name(self.name());
        if let Some(dtype) = &self.dtype {
            field = field.try_with_dtype(dtype.clone())?;
        }
        if let Some(nullable) = self.nullable {
            field = field.with_nullable(nullable);
        }
        if !self.metadata.is_empty() {
            field = field.try_with_metadata_entries(self.metadata.iter())?;
        }
        Ok(field)
    }

    /// The item of the one serie an `unnest` projection's arguments read.
    fn unnested_item(&self, arguments: &[Term], schema: &Field) -> Result<Field> {
        let [argument] = arguments else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", self.name()),
                reason: format_smolstr!(
                    "expected unnest to take one serie, got {} arguments in `{}`",
                    arguments.len(),
                    self.term
                ),
            });
        };
        let serie = argument.field(schema)?;
        super::typing::unwrap_dictionary(serie.dtype())
            .serie_item()
            .cloned()
            .ok_or_else(|| Error::InvalidRecord {
                path: format_smolstr!("$.{}", self.name()),
                reason: format_smolstr!(
                    "expected a serie to unnest, got {} in `{}`",
                    serie.dtype(),
                    self.term
                ),
            })
    }

    /// The same projection over a simplified term.
    ///
    /// An alias that only restates the column it names is dropped, so
    /// `ccy as ccy` and `ccy` are one projection.
    #[must_use]
    pub fn simplify(&self) -> Self {
        let term = self.term.simplify();
        let alias = self
            .alias
            .clone()
            .filter(|alias| term.as_column() != Some(alias.as_str()));
        Self {
            term,
            alias,
            dtype: self.dtype.clone(),
            nullable: self.nullable,
            metadata: self.metadata.clone(),
        }
    }
}

impl From<Term> for Projection {
    fn from(term: Term) -> Self {
        Self::new(term)
    }
}

impl From<FieldPath> for Projection {
    fn from(path: FieldPath) -> Self {
        let alias = path.alias().map(SmolStr::new);
        Self {
            term: Term::Path(path.shared_segments()),
            alias,
            dtype: None,
            nullable: None,
            metadata: Metadata::new(),
        }
    }
}

impl FromStr for Projection {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        super::parser::parse_projection(input)
    }
}

/// The `select` block: an optional `*` - every column, less the ones it
/// excludes - then the projections appended after it, in order.
///
/// ```
/// use yggdryl::expression::Selector;
/// use yggdryl::Field;
///
/// # fn main() -> yggdryl::Result<()> {
/// let schema: Field = "trades:struct<ccy:utf8,price:decimal(9,2),size:bigint>".parse()?;
/// let selector: Selector = "ccy, price * size as notional".parse()?;
/// let projected = selector.apply_field(&schema)?;
/// assert_eq!(projected.fields()[0].name(), "ccy");
/// assert_eq!(projected.fields()[1].name(), "notional");
/// assert_eq!(selector.to_string(), "ccy, price * size as notional");
/// assert_eq!(Selector::all().to_string(), "*");
///
/// // A `*` keeps every column it does not exclude, the appended ones last.
/// let starred: Selector = "* exclude (price), price * size as notional".parse()?;
/// let names: Vec<String> = starred
///     .apply_field(&schema)?
///     .fields()
///     .iter()
///     .map(|field| field.name().to_owned())
///     .collect();
/// assert_eq!(names, ["ccy", "size", "notional"]);
/// assert_eq!(starred.to_string(), "* exclude (price), price * size as notional");
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
#[serde(try_from = "SelectorWire", into = "SelectorWire")]
pub struct Selector {
    /// Whether the selector opens with `*`. A selector with no `*` holds at
    /// least one projection: the empty list is `*`, and nothing else spells
    /// it.
    star: bool,
    /// The columns `*` leaves out; empty without a `*`.
    exclude: Arc<[SmolStr]>,
    /// The projections, after the `*` when there is one.
    projections: Arc<[Projection]>,
}

/// The structural document of a [`Selector`]: its three parts, each written
/// only when it says something.
#[derive(::serde::Serialize, ::serde::Deserialize)]
struct SelectorWire {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    star: bool,
    #[serde(default, skip_serializing_if = "<[SmolStr]>::is_empty")]
    exclude: Arc<[SmolStr]>,
    #[serde(default, skip_serializing_if = "<[Projection]>::is_empty")]
    projections: Arc<[Projection]>,
}

impl From<Selector> for SelectorWire {
    fn from(selector: Selector) -> Self {
        Self {
            star: selector.star,
            exclude: selector.exclude,
            projections: selector.projections,
        }
    }
}

impl TryFrom<SelectorWire> for Selector {
    type Error = Error;

    fn try_from(wire: SelectorWire) -> Result<Self> {
        if !wire.star && !wire.exclude.is_empty() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.exclude"),
                reason: SmolStr::new_static(
                    "expected `exclude` only beside `star`, which it narrows",
                ),
            });
        }
        Ok(Self::from_parts(wire.star, wire.exclude, wire.projections))
    }
}

impl Default for Selector {
    fn default() -> Self {
        Self::from_parts(true, Arc::from([]), Arc::from([]))
    }
}

impl Selector {
    /// The one assembly of the three parts: a list with no `*` and nothing
    /// in it is `*`.
    fn from_parts(star: bool, exclude: Arc<[SmolStr]>, projections: Arc<[Projection]>) -> Self {
        let star = star || projections.is_empty();
        Self {
            star,
            exclude,
            projections,
        }
    }

    /// Select every column unchanged: `select *`.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Select the given projections, in order; none at all is `*`.
    pub fn new(projections: impl IntoIterator<Item = Projection>) -> Self {
        Self::from_parts(false, Arc::from([]), projections.into_iter().collect())
    }

    /// Select every column but the named ones: `select * exclude (a, b)`.
    ///
    /// The columns are named exactly and matched ASCII case-insensitively
    /// against the schema the selector is applied to; a name the schema does
    /// not hold excludes nothing. Projections appended with
    /// [`Self::with_projection`] follow the columns the `*` keeps.
    pub fn all_except<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        Self::from_parts(
            true,
            names.into_iter().map(Into::into).collect(),
            Arc::from([]),
        )
    }

    /// The selector a parsed `*`, its exclusions and the projections after
    /// it spell.
    pub(crate) fn starred(exclude: Vec<SmolStr>, projections: Vec<Projection>) -> Self {
        Self::from_parts(true, Arc::from(exclude), Arc::from(projections))
    }

    /// The columns `*` leaves out; empty unless the selector excludes.
    #[must_use]
    pub fn excluded(&self) -> &[SmolStr] {
        &self.exclude
    }

    /// Return whether this selector opens with `*`, so it publishes every
    /// stored column it does not exclude - whatever it appends after.
    ///
    /// This is the question a projection pushdown asks: a `*` reads every
    /// column, since the ones it keeps are named by the schema and not by the
    /// selector.
    #[must_use]
    pub const fn has_star(&self) -> bool {
        self.star
    }

    /// The explicit projections this selector amounts to over `schema`: a
    /// `*` becomes every column, minus what it excludes, and the appended
    /// projections follow.
    fn expanded(&self, schema: &Field) -> Vec<Projection> {
        if !self.star {
            return self.projections.to_vec();
        }
        schema
            .fields()
            .iter()
            .filter(|field| {
                !self
                    .exclude
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(field.name()))
            })
            .map(|field| Projection::column(field.name()))
            .chain(self.projections.iter().cloned())
            .collect()
    }

    /// Read a selector from the scalar that spells one.
    ///
    /// Text parses as the clause; a sequence is one projection per item,
    /// each the text of one; a mapping or record is `term as name` per
    /// entry; null is `*`. This is the one reading every binding's inputs
    /// cross through, so a list of names, a dict of aliases and the text of
    /// a clause mean the same thing in every language.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a selector, and an error
    /// naming the scalar for any other shape.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.parse();
        }
        if value.is_null() {
            return Ok(Self::all());
        }
        if let Some(items) = value.as_serie() {
            let mut projections = Vec::with_capacity(items.len());
            for item in items.iter() {
                projections.push(Projection::from_scalar(&item)?);
            }
            return Ok(Self::new(projections));
        }
        if let Some(entries) = value.as_mapping() {
            let mut projections = Vec::with_capacity(entries.len());
            for (name, term) in entries {
                let Some(alias) = name.as_str() else {
                    return Err(selector_shape_error(value));
                };
                projections.push(Projection::aliased(Term::from_scalar(term)?, alias));
            }
            return Ok(Self::new(projections));
        }
        if let Some(entries) = value.as_struct() {
            let mut projections = Vec::with_capacity(entries.len());
            for (alias, term) in entries {
                projections.push(Projection::aliased(Term::from_scalar(term)?, alias.clone()));
            }
            return Ok(Self::new(projections));
        }
        Err(selector_shape_error(value))
    }

    /// Select the named columns unchanged, in order.
    ///
    /// Each name is one column, spelled exactly: a dotted name is a column
    /// carrying a dot, not a path, because a list of names is a list of names.
    pub fn from_columns<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::new(
            names
                .into_iter()
                .map(|name| Projection::column(name.as_ref())),
        )
    }

    /// Return a deterministic hash of the canonical selector text.
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_display(self)
    }

    /// The projections, in output order after the `*` when there is one.
    /// Empty means every column.
    #[must_use]
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }

    /// Return whether this selector publishes every column unchanged: a `*`
    /// that excludes nothing and appends nothing.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.star && self.exclude.is_empty() && self.projections.is_empty()
    }

    /// Return whether this selector names nothing - the same empty list read
    /// as a match key rather than as a projection.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.projections.is_empty()
    }

    /// How many projections this selector names; zero for `*`, and the
    /// appended ones for a `*` that appends.
    #[must_use]
    pub fn len(&self) -> usize {
        self.projections.len()
    }

    /// The names this selector's projections publish, in order: the ones a
    /// `*` keeps are the schema's to name.
    #[must_use]
    pub fn names(&self) -> Vec<SmolStr> {
        self.projections.iter().map(Projection::name).collect()
    }

    /// Return whether a column this selector publishes answers to `name`
    /// whatever the schema: a projection's name, or a column an `unnest`
    /// publishes - its name, or `<name>.<child>` for a struct item.
    pub(crate) fn publishes(&self, name: &str) -> bool {
        self.projections.iter().any(|projection| {
            let published = projection.name();
            if published.eq_ignore_ascii_case(name) {
                return true;
            }
            let prefix = published.len();
            projection.term.as_unnest().is_some()
                && name.len() > prefix + 1
                && name.as_bytes()[prefix] == b'.'
                && name
                    .get(..prefix)
                    .is_some_and(|head| head.eq_ignore_ascii_case(&published))
        })
    }

    /// Return whether a projection of this selector is an `unnest`, so it
    /// publishes as many rows as the serie holds elements rather than one
    /// per row.
    pub(crate) fn unnests(&self) -> bool {
        self.projections
            .iter()
            .any(|projection| projection.term.as_unnest().is_some())
    }

    /// Return this selector with one more projection, appended after
    /// everything it already publishes - a `*` and its exclusions included.
    #[must_use]
    pub fn with_projection(&self, projection: Projection) -> Self {
        Self::from_parts(
            self.star,
            Arc::clone(&self.exclude),
            self.projections
                .iter()
                .cloned()
                .chain(std::iter::once(projection))
                .collect(),
        )
    }

    /// Return this selector without the named columns.
    ///
    /// Only a projection publishing one bare column can be named by a column
    /// name; a computed projection stays whatever the list says, and a `*`
    /// keeps what it keeps.
    #[must_use]
    pub fn without_columns(&self, names: &[&str]) -> Self {
        Self::from_parts(
            self.star,
            Arc::clone(&self.exclude),
            self.projections
                .iter()
                .filter(|projection| {
                    !projection.term.as_column().is_some_and(|column| {
                        names.iter().any(|name| name.eq_ignore_ascii_case(column))
                    })
                })
                .cloned()
                .collect(),
        )
    }

    /// Every top-level column this selector's projections read, in
    /// first-seen order.
    ///
    /// The source side of the projection - what an encoding has to decode -
    /// as opposed to [`Self::names`], the published side. A `*` reads every
    /// column besides, which [`Self::has_star`] says.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for projection in self.projections.iter() {
            for column in projection.term.columns() {
                if !names.iter().any(|held| held.eq_ignore_ascii_case(&column)) {
                    names.push(column);
                }
            }
        }
        names
    }

    /// Every parameter this selector names, in first-seen order.
    #[must_use]
    pub fn parameters(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for projection in self.projections.iter() {
            for parameter in projection.term.parameters() {
                if !found.contains(&parameter) {
                    found.push(parameter);
                }
            }
        }
        found
    }

    /// Return whether every projection publishes a column as it is stored.
    #[must_use]
    pub fn is_columns(&self) -> bool {
        self.projections.iter().all(Projection::is_column)
    }

    /// The same selector over simplified terms.
    #[must_use]
    pub fn simplify(&self) -> Self {
        Self::from_parts(
            self.star,
            Arc::clone(&self.exclude),
            self.projections.iter().map(Projection::simplify).collect(),
        )
    }

    /// Refuse a selector holding a term past the depth or node budget.
    ///
    /// # Errors
    ///
    /// [`Term::check_budget`] carries the rule.
    pub fn check_budget(&self) -> Result<()> {
        for projection in self.projections.iter() {
            projection.term.check_budget()?;
        }
        Ok(())
    }

    /// The struct datatype this selector publishes from the struct `dtype`.
    ///
    /// `*` publishes the datatype itself. Otherwise the result holds one
    /// child per projection, typed by [`Projection::field`] against a root of
    /// that datatype - an `unnest` of struct items one per child of the item.
    /// Two columns publishing one name are refused, because a batch cannot
    /// carry them. This is the schema question every other schema
    /// application - [`apply_field`](Self::apply_field), a plan's `create`
    /// section, a reader's declared shape - is answered by.
    ///
    /// # Errors
    ///
    /// Returns an error when `dtype` is not a struct, when a term cannot be
    /// typed against it, when two columns share a name, or when an `unnest`
    /// is not the only one or not of a serie.
    pub fn apply_datatype(&self, dtype: &DataType) -> Result<DataType> {
        if self.is_all() {
            return Ok(dtype.clone());
        }
        let root = Field::new(crate::media::DEFAULT_ROOT_NAME, dtype.clone(), false);
        let (_, children, _) = self.published(&root)?;
        StructType::from_fields(children).map(DataType::from)
    }

    /// Everything this selector publishes from `root`: the projections it
    /// amounts to, the columns they publish, and the `unnest` among them.
    ///
    /// The one place a selector's output is decided, so the schema a
    /// selector answers and the batches its bound form produces agree.
    fn published(&self, root: &Field) -> Result<(Vec<Projection>, Vec<Field>, Option<Unnested>)> {
        root.require_struct()?;
        let projections = self.expanded(root);
        let mut children: Vec<Field> = Vec::with_capacity(projections.len());
        let mut unnested: Option<Unnested> = None;
        for (position, projection) in projections.iter().enumerate() {
            let child = projection.field(root)?;
            if projection.term.as_unnest().is_none() {
                push_published(&mut children, child)?;
                continue;
            }
            if let Some(held) = &unnested {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}", child.name()),
                    reason: format_smolstr!(
                        "expected at most one unnest in a select, got `{}` and `{}`",
                        projections[held.position].term,
                        projection.term
                    ),
                });
            }
            let expand = child.is_struct();
            if expand && !projection.metadata.is_empty() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}", child.name()),
                    reason: format_smolstr!(
                        "expected no `with (...)` on `{}`, which publishes its struct item as \
                         one column per child",
                        projection.term
                    ),
                });
            }
            if expand {
                // One column per child, named under the projection: a child of
                // a null item is null.
                for member in child.fields() {
                    let nullable = child.is_nullable() || member.is_nullable();
                    let name = format_smolstr!("{}.{}", child.name(), member.name());
                    push_published(
                        &mut children,
                        member.clone().with_name(name).with_nullable(nullable),
                    )?;
                }
            } else {
                push_published(&mut children, child.clone())?;
            }
            unnested = Some(Unnested {
                position,
                item: child,
                expand,
            });
        }
        Ok((projections, children, unnested))
    }

    /// The struct root this selector publishes from `root`.
    ///
    /// The root keeps its name, nullability and metadata around the datatype
    /// [`apply_datatype`](Self::apply_datatype) publishes.
    ///
    /// # Errors
    ///
    /// Returns the error [`apply_datatype`](Self::apply_datatype) does.
    pub fn apply_field(&self, root: &Field) -> Result<Field> {
        root.clone()
            .try_with_dtype(self.apply_datatype(root.dtype())?)
    }

    /// The row this selector publishes from one row of `root`.
    ///
    /// The one-row spelling of the batch application, bound here and answered
    /// at once; a caller with many rows [binds](Self::bind) once.
    ///
    /// # Errors
    ///
    /// Returns an error when the selector does not bind against `root`, the
    /// row does not match it, or a computed value does not fit what its
    /// projection declares.
    pub fn apply_scalar(&self, root: &Field, row: &Scalar) -> Result<Scalar> {
        self.bind(root)?.apply_scalar(row)
    }

    /// Resolve this selector against a schema.
    ///
    /// # Errors
    ///
    /// Returns an error when a term cannot be resolved or two projections
    /// share a name.
    pub fn bind(&self, schema: &Field) -> Result<BoundSelector> {
        self.bind_with(schema, &[])
    }

    /// Resolve this selector against a schema, supplying its parameters.
    ///
    /// # Errors
    ///
    /// Returns an error when a parameter is missing or a term cannot resolve.
    pub fn bind_with(
        &self,
        schema: &Field,
        parameters: &[(&str, Scalar)],
    ) -> Result<BoundSelector> {
        let (expanded, children, unnested) = self.published(schema)?;
        let output = if self.is_all() {
            schema.clone()
        } else {
            schema
                .clone()
                .try_with_dtype(StructType::from_fields(children).map(DataType::from)?)?
        };
        let mut projections = Vec::with_capacity(expanded.len());
        for projection in &expanded {
            // An unnest binds the serie it reads; the rows it publishes are
            // the selector's to lay out.
            let term = match projection.term.as_unnest() {
                Some([argument]) => argument,
                _ => &projection.term,
            };
            projections.push(term.bind_with(schema, parameters)?);
        }
        // A selector that republishes every stored column, in order, as it is
        // stored has nothing to do per batch: the plan is the identity and is
        // skipped, whatever aliases or declared types spelled it.
        let identity = output.fields() == schema.fields()
            && expanded.len() == schema.field_len()
            && expanded
                .iter()
                .zip(schema.fields())
                .all(|(projection, stored)| projection.term.as_column() == Some(stored.name()));
        Ok(BoundSelector {
            schema: schema.clone(),
            output,
            casts: projections.iter().map(|_| ColumnCast::default()).collect(),
            projections,
            unnested,
            identity,
        })
    }

    /// Read the selector a struct field holds: the `create table` reading of
    /// a schema.
    ///
    /// Every child becomes one column declaration - its datatype, its
    /// nullability and its metadata, protocol views included - published
    /// under the child's name. A child carrying an explicit
    /// `TRANSFORM:expression` is read as the term that computes it, aliased
    /// to the child's name; a declaration that does not parse is kept as the
    /// metadata it is and refused where it is read. A root that is not a
    /// struct is one column named after it. Nothing is lost:
    /// [`Self::declared_field`] gives the field back.
    #[must_use]
    pub fn from_field(field: &Field) -> Self {
        if !field.is_struct() {
            return Self::new([Self::column_declaration(field)]);
        }
        Self::new(field.fields().iter().map(Self::column_declaration))
    }

    /// One child as the column declaration that recreates it.
    fn column_declaration(child: &Field) -> Projection {
        // A partition column's own declaration is not a selector's to
        // restate: only the transform protocol's two spellings are read.
        let transform = child.as_transform();
        let declared = (transform.is_derived() && !child.as_partition().is_derived())
            .then(|| transform.term().ok().flatten())
            .flatten();
        let (term, alias, metadata) = match declared {
            Some(term) if term.as_column() != Some(child.name()) => {
                let mut metadata = child.as_metadata().clone();
                for key in super::TRANSFORM_KEYS {
                    metadata.remove(key);
                }
                (term, Some(SmolStr::new(child.name())), metadata)
            }
            _ => (
                Term::column(child.name()),
                None,
                child.as_metadata().clone(),
            ),
        };
        Projection {
            term,
            alias,
            dtype: Some(child.dtype().clone()),
            nullable: Some(child.is_nullable()),
            metadata,
        }
    }

    /// The struct root this selector declares, named `name`.
    ///
    /// A projection that is a bare column carrying a datatype is a column
    /// declaration and needs no rows: the column exists as declared, nullable
    /// unless said otherwise. Every other projection is typed against `root`
    /// as [`Projection::field`] types it, and one that computes something
    /// other than its own column keeps the term as its
    /// [`TRANSFORM:`](crate::TransformField) declaration, so the field it
    /// declares knows how to derive the column and [`Self::from_field`]
    /// reads the selector back. When `root` holds a column a declaration
    /// names, the declaration restates that column: the same column, cast to
    /// what is declared.
    ///
    /// # Errors
    ///
    /// Returns an error when a projection computes a value and there is no
    /// root to type it against, when a projection cannot be typed against
    /// the root, or when two columns share a name.
    pub fn declared_field(&self, root: Option<&Field>, name: &str) -> Result<Field> {
        // With no root to expand it against, a `*` declares nothing and the
        // projections after it declare themselves.
        let projections = match root {
            Some(root) => self.expanded(root),
            None => self.projections.to_vec(),
        };
        let mut children: Vec<Field> = Vec::with_capacity(projections.len());
        for projection in &projections {
            // A declared column is one column of one row; the rows an unnest
            // publishes are a select's alone.
            if projection.term.as_unnest().is_some() {
                return Err(super::typing::unnest_misplaced(
                    &projection.term,
                    "in a column declaration",
                ));
            }
            let declared = projection.term.as_column().and_then(|column| {
                let stored = root.is_some_and(|root| root.index_of(column).is_some());
                (!stored).then_some(column).zip(projection.dtype.as_ref())
            });
            let mut child = match (declared, root) {
                (Some((_, dtype)), _) => Field::new(
                    projection.name(),
                    dtype.clone(),
                    projection.nullable.unwrap_or(true),
                ),
                (None, Some(root)) => {
                    let mut child = projection.field(root)?;
                    if projection.term.as_column() != Some(child.name()) {
                        child.as_transform_mut().set_term(&projection.term)?;
                    }
                    child
                }
                (None, None) => {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("$.{}", projection.name()),
                        reason: format_smolstr!(
                            "expected a declared datatype for a column declared without rows \
                             to type it against, got `{}`",
                            projection.term
                        ),
                    });
                }
            };
            if !projection.metadata.is_empty() {
                child = child.try_with_metadata_entries(projection.metadata.iter())?;
            }
            if children
                .iter()
                .any(|held| held.name().eq_ignore_ascii_case(child.name()))
            {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}", child.name()),
                    reason: format_smolstr!(
                        "expected every column to publish its own name, got {:?} twice",
                        child.name()
                    ),
                });
            }
            children.push(child);
        }
        Ok(DataType::from(StructType::from_fields(children)?).required_field(name))
    }

    /// Write this selector into the struct root it publishes from `root`.
    ///
    /// The result is [`Self::apply_field`] with the plan kept, exactly as
    /// [`Self::declared_field`] keeps it, under the root's own name,
    /// nullability and metadata.
    ///
    /// # Errors
    ///
    /// [`Self::declared_field`] carries the rule.
    pub fn into_field(&self, root: &Field) -> Result<Field> {
        if self.is_all() {
            return Ok(root.clone());
        }
        let declared = self.declared_field(Some(root), root.name())?;
        root.clone().try_with_dtype(declared.dtype().clone())
    }
}

impl From<Projection> for Selector {
    fn from(projection: Projection) -> Self {
        Self::new([projection])
    }
}

impl From<FieldPath> for Selector {
    fn from(path: FieldPath) -> Self {
        Self::new([Projection::from(path)])
    }
}

impl FromIterator<Projection> for Selector {
    fn from_iter<I: IntoIterator<Item = Projection>>(projections: I) -> Self {
        Self::new(projections)
    }
}

impl FromStr for Selector {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        super::parser::parse_selector(input)
    }
}

impl TryFrom<&str> for Selector {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        value.parse()
    }
}

/// Anything a call site may hand over where a selector is wanted.
///
/// Text parses through the selector grammar; a list of names is a list of
/// columns, each spelled exactly; a term or a path is one projection.
pub trait IntoSelector {
    /// Produce the selector this value stands for.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a selector, or
    /// an error naming the clause when it is an expression of another kind.
    fn into_selector(self) -> Result<Selector>;
}

impl IntoSelector for Selector {
    fn into_selector(self) -> Result<Self> {
        Ok(self)
    }
}

impl IntoSelector for &Selector {
    fn into_selector(self) -> Result<Selector> {
        Ok(self.clone())
    }
}

impl IntoSelector for Projection {
    fn into_selector(self) -> Result<Selector> {
        Ok(Selector::from(self))
    }
}

impl IntoSelector for FieldPath {
    fn into_selector(self) -> Result<Selector> {
        Ok(Selector::from(self))
    }
}

impl IntoSelector for Term {
    fn into_selector(self) -> Result<Selector> {
        Ok(Selector::from(Projection::new(self)))
    }
}

impl IntoSelector for &str {
    fn into_selector(self) -> Result<Selector> {
        self.parse()
    }
}

impl IntoSelector for &String {
    fn into_selector(self) -> Result<Selector> {
        self.parse()
    }
}

impl IntoSelector for String {
    fn into_selector(self) -> Result<Selector> {
        self.parse()
    }
}

impl<S: AsRef<str>, const N: usize> IntoSelector for [S; N] {
    fn into_selector(self) -> Result<Selector> {
        Ok(Selector::from_columns(self))
    }
}

impl<S: AsRef<str>> IntoSelector for Vec<S> {
    fn into_selector(self) -> Result<Selector> {
        Ok(Selector::from_columns(self))
    }
}

impl<S: AsRef<str>> IntoSelector for &[S] {
    fn into_selector(self) -> Result<Selector> {
        Ok(Selector::from_columns(self))
    }
}

impl IntoSelector for super::Expression {
    fn into_selector(self) -> Result<Selector> {
        match self {
            super::Expression::Selector(selector) => Ok(selector),
            other => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!("expected a `select` clause, got `{other}`"),
            }),
        }
    }
}

impl IntoSelector for &super::Expression {
    fn into_selector(self) -> Result<Selector> {
        self.clone().into_selector()
    }
}

/// Add one published column, refusing a second of the same name.
fn push_published(children: &mut Vec<Field>, child: Field) -> Result<()> {
    if children
        .iter()
        .any(|held| held.name().eq_ignore_ascii_case(child.name()))
    {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("$.{}", child.name()),
            reason: format_smolstr!(
                "expected every projection to publish its own name, got {:?} twice",
                child.name()
            ),
        });
    }
    children.push(child);
    Ok(())
}

/// The one `unnest` a selector holds: which projection it is, and the item
/// it publishes.
#[derive(Clone, Debug)]
pub(crate) struct Unnested {
    /// Its place among the projections.
    pub(crate) position: usize,
    /// The item, named and declared as the projection says.
    pub(crate) item: Field,
    /// Whether the item is a struct published as one column per child.
    pub(crate) expand: bool,
}

impl Unnested {
    /// How many columns the item publishes.
    fn width(&self) -> usize {
        if self.expand {
            self.item.field_len()
        } else {
            1
        }
    }
}

/// A [`Selector`] resolved against one schema.
///
/// Built once per stream, so a projection over a thousand batches types its
/// terms once and evaluates them a thousand times.
#[derive(Clone, Debug)]
pub struct BoundSelector {
    schema: Field,
    output: Field,
    /// One bound term per projection; an `unnest` binds the serie it reads.
    projections: Vec<Bound>,
    /// One held cast per projection, into the column it declares - an
    /// `unnest`'s into its item.
    casts: Arc<[ColumnCast]>,
    /// The projection that multiplies rows, when one does.
    unnested: Option<Unnested>,
    identity: bool,
}

impl BoundSelector {
    /// The struct root this selector was bound against.
    #[must_use]
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// The struct root this selector publishes.
    #[must_use]
    pub const fn output(&self) -> &Field {
        &self.output
    }

    /// The bound terms, one per projection in output order - an `unnest`'s
    /// the serie it reads. Empty means every column.
    #[must_use]
    pub fn projections(&self) -> &[Bound] {
        &self.projections
    }

    /// The projection that multiplies rows, when one does.
    pub(crate) const fn unnested(&self) -> Option<&Unnested> {
        self.unnested.as_ref()
    }

    /// Return whether this selector publishes every column unchanged.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.projections.is_empty()
    }

    /// Return whether applying this selector changes nothing.
    ///
    /// `*` is one identity; a selector that names every stored column in its
    /// stored order, type and nullability is the other, and both are skipped
    /// rather than evaluated.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.projections.is_empty() || self.identity
    }

    /// The row this selector publishes from one row of its schema.
    ///
    /// Each term is evaluated, converted into what its projection declares,
    /// and checked against the nullability the output field carries.
    ///
    /// # Errors
    ///
    /// Returns an error when the row does not match the schema, a term fails,
    /// or a computed value does not fit its declared column.
    pub fn apply_scalar(&self, row: &Scalar) -> Result<Scalar> {
        if self.is_identity() {
            return Ok(row.clone());
        }
        if let Some(unnested) = &self.unnested {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", unnested.item.name()),
                reason: format_smolstr!(
                    "expected a select that publishes one row per row, got an unnest of {:?}, \
                     which publishes one per element; apply it to records, a batch or a stream",
                    unnested.item.name()
                ),
            });
        }
        let mut values = Vec::with_capacity(self.projections.len());
        for (bound, field) in self.projections.iter().zip(self.output.fields()) {
            let safety = if field.is_nullable() {
                Safety::Safe
            } else {
                Safety::Strict
            };
            let value = convert(field.dtype(), &bound.eval(row)?, safety)?;
            require_present(field, value.is_null())?;
            values.push(value);
        }
        Ok(Scalar::from_sequence(values))
    }
}

/// Refuse a null in a column the selector publishes as required.
pub(crate) fn require_present(field: &Field, null: bool) -> Result<()> {
    if null && !field.is_nullable() {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("$.{}", field.name()),
            reason: format_smolstr!(
                "expected a value for the required column {:?}, got null",
                field.name()
            ),
        });
    }
    Ok(())
}

mod arrow {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, RecordBatch, RecordBatchReader, StructArray};
    use arrow_schema::{ArrowError, SchemaRef};

    use super::{BoundSelector, ColumnCast, Selector};
    use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
    use crate::cast::ArrowCastOptions;
    use crate::expression::arrow::{collected, one_batch, struct_batch, struct_children, unnest};
    use crate::{Error, Field, Result};

    impl Selector {
        /// Wrap a reader so every batch it yields is what this selector
        /// publishes.
        ///
        /// This is the streamed entry: the selector is bound once against the
        /// reader's schema, the returned reader answers its output schema
        /// before the first batch, and each batch is projected as it arrives.
        /// A projection that is a bare column reuses the batch's own
        /// `ArrayRef`, so selecting and reordering columns copies no buffer;
        /// a computed projection is evaluated through the vectorized tier and
        /// cast strictly into what it declares.
        ///
        /// # Errors
        ///
        /// Returns an error when the selector cannot be resolved against the
        /// reader's schema.
        pub fn apply_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            if self.is_all() {
                return Ok(reader);
            }
            let schema =
                field_from_arrow_schema(crate::media::DEFAULT_ROOT_NAME, &reader.schema())?;
            self.bind(&schema)?.apply_arrow_reader(reader)
        }

        /// The batch this selector publishes from one batch.
        ///
        /// One batch is the one-batch stream: it goes through
        /// [`Self::apply_arrow_reader`] and comes back as the batch that
        /// stream yields. A caller with many batches hands the stream over
        /// instead, or [binds](Self::bind) once.
        ///
        /// # Errors
        ///
        /// Returns an error when a term cannot be resolved against the batch,
        /// or a computed value does not fit its declared column.
        pub fn apply_arrow_batch(&self, batch: &RecordBatch) -> Result<RecordBatch> {
            collected(self.apply_arrow_reader(one_batch(batch))?)
        }

        /// The struct array this selector publishes from one struct array.
        ///
        /// The array's own null mask is kept: a struct that was null stays
        /// null whatever its projections compute.
        ///
        /// # Errors
        ///
        /// Returns an error when the array is not a struct, or the batch
        /// application fails.
        pub fn apply_arrow_array(&self, array: &ArrayRef) -> Result<ArrayRef> {
            if self.is_all() {
                return Ok(Arc::clone(array));
            }
            let (held, batch) = struct_batch(array, "select from")?;
            let projected = self.apply_arrow_batch(&batch)?;
            rebuilt_struct(held, &projected)
        }
    }

    impl BoundSelector {
        /// The batch this selector publishes from one batch of its schema.
        ///
        /// This is the per-batch kernel the streamed reader runs; the plan
        /// was settled at bind and nothing is resolved here.
        ///
        /// # Errors
        ///
        /// Returns an error when the batch is missing a column, a term fails,
        /// or a computed value does not fit its declared column.
        pub fn apply_arrow_batch(&self, batch: &RecordBatch) -> Result<RecordBatch> {
            if self.is_identity() {
                return Ok(batch.clone());
            }
            self.projected(batch, None)
        }

        /// The batch this selector publishes from one batch, under the
        /// output's schema when the caller already holds it.
        fn projected(
            &self,
            batch: &RecordBatch,
            schema: Option<&SchemaRef>,
        ) -> Result<RecordBatch> {
            // An unnest lays its serie's elements out first: the parent row
            // of every element is the take every other column is read by.
            let unnested = match &self.unnested {
                Some(unnested) => {
                    let serie = &self.projections[unnested.position];
                    let (parents, items) = unnest(serie.field(), &serie.evaluate(batch)?)?;
                    Some((unnested, parents, items))
                }
                None => None,
            };
            let rows = unnested
                .as_ref()
                .map_or(batch.num_rows(), |(_, parents, _)| parents.len());
            let mut columns = Vec::with_capacity(self.output.field_len());
            let mut fields = self.output.fields().iter();
            for (position, (bound, cast)) in
                self.projections.iter().zip(self.casts.iter()).enumerate()
            {
                if let Some((unnested, _, items)) = unnested
                    .as_ref()
                    .filter(|(unnested, ..)| unnested.position == position)
                {
                    let items = declared(&unnested.item, cast, Arc::clone(items))?;
                    let published = if unnested.expand {
                        struct_children(&items)?
                    } else {
                        vec![items]
                    };
                    for (field, array) in fields.by_ref().take(unnested.width()).zip(published) {
                        super::require_present(field, array.null_count() > 0)?;
                        columns.push(array);
                    }
                    continue;
                }
                let Some(field) = fields.next() else {
                    break;
                };
                let mut array = declared(field, cast, bound.evaluate(batch)?)?;
                if let Some((_, parents, _)) = &unnested {
                    array = arrow_select::take::take(array.as_ref(), parents, None)
                        .map_err(|error| Error::from(crate::arrow::Error::Arrow(error)))?;
                }
                super::require_present(field, array.null_count() > 0)?;
                columns.push(array);
            }
            let schema = match schema {
                Some(schema) => Arc::clone(schema),
                None => arrow_schema_from_field(&self.output)?,
            };
            let options = arrow_array::RecordBatchOptions::new().with_row_count(Some(rows));
            RecordBatch::try_new_with_options(schema, columns, &options)
                .map_err(|error| Error::from(crate::arrow::Error::Arrow(error)))
        }

        /// The struct array this selector publishes from one struct array.
        ///
        /// # Errors
        ///
        /// [`Selector::apply_arrow_array`] carries the rule.
        pub fn apply_arrow_array(&self, array: &ArrayRef) -> Result<ArrayRef> {
            if self.is_identity() {
                return Ok(Arc::clone(array));
            }
            let (held, batch) = struct_batch(array, "select from")?;
            let projected = self.apply_arrow_batch(&batch)?;
            rebuilt_struct(held, &projected)
        }

        /// Wrap a reader so every batch it yields is what this selector
        /// publishes.
        ///
        /// # Errors
        ///
        /// Returns an error when the output schema cannot be materialized.
        pub fn apply_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            if self.is_identity() {
                return Ok(reader);
            }
            let schema = arrow_schema_from_field(&self.output)?;
            Ok(Box::new(Projected {
                inner: reader,
                selector: self.clone(),
                schema,
            }))
        }
    }

    /// One evaluated column as the column its projection declares.
    fn declared(field: &Field, cast: &ColumnCast, evaluated: ArrayRef) -> Result<ArrayRef> {
        if evaluated.data_type() == field.as_arrow_field_ref()?.data_type() {
            return Ok(evaluated);
        }
        // The projection declared a datatype the term does not produce, so it
        // casts by the declared-column rule: a value a nullable column cannot
        // hold becomes null, and a `not null` column refuses that value or a
        // null by name.
        cast.reconcile(field, None, evaluated, ArrowCastOptions::declared(true))
            .map_err(|error| Error::InvalidRecord {
                path: smol_str::format_smolstr!("$.{}", field.name()),
                reason: smol_str::format_smolstr!(
                    "expected every value to fit the required column {:?}, got {error}",
                    field.name()
                ),
            })
    }

    /// One struct's projected columns under the struct's own null mask.
    fn rebuilt_struct(held: &StructArray, projected: &RecordBatch) -> Result<ArrayRef> {
        if projected.num_rows() != held.len() {
            // Only an unnest changes how many rows there are, and a struct
            // array has one mask for the rows it held.
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!(
                    "expected one row out per struct in, got {} rows from {}: an unnest \
                     publishes one row per element, so apply it to a batch or a stream",
                    projected.num_rows(),
                    held.len()
                ),
            });
        }
        let rebuilt = StructArray::try_new_with_length(
            projected.schema().fields().clone(),
            projected.columns().to_vec(),
            held.nulls().cloned(),
            held.len(),
        )
        .map_err(|error| Error::from(crate::arrow::Error::Arrow(error)))?;
        Ok(Arc::new(rebuilt))
    }

    /// One reader's batches, each projected by one bound selector.
    struct Projected {
        inner: BatchReader,
        selector: BoundSelector,
        schema: SchemaRef,
    }

    impl Iterator for Projected {
        type Item = std::result::Result<RecordBatch, ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            let batch = match self.inner.next()? {
                Ok(batch) => batch,
                Err(error) => return Some(Err(error)),
            };
            Some(
                self.selector
                    .projected(&batch, Some(&self.schema))
                    .map_err(|error| ArrowError::ExternalError(Box::new(error))),
            )
        }
    }

    impl RecordBatchReader for Projected {
        fn schema(&self) -> SchemaRef {
            Arc::clone(&self.schema)
        }
    }
}

impl Projection {
    /// Read one projection from the scalar that spells it: text, or a
    /// `(term, alias)` pair spelled as a two-item sequence.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a projection, and an error
    /// naming the scalar for any other shape.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.parse();
        }
        if let Some([term, alias]) = value.sequence_rows().as_deref() {
            if let Some(alias) = alias.as_str() {
                return Ok(Self::aliased(Term::from_scalar(term)?, alias));
            }
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "the text of a projection or a (term, alias) pair",
                format_args!("{value:?}"),
            ),
        })
    }
}

/// The refusal for a scalar shape no selector reads.
fn selector_shape_error(value: &Scalar) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: crate::text::expected_got(
            "the text of a select clause, a sequence of projections, a mapping of aliases to terms, or null",
            format_args!("{value:?}"),
        ),
    }
}

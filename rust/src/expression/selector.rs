//! The `select` block: which columns a read or write publishes, computed how,
//! typed as what.
//!
//! A [`Selector`] is a list of [`Projection`]s, and a projection is one output
//! column: a [`Term`] that computes it, the name it is published under, and -
//! the way a `create table` column definition says it - the datatype and
//! nullability it is published as. `select *` is the empty list, which
//! publishes the rows unchanged. One selector therefore spells everything from
//! `id, price` through `cast(price as float64) * size as notional` to the
//! column list a table is created with, `id int64 not null, price decimal(9,2)`.
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
//! or not. With the [`transform:`](super::TransformField) protocol it also
//! says how a column is computed, so [`Selector::into_field`] writes a
//! selector into a field and [`Selector::from_field`] reads it back - which is
//! what lets a declared field on record options carry a whole projection, and
//! a derived partition column be the projection it always was.

use std::str::FromStr;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::Safety;
use super::attribute::Attribute;
use super::bind::Bound;
use super::eval::convert;
use super::path::FieldPath;
use super::term::Term;
use crate::{DataType, Error, Field, Metadata, Result, Scalar};

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
    /// reaches; anything else is named by its canonical text, so a selector
    /// never produces two columns that are impossible to tell apart.
    #[must_use]
    pub fn name(&self) -> SmolStr {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        match &self.term {
            Term::Path(steps) => FieldPath::from_shared(Arc::clone(steps), None)
                .column_name()
                .map_or_else(|| SmolStr::new(self.term.to_string()), SmolStr::new),
            other => SmolStr::new(other.to_string()),
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
    /// restated as whatever the projection declares.
    ///
    /// # Errors
    ///
    /// Returns an error when the term cannot be typed against the schema.
    pub fn field(&self, schema: &Field) -> Result<Field> {
        let mut field = self.term.field(schema)?.with_name(self.name());
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

/// The `select` block: the columns published, in order. Empty is `*`.
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
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone,
    Debug,
    Default,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
pub struct Selector {
    #[serde(default, skip_serializing_if = "<[Projection]>::is_empty")]
    projections: Arc<[Projection]>,
    #[serde(default, skip_serializing_if = "<[SmolStr]>::is_empty")]
    exclude: Arc<[SmolStr]>,
}

impl Selector {
    /// Select every column unchanged: `select *`.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Select the given projections, in order.
    pub fn new(projections: impl IntoIterator<Item = Projection>) -> Self {
        Self {
            projections: projections.into_iter().collect(),
            exclude: Arc::from([]),
        }
    }

    /// Select every column but the named ones: `select * exclude (a, b)`.
    ///
    /// The columns are named exactly and matched ASCII case-insensitively
    /// against the schema the selector is applied to; a name the schema does
    /// not hold excludes nothing.
    pub fn all_except<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        Self {
            projections: Arc::from([]),
            exclude: names.into_iter().map(Into::into).collect(),
        }
    }

    /// The columns `*` leaves out; empty unless the selector excludes.
    #[must_use]
    pub fn excluded(&self) -> &[SmolStr] {
        &self.exclude
    }

    /// The explicit projections this selector amounts to over `schema`: a
    /// `*` becomes every column, minus what it excludes.
    fn expanded(&self, schema: &Field) -> Vec<Projection> {
        if !self.projections.is_empty() {
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
        if let Some(items) = value.as_sequence() {
            let mut projections = Vec::with_capacity(items.len());
            for item in items {
                projections.push(Projection::from_scalar(item)?);
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
        if let Some(entries) = value.as_record() {
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

    /// The projections, in output order. Empty means every column.
    #[must_use]
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }

    /// Return whether this selector publishes every column unchanged.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.projections.is_empty() && self.exclude.is_empty()
    }

    /// Return whether this selector names nothing - the same empty list read
    /// as a match key rather than as a projection.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.projections.is_empty()
    }

    /// How many columns this selector publishes; zero for `*`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.projections.len()
    }

    /// The names this selector publishes, in order.
    #[must_use]
    pub fn names(&self) -> Vec<SmolStr> {
        self.projections.iter().map(Projection::name).collect()
    }

    /// Return this selector with one more projection.
    #[must_use]
    pub fn with_projection(&self, projection: Projection) -> Self {
        Self::new(
            self.projections
                .iter()
                .cloned()
                .chain(std::iter::once(projection)),
        )
    }

    /// Return this selector without the named columns.
    ///
    /// Only a projection publishing one bare column can be named by a column
    /// name; a computed projection stays whatever the list says.
    #[must_use]
    pub fn without_columns(&self, names: &[&str]) -> Self {
        Self::new(
            self.projections
                .iter()
                .filter(|projection| {
                    !projection.term.as_column().is_some_and(|column| {
                        names.iter().any(|name| name.eq_ignore_ascii_case(column))
                    })
                })
                .cloned(),
        )
    }

    /// Every top-level column this selector reads, in first-seen order.
    ///
    /// The source side of the projection - what an encoding has to decode -
    /// as opposed to [`Self::names`], the published side.
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

    /// Every handle attribute this selector reads, in first-seen order.
    #[must_use]
    pub fn attributes(&self) -> Vec<Attribute> {
        let mut found: Vec<Attribute> = Vec::new();
        for projection in self.projections.iter() {
            for attribute in projection.term.attributes() {
                if !found.contains(&attribute) {
                    found.push(attribute);
                }
            }
        }
        found
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
        Self {
            projections: self.projections.iter().map(Projection::simplify).collect(),
            exclude: Arc::clone(&self.exclude),
        }
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
    /// that datatype. Two projections publishing one name are refused,
    /// because a batch cannot carry them. This is the schema question every
    /// other schema application - [`apply_field`](Self::apply_field), a
    /// plan's `create` section, a reader's declared shape - is answered by.
    ///
    /// # Errors
    ///
    /// Returns an error when `dtype` is not a struct, when a term cannot be
    /// typed against it, or when two projections share a name.
    pub fn apply_datatype(&self, dtype: &DataType) -> Result<DataType> {
        if self.is_all() {
            return Ok(dtype.clone());
        }
        let root = Field::new(crate::media::DEFAULT_ROOT_NAME, dtype.clone(), false);
        root.require_struct()?;
        let projections = self.expanded(&root);
        let mut children = Vec::with_capacity(projections.len());
        for projection in &projections {
            let child = projection.field(&root)?;
            if children
                .iter()
                .any(|held: &Field| held.name().eq_ignore_ascii_case(child.name()))
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
        }
        DataType::from_fields(children)
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
        schema.require_struct()?;
        let output = self.apply_field(schema)?;
        let expanded = self.expanded(schema);
        let mut projections = Vec::with_capacity(expanded.len());
        for projection in &expanded {
            projections.push(projection.term.bind_with(schema, parameters)?);
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
            projections,
            identity,
        })
    }

    /// Read the selector a struct field holds: the `create table` reading of
    /// a schema.
    ///
    /// Every child becomes one column declaration - its datatype, its
    /// nullability and its metadata, protocol views included - published
    /// under the child's name. A child carrying an explicit
    /// `transform:expression` is read as the term that computes it, aliased
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
    /// [`transform:`](crate::TransformField) declaration, so the field it
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
        let projections = match root {
            Some(root) if self.projections.is_empty() => self.expanded(root),
            _ => self.projections.to_vec(),
        };
        let mut children: Vec<Field> = Vec::with_capacity(projections.len());
        for projection in &projections {
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
        Ok(DataType::from_fields(children)?.required_field(name))
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

/// A [`Selector`] resolved against one schema.
///
/// Built once per stream, so a projection over a thousand batches types its
/// terms once and evaluates them a thousand times.
#[derive(Clone, Debug)]
pub struct BoundSelector {
    schema: Field,
    output: Field,
    projections: Vec<Bound>,
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

    /// The bound terms, in output order. Empty means every column.
    #[must_use]
    pub fn projections(&self) -> &[Bound] {
        &self.projections
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

    use super::{BoundSelector, Selector};
    use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
    use crate::expression::arrow::{collected, one_batch, struct_batch};
    use crate::types::cast::ArrowCastOptions;
    use crate::types::FieldValue as _;
    use crate::{Error, Result};

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
            let mut columns = Vec::with_capacity(self.projections.len());
            for (bound, field) in self.projections.iter().zip(self.output.fields()) {
                let evaluated = bound.evaluate(batch)?;
                let declared = field.clone().into_arrow_field_ref()?;
                let array = if evaluated.data_type() == declared.data_type() {
                    evaluated
                } else {
                    // The projection declared a datatype the term does not
                    // produce. A value the column cannot hold becomes null,
                    // the best-effort reading of a cast - unless the column
                    // is declared `not null`, where a null is refused anyway
                    // and the cast says which value could not be held.
                    field
                        .cast_arrow_array(
                            evaluated,
                            ArrowCastOptions::new().with_safe(field.is_nullable()),
                        )
                        .map_err(|error| Error::InvalidRecord {
                            path: smol_str::format_smolstr!("$.{}", field.name()),
                            reason: smol_str::format_smolstr!(
                                "expected every value to fit the required column {:?}, got {error}",
                                field.name()
                            ),
                        })?
                };
                super::require_present(field, array.null_count() > 0)?;
                columns.push(array);
            }
            let schema = arrow_schema_from_field(&self.output)?;
            let options =
                arrow_array::RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
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

    /// One struct's projected columns under the struct's own null mask.
    fn rebuilt_struct(held: &StructArray, projected: &RecordBatch) -> Result<ArrayRef> {
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
                    .apply_arrow_batch(&batch)
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
        if let Some([term, alias]) = value.as_sequence() {
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

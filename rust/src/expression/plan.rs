//! The plan: an expression with sections, one per clause a statement has.
//!
//! A `select` publishes columns and a `where` keeps rows; on their own they
//! leave the rows where they found them. A [`Plan`] is what surrounds them:
//! where the rows come from (`from`), where they go (`create`, `insert`,
//! `upsert`, `delete`), in what order and how many (`order by`, `limit`,
//! `offset`). Every section is optional and a plan is read section by
//! section, so the same type is the schema a record option declares, the
//! query a caller runs, and the statement a caller writes with:
//!
//! ```text
//! [create [<target>] (<columns>)]
//! [insert into | insert overwrite | upsert into ... by (<keys>) | delete from [<target>]]
//! [select <projections>]
//! [from <target> | from (<plan>)]
//! [where <term>]
//! [order by <term> [asc|desc] [nulls first|last], ...]
//! [limit <n>] [offset <n>]
//! ```
//!
//! # Targets and sources
//!
//! A [`Target`] is a [`Location`] plus the properties that open it. A
//! location is a URL - the canonical spelling, which every holder of this
//! crate is reached by - or a dotted path of parts such as
//! `catalog.schema.table`, the spelling a catalog gives a table. A part is
//! quoted when it carries a break character: `"my catalog".trades`,
//! `` `my catalog`.trades `` and `[my catalog].trades` all read as one part
//! called `my catalog`, and print back double-quoted. A parts location
//! resolves against a base URL, so `lake.trades` under `file:///data` is
//! `file:///data/lake/trades`; without a base it names the handle a record
//! option is given to.
//!
//! # Verbs
//!
//! One spelling is canonical per verb, and the spellings other engines use
//! are read as the same verb: `insert into` (or `append into`) adds rows
//! after the ones stored; `insert overwrite` (or `overwrite into`, `replace
//! into`) replaces them; `upsert into ... by (keys)` (or `merge into ... on
//! (keys)`) matches stored rows by key; `delete from ... where` removes the
//! rows a predicate keeps.
//!
//! # Running
//!
//! [`Plan::execute`] reads the `from` source - a target through its holder,
//! pushing `where`, `select` and `limit` into the read, or a nested plan
//! run first - and hands the stream to [`Plan::apply_arrow_reader`], which
//! is what a plan does to any stream: keep, shape, order, bound, then write.
//! Ordering is the one section that cannot stream, because the last row can
//! sort first; every other section is one batch at a time.

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use super::filter::{Filter, IntoFilter};
use super::selector::{IntoSelector, Selector};
use super::term::Term;
use super::{Attribute, Expression};
use crate::{Error, Field, Result, Url};

/// Where a target is: a URL, or the parts of a catalog path.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    /// A location every holder is reached by.
    Url(Url),
    /// `catalog.schema.table`: parts resolved against a base URL.
    Parts(Vec<SmolStr>),
}

impl Location {
    /// The parts of a catalog path, spelled exactly.
    pub fn parts<I, S>(parts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        Self::Parts(parts.into_iter().map(Into::into).collect())
    }

    /// The URL this location names, resolved against `base` when it is a
    /// path of parts.
    ///
    /// # Errors
    ///
    /// Returns an error when the location is a path of parts and there is
    /// no base to resolve it against, or a part cannot be joined.
    pub fn url(&self, base: Option<&Url>) -> Result<Url> {
        match self {
            Self::Url(url) => Ok(url.clone()),
            Self::Parts(parts) => {
                let Some(base) = base else {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.from"),
                        reason: format_smolstr!(
                            "expected a URL, or a base to resolve `{self}` against; a catalog \
                             path names a handle only through the one it is given to"
                        ),
                    });
                };
                parts
                    .iter()
                    .try_fold(base.clone(), |held, part| held.joinpath(part))
            }
        }
    }

    /// The last part, which is what a table is called.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Url(url) => url.stem(),
            Self::Parts(parts) => parts.last().map(SmolStr::as_str),
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => super::display::write_text_literal(formatter, &url.to_string()),
            Self::Parts(parts) => {
                for (index, part) in parts.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(".")?;
                    }
                    super::display::write_identifier(formatter, part)?;
                }
                Ok(())
            }
        }
    }
}

impl From<Url> for Location {
    fn from(url: Url) -> Self {
        Self::Url(url)
    }
}

/// Where a plan reads from or writes to: a location, and the properties
/// that open it.
///
/// The properties are the `with (...)` clause, kept in the order they were
/// written. The ones a holder reads - `media_type`, `codec`, an object
/// store's endpoint and credentials - open the location; the ones a write
/// reads - `safe`, `batch_row_size`, `commit_row_size`, `max_row_size` and
/// their byte counterparts - shape the read or write. Anything else travels
/// along unread, the way a catalog's properties do.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct Target {
    location: Location,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    properties: Vec<(String, String)>,
}

impl Target {
    /// Name a location, with no properties.
    #[must_use]
    pub const fn new(location: Location) -> Self {
        Self {
            location,
            properties: Vec::new(),
        }
    }

    /// Name a URL.
    #[must_use]
    pub const fn url(url: Url) -> Self {
        Self::new(Location::Url(url))
    }

    /// Name a catalog path.
    pub fn parts<I, S>(parts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        Self::new(Location::parts(parts))
    }

    /// Name a location by its text: a URL, or a dotted catalog path.
    ///
    /// # Errors
    ///
    /// Returns an error when the text is neither.
    pub fn parse(text: &str) -> Result<Self> {
        super::parser::parse_target(text)
    }

    /// Return this target with one more property.
    ///
    /// A property already set is replaced in place, so a target never
    /// carries one name twice.
    #[must_use]
    pub fn with_property(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let (name, value) = (name.into(), value.into());
        match self.properties.iter_mut().find(|(held, _)| *held == name) {
            Some(held) => held.1 = value,
            None => self.properties.push((name, value)),
        }
        self
    }

    /// Return this target with every property of an iterator.
    #[must_use]
    pub fn with_properties<K, V>(self, properties: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        properties.into_iter().fold(self, |target, (name, value)| {
            target.with_property(name, value)
        })
    }

    /// The location.
    #[must_use]
    pub const fn location(&self) -> &Location {
        &self.location
    }

    /// The properties, in the order they were written.
    #[must_use]
    pub fn properties(&self) -> &[(String, String)] {
        &self.properties
    }

    /// One property by name.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    /// Read one property as the type a knob has.
    pub fn knob<T: std::str::FromStr>(&self, name: &str, expected: &str) -> Result<Option<T>> {
        self.property(name)
            .map(|value| {
                value.trim().parse().map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("$.with.{name}"),
                    reason: crate::text::expected_got(expected, format_args!("{value:?}")),
                })
            })
            .transpose()
    }

    /// Write the `with (...)` clause, when there is one.
    fn write_properties(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.properties.is_empty() {
            return Ok(());
        }
        formatter.write_str(" with (")?;
        for (index, (name, value)) in self.properties.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            super::display::write_identifier(formatter, name)?;
            formatter.write_str(" = ")?;
            super::display::write_text_literal(formatter, value)?;
        }
        formatter.write_str(")")
    }
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.location)?;
        self.write_properties(formatter)
    }
}

impl From<Url> for Target {
    fn from(url: Url) -> Self {
        Self::url(url)
    }
}

impl From<Location> for Target {
    fn from(location: Location) -> Self {
        Self::new(location)
    }
}

/// Where a plan reads: a target, or another plan run first.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A location read through its holder.
    Target(Target),
    /// A nested plan: its rows are this plan's rows.
    Plan(Box<Plan>),
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Target(target) => write!(formatter, "{target}"),
            Self::Plan(plan) => write!(formatter, "({plan})"),
        }
    }
}

impl From<Target> for Source {
    fn from(target: Target) -> Self {
        Self::Target(target)
    }
}

impl From<Plan> for Source {
    fn from(plan: Plan) -> Self {
        Self::Plan(Box::new(plan))
    }
}

/// What a write does to the rows already stored.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    /// `insert into`: add the rows after the stored ones.
    Insert,
    /// `insert overwrite`: replace the stored rows.
    Overwrite,
    /// `upsert into ... by (keys)`: replace the stored rows the keys match,
    /// add the rest.
    Upsert,
    /// `delete from ... where`: remove the stored rows the predicate keeps.
    Delete,
}

impl Verb {
    /// The canonical spelling, with the word that introduces its target.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Insert => "insert into",
            Self::Overwrite => "insert overwrite",
            Self::Upsert => "upsert into",
            Self::Delete => "delete from",
        }
    }

    /// The verb alone, as a write with no target spells it.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Overwrite => "insert overwrite",
            Self::Upsert => "upsert",
            Self::Delete => "delete",
        }
    }
}

impl fmt::Display for Verb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The write section: a verb, the target it writes, and an upsert's keys.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct Write {
    verb: Verb,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<Target>,
    #[serde(default, skip_serializing_if = "Selector::is_empty")]
    merge_by: Selector,
}

impl Write {
    /// A write of one verb to the handle a plan is given to.
    #[must_use]
    pub fn new(verb: Verb) -> Self {
        Self {
            verb,
            target: None,
            merge_by: Selector::all(),
        }
    }

    /// Return this write aimed at a target.
    #[must_use]
    pub fn into(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    /// Return this write matching stored rows by these keys.
    #[must_use]
    pub fn by(mut self, merge_by: Selector) -> Self {
        self.merge_by = merge_by;
        self
    }

    /// The verb.
    #[must_use]
    pub const fn verb(&self) -> Verb {
        self.verb
    }

    /// The target, when the write names one.
    #[must_use]
    pub const fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    /// The keys an upsert matches on; empty for the other verbs.
    #[must_use]
    pub const fn merge_by(&self) -> &Selector {
        &self.merge_by
    }
}

impl fmt::Display for Write {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.target {
            Some(target) => write!(formatter, "{} {target}", self.verb.as_str())?,
            None => formatter.write_str(self.verb.word())?,
        }
        if !self.merge_by.is_empty() {
            write!(formatter, " by ({})", self.merge_by)?;
        }
        Ok(())
    }
}

/// One `order by` key.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct Ordering {
    term: Term,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    descending: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    nulls_first: bool,
}

impl Ordering {
    /// Order by a term, ascending, nulls last.
    #[must_use]
    pub const fn asc(term: Term) -> Self {
        Self {
            term,
            descending: false,
            nulls_first: false,
        }
    }

    /// Order by a term, descending, nulls last.
    #[must_use]
    pub const fn desc(term: Term) -> Self {
        Self {
            term,
            descending: true,
            nulls_first: false,
        }
    }

    /// Return this key with nulls sorted first.
    #[must_use]
    pub const fn nulls_first(mut self, nulls_first: bool) -> Self {
        self.nulls_first = nulls_first;
        self
    }

    /// The term ordered by.
    #[must_use]
    pub const fn term(&self) -> &Term {
        &self.term
    }

    /// Whether the order is descending.
    #[must_use]
    pub const fn is_descending(&self) -> bool {
        self.descending
    }

    /// Whether nulls sort first.
    #[must_use]
    pub const fn is_nulls_first(&self) -> bool {
        self.nulls_first
    }
}

impl fmt::Display for Ordering {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.term)?;
        if self.descending {
            formatter.write_str(" desc")?;
        }
        if self.nulls_first {
            formatter.write_str(" nulls first")?;
        }
        Ok(())
    }
}

/// An expression with sections: where rows come from, what is kept and
/// published, in what order and how many, and where they go.
///
/// The module documentation gives the grammar. Every section is optional;
/// [`Self::is_empty`] is the plan with none, which does nothing to a stream.
///
/// ```
/// use yggdryl::expression::{Plan, Verb};
/// use yggdryl::{DataType, Expression};
///
/// # fn main() -> yggdryl::Result<()> {
/// let plan: Plan = "upsert into 'file:///lake/trades.parquet' by (id) \
///                   select id, upper(ccy) as ccy from lake.raw where price > 0 \
///                   order by id limit 10"
///     .parse()?;
/// assert_eq!(plan.write_section().map(|write| write.verb()), Some(Verb::Upsert));
/// assert_eq!(plan.merge_by().to_string(), "id");
/// assert_eq!(plan.source().map(|from| from.to_string()), Some("lake.raw".to_owned()));
/// assert_eq!(plan.row_limit(), Some(10));
/// assert_eq!(Expression::Plan(Box::new(plan.clone())).to_string().parse::<Plan>()?, plan);
///
/// // A field is a plan with a `create` section: the schema it declares.
/// let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
///     .required_field("trades");
/// let declared = Plan::from_field(&schema);
/// assert_eq!(declared.to_string(), "create trades (id int64 not null)");
/// assert_eq!(declared.field()?, Some(schema));
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
pub struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    create: Option<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schema: Option<Selector>,
    #[serde(default, skip_serializing_if = "crate::Metadata::is_empty")]
    root_metadata: crate::Metadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    write: Option<Write>,
    #[serde(default, skip_serializing_if = "Selector::is_all")]
    selector: Selector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<Source>,
    #[serde(default, skip_serializing_if = "Filter::is_always_true")]
    filter: Filter,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    order_by: Vec<Ordering>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<u64>,
}

impl Plan {
    /// The plan with no section.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The plan a struct field declares: a `create` section holding every
    /// column with its datatype, nullability and metadata, named after the
    /// field when the field is not named [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME).
    ///
    /// [`Self::field`] reads the field back, so a plan is where a record
    /// option keeps its declared schema.
    /// Read a plan from the scalar that spells one: text, or null for the
    /// empty plan.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a plan, and an error naming
    /// the scalar for any other shape.
    pub fn from_scalar(value: &crate::Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.into_plan();
        }
        if value.is_null() {
            return Ok(Self::new());
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "the text of a plan or null",
                format_args!("{value:?}"),
            ),
        })
    }

    #[must_use]
    pub fn from_field(field: &Field) -> Self {
        let create = (field.name() != crate::media::DEFAULT_ROOT_NAME)
            .then(|| Target::parts([field.name()]));
        Self {
            create,
            schema: Some(Selector::from_field(field)),
            root_metadata: field.as_metadata().clone(),
            ..Self::default()
        }
    }

    /// Return this plan creating `schema` at a target.
    #[must_use]
    pub fn create(mut self, target: Option<Target>, schema: Selector) -> Self {
        self.create = target;
        self.schema = Some(schema);
        self
    }

    /// Return this plan with a write section.
    #[must_use]
    pub fn write(mut self, write: Write) -> Self {
        self.write = Some(write);
        self
    }

    /// Return this plan with a `select` section.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a selector.
    pub fn select(mut self, selector: impl IntoSelector) -> Result<Self> {
        self.selector = selector.into_selector()?;
        Ok(self)
    }

    /// Return this plan reading from a source.
    #[must_use]
    pub fn read_from(mut self, source: impl Into<Source>) -> Self {
        self.from = Some(source.into());
        self
    }

    /// Return this plan with a `where` section.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a filter.
    pub fn filter(mut self, filter: impl IntoFilter) -> Result<Self> {
        self.filter = filter.into_filter()?;
        Ok(self)
    }

    /// Return this plan ordered by keys.
    #[must_use]
    pub fn order_by(mut self, keys: impl IntoIterator<Item = Ordering>) -> Self {
        self.order_by = keys.into_iter().collect();
        self
    }

    /// Return this plan bounded to `limit` rows.
    #[must_use]
    pub const fn limit(mut self, limit: Option<u64>) -> Self {
        self.limit = limit;
        self
    }

    /// Return this plan skipping `offset` rows.
    #[must_use]
    pub const fn offset(mut self, offset: Option<u64>) -> Self {
        self.offset = offset;
        self
    }

    /// Return a deterministic hash of the canonical plan text.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_display(self)
    }

    /// The `create` target, when the plan names one.
    #[must_use]
    pub const fn create_target(&self) -> Option<&Target> {
        self.create.as_ref()
    }

    /// The columns the `create` section declares, when there is one.
    #[must_use]
    pub const fn schema(&self) -> Option<&Selector> {
        self.schema.as_ref()
    }

    /// Declare, or clear, the `create` section's columns.
    ///
    /// Clearing drops the root's metadata with them.
    pub fn set_schema(&mut self, schema: Option<Selector>) {
        if schema.is_none() {
            self.root_metadata = crate::Metadata::new();
        }
        self.schema = schema;
    }

    /// The metadata the `create` section's root carries.
    #[must_use]
    pub const fn root_metadata(&self) -> &crate::Metadata {
        &self.root_metadata
    }

    /// Set the metadata the `create` section's root carries.
    pub fn set_root_metadata(&mut self, metadata: crate::Metadata) {
        self.root_metadata = metadata;
    }

    /// The write section, when there is one.
    #[must_use]
    pub const fn write_section(&self) -> Option<&Write> {
        self.write.as_ref()
    }

    /// The `select` section; `*` when there is none.
    #[must_use]
    pub const fn selector(&self) -> &Selector {
        &self.selector
    }

    /// Set the `select` section.
    pub fn set_selector(&mut self, selector: Selector) {
        self.selector = selector;
    }

    /// The `from` section, when there is one.
    #[must_use]
    pub const fn source(&self) -> Option<&Source> {
        self.from.as_ref()
    }

    /// The `where` section; always true when there is none.
    #[must_use]
    pub const fn filter_section(&self) -> &Filter {
        &self.filter
    }

    /// Set the `where` section.
    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
    }

    /// The `order by` keys, in order.
    #[must_use]
    pub fn ordering(&self) -> &[Ordering] {
        &self.order_by
    }

    /// The `limit`, when there is one.
    #[must_use]
    pub const fn row_limit(&self) -> Option<u64> {
        self.limit
    }

    /// The `offset`, when there is one.
    #[must_use]
    pub const fn row_offset(&self) -> Option<u64> {
        self.offset
    }

    /// The keys the write section matches on; empty without an upsert.
    #[must_use]
    pub fn merge_by(&self) -> &Selector {
        static NONE: std::sync::OnceLock<Selector> = std::sync::OnceLock::new();
        self.write
            .as_ref()
            .map_or_else(|| NONE.get_or_init(Selector::all), Write::merge_by)
    }

    /// Set the keys the write section matches on: an upsert with keys, no
    /// write section without.
    pub fn set_merge_by(&mut self, merge_by: Selector) {
        if merge_by.is_empty() {
            if self
                .write
                .as_ref()
                .is_some_and(|write| write.verb == Verb::Upsert)
            {
                self.write = None;
            }
            return;
        }
        let target = self.write.as_ref().and_then(|write| write.target.clone());
        self.write = Some(Write {
            verb: Verb::Upsert,
            target,
            merge_by,
        });
    }

    /// The name of the root this plan declares or creates.
    ///
    /// The last part of a `create` target's path; the default root name
    /// when the plan creates nothing, or creates at a URL.
    #[must_use]
    pub fn root_name(&self) -> &str {
        match self.create.as_ref().map(Target::location) {
            Some(Location::Parts(parts)) => parts
                .last()
                .map_or(crate::media::DEFAULT_ROOT_NAME, SmolStr::as_str),
            _ => crate::media::DEFAULT_ROOT_NAME,
        }
    }

    /// Name the root this plan declares.
    pub fn set_root_name(&mut self, name: impl Into<SmolStr>) {
        let name: SmolStr = name.into();
        if name == crate::media::DEFAULT_ROOT_NAME
            && !matches!(
                self.create.as_ref().map(Target::location),
                Some(Location::Url(_))
            )
        {
            self.create = None;
            return;
        }
        match &mut self.create {
            Some(target) if matches!(target.location, Location::Url(_)) => {}
            Some(target) => target.location = Location::Parts(vec![name]),
            None => self.create = Some(Target::parts([name])),
        }
    }

    /// Return whether the plan has no section at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.create.is_none()
            && self.schema.is_none()
            && self.root_metadata.is_empty()
            && self.write.is_none()
            && self.selector.is_all()
            && self.from.is_none()
            && self.filter.is_always_true()
            && self.order_by.is_empty()
            && self.limit.is_none()
            && self.offset.is_none()
    }

    /// Return whether applying the plan to a stream changes nothing.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.write.is_none()
            && self.create.is_none()
            && self.selector.is_all()
            && self.filter.is_always_true()
            && self.order_by.is_empty()
            && self.limit.is_none()
            && self.offset.is_none()
    }

    /// The sections that shape a read: `select`, `where`, `order by`,
    /// `limit` and `offset`, without any target.
    ///
    /// This is what a source is read with, so a media pushes the same
    /// projection and predicate down that the plan would apply.
    #[must_use]
    pub fn read_sections(&self) -> Self {
        Self {
            selector: self.selector.clone(),
            filter: self.filter.clone(),
            order_by: self.order_by.clone(),
            limit: self.limit,
            offset: self.offset,
            ..Self::default()
        }
    }

    /// Drop the sections that shape a read, keeping the schema, the write and
    /// the source.
    pub fn clear_read_sections(&mut self) {
        self.selector = Selector::all();
        self.filter = Filter::always_true();
        self.order_by.clear();
        self.limit = None;
        self.offset = None;
    }

    /// Return whether every column the `order by` keys read is one of
    /// `names`, so the keys can be evaluated over rows of those columns.
    fn ordering_reads_only<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> bool {
        let names: Vec<&str> = names.into_iter().collect();
        self.order_by.iter().all(|key| {
            key.term
                .columns()
                .iter()
                .all(|column| names.iter().any(|name| name.eq_ignore_ascii_case(column)))
        })
    }

    /// The struct root the `create` section declares, when every column it
    /// declares carries a datatype.
    ///
    /// `None` is a plan that creates nothing. A column computed from rows,
    /// which needs a root to be typed against, is an error here; it is what
    /// [`Self::field_from`] types.
    ///
    /// # Errors
    ///
    /// Returns an error when a created column is computed rather than
    /// declared, or two columns share a name.
    pub fn field(&self) -> Result<Option<Field>> {
        self.schema
            .as_ref()
            .map(|schema| self.rooted(schema.declared_field(None, self.root_name())?))
            .transpose()
    }

    /// One declared root with the metadata the plan keeps for it.
    fn rooted(&self, field: Field) -> Result<Field> {
        if self.root_metadata.is_empty() {
            return Ok(field);
        }
        field.try_with_metadata_entries(self.root_metadata.iter())
    }

    /// The struct root this plan leaves a stream at, from `root`.
    ///
    /// The `create` section types its computed columns against the root;
    /// otherwise `where` keeps the root and `select` publishes from it.
    ///
    /// # Errors
    ///
    /// Returns an error when a section does not bind against the root.
    pub fn field_from(&self, root: &Field) -> Result<Field> {
        if let Some(schema) = &self.schema {
            return self.rooted(schema.declared_field(Some(root), self.root_name())?);
        }
        root.clone()
            .try_with_dtype(self.apply_datatype(root.dtype())?)
    }

    /// The struct datatype this plan leaves a stream at, from the struct
    /// `dtype`.
    ///
    /// The `create` section types its computed columns against the datatype;
    /// otherwise `where` keeps it and `select` publishes from it.
    ///
    /// # Errors
    ///
    /// Returns an error when a section does not bind against the datatype.
    pub fn apply_datatype(&self, dtype: &crate::DataType) -> Result<crate::DataType> {
        if let Some(schema) = &self.schema {
            let root = Field::new(crate::media::DEFAULT_ROOT_NAME, dtype.clone(), false);
            return Ok(schema
                .declared_field(Some(&root), self.root_name())?
                .dtype()
                .clone());
        }
        self.selector
            .apply_datatype(&self.filter.apply_datatype(dtype)?)
    }

    /// Every top-level column this plan reads, in first-seen order.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut names = self.filter.columns();
        let mut push = |column: String| {
            if !names.iter().any(|held| held.eq_ignore_ascii_case(&column)) {
                names.push(column);
            }
        };
        for column in self.selector.columns() {
            push(column);
        }
        for key in &self.order_by {
            for column in key.term.columns() {
                push(column);
            }
        }
        for column in self.merge_by().columns() {
            push(column);
        }
        names
    }

    /// The stored columns a read has to decode for this plan, when the plan
    /// narrows them; `None` reads everything.
    ///
    /// A `select *` reads every column; anything else reads what its own
    /// clauses name.
    #[must_use]
    pub fn read_columns(&self) -> Option<Vec<String>> {
        if self.selector.is_all() {
            return None;
        }
        Some(self.columns())
    }

    /// Every handle attribute this plan reads, in first-seen order.
    #[must_use]
    pub fn attributes(&self) -> Vec<Attribute> {
        let mut attributes = self.filter.attributes();
        for attribute in self.selector.attributes() {
            if !attributes.contains(&attribute) {
                attributes.push(attribute);
            }
        }
        attributes
    }

    /// Every parameter this plan names, in first-seen order.
    #[must_use]
    pub fn parameters(&self) -> Vec<String> {
        let mut parameters = self.filter.parameters();
        for parameter in self.selector.parameters() {
            if !parameters.contains(&parameter) {
                parameters.push(parameter);
            }
        }
        parameters
    }

    /// The same plan over simplified terms.
    #[must_use]
    pub fn simplify(&self) -> Self {
        Self {
            create: self.create.clone(),
            schema: self.schema.as_ref().map(Selector::simplify),
            root_metadata: self.root_metadata.clone(),
            write: self.write.as_ref().map(|write| Write {
                verb: write.verb,
                target: write.target.clone(),
                merge_by: write.merge_by.simplify(),
            }),
            selector: self.selector.simplify(),
            from: self.from.as_ref().map(|from| match from {
                Source::Target(target) => Source::Target(target.clone()),
                Source::Plan(plan) => Source::Plan(Box::new(plan.simplify())),
            }),
            filter: self.filter.simplify(),
            order_by: self
                .order_by
                .iter()
                .map(|key| Ordering {
                    term: key.term.simplify(),
                    descending: key.descending,
                    nulls_first: key.nulls_first,
                })
                .collect(),
            limit: self.limit,
            offset: self.offset,
        }
    }

    /// Refuse a plan holding a term past the depth or node budget.
    ///
    /// # Errors
    ///
    /// [`Term::check_budget`] carries the rule.
    pub fn check_budget(&self) -> Result<()> {
        if let Some(schema) = &self.schema {
            schema.check_budget()?;
        }
        self.merge_by().check_budget()?;
        self.selector.check_budget()?;
        self.filter.check_budget()?;
        for key in &self.order_by {
            key.term.check_budget()?;
        }
        if let Some(Source::Plan(plan)) = &self.from {
            plan.check_budget()?;
        }
        Ok(())
    }

    /// The expression this plan is: a bare `select` or `where` is the
    /// clause itself, anything else is the plan.
    #[must_use]
    pub fn into_expression(self) -> Expression {
        let only_select = self.create.is_none()
            && self.schema.is_none()
            && self.write.is_none()
            && self.from.is_none()
            && self.filter.is_always_true()
            && self.order_by.is_empty()
            && self.limit.is_none()
            && self.offset.is_none();
        if only_select {
            return Expression::Selector(self.selector);
        }
        let only_where = self.create.is_none()
            && self.schema.is_none()
            && self.write.is_none()
            && self.from.is_none()
            && self.selector.is_all()
            && self.order_by.is_empty()
            && self.limit.is_none()
            && self.offset.is_none();
        if only_where {
            return Expression::Filter(self.filter);
        }
        Expression::Plan(Box::new(self))
    }

    /// The `where` and `select` sections, in the order they run.
    #[must_use]
    pub fn clauses(&self) -> Vec<Expression> {
        let mut clauses = Vec::with_capacity(2);
        if !self.filter.is_always_true() {
            clauses.push(Expression::Filter(self.filter.clone()));
        }
        if !self.selector.is_all() {
            clauses.push(Expression::Selector(self.selector.clone()));
        }
        clauses
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut written = false;
        let mut space = |formatter: &mut fmt::Formatter<'_>| -> fmt::Result {
            if written {
                formatter.write_str(" ")?;
            }
            written = true;
            Ok(())
        };
        if let Some(schema) = &self.schema {
            space(formatter)?;
            formatter.write_str("create")?;
            if let Some(target) = &self.create {
                write!(formatter, " {target}")?;
            }
            write!(formatter, " ({schema})")?;
            if !self.root_metadata.is_empty() {
                formatter.write_str(" with (")?;
                for (index, (key, value)) in self.root_metadata.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    super::display::write_identifier(formatter, key)?;
                    formatter.write_str(" = ")?;
                    super::display::write_text_literal(formatter, value)?;
                }
                formatter.write_str(")")?;
            }
        } else if let Some(target) = &self.create {
            space(formatter)?;
            write!(formatter, "create {target}")?;
        }
        if let Some(write) = &self.write {
            space(formatter)?;
            write!(formatter, "{write}")?;
        }
        // `select *` is spelled when the plan reads and nothing else says
        // so: a plan with a write or a schema, or a bare `where`, leaves the
        // identity projection implicit.
        if !self.selector.is_all()
            || (self.write.is_none() && self.schema.is_none() && self.filter.is_always_true())
        {
            space(formatter)?;
            write!(formatter, "select {}", self.selector)?;
        }
        if let Some(from) = &self.from {
            space(formatter)?;
            write!(formatter, "from {from}")?;
        }
        if !self.filter.is_always_true() {
            space(formatter)?;
            write!(formatter, "where {}", self.filter)?;
        }
        if !self.order_by.is_empty() {
            space(formatter)?;
            formatter.write_str("order by ")?;
            for (index, key) in self.order_by.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                write!(formatter, "{key}")?;
            }
        }
        if let Some(limit) = self.limit {
            space(formatter)?;
            write!(formatter, "limit {limit}")?;
        }
        if let Some(offset) = self.offset {
            space(formatter)?;
            write!(formatter, "offset {offset}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Plan {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        super::parser::parse_plan(input)
    }
}

impl From<Selector> for Plan {
    fn from(selector: Selector) -> Self {
        Self {
            selector,
            ..Self::default()
        }
    }
}

impl From<Filter> for Plan {
    fn from(filter: Filter) -> Self {
        Self {
            filter,
            ..Self::default()
        }
    }
}

impl From<&Field> for Plan {
    fn from(field: &Field) -> Self {
        Self::from_field(field)
    }
}

impl From<Field> for Plan {
    fn from(field: Field) -> Self {
        Self::from_field(&field)
    }
}

/// Anything a call site may hand over where a plan is wanted.
///
/// Text parses as an expression and folds into its sections; a field is
/// the schema it declares; a selector or a filter is its one section; an
/// expression is its plan, except a sequence, which has no single plan.
pub trait IntoPlan {
    /// Produce the plan this value stands for.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not an
    /// expression, or an error when it is a sequence.
    fn into_plan(self) -> Result<Plan>;
}

impl IntoPlan for Plan {
    fn into_plan(self) -> Result<Plan> {
        Ok(self)
    }
}

impl IntoPlan for &Plan {
    fn into_plan(self) -> Result<Plan> {
        Ok(self.clone())
    }
}

impl IntoPlan for Field {
    fn into_plan(self) -> Result<Plan> {
        Ok(Plan::from_field(&self))
    }
}

impl IntoPlan for &Field {
    fn into_plan(self) -> Result<Plan> {
        Ok(Plan::from_field(self))
    }
}

impl IntoPlan for Selector {
    fn into_plan(self) -> Result<Plan> {
        Ok(Plan::from(self))
    }
}

impl IntoPlan for Filter {
    fn into_plan(self) -> Result<Plan> {
        Ok(Plan::from(self))
    }
}

impl IntoPlan for Expression {
    fn into_plan(self) -> Result<Plan> {
        match self {
            Expression::Selector(selector) => Ok(Plan::from(selector)),
            Expression::Filter(filter) => Ok(Plan::from(filter)),
            Expression::Plan(plan) => Ok(*plan),
            Expression::Sequence(steps) => {
                // A sequence of clauses folds into one plan when each step
                // adds a section the plan does not hold yet, in the order the
                // plan runs them; anything else has no single plan.
                let mut plan = Plan::new();
                for step in steps {
                    match step {
                        Expression::Filter(filter)
                            if plan.filter.is_always_true() && plan.selector.is_all() =>
                        {
                            plan.filter = filter;
                        }
                        Expression::Selector(selector) if plan.selector.is_all() => {
                            plan.selector = selector;
                        }
                        other => {
                            return Err(Error::InvalidRecord {
                                path: SmolStr::new_static("$.plan"),
                                reason: format_smolstr!(
                                    "expected one plan, got a sequence that `{other}` cannot fold \
                                     into; apply the sequence to a stream instead"
                                ),
                            });
                        }
                    }
                }
                Ok(plan)
            }
        }
    }
}

impl IntoPlan for &str {
    fn into_plan(self) -> Result<Plan> {
        self.parse::<Expression>()?.into_plan()
    }
}

impl IntoPlan for &String {
    fn into_plan(self) -> Result<Plan> {
        self.as_str().into_plan()
    }
}

impl IntoPlan for String {
    fn into_plan(self) -> Result<Plan> {
        self.as_str().into_plan()
    }
}

/// The error a plan raises when its target cannot be written or read.
pub(crate) fn unreachable(target: &Target, reason: impl fmt::Display) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{}", target.location()),
        reason: format_smolstr!("{reason}"),
    }
}

mod arrow {
    use std::sync::Arc;

    use arrow_array::RecordBatch;
    use arrow_ord::sort::{SortColumn, SortOptions, lexsort_to_indices};

    use super::{Plan, Source, Target, Verb};
    use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
    use crate::expression::arrow::{collected, one_batch};
    use crate::holder::Holder;
    use crate::media::{IORecordOptions, RecordOptions};
    use crate::{Field, IOMedia, Result, Url};

    impl Target {
        /// Hold the location, opened with this target's properties.
        ///
        /// A path of parts resolves against `base`.
        ///
        /// # Errors
        ///
        /// [`Holder::from_url`] carries the rule.
        pub fn holder(&self, base: Option<&Url>) -> Result<Holder> {
            let url = self.location.url(base)?;
            Holder::from_url(&url, self.properties.iter().map(|(k, v)| (k, v)))
        }

        /// The record options a read or write through `holder` runs with,
        /// shaped by this target's properties.
        ///
        /// # Errors
        ///
        /// Returns an error when the holder has no record encoding, or a
        /// property that shapes the write does not parse as its knob.
        pub fn record_options(&self, holder: &Holder) -> Result<RecordOptions> {
            let mut options = holder.record_options()?;
            if let Some(safe) = self.knob::<bool>("safe", "`true` or `false`")? {
                options.set_safe(safe);
            }
            if let Some(rows) = self.knob("batch_row_size", "a row count")? {
                options.set_batch_row_size(Some(rows));
            }
            if let Some(bytes) = self.knob("batch_byte_size", "a byte count")? {
                options.set_batch_byte_size(Some(bytes));
            }
            if let Some(rows) = self.knob("commit_row_size", "a row count")? {
                options.set_commit_row_size(Some(rows));
            }
            if let Some(rows) = self.knob("max_row_size", "a row count")? {
                options.set_max_row_size(Some(rows));
            }
            if let Some(bytes) = self.knob("max_byte_size", "a byte count")? {
                options.set_max_byte_size(Some(bytes));
            }
            Ok(options)
        }
    }

    impl Plan {
        /// Run this plan from its own source.
        ///
        /// A target source is read through its holder with the read sections
        /// pushed into the read, so the media prunes and projects; a nested
        /// plan is executed first. A plan with no source starts from the empty
        /// stream, which is what `create` alone needs. The stream then goes
        /// through [`Self::apply_arrow_reader`].
        ///
        /// # Errors
        ///
        /// Returns an error when the source cannot be held or read, a section
        /// does not bind, or the target cannot be written.
        pub fn execute(&self) -> Result<BatchReader> {
            match &self.from {
                Some(Source::Target(target)) => {
                    let holder = target.holder(None)?;
                    // Ordering cannot be pushed down, and a limit after an
                    // ordering counts sorted rows, so both stay here when the
                    // plan orders; otherwise the media applies the limit too.
                    // A projection the keys do not survive stays here as well,
                    // so `select name ... order by id` still orders by `id`.
                    let mut pushed = self.read_sections();
                    let mut rest = Self::new();
                    if !self.order_by.is_empty() {
                        pushed.order_by.clear();
                        pushed.limit = None;
                        pushed.offset = None;
                        rest.order_by.clone_from(&self.order_by);
                        rest.limit = self.limit;
                        rest.offset = self.offset;
                        let published: Vec<smol_str::SmolStr> = self
                            .selector
                            .projections()
                            .iter()
                            .map(crate::expression::Projection::name)
                            .collect();
                        if !self.selector.is_all()
                            && !self.ordering_reads_only(published.iter().map(|name| name.as_str()))
                        {
                            pushed.selector = super::Selector::all();
                            rest.selector = self.selector.clone();
                        }
                    }
                    let mut options = target.record_options(&holder)?;
                    options.set_plan(pushed)?;
                    let rows = holder
                        .read_arrow_reader(&options)
                        .map_err(|error| super::unreachable(target, error))?;
                    let rows = rest.shape_arrow_reader(rows)?;
                    self.write_arrow_reader(rows)
                }
                Some(Source::Plan(inner)) => {
                    let rows = inner.execute()?;
                    self.apply_arrow_reader(rows)
                }
                None => {
                    let field = self.field()?;
                    let schema = match &field {
                        Some(field) => arrow_schema_from_field(field)?,
                        None => Arc::new(arrow_schema::Schema::empty()),
                    };
                    let empty: [RecordBatch; 0] = [];
                    self.apply_arrow_reader(crate::arrow::batch_reader(schema, empty))
                }
            }
        }

        /// Apply this plan to a stream: keep, shape, order, bound, then write.
        ///
        /// The stream is the plan's rows whatever its `from` names - the
        /// source is what [`Self::execute`] reads when the plan runs on its
        /// own. A plan with a `create` or write section is a sink: it writes
        /// the shaped stream to its target and yields the empty stream under
        /// the schema it wrote. Without one, the shaped stream is yielded.
        ///
        /// # Errors
        ///
        /// Returns an error when a section does not bind against the stream,
        /// or the target cannot be held or written.
        pub fn apply_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            if self
                .write
                .as_ref()
                .is_some_and(|write| write.verb == Verb::Delete)
            {
                // A delete's `where` names the stored rows to remove; the
                // stream it is given carries nothing to shape.
                return self.write_arrow_reader(reader);
            }
            let shaped = self.read_sections().shape_arrow_reader(reader)?;
            self.write_arrow_reader(shaped)
        }

        /// Run the read sections over a stream, in order: `where`, `order by`,
        /// `select`, `offset`, `limit`.
        ///
        /// The keys order the rows `where` keeps, so a key can name a column
        /// the projection drops; a key that names what the projection
        /// publishes - an alias - orders after it instead.
        pub(crate) fn shape_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            // A `where` over an alias runs after the projection that
            // publishes it; every other `where` runs first, where it prunes.
            let late = super::super::filter_after_select(
                &self.filter,
                &self.selector,
                reader
                    .schema()
                    .fields()
                    .iter()
                    .map(|field| field.name().as_str())
                    .collect::<Vec<_>>(),
            );
            let mut reader = if late {
                reader
            } else {
                self.filter.apply_arrow_reader(reader)?
            };
            let mut ordered = self.order_by.is_empty();
            if !ordered {
                let input = reader.schema();
                if self
                    .ordering_reads_only(input.fields().iter().map(|field| field.name().as_str()))
                {
                    reader = self.sorted_arrow_reader(reader)?;
                    ordered = true;
                }
            }
            reader = self.selector.apply_arrow_reader(reader)?;
            if late {
                reader = self.filter.apply_arrow_reader(reader)?;
            }
            if !ordered {
                reader = self.sorted_arrow_reader(reader)?;
            }
            if self.offset.is_some() || self.limit.is_some() {
                reader = crate::arrow::sliced_reader(reader, self.offset.unwrap_or(0), self.limit);
            }
            Ok(reader)
        }

        /// Order a whole stream, which is the one section that has to collect.
        fn sorted_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            let root = field_from_arrow_schema(crate::media::DEFAULT_ROOT_NAME, &reader.schema())?;
            let batch = collected(reader)?;
            let mut columns = Vec::with_capacity(self.order_by.len());
            for key in &self.order_by {
                let bound = key.term.bind(&root)?;
                columns.push(SortColumn {
                    values: bound.evaluate(&batch)?,
                    options: Some(SortOptions {
                        descending: key.descending,
                        nulls_first: key.nulls_first,
                    }),
                });
            }
            let indices = lexsort_to_indices(&columns, None)
                .map_err(|error| crate::Error::from(crate::arrow::Error::Arrow(error)))?;
            let sorted = arrow_select::take::take_record_batch(&batch, &indices)
                .map_err(|error| crate::Error::from(crate::arrow::Error::Arrow(error)))?;
            Ok(one_batch(&sorted))
        }

        /// Write a shaped stream where the plan says, or hand it back.
        fn write_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            let root = field_from_arrow_schema(crate::media::DEFAULT_ROOT_NAME, &reader.schema())?;
            let Some(target) = self.write_target() else {
                if let Some(schema) = &self.schema {
                    // A `create` with no target declares; the stream is cast to
                    // what it declares, under the selector's rule for a
                    // declared column, and handed on.
                    let field = schema.declared_field(
                        Some(&root).filter(|root| root.field_len() > 0),
                        self.root_name(),
                    )?;
                    return super::Selector::from_field(&field).apply_arrow_reader(reader);
                }
                return Ok(reader);
            };
            let mut holder = target.holder(None)?;
            let mut options = target.record_options(&holder)?;
            let mut plan = Self::new();
            if let Some(schema) = &self.schema {
                let typed = Some(&root).filter(|root| root.field_len() > 0);
                plan.schema = Some(super::Selector::from_field(
                    &schema.declared_field(typed, self.root_name())?,
                ));
                plan.create = self.create.clone();
            }
            let written = match &plan.schema {
                Some(_) => plan.field()?.unwrap_or_else(|| root.clone()),
                None => root.clone(),
            };
            let verb = self
                .write
                .as_ref()
                .map_or(Verb::Overwrite, |write| write.verb);
            if verb == Verb::Upsert {
                plan.set_merge_by(self.merge_by().clone());
            }
            options.set_plan(plan)?;
            let outcome = match verb {
                Verb::Overwrite => holder.overwrite_arrow_reader(reader, &options),
                Verb::Insert => holder.append_arrow_reader(reader, &options),
                Verb::Upsert => holder.merge_arrow_reader(reader, &options),
                Verb::Delete => self.delete_from(&mut holder, &options),
            };
            outcome.map_err(|error| super::unreachable(target, error))?;
            empty_reader(&written)
        }

        /// The target a write goes to: the write section's, else the
        /// `create` section's when it names a URL or a path.
        fn write_target(&self) -> Option<&Target> {
            self.write
                .as_ref()
                .and_then(Write::target)
                .or(self.create.as_ref())
        }

        /// Remove the stored rows the `where` section keeps.
        ///
        /// The rows kept are read back, which collects the stream: the store
        /// is rewritten from what it held, and a rewrite cannot read the same
        /// resource it is replacing one batch at a time.
        fn delete_from(&self, holder: &mut Holder, options: &RecordOptions) -> Result<()> {
            let mut reading = options.clone();
            let mut kept = Self::new();
            kept.filter = self.filter.clone().not();
            reading.set_plan(kept)?;
            let remaining = collected(holder.read_arrow_reader(&reading)?)?;
            holder.overwrite_arrow_reader(one_batch(&remaining), options)
        }
    }

    use super::Write;

    /// The empty stream under one schema.
    fn empty_reader(field: &Field) -> Result<BatchReader> {
        let schema = arrow_schema_from_field(field)?;
        let empty: [RecordBatch; 0] = [];
        Ok(crate::arrow::batch_reader(schema, empty))
    }
}

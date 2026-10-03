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
//! [[inner | left [outer] | right [outer] | full [outer] | semi | anti] join <target> | (<plan>)
//!     on <left> = <right> [and <left> = <right>]... | using (<name>, ...)]...
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
//! is what a plan does to any stream: join, keep, shape, order, bound, then
//! write. Ordering is the one section that cannot stream, because the last
//! row can sort first; every other section is one batch at a time.
//!
//! # Joins
//!
//! `from a join b using (id) left join (<plan>) on venue = mic` reads left
//! to right: each [`Join`]'s left side is the plan's rows so far, its right
//! side the source it names, read whole and held while the left streams
//! through it - the engine and its semantics are
//! [`SerieReader::join_with`](crate::SerieReader::join_with)'s. A bare
//! `join` is `inner`. `on` is equalities joined by `and`, each a left term
//! against a right term, the left bound against the rows so far and the
//! right against the joined source; `using (a, b)` is the same column on
//! both sides, coalesced into one. The clause states no
//! [`JoinOptions`](crate::JoinOptions) of its own, so a join runs under
//! [`JoinOptions::new`](crate::JoinOptions::new): a colliding right name
//! takes the `_right` suffix and a `using` key is one column. Every other
//! read section - `where`, `select`, `order by`, `offset`, `limit` - runs
//! over the joined rows, so it may name a right column.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use super::Expression;
use super::filter::{Filter, IntoFilter};
use super::join::{IntoJoinKeys, JoinKeys};
use super::selector::{IntoSelector, Selector};
use super::term::Term;
use crate::boolean::{BOOLEAN_SPELLINGS, bool_from_text};
use crate::integer::{INTEGER_SPELLINGS, integer_from_text_as};
use crate::{Error, Field, JoinKind, JoinOptions, Result, SortOptions, Url};

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
/// reads - `safe`, `batch_row_size`, `commit_batch_num`, `max_row_size`,
/// `row_offset` and their byte counterparts - shape the read or write. Anything else travels
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

    /// Read one property as a flag, in every spelling a boolean is read from:
    /// `true`, `yes`, `on`, `1` and their opposites, in any case.
    ///
    /// # Errors
    ///
    /// Returns an error naming `$.with.<name>` and the text when the
    /// property spells no boolean.
    pub fn knob_bool(&self, name: &str) -> Result<Option<bool>> {
        self.knob_read(name, BOOLEAN_SPELLINGS, bool_from_text)
    }

    /// Read one property as a count, at the width of the option it shapes.
    ///
    /// # Errors
    ///
    /// Returns an error naming `$.with.<name>` and the text when the
    /// property is not a whole number `T` holds.
    pub fn knob_count<T: TryFrom<i128> + TryFrom<u128>>(&self, name: &str) -> Result<Option<T>> {
        self.knob_read(name, INTEGER_SPELLINGS, integer_from_text_as)
    }

    /// Read one property through the reader of the type it has.
    fn knob_read<T>(
        &self,
        name: &str,
        expected: &str,
        read: impl FnOnce(&str) -> Option<T>,
    ) -> Result<Option<T>> {
        self.property(name)
            .map(|value| {
                read(value).ok_or_else(|| Error::InvalidRecord {
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

impl Source {
    /// The same source over simplified terms.
    fn simplify(&self) -> Self {
        match self {
            Self::Target(target) => Self::Target(target.clone()),
            Self::Plan(plan) => Self::Plan(Box::new(plan.simplify())),
        }
    }

    /// The root this source's rows are typed by, when the plan states it
    /// without reading anything: a nested plan's, from what it declares or
    /// from its own source's root. A target states its root only where it
    /// is read, so it answers `None`, as a nested plan reading one does.
    ///
    /// # Errors
    ///
    /// Returns an error when a nested plan's sections do not bind against
    /// the root its source states.
    pub(crate) fn static_root(&self) -> Result<Option<Field>> {
        match self {
            Self::Target(_) => Ok(None),
            Self::Plan(plan) => plan.static_root(),
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

impl std::str::FromStr for Verb {
    type Err = Error;

    /// Read a write verb in any spelling the grammar reads - `insert`,
    /// `append`, `insert overwrite`, `overwrite`, `replace`, `upsert`,
    /// `merge`, `delete` - with or without the `into`, `to` or `from` that
    /// introduces a target, in any case; the one table the plan grammar and
    /// the bindings share.
    fn from_str(input: &str) -> Result<Self> {
        super::parser::parse_verb(input)
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

/// One join section: which rows it keeps, the source it joins, and the keys
/// it matches on.
///
/// A plan holds its joins in the order they are written, each joining the
/// rows so far - the `from` source's, then each join's output - with its own
/// source. [`Plan::join`] adds one; `Display` writes the clause, `using
/// (...)` where every key is one bare column on both sides and `on l = r and
/// ...` otherwise.
///
/// ```
/// use yggdryl::JoinKind;
/// use yggdryl::expression::{Plan, Target};
///
/// # fn main() -> yggdryl::Result<()> {
/// let plan = Plan::new()
///     .read_from(Target::parse("trades")?)
///     .join(JoinKind::Left, Target::parse("venues")?, "venue = mic")?;
/// let [join] = plan.joins() else { unreachable!() };
/// assert_eq!(join.how(), JoinKind::Left);
/// assert_eq!(join.source().to_string(), "venues");
/// assert_eq!(join.keys().to_string(), "venue = mic");
/// assert_eq!(join.to_string(), "left join venues on venue = mic");
/// assert_eq!(plan.to_string(), "select * from trades left join venues on venue = mic");
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct Join {
    how: JoinKind,
    source: Source,
    keys: JoinKeys,
}

impl Join {
    /// Which rows the join keeps.
    #[must_use]
    pub const fn how(&self) -> JoinKind {
        self.how
    }

    /// The source joined: the right side.
    #[must_use]
    pub const fn source(&self) -> &Source {
        &self.source
    }

    /// The keys matched on, most significant first.
    #[must_use]
    pub const fn keys(&self) -> &JoinKeys {
        &self.keys
    }

    /// Whether every key is one bare column on both sides: what `using`
    /// spells.
    fn is_using(&self) -> bool {
        self.keys.iter().all(|key| key.using_column().is_some())
    }
}

impl fmt::Display for Join {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} join {}", self.how, self.source)?;
        if self.is_using() {
            formatter.write_str(" using (")?;
            for (index, key) in self.keys.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(", ")?;
                }
                super::display::write_identifier(
                    formatter,
                    key.using_column().unwrap_or_default(),
                )?;
            }
            return formatter.write_str(")");
        }
        formatter.write_str(" on ")?;
        for (index, key) in self.keys.iter().enumerate() {
            if index != 0 {
                formatter.write_str(" and ")?;
            }
            write!(formatter, "{}", key.equality())?;
        }
        Ok(())
    }
}

/// One `order by` key: a term and the [`SortOptions`] it sorts under.
///
/// `Display` writes the key as the grammar spells it - `price desc nulls
/// first` - and `FromStr` reads one back, which is also how a `SORT:by`
/// entry is stored and read ([`SortField`](crate::SortField)).
///
/// ```
/// use yggdryl::SortOptions;
/// use yggdryl::expression::Ordering;
///
/// # fn main() -> yggdryl::Result<()> {
/// let key: Ordering = "price desc nulls first".parse()?;
/// assert_eq!(key.term().to_string(), "price");
/// assert_eq!(key.options(), SortOptions::descending().with_nulls_first(true));
/// assert_eq!(key.to_string(), "price desc nulls first");
/// assert_eq!("ts".parse::<Ordering>()?.options(), SortOptions::default());
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
#[serde(from = "OrderingWire", into = "OrderingWire")]
pub struct Ordering {
    term: Term,
    options: SortOptions,
}

/// The document shape of an [`Ordering`]: the term beside its two facts,
/// each written only when it is not the default.
#[derive(::serde::Serialize, ::serde::Deserialize)]
struct OrderingWire {
    term: Term,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    descending: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    nulls_first: bool,
}

impl From<OrderingWire> for Ordering {
    fn from(wire: OrderingWire) -> Self {
        let direction = if wire.descending {
            SortOptions::descending()
        } else {
            SortOptions::ascending()
        };
        Self::new(wire.term, direction.with_nulls_first(wire.nulls_first))
    }
}

impl From<Ordering> for OrderingWire {
    fn from(ordering: Ordering) -> Self {
        Self {
            descending: ordering.options.is_descending(),
            nulls_first: ordering.options.is_nulls_first(),
            term: ordering.term,
        }
    }
}

impl Ordering {
    /// Order by a term under `options`.
    #[must_use]
    pub const fn new(term: Term, options: SortOptions) -> Self {
        Self { term, options }
    }

    /// Order by a term, ascending, nulls last.
    #[must_use]
    pub const fn asc(term: Term) -> Self {
        Self::new(term, SortOptions::ascending())
    }

    /// Order by a term, descending, nulls last.
    #[must_use]
    pub const fn desc(term: Term) -> Self {
        Self::new(term, SortOptions::descending())
    }

    /// Return this key with nulls sorted first.
    #[must_use]
    pub const fn nulls_first(mut self, nulls_first: bool) -> Self {
        self.options = self.options.with_nulls_first(nulls_first);
        self
    }

    /// The term ordered by.
    #[must_use]
    pub const fn term(&self) -> &Term {
        &self.term
    }

    /// The direction and the nulls placement, as one value.
    #[must_use]
    pub const fn options(&self) -> SortOptions {
        self.options
    }

    /// Whether the order is descending.
    #[must_use]
    pub const fn is_descending(&self) -> bool {
        self.options.is_descending()
    }

    /// Whether nulls sort first.
    #[must_use]
    pub const fn is_nulls_first(&self) -> bool {
        self.options.is_nulls_first()
    }
}

impl Ordering {
    /// Read one key from the scalar that spells it: text in the `order by`
    /// key grammar, or a record of `term` - text, or the literal it is, as
    /// [`Term::from_scalar`] reads - beside the optional flags `descending`
    /// and `nulls_first`, each a boolean or the text the crate's one boolean
    /// table reads (`"yes"`, `"0"`). This is the reading every binding's
    /// keys cross through, so `{"term": "price", "descending": true}` and
    /// `"price desc"` are one key.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a key, and an error naming
    /// the shape for a record without a `term`, with a key it does not know,
    /// or with a flag that spells no boolean.
    pub fn from_scalar(value: &crate::Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.parse();
        }
        let shape = || Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "the text of an `order by` key, or a record of `term`, `descending` and `nulls_first` (`nullsFirst` too)",
                format_args!("{value:?}"),
            ),
        };
        let entries: Vec<(&str, &crate::Scalar)> = if let Some(entries) = value.as_struct() {
            entries
                .iter()
                .map(|(key, held)| (key.as_str(), held))
                .collect()
        } else if let Some(entries) = value.as_mapping() {
            entries
                .iter()
                .map(|(key, held)| key.as_str().map(|key| (key, held)).ok_or_else(shape))
                .collect::<Result<Vec<_>>>()?
        } else {
            return Err(shape());
        };
        let mut term = None;
        let mut options = SortOptions::default();
        for (key, held) in entries {
            let flag = || {
                crate::boolean::bool_of(held).ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("$.{key}"),
                    reason: crate::text::expected_got(BOOLEAN_SPELLINGS, format_args!("{held:?}")),
                })
            };
            match key {
                "term" => term = Some(Term::from_scalar(held)?),
                "descending" => {
                    options = if flag()? {
                        SortOptions::descending().with_nulls_first(options.is_nulls_first())
                    } else {
                        SortOptions::ascending().with_nulls_first(options.is_nulls_first())
                    };
                }
                // Both spellings of the one key: what a JavaScript record
                // arrives as, and what the grammar writes.
                "nulls_first" | "nullsFirst" => options = options.with_nulls_first(flag()?),
                other => {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("$.{other}"),
                        reason: SmolStr::new_static(
                            "an `order by` key has `term`, `descending` and `nulls_first`",
                        ),
                    });
                }
            }
        }
        let term = term.ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$.term"),
            reason: SmolStr::new_static("an `order by` key names the term it orders by"),
        })?;
        Ok(Self::new(term, options))
    }
}

impl fmt::Display for Ordering {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}{}", self.term, self.options())
    }
}

impl FromStr for Ordering {
    type Err = Error;

    /// Read one `order by` key as the grammar spells it: a term, then an
    /// optional `asc` or `desc`, then an optional `nulls first` or `nulls
    /// last` - `price desc nulls first`.
    fn from_str(input: &str) -> Result<Self> {
        super::parser::parse_ordering(input)
    }
}

/// Any value that stands for the keys of an `order by`: what every sort
/// verb takes as its `by`.
///
/// The spellings, each resolved once into the keys it names: the clause's
/// text without its keywords (`"venue, price desc nulls first"`); one
/// [`Ordering`], a vector, a slice or an array of them; a list of key texts;
/// a [`Selector`], every projection ascending with nulls last; and a
/// [`Scalar`](crate::Scalar) - a text, a list of texts, or a list of
/// `{term, descending, nulls_first}` records - which is how a binding's keys
/// cross ([`Ordering::from_scalar`]).
///
/// ```
/// use yggdryl::SortOptions;
/// use yggdryl::expression::{IntoOrderings, Ordering};
///
/// # fn main() -> yggdryl::Result<()> {
/// let keys = "venue, price desc nulls first".into_orderings()?;
/// assert_eq!(keys.len(), 2);
/// assert_eq!(keys[1].options(), SortOptions::descending().with_nulls_first(true));
/// assert_eq!(["venue", "price desc nulls first"].into_orderings()?, keys);
/// assert_eq!(keys.clone().into_orderings()?, keys);
/// assert_eq!("price".parse::<Ordering>()?.into_orderings()?.len(), 1);
/// # Ok(())
/// # }
/// ```
pub trait IntoOrderings {
    /// Produce the keys this value stands for, most significant first.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a list of
    /// keys, and an error naming the shape for a scalar that is none of the
    /// shapes above.
    fn into_orderings(self) -> Result<Vec<Ordering>>;
}

impl IntoOrderings for Ordering {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        Ok(vec![self])
    }
}

impl IntoOrderings for &Ordering {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        Ok(vec![self.clone()])
    }
}

impl IntoOrderings for Vec<Ordering> {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        Ok(self)
    }
}

impl IntoOrderings for &[Ordering] {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        Ok(self.to_vec())
    }
}

impl<const N: usize> IntoOrderings for [Ordering; N] {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        Ok(self.into())
    }
}

impl IntoOrderings for &str {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        super::parser::parse_orderings(self)
    }
}

impl IntoOrderings for String {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        super::parser::parse_orderings(&self)
    }
}

impl IntoOrderings for &String {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        super::parser::parse_orderings(self)
    }
}

/// One key per text, each read through the key grammar.
fn orderings_of<'a>(texts: impl IntoIterator<Item = &'a str>) -> Result<Vec<Ordering>> {
    texts.into_iter().map(str::parse).collect()
}

impl IntoOrderings for Vec<&str> {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        orderings_of(self)
    }
}

impl IntoOrderings for &[&str] {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        orderings_of(self.iter().copied())
    }
}

impl<const N: usize> IntoOrderings for [&str; N] {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        orderings_of(self)
    }
}

impl IntoOrderings for Vec<String> {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        orderings_of(self.iter().map(String::as_str))
    }
}

impl IntoOrderings for &[String] {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        orderings_of(self.iter().map(String::as_str))
    }
}

impl<const N: usize> IntoOrderings for [String; N] {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        orderings_of(self.iter().map(String::as_str))
    }
}

impl IntoOrderings for &Selector {
    /// Every projection, ascending with nulls last; an `unnest` is refused,
    /// because a key is one value per row.
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        self.refuse_unnest("in an `order by` key")?;
        Ok(self
            .projections()
            .iter()
            .map(|projection| Ordering::asc(projection.term().clone()))
            .collect())
    }
}

impl IntoOrderings for Selector {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        (&self).into_orderings()
    }
}

impl IntoOrderings for &crate::Scalar {
    /// A text is the clause, a list is one key per item, anything else is
    /// one key, each as [`Ordering::from_scalar`] reads it.
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        if let Some(text) = self.as_str() {
            return super::parser::parse_orderings(text);
        }
        if let Some(items) = self.sequence_rows() {
            return items.iter().map(Ordering::from_scalar).collect();
        }
        Ok(vec![Ordering::from_scalar(self)?])
    }
}

impl IntoOrderings for crate::Scalar {
    fn into_orderings(self) -> Result<Vec<Ordering>> {
        (&self).into_orderings()
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
/// use yggdryl::{DataType, Expression, StructType};
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
/// let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    joins: Vec<Join>,
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

    /// The plan a struct field declares: a `create` section holding every
    /// column with its datatype, nullability and metadata, named after the
    /// field when the field is not named [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME),
    /// and the `order by` keys the field's `SORT:by` declares.
    ///
    /// The `order by` section is the one owner of the sort order, so the
    /// `SORT:by` property leaves the root's metadata here and
    /// [`Self::field`] writes it back from the section. A declaration that
    /// does not parse stays in the metadata as it is and is refused where it
    /// is read.
    ///
    /// [`Self::field`] reads the field back, so a plan is where a record
    /// option keeps its declared schema.
    #[must_use]
    pub fn from_field(field: &Field) -> Self {
        let create = (field.name() != crate::media::DEFAULT_ROOT_NAME)
            .then(|| Target::parts([field.name()]));
        let mut root_metadata = field.as_metadata().clone();
        let order_by = match field.as_sort().by() {
            Ok(Some(keys)) => {
                root_metadata.remove(crate::metadata::SORT_BY_KEY);
                keys
            }
            _ => Vec::new(),
        };
        Self {
            create,
            schema: Some(Selector::from_field(field)),
            root_metadata,
            order_by,
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

    /// Return this plan joining the rows so far with `source` on `keys`,
    /// keeping the rows `how` keeps: one more [`Join`], after the ones the
    /// plan already holds.
    ///
    /// `keys` is any [`IntoJoinKeys`]: `"id"` - the same column on both
    /// sides - `"venue = mic"`, a list of either, a pair of selectors. The
    /// left term of a key is bound against the rows so far, the right term
    /// against `source`'s.
    ///
    /// ```
    /// use yggdryl::JoinKind;
    /// use yggdryl::expression::{Plan, Target};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let plan = Plan::new()
    ///     .read_from(Target::parse("trades")?)
    ///     .join(JoinKind::Inner, Target::parse("venues")?, ["venue", "desk"])?
    ///     .join(JoinKind::Anti, "select id from halted".parse::<Plan>()?, "id")?;
    /// assert_eq!(
    ///     plan.to_string(),
    ///     "select * from trades inner join venues using (venue, desk) \
    ///      anti join (select id from halted) using (id)"
    /// );
    /// assert_eq!(plan.to_string().parse::<Plan>()?, plan);
    /// assert!(Plan::new().join(JoinKind::Inner, Target::parse("venues")?, Vec::<&str>::new()).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a parse error for key text that is not a key list, and an
    /// error for an empty key list or a key past the depth or node budget.
    pub fn join(
        mut self,
        how: JoinKind,
        source: impl Into<Source>,
        keys: impl IntoJoinKeys,
    ) -> Result<Self> {
        let keys = keys.into_join_keys()?;
        if keys.is_empty() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.join"),
                reason: SmolStr::new_static(
                    "expected at least one key to join on, got an empty key list",
                ),
            });
        }
        for key in &keys {
            key.left().check_budget()?;
            key.right().check_budget()?;
        }
        self.joins.push(Join {
            how,
            source: source.into(),
            keys,
        });
        Ok(self)
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

    /// The joins, in the order they run.
    #[must_use]
    pub fn joins(&self) -> &[Join] {
        &self.joins
    }

    /// This plan with every source it reads - its `from`, then each join's,
    /// in the order they run - replaced by what `map` answers for it. A
    /// nested plan is handed over whole, so `map` decides whether to reach
    /// into it. This is the one door that rewrites where a plan reads, which
    /// is how a catalog resolves the tables a plan names.
    ///
    /// # Errors
    ///
    /// The first error `map` answers, the plan dropped with it.
    pub(crate) fn map_sources(
        mut self,
        mut map: impl FnMut(Source) -> Result<Source>,
    ) -> Result<Self> {
        if let Some(from) = self.from.take() {
            self.from = Some(map(from)?);
        }
        self.joins = std::mem::take(&mut self.joins)
            .into_iter()
            .map(|join| {
                Ok(Join {
                    source: map(join.source)?,
                    ..join
                })
            })
            .collect::<Result<_>>()?;
        Ok(self)
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
            && self.joins.is_empty()
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
            && self.joins.is_empty()
            && self.selector.is_all()
            && self.filter.is_always_true()
            && self.order_by.is_empty()
            && self.limit.is_none()
            && self.offset.is_none()
    }

    /// The sections that shape a read: the joins, `select`, `where`,
    /// `order by`, `limit` and `offset`, without any target.
    ///
    /// This is what a stream is shaped with, so a media pushes the same
    /// projection and predicate down that the plan would apply. A join is a
    /// read section that no media answers: [`Self::execute`] runs it over the
    /// rows the media reads, and pushes only its key filter down.
    #[must_use]
    pub fn read_sections(&self) -> Self {
        Self {
            joins: self.joins.clone(),
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
        self.joins.clear();
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

    /// One declared root with the metadata the plan keeps for it, the
    /// `order by` keys written as its `SORT:by`.
    fn rooted(&self, field: Field) -> Result<Field> {
        if self.root_metadata.is_empty() && self.order_by.is_empty() {
            return Ok(field);
        }
        let mut field = field.try_with_metadata_entries(self.root_metadata.iter())?;
        if !self.order_by.is_empty() {
            field.as_sort_mut().set_by(self.order_by.iter().cloned())?;
        }
        Ok(field)
    }

    /// The struct root this plan leaves a stream at, from `root`.
    ///
    /// The joins run first, `root` their left side: the output of each is
    /// the record the join engine lays out - the left columns, then the right
    /// ones, a `using` key once, a colliding right name suffixed `_right`, a
    /// side the kind makes optional nullable - typed against the root its
    /// source states. Then the `create` section types its computed columns
    /// against the joined root; otherwise `where` keeps it and `select`
    /// publishes from it.
    ///
    /// The `from` source is never read here: `root` is its rows. A join's
    /// source is not read either, so its root is known only when the plan
    /// states it - a nested plan that declares its columns, or whose own
    /// source is a nested plan that does. A target's root is known only once
    /// it is read, and a join over one is refused here naming it;
    /// [`Self::execute`] reads it and types the rows as they join.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, StructType};
    /// use yggdryl::expression::Plan;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let trades = DataType::from(StructType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::utf8().required_field("venue"),
    /// ])?)
    /// .required_field("trades");
    /// let plan: Plan = "select id, city from trades \
    ///                   left join (create (venue utf8 not null, city utf8 not null)) using (venue)"
    ///     .parse()?;
    /// let out = plan.field_from(&trades)?;
    /// let names: Vec<&str> = out.fields().iter().map(Field::name).collect();
    /// assert_eq!(names, ["id", "city"]);
    /// // A left join makes the right side optional.
    /// assert!(out.fields()[1].is_nullable());
    /// // A target's columns are known only once it is read.
    /// let read: Plan = "select * from trades join venues using (venue)".parse()?;
    /// assert!(read.field_from(&trades).unwrap_err().to_string().contains("venues"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a section does not bind against the root, a
    /// join's source states no root without being read, or a join's keys do
    /// not bind against their sides.
    pub fn field_from(&self, root: &Field) -> Result<Field> {
        let root = self.joined_root(root)?;
        if let Some(schema) = &self.schema {
            return self.rooted(schema.declared_field(Some(&root), self.root_name())?);
        }
        let dtype = self.shaped_datatype(&root)?;
        root.into_owned().try_with_dtype(dtype)
    }

    /// The struct datatype this plan leaves a stream at, from the struct
    /// `dtype`.
    ///
    /// The joins run first, typed as [`Self::field_from`] types them. Then
    /// the `create` section types its computed columns against the datatype;
    /// otherwise `where` keeps it and `select` publishes from it - a `where`
    /// that names what only the `select` publishes typed after it, where it
    /// runs.
    ///
    /// # Errors
    ///
    /// Returns an error when a section does not bind against the datatype,
    /// or a join's source states no root without being read.
    pub fn apply_datatype(&self, dtype: &crate::DataType) -> Result<crate::DataType> {
        let root = Field::new(crate::media::DEFAULT_ROOT_NAME, dtype.clone(), false);
        let root = self.joined_root(&root)?;
        if let Some(schema) = &self.schema {
            return Ok(schema
                .declared_field(Some(&root), self.root_name())?
                .dtype()
                .clone());
        }
        self.shaped_datatype(&root)
    }

    /// The datatype the `where` and `select` sections leave a joined root
    /// at.
    fn shaped_datatype(&self, root: &Field) -> Result<crate::DataType> {
        let dtype = root.dtype();
        let input = root.fields().iter().map(Field::name);
        if super::filter_after_select(&self.filter, &self.selector, input) {
            return self
                .filter
                .apply_datatype(&self.selector.apply_datatype(dtype)?);
        }
        self.selector
            .apply_datatype(&self.filter.apply_datatype(dtype)?)
    }

    /// The root the joins leave `root` at, left to right; `root` itself for
    /// a plan with none.
    fn joined_root<'a>(&self, root: &'a Field) -> Result<Cow<'a, Field>> {
        if self.joins.is_empty() {
            return Ok(Cow::Borrowed(root));
        }
        let mut left = root.clone();
        for join in &self.joins {
            let Some(right) = join.source.static_root()? else {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.join"),
                    reason: format_smolstr!(
                        "expected a join source whose columns the plan states, got `{join}`, \
                         whose columns are known only once it is read: run the plan, or join \
                         a plan that declares them"
                    ),
                });
            };
            left = Field::clone(
                crate::join::JoinPlan::compile(
                    left,
                    right,
                    &join.keys,
                    join.how,
                    &JoinOptions::new(),
                    None,
                    None,
                )?
                .output(),
            );
        }
        Ok(Cow::Owned(left))
    }

    /// The root this plan leaves its own rows at, when it states one without
    /// reading anything: its source's static root shaped by the plan, or -
    /// with no source - the empty stream under what its `create` section
    /// declares, which is what [`Self::execute`] starts from. A plan reading
    /// a target states the root its `create` section declares, when every
    /// column of it carries a datatype, and none otherwise.
    pub(crate) fn static_root(&self) -> Result<Option<Field>> {
        let input = match &self.from {
            Some(source) => source.static_root()?,
            None => Some(match self.field()? {
                Some(field) => field,
                None => Field::new(
                    crate::media::DEFAULT_ROOT_NAME,
                    crate::DataType::from(crate::StructType::from_fields(Vec::<Field>::new())?),
                    false,
                ),
            }),
        };
        match input {
            Some(root) => self.field_from(&root).map(Some),
            // A column computed from rows cannot be typed without them: the
            // root is unknown, not wrong.
            None => Ok(self.field().ok().flatten()),
        }
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
        for join in &self.joins {
            for key in &join.keys {
                for column in key.left().columns() {
                    push(column);
                }
                for column in key.right().columns() {
                    push(column);
                }
            }
        }
        names
    }

    /// The stored columns a read has to decode for this plan, when the plan
    /// narrows them; `None` reads everything.
    ///
    /// A `select` holding a `*` reads every column - whatever it excludes,
    /// since what it keeps is the schema's to name, and whatever it appends;
    /// anything else reads what its own clauses name.
    #[must_use]
    pub fn read_columns(&self) -> Option<Vec<String>> {
        if self.selector.has_star() {
            return None;
        }
        Some(self.columns())
    }

    /// Every parameter this plan names, in first-seen order.
    #[must_use]
    pub fn parameters(&self) -> Vec<String> {
        let mut parameters = self.filter.parameters();
        let keys = self
            .joins
            .iter()
            .flat_map(|join| join.keys.iter())
            .flat_map(|key| [key.left(), key.right()]);
        for parameter in self
            .selector
            .parameters()
            .into_iter()
            .chain(keys.flat_map(Term::parameters))
        {
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
            from: self.from.as_ref().map(Source::simplify),
            joins: self
                .joins
                .iter()
                .map(|join| Join {
                    how: join.how,
                    source: join.source.simplify(),
                    keys: JoinKeys::new(join.keys.iter().map(|key| {
                        super::JoinKey::new(key.left().simplify(), key.right().simplify())
                    })),
                })
                .collect(),
            filter: self.filter.simplify(),
            order_by: self
                .order_by
                .iter()
                .map(|key| Ordering::new(key.term.simplify(), key.options))
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
        for join in &self.joins {
            for key in &join.keys {
                key.left().check_budget()?;
                key.right().check_budget()?;
            }
            if let Source::Plan(plan) = &join.source {
                plan.check_budget()?;
            }
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
            && self.joins.is_empty()
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
            && self.joins.is_empty()
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
        for join in &self.joins {
            space(formatter)?;
            write!(formatter, "{join}")?;
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

    use arrow_array::{Array, ArrayRef, RecordBatch, StructArray, UInt32Array};
    use arrow_ord::sort::{SortColumn, lexsort_to_indices};

    use super::{Join, Plan, Source, Target, Verb};
    use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
    use crate::expression::Filter;
    use crate::expression::arrow::{collected, one_batch, scattered, struct_rows};
    use crate::holder::Holder;
    use crate::media::{IORecordOptions, RecordOptions};
    use crate::{
        ArrowCastOptions, ChunkedSerie, Field, IOMedia, JoinOptions, JoinSide, Result, SerieReader,
        SerieSource, Url,
    };

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
            if let Some(safe) = self.knob_bool("safe")? {
                options.set_safe(safe);
            }
            if let Some(rows) = self.knob_count("batch_row_size")? {
                options.set_batch_row_size(Some(rows));
            }
            if let Some(bytes) = self.knob_count("batch_byte_size")? {
                options.set_batch_byte_size(Some(bytes));
            }
            if let Some(threads) = self.knob_count("num_threads")? {
                options.set_num_threads(Some(threads));
            }
            if let Some(batches) = self.knob_count("commit_batch_num")? {
                options.set_commit_batch_num(Some(batches));
            }
            if let Some(rows) = self.knob_count("max_row_size")? {
                options.set_max_row_size(Some(rows));
            }
            if let Some(rows) = self.knob_count("row_offset")? {
                options.set_row_offset(Some(rows));
            }
            if let Some(bytes) = self.knob_count("max_byte_size")? {
                options.set_max_byte_size(Some(bytes));
            }
            Ok(options)
        }
    }

    impl Join {
        /// Read this join's source whole and hold it, one chunk per batch it
        /// reads as: a target through its holder under its own record
        /// options, a nested plan executed first. This is the build side.
        fn held(&self) -> Result<ChunkedSerie> {
            let rows = match &self.source {
                Source::Target(target) => {
                    let holder = target.holder(None)?;
                    let options = target.record_options(&holder)?;
                    holder
                        .read_arrow_reader(&options)
                        .map_err(|error| super::unreachable(target, error))?
                }
                Source::Plan(plan) => plan.execute()?,
            };
            Ok(ChunkedSerie::from_arrow_reader(
                None,
                rows,
                ArrowCastOptions::new(),
            )?)
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
        /// A plan with joins reads each join's source whole and holds it,
        /// first to last, and streams the rows so far through it under
        /// [`JoinOptions::new`]: the left source probes and is never held.
        /// The `where`, `select`, `order by`, `offset` and `limit` sections
        /// all run over the joined rows, so none of them is pushed into the
        /// left read as it stands. What is pushed is pruning that loses no
        /// joined row, and only into a target `from`:
        ///
        /// - the first join's key filter: for one key and a kind that emits
        ///   no unmatched left row (`inner`, `right`, `semi`), the left key
        ///   `in` the distinct keys its held source states, up to
        ///   [`DEFAULT_PUSHDOWN_KEYS`](crate::DEFAULT_PUSHDOWN_KEYS) of them -
        ///   past that, or for any other join, the left is read whole. A
        ///   later join probes the rows already in hand and pushes nothing.
        /// - the `where` conjuncts that name only columns the left source
        ///   stores, when no join of the plan emits an unmatched right row
        ///   (`right`, `full`), so every left column keeps the left's value;
        ///   the whole `where` still runs after the joins.
        ///
        /// # Errors
        ///
        /// Returns an error when the source cannot be held or read, a section
        /// does not bind, a join's keys do not bind against their sides, or
        /// the target cannot be written.
        pub fn execute(&self) -> Result<BatchReader> {
            match &self.from {
                Some(Source::Target(target)) if !self.joins.is_empty() => {
                    let holder = target.holder(None)?;
                    let mut options = target.record_options(&holder)?;
                    let first = &self.joins[0];
                    // The build side is read before the probe, so its keys
                    // can prune the probe's read.
                    let held = first.held()?;
                    let left_root = holder
                        .read_arrow_field(&options)
                        .map_err(|error| super::unreachable(target, error))?;
                    let right_root = SerieReader::root_of(held.field())?;
                    let mut pushed = Self::new();
                    pushed.filter = self.left_pruning(&left_root);
                    if let Some(term) = crate::join::pushdown_term(
                        &left_root,
                        &right_root,
                        &first.keys,
                        first.how,
                        JoinSide::Right,
                        &JoinOptions::new(),
                        &held,
                    )? {
                        pushed.filter = pushed.filter.and(Filter::new(term));
                    }
                    options.set_plan(pushed)?;
                    let rows = holder
                        .read_arrow_reader(&options)
                        .map_err(|error| super::unreachable(target, error))?;
                    let rest = self.read_sections();
                    let rows = rest.joined_arrow_reader(rows, Some(held))?;
                    let rows = rest.shaped_arrow_reader(rows)?;
                    self.write_arrow_reader(rows)
                }
                Some(Source::Target(target)) => {
                    let holder = target.holder(None)?;
                    // Ordering and an offset cannot be pushed down - record
                    // options hold neither - and a limit after either counts
                    // the rows they leave, so all three stay here whenever the
                    // plan orders or skips; otherwise the media applies the
                    // limit too. A projection the keys do not survive stays
                    // here as well, so `select name ... order by id` still
                    // orders by `id`.
                    let mut pushed = self.read_sections();
                    let mut rest = Self::new();
                    if !self.order_by.is_empty() || self.offset.is_some() {
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

        /// Run the read sections over a stream, in order: the joins, `where`,
        /// `order by`, `select`, `offset`, `limit`.
        ///
        /// The keys order the rows `where` keeps, so a key can name a column
        /// the projection drops; a key that names what the projection
        /// publishes - an alias - orders after it instead.
        pub(crate) fn shape_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            let reader = self.joined_arrow_reader(reader, None)?;
            self.shaped_arrow_reader(reader)
        }

        /// The stream joined with every join's source, left to right: each
        /// source read whole and held - `first`, when the caller already
        /// holds the first one - and the stream probing it one batch at a
        /// time. A plan with no join hands the stream back untouched.
        fn joined_arrow_reader(
            &self,
            reader: BatchReader,
            mut first: Option<ChunkedSerie>,
        ) -> Result<BatchReader> {
            if self.joins.is_empty() {
                return Ok(reader);
            }
            let mut rows = SerieReader::from_arrow_reader(None, reader, ArrowCastOptions::new())?;
            for join in &self.joins {
                let held = match first.take() {
                    Some(held) => held,
                    None => join.held()?,
                };
                rows = rows.join_with(
                    SerieSource::Chunked(held),
                    &join.keys,
                    join.how,
                    &JoinOptions::new(),
                )?;
            }
            Ok(rows.into_arrow_reader())
        }

        /// The `where` conjuncts a target `from` can be filtered by before
        /// the joins run without losing a joined row: those naming only
        /// columns `left_root` stores, when no join emits an unmatched right
        /// row - which is what nulls a left column the source stated.
        fn left_pruning(&self, left_root: &Field) -> Filter {
            if self
                .joins
                .iter()
                .any(|join| join.how.keeps_unmatched_right())
            {
                return Filter::always_true();
            }
            Filter::all(self.filter.conjuncts().into_iter().filter(|conjunct| {
                let columns = conjunct.columns();
                !columns.is_empty()
                    && columns
                        .iter()
                        .all(|column| left_root.index_of(column).is_some())
            }))
        }

        /// Run the sections after the joins over a stream, in order: `where`,
        /// `order by`, `select`, `offset`, `limit`.
        fn shaped_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            let reader = self.narrowed_arrow_reader(reader)?;
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

        /// The stream with only the columns some section reads.
        ///
        /// A column no section reads is dropped first, as the bare column it
        /// is - which moves no buffer - so a `where` that keeps some rows
        /// never copies a column the `select` would drop after it. A `*`
        /// reads every column it does not exclude. A stream whose names two
        /// columns spell alike is left whole, since only a quoted name tells
        /// them apart.
        fn narrowed_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            if self.selector.is_all() {
                return Ok(reader);
            }
            let schema = reader.schema();
            let names: Vec<&str> = schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect();
            let alike = names.iter().enumerate().any(|(index, name)| {
                names[..index]
                    .iter()
                    .any(|held| held.eq_ignore_ascii_case(name))
            });
            if alike {
                return Ok(reader);
            }
            let read = self.columns();
            let kept: Vec<&str> = names
                .iter()
                .copied()
                .filter(|name| {
                    read.iter().any(|column| column.eq_ignore_ascii_case(name))
                        || (self.selector.has_star()
                            && !self
                                .selector
                                .excluded()
                                .iter()
                                .any(|excluded| excluded.eq_ignore_ascii_case(name)))
                })
                .collect();
            if kept.is_empty() || kept.len() == names.len() {
                return Ok(reader);
            }
            super::Selector::from_columns(kept).apply_arrow_reader(reader)
        }

        /// Order a whole stream, which is the one section that has to collect.
        fn sorted_arrow_reader(&self, reader: BatchReader) -> Result<BatchReader> {
            let batch = collected(reader)?;
            let indices = self.order_indices(&batch, None)?;
            let sorted = arrow_select::take::take_record_batch(&batch, &indices)
                .map_err(|error| crate::Error::from(crate::arrow::Error::Arrow(error)))?;
            Ok(one_batch(&sorted))
        }

        /// The order the `order by` keys put rows in.
        ///
        /// The ordering is stable: rows the keys cannot tell apart keep the
        /// order they arrived in, because the row's position is the last key.
        /// The keys read `batch`; with `scatter`, each key is laid out at the
        /// struct positions it names first, null at a null struct row - a row
        /// every key answers unknown for.
        fn order_indices(
            &self,
            batch: &RecordBatch,
            scatter: Option<&UInt32Array>,
        ) -> Result<UInt32Array> {
            let root = field_from_arrow_schema(crate::media::DEFAULT_ROOT_NAME, &batch.schema())?;
            let rows = scatter.map_or(batch.num_rows(), Array::len);
            let positions = u32::try_from(rows).map_err(|_| crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!(
                    "expected at most {} rows to order, got {rows}",
                    u32::MAX
                ),
            })?;
            let mut columns = Vec::with_capacity(self.order_by.len() + 1);
            for key in &self.order_by {
                let mut values = key.term.bind(&root)?.evaluate(batch)?;
                if let Some(scatter) = scatter {
                    values = scattered(&values, scatter)?;
                }
                columns.push(SortColumn {
                    values,
                    options: Some(key.options().into_arrow()),
                });
            }
            columns.push(SortColumn {
                values: Arc::new(UInt32Array::from_iter_values(0..positions)),
                options: None,
            });
            lexsort_to_indices(&columns, None)
                .map_err(|error| crate::Error::from(crate::arrow::Error::Arrow(error)))
        }

        /// Apply this plan to one struct array: the plan over the struct's
        /// rows, a null struct a row every term answers unknown for.
        ///
        /// [`Expression::apply_arrow_array`](crate::Expression::apply_arrow_array)
        /// carries the rule.
        pub(crate) fn apply_arrow_array(&self, array: &ArrayRef) -> Result<ArrayRef> {
            let rows = struct_rows(array, "run a plan over")?;
            let deletes = self
                .write
                .as_ref()
                .is_some_and(|write| write.verb == Verb::Delete);
            // A `where` drops a null row, an unnest lays out none for it, a
            // join matches none, and a delete reads no row of the stream: the
            // plan answers the struct's other rows alone.
            let Some(scatter) = rows.scatter.as_ref().filter(|_| {
                self.joins.is_empty()
                    && self.filter.is_always_true()
                    && !self.selector.unnests()
                    && !deletes
            }) else {
                let applied = collected(self.apply_arrow_reader(one_batch(&rows.batch))?)?;
                return Ok(Arc::new(StructArray::from(applied)));
            };
            if let Some(target) = self.write_target() {
                let row = scatter
                    .iter()
                    .position(|at| at.is_none())
                    .unwrap_or_default();
                return Err(crate::Error::InvalidRecord {
                    path: smol_str::format_smolstr!("$[{row}]"),
                    reason: smol_str::format_smolstr!(
                        "expected a record to write to {}, got a null struct at row {row}",
                        target.location()
                    ),
                });
            }
            // Every null row survives: the struct's other rows are shaped and
            // cast one for one, laid back out at their positions under the
            // struct's mask, then ordered and bounded among the null rows.
            let mut shaping = self.read_sections();
            shaping.order_by.clear();
            shaping.limit = None;
            shaping.offset = None;
            let shaped = collected(shaping.shape_arrow_reader(one_batch(&rows.batch))?)?;
            let declared = collected(self.write_arrow_reader(one_batch(&shaped))?)?;
            let columns = declared
                .columns()
                .iter()
                .map(|column| scattered(column, scatter))
                .collect::<Result<Vec<_>>>()?;
            let mut laid: ArrayRef = Arc::new(
                StructArray::try_new_with_length(
                    declared.schema().fields().clone(),
                    columns,
                    rows.held.nulls().cloned(),
                    rows.held.len(),
                )
                .map_err(|error| crate::Error::from(crate::arrow::Error::Arrow(error)))?,
            );
            if !self.order_by.is_empty() {
                // Keys over stored columns read the struct's rows; a key over
                // what the select publishes reads the shaped ones.
                let stored = rows.batch.schema();
                let keyed = if self
                    .ordering_reads_only(stored.fields().iter().map(|field| field.name().as_str()))
                {
                    &rows.batch
                } else {
                    &shaped
                };
                let indices = self.order_indices(keyed, Some(scatter))?;
                laid = arrow_select::take::take(laid.as_ref(), &indices, None)
                    .map_err(|error| crate::Error::from(crate::arrow::Error::Arrow(error)))?;
            }
            let bound = |count: Option<u64>| {
                count.map(|count| usize::try_from(count).unwrap_or(usize::MAX))
            };
            let offset = bound(self.offset).unwrap_or(0).min(laid.len());
            let length = bound(self.limit)
                .unwrap_or(usize::MAX)
                .min(laid.len() - offset);
            Ok(laid.slice(offset, length))
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
                    let declared = super::Selector::from_field(&field);
                    if declared.is_all() {
                        return Ok(reader);
                    }
                    return declared.bind(&root)?.apply_arrow_reader(reader);
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
            // One dispatcher: the verb is the write mode it names, and the
            // handle's own `write_arrow_reader` validates it against the
            // options before the reader is pulled.
            let outcome = match verb {
                Verb::Overwrite => {
                    holder.write_arrow_reader(reader, crate::IOMode::Overwrite, &options)
                }
                Verb::Insert => holder.write_arrow_reader(reader, crate::IOMode::Append, &options),
                Verb::Upsert => holder.write_arrow_reader(reader, crate::IOMode::Merge, &options),
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
        /// The rows kept are read back and held before the store is
        /// rewritten from them, because a rewrite cannot read the same
        /// resource it is replacing one batch at a time. They are held as a
        /// [`ChunkedSerie`] - one chunk per batch, settled under the process
        /// spill bound as each lands - so a table larger than the bound is
        /// deleted from under it rather than collected whole.
        fn delete_from(&self, holder: &mut Holder, options: &RecordOptions) -> Result<()> {
            let mut reading = options.clone();
            let mut kept = Self::new();
            kept.filter = self.filter.clone().not();
            reading.set_plan(kept)?;
            let remaining =
                ChunkedSerie::from_serie_reader(crate::SerieReader::from_arrow_reader(
                    None,
                    holder.read_arrow_reader(&reading)?,
                    ArrowCastOptions::default(),
                )?)?;
            holder.write_serie(
                SerieSource::from(remaining),
                crate::IOMode::Overwrite,
                Some(options),
            )
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

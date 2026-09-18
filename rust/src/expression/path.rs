//! The one path into a nested schema or value.
//!
//! Every surface that addresses a child by path resolves it here, once, and
//! carries the resolved [`FieldPath`] afterwards. Before this module each
//! surface split a string at `.` in its own way: the schema lookup backtracked
//! so a field genuinely named `a.b` resolved, the scalar navigator did not so
//! the same path selected a column and then failed to select its value, the
//! FIX navigator read a decimal segment as a repeating-group index, and none of
//! the plain splitters could address a list or a map at all.
//!
//! One grammar answers all of it: `.name` for a struct child, `[0]` and `[-1]`
//! for a list element, `['key']` for a map entry, `[1:3]` for a run of list
//! elements, `[ccy = 'EUR']` for the elements of a list of structs a predicate
//! over the element's own fields keeps. A name the bare spelling cannot carry
//! is quoted, so `a.b` has exactly one spelling and it is not two levels. A
//! trailing `as name`, spelled the way SQL spells it, says what to call what
//! the path reached - the one thing a selector cannot say by itself.
//!
//! The grammar is the expression grammar's own: a path is what a
//! [`Term`](super::Term) reads at its leaf and what a [`Selector`](super::Selector)
//! projects, and there is one parser behind all three. A path applies the same
//! way everywhere - [`FieldSegment::apply_field`] types one step,
//! [`FieldSegment::apply_scalar`] takes it through a value, and the Arrow tier
//! takes it through a column - so a schema, a row, and a batch cannot disagree
//! about what `legs[0].ccy` reaches.
//!
//! This is a *selector*: it says which child a caller wants. It is not the
//! crate-private `Path` cons-list a recursive walk carries to report where a
//! failure happened. The two never merge - one is caller input resolved once,
//! the other is walker state rendered only on error.

use std::borrow::Cow;
use std::fmt::{self, Write as _};
use std::str::FromStr;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::typing::{common_type, unwrap_dictionary};
use super::{Literal, Term};
use crate::{DataType, Error, Field, Result, Scalar};

/// What a parse failure names itself as.
pub(crate) const TARGET: &str = "field path";

/// One step of a path into a nested schema or value.
///
/// Written once in the grammar, resolved once against the container's
/// datatype, and applied identically by every walk that takes a path.
#[derive(Clone, Debug, Eq, PartialEq, Hash, ::serde::Serialize, ::serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSegment {
    /// `.name` - a struct child, resolved ASCII case-insensitively the way
    /// every cast in this crate resolves a name.
    Field(SmolStr),
    /// `[0]`, `[-1]` - one list element by position, 0-based, a negative index
    /// counting back from the end. Out of range is null rather than an error,
    /// because absence is not a failure on the read path anywhere else here.
    Index(i64),
    /// `['k']` - one map entry by key, the key read once through the map's own
    /// key type. A struct child may also be reached this way when the key is
    /// text, which is the spelling JSON tooling already uses.
    Key(Literal),
    /// `[1:3]`, `[:2]`, `[-2:]` - a run of list elements, half-open the way
    /// every slice in this crate is, a negative end counting back from the
    /// end. The result is a list of the same item type, and a run that
    /// reaches past either end is clipped rather than refused.
    Range {
        /// The first position kept; absent means the start of the list.
        start: Option<i64>,
        /// The first position not kept; absent means the end of the list.
        end: Option<i64>,
    },
    /// `[ccy = 'EUR']` - the elements of a list of structs a predicate keeps,
    /// JSONPath's `[?(...)]` without the `?`. The term is a boolean over the
    /// element's own fields: a name inside it resolves against the element
    /// struct, never against the row. The result is a list of the same item
    /// type; an element the predicate answers false or unknown for is
    /// dropped, a null element is dropped, and a null list stays null.
    ///
    /// Inside brackets an integer is a position, a text literal a key, a
    /// colon a run, and anything else - a bare boolean column included - is
    /// this.
    Where(Box<Term>),
}

impl FieldSegment {
    /// Name a struct child.
    #[must_use]
    pub fn field(name: impl Into<SmolStr>) -> Self {
        Self::Field(name.into())
    }

    /// Name a list element by position.
    #[must_use]
    pub const fn index(position: i64) -> Self {
        Self::Index(position)
    }

    /// Name a run of list elements, half-open.
    #[must_use]
    pub const fn range(start: Option<i64>, end: Option<i64>) -> Self {
        Self::Range { start, end }
    }

    /// Keep the elements of a list of structs a predicate answers true for.
    ///
    /// The predicate is typed and bound against the element struct when the
    /// segment is applied, so the names it reads are the element's fields.
    #[must_use]
    pub fn filter(predicate: Term) -> Self {
        Self::Where(Box::new(predicate))
    }

    /// The predicate this segment keeps elements by, when it is one.
    #[must_use]
    pub fn as_predicate(&self) -> Option<&Term> {
        match self {
            Self::Where(predicate) => Some(predicate),
            Self::Field(_) | Self::Index(_) | Self::Key(_) | Self::Range { .. } => None,
        }
    }

    /// Name a map entry by text key.
    ///
    /// A path key is text, because text is what a path can write down and read
    /// back unambiguously. A whole number in brackets is a list position, and
    /// a map keyed by anything else is reached through the expression
    /// grammar's own accessor, which takes a computed key and never has to
    /// render it.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not text, or when it and the
    /// datatype it is paired with disagree.
    pub fn key(value: Scalar) -> Result<Self> {
        if value.as_str().is_none() {
            return Err(Error::Parse {
                target: TARGET,
                position: 0,
                reason: format_smolstr!("expected a text map key, got {}", value.kind()),
            });
        }
        Ok(Self::Key(Literal::infer(value)?))
    }

    /// The name this segment addresses a child by, where it addresses one.
    ///
    /// A text key names a child too: `['price']` and `.price` reach the same
    /// struct member, and a caller asking for the name should not have to know
    /// which spelling arrived.
    #[must_use]
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Field(name) => Some(name.as_str()),
            Self::Key(key) => key.value().as_str(),
            Self::Index(_) | Self::Range { .. } | Self::Where(_) => None,
        }
    }

    /// The position this segment addresses an element by, where it addresses
    /// one.
    ///
    /// Only a position segment answers. A key that happens to hold a number is
    /// a key: reading it as a position is the exact ambiguity this one grammar
    /// exists to remove.
    #[must_use]
    pub const fn as_index(&self) -> Option<i64> {
        match self {
            Self::Index(position) => Some(*position),
            Self::Field(_) | Self::Key(_) | Self::Range { .. } | Self::Where(_) => None,
        }
    }

    /// The field one step through `field` reaches.
    ///
    /// The one place a step is typed: the scalar walk, the Arrow walk, and a
    /// selector's output schema all ask here, which is what keeps the three
    /// from disagreeing about what a step produces. A child reached through a
    /// step is nullable even when it is declared required, because the parent
    /// may be null and then the whole path is.
    ///
    /// # Errors
    ///
    /// Returns an error naming the datatype when it has no such child, no
    /// elements to index, or no entries to key.
    pub fn apply_field(&self, field: &Field) -> Result<Field> {
        let dtype = unwrap_dictionary(field.dtype());
        match self {
            Self::Field(name) => match dtype {
                DataType::Struct(_) => struct_child_field(field, name),
                DataType::Mapping(map) => Ok(map_value_field(map)?.with_nullable(true)),
                other => Err(typing_error(format_smolstr!(
                    "expected a struct or a map to reach .{name} through, got {other}"
                ))),
            },
            Self::Index(_) => match list_item(dtype) {
                Some(item) => Ok(item.clone().with_nullable(true)),
                None => Err(typing_error(format_smolstr!(
                    "expected a list to index into, got {dtype}"
                ))),
            },
            Self::Range { .. } => match list_item(dtype) {
                Some(item) => Ok(kept_list_field(field, item)),
                None => Err(typing_error(format_smolstr!(
                    "expected a list to take a range of, got {dtype}"
                ))),
            },
            Self::Where(predicate) => {
                let element = element_field(field)?;
                require_predicate(&predicate.field(&element)?, predicate)?;
                Ok(kept_list_field(field, &element))
            }
            Self::Key(key) => match dtype {
                DataType::Mapping(map) => {
                    let keys = map_key_field(map)?;
                    common_type(keys.dtype(), key.dtype()).ok_or_else(|| {
                        typing_error(format_smolstr!(
                            "expected a key comparable with {}, got {}",
                            keys.dtype(),
                            key.dtype()
                        ))
                    })?;
                    Ok(map_value_field(map)?.with_nullable(true))
                }
                DataType::Struct(_) => match key.value().as_str() {
                    Some(name) => struct_child_field(field, name),
                    None => Err(typing_error(format_smolstr!(
                        "expected a text key to reach a struct child, got {}",
                        key.dtype()
                    ))),
                },
                other => Err(typing_error(format_smolstr!(
                    "expected a map or a struct to key into, got {other}"
                ))),
            },
        }
    }

    /// The value one step through `value`, typed as `field`, reaches.
    ///
    /// Absence is null rather than an error on the read path: a position past
    /// the end, a key no entry holds, and a null container all answer null,
    /// the way a missing map key answers everywhere else in this crate.
    ///
    /// A predicate segment binds its term against the element struct here,
    /// once per call; a caller with many rows [binds](Term::bind) the whole
    /// path once instead, and the bound path carries the bound predicate.
    ///
    /// # Errors
    ///
    /// Returns an error when a predicate segment is applied to anything but
    /// a list of structs, its term does not bind against the element struct
    /// or answers no boolean, or its evaluation refuses an element.
    pub fn apply_scalar(&self, field: &Field, value: &Scalar) -> Result<Scalar> {
        if value.is_null() {
            return Ok(Scalar::Null);
        }
        Ok(match self {
            Self::Field(name) => struct_child(field, value, name),
            Self::Index(position) => {
                let Some(items) = value.as_sequence() else {
                    return Ok(Scalar::Null);
                };
                resolve_index(*position, items.len())
                    .and_then(|index| items.get(index))
                    .cloned()
                    .unwrap_or(Scalar::Null)
            }
            Self::Range { start, end } => {
                let Some(items) = value.as_sequence() else {
                    return Ok(Scalar::Null);
                };
                let (from, until) = resolve_range(*start, *end, items.len());
                Scalar::from_sequence(items[from..until].iter().cloned())
            }
            Self::Key(key) => {
                if let Some(entries) = value.as_mapping() {
                    let dtype = key.dtype();
                    return Ok(entries
                        .iter()
                        .find(|(held, _)| {
                            super::eval::compare(
                                dtype,
                                held,
                                super::Comparison::IsNotDistinctFrom,
                                key.value(),
                            )
                            .as_bool()
                                == Some(true)
                        })
                        .map_or(Scalar::Null, |(_, held)| held.clone()));
                }
                match key.value().as_str() {
                    Some(name) => struct_child(field, value, name),
                    None => Scalar::Null,
                }
            }
            Self::Where(predicate) => {
                let element = element_field(field)?;
                let bound = predicate.bind(&element)?;
                require_predicate(bound.field(), predicate)?;
                super::eval::keep_elements(&element, bound.node(), value, None)?
            }
        })
    }
}

/// The element struct a predicate segment binds against.
///
/// # Errors
///
/// Returns an error naming the datatype when it is not a list of structs.
pub(crate) fn element_field(field: &Field) -> Result<Field> {
    let dtype = unwrap_dictionary(field.dtype());
    match list_item(dtype) {
        Some(item) if item.is_struct() => Ok(item.clone()),
        _ => Err(typing_error(format_smolstr!(
            "expected a list of structs to keep elements of by a predicate, got {dtype}"
        ))),
    }
}

/// Refuse a predicate segment whose term answers anything but a boolean.
pub(crate) fn require_predicate(answer: &Field, predicate: &Term) -> Result<()> {
    if matches!(answer.dtype(), DataType::Boolean | DataType::Null) {
        return Ok(());
    }
    Err(typing_error(format_smolstr!(
        "expected a boolean predicate to keep list elements by, got [{predicate}] of {}",
        answer.dtype()
    )))
}

/// The list a run or a predicate leaves: the same item type, and nullable,
/// because the run or the kept elements of a null list are null.
pub(crate) fn kept_list_field(field: &Field, item: &Field) -> Field {
    Field::new(field.name(), DataType::list(item.clone()), true)
}

/// One struct value as the column values its field orders, whichever spelling
/// holds it.
///
/// A schema-ordered sequence is borrowed; a record or a mapping is read child
/// by child; a spelling that is no struct at all answers nothing.
pub(crate) fn struct_values<'value>(
    field: &Field,
    value: &'value Scalar,
) -> Option<Cow<'value, [Scalar]>> {
    let width = field.field_len();
    if let Some(values) = value.as_sequence() {
        if values.len() == width {
            return Some(Cow::Borrowed(values));
        }
        // A short or long sequence still reads in schema order: what is
        // missing is null, and what is past the schema is not there to read.
        let mut padded: Vec<Scalar> = values.iter().take(width).cloned().collect();
        padded.resize(width, Scalar::Null);
        return Some(Cow::Owned(padded));
    }
    if value.as_record().is_none() && value.as_mapping().is_none() {
        return None;
    }
    Some(Cow::Owned(
        field
            .fields()
            .iter()
            .map(|child| struct_child(field, value, child.name()))
            .collect(),
    ))
}

/// The position a possibly negative index names in a run of `length`.
pub(crate) fn resolve_index(position: i64, length: usize) -> Option<usize> {
    let length = i64::try_from(length).unwrap_or(i64::MAX);
    let resolved = if position < 0 {
        length + position
    } else {
        position
    };
    (resolved >= 0 && resolved < length).then(|| usize::try_from(resolved).unwrap_or(0))
}

/// The clipped `[from, until)` a half-open range names in a run of `length`.
pub(crate) fn resolve_range(start: Option<i64>, end: Option<i64>, length: usize) -> (usize, usize) {
    let bound = |position: Option<i64>, default: usize| -> usize {
        let Some(position) = position else {
            return default;
        };
        let signed = i64::try_from(length).unwrap_or(i64::MAX);
        let resolved = if position < 0 {
            signed + position
        } else {
            position
        };
        usize::try_from(resolved.clamp(0, signed)).unwrap_or(0)
    };
    let from = bound(start, 0);
    let until = bound(end, length).max(from);
    (from, until)
}

/// Read one struct child from its mapping or schema-ordered sequence spelling.
fn struct_child(field: &Field, value: &Scalar, name: &str) -> Scalar {
    if let Some(entries) = value.as_mapping() {
        return entries
            .iter()
            .find(|(key, _)| {
                key.as_str()
                    .is_some_and(|held| held.eq_ignore_ascii_case(name))
            })
            .map_or(Scalar::Null, |(_, held)| held.clone());
    }
    if let Some(record) = value.as_record() {
        return record
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map_or(Scalar::Null, |(_, held)| held.clone());
    }
    // A struct spelled as a bare sequence takes its order from the schema.
    if let (Some(values), DataType::Struct(fields)) =
        (value.as_sequence(), unwrap_dictionary(field.dtype()))
    {
        return fields
            .as_fields()
            .iter()
            .position(|child| child.name().eq_ignore_ascii_case(name))
            .and_then(|index| values.get(index))
            .cloned()
            .unwrap_or(Scalar::Null);
    }
    Scalar::Null
}

fn struct_child_field(field: &Field, name: &str) -> Result<Field> {
    field
        .fields()
        .iter()
        .find(|child| child.name().eq_ignore_ascii_case(name))
        .cloned()
        .map(|child| child.with_nullable(true))
        .ok_or_else(|| {
            typing_error(format_smolstr!(
                "expected a child of {}, got {name:?}",
                field.dtype()
            ))
        })
}

/// The item field of a list-shaped datatype, whichever layout it uses.
pub(crate) fn list_item(dtype: &DataType) -> Option<&Field> {
    match dtype {
        DataType::List(item)
        | DataType::ListView(item)
        | DataType::FixedSizeList(item, _)
        | DataType::LargeList(item)
        | DataType::LargeListView(item) => Some(item.as_ref()),
        _ => None,
    }
}

fn map_key_field(map: &crate::MappingType) -> Result<Field> {
    map.entries()
        .get_field(0)
        .cloned()
        .ok_or_else(|| typing_error("expected a map whose entries carry a key field"))
}

fn map_value_field(map: &crate::MappingType) -> Result<Field> {
    map.entries()
        .get_field(1)
        .cloned()
        .ok_or_else(|| typing_error("expected a map whose entries carry a value field"))
}

fn typing_error(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: reason.into(),
    }
}

impl Ord for FieldSegment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Field(left), Self::Field(right)) => left.cmp(right),
            (Self::Index(left), Self::Index(right)) => left.cmp(right),
            (Self::Key(left), Self::Key(right)) => left.cmp(right),
            (
                Self::Range {
                    start: left_start,
                    end: left_end,
                },
                Self::Range {
                    start: right_start,
                    end: right_end,
                },
            ) => left_start
                .cmp(right_start)
                .then_with(|| left_end.cmp(right_end)),
            (Self::Where(left), Self::Where(right)) => left.cmp(right),
            (left, right) => left.rank().cmp(&right.rank()),
        }
    }
}

impl PartialOrd for FieldSegment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl FieldSegment {
    /// The order kinds sort in when two segments are not the same kind.
    const fn rank(&self) -> u8 {
        match self {
            Self::Field(_) => 0,
            Self::Index(_) => 1,
            Self::Key(_) => 2,
            Self::Range { .. } => 3,
            Self::Where(_) => 4,
        }
    }
}

impl fmt::Display for FieldSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(name) => {
                formatter.write_char('.')?;
                write_identifier(formatter, name)
            }
            Self::Index(index) => write!(formatter, "[{index}]"),
            Self::Range { start, end } => {
                formatter.write_char('[')?;
                if let Some(start) = start {
                    write!(formatter, "{start}")?;
                }
                formatter.write_char(':')?;
                if let Some(end) = end {
                    write!(formatter, "{end}")?;
                }
                formatter.write_char(']')
            }
            Self::Key(key) => {
                formatter.write_char('[')?;
                write_key(formatter, key)?;
                formatter.write_char(']')
            }
            Self::Where(predicate) => {
                // The brackets already delimit the term, so it starts at the
                // loosest level and never grows braces of its own.
                formatter.write_char('[')?;
                super::display::write_at(
                    formatter,
                    predicate,
                    super::display::Precedence::Disjunction,
                )?;
                formatter.write_char(']')
            }
        }
    }
}

/// One resolved path into a nested schema or value, and what to call what it
/// reaches.
///
/// Cloning shares the segments rather than copying them, so a path hoisted out
/// of a loop and handed to each iteration costs one reference count.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FieldPath {
    segments: Arc<[FieldSegment]>,
    alias: Option<SmolStr>,
}

impl FieldPath {
    /// The empty path, which selects the value it is applied to.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// Build a path from segments already resolved.
    ///
    /// The way to build a path from parts: nothing is rendered to text and
    /// nothing is parsed back.
    pub fn new(segments: impl IntoIterator<Item = FieldSegment>) -> Self {
        Self {
            segments: segments.into_iter().collect(),
            alias: None,
        }
    }

    /// Build a path over segments already shared.
    pub(crate) fn from_shared(segments: Arc<[FieldSegment]>, alias: Option<SmolStr>) -> Self {
        Self { segments, alias }
    }

    /// Parse one path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position and what was expected.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Borrow the resolved segments.
    #[must_use]
    pub fn segments(&self) -> &[FieldSegment] {
        &self.segments
    }

    /// Share the resolved segments.
    pub(crate) fn shared_segments(&self) -> Arc<[FieldSegment]> {
        Arc::clone(&self.segments)
    }

    /// The number of segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Whether this is the empty path.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Whether this path selects the value it is applied to.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// The first segment.
    #[must_use]
    pub fn first(&self) -> Option<&FieldSegment> {
        self.segments.first()
    }

    /// The last segment.
    #[must_use]
    pub fn last(&self) -> Option<&FieldSegment> {
        self.segments.last()
    }

    /// The path without its last segment.
    ///
    /// The alias is not carried up: it names what the whole path reached, and
    /// the parent reaches something else.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let (_, head) = self.segments.split_last()?;
        Some(Self::new(head.iter().cloned()))
    }

    /// This path with one more segment.
    ///
    /// The alias is dropped for the same reason [`Self::parent`] drops it.
    #[must_use]
    pub fn join(&self, segment: FieldSegment) -> Self {
        Self::new(
            self.segments
                .iter()
                .cloned()
                .chain(std::iter::once(segment)),
        )
    }

    /// What to call what this path reaches.
    ///
    /// Written the way SQL writes it - `order.line[0].price as price` - and it
    /// answers the one question a selector cannot: a path says which value to
    /// take, and an alias says what the column holding it is called. Without
    /// one, a caller naming a column from a path falls back to the last
    /// segment's own name.
    #[must_use]
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    /// Set or clear what to call what this path reaches.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for an empty alias, or for one on the root
    /// path: the root reaches the value it is applied to, so there is nothing
    /// there for a name to be about. Failure leaves the path unchanged.
    pub fn set_alias(&mut self, alias: Option<&str>) -> Result<()> {
        let Some(alias) = alias else {
            self.alias = None;
            return Ok(());
        };
        if alias.is_empty() {
            return Err(Error::Parse {
                target: TARGET,
                position: 0,
                reason: SmolStr::new_static("expected a name after `as`, got an empty one"),
            });
        }
        if self.is_root() {
            return Err(Error::Parse {
                target: TARGET,
                position: 0,
                reason: SmolStr::new_static(
                    "expected a path to alias, got the root; the root reaches what it is applied to",
                ),
            });
        }
        self.alias = Some(SmolStr::new(alias));
        Ok(())
    }

    /// Return this path with an alias.
    ///
    /// # Errors
    ///
    /// Returns the same refusals as [`Self::set_alias`].
    pub fn try_with_alias(mut self, alias: &str) -> Result<Self> {
        self.set_alias(Some(alias))?;
        Ok(self)
    }

    /// The name this path gives what it reaches.
    ///
    /// The alias where one is written, and the last segment's own name
    /// otherwise. This is what a caller building a column from a path reads,
    /// so the fallback lives here rather than at each call site.
    #[must_use]
    pub fn column_name(&self) -> Option<&str> {
        self.alias()
            .or_else(|| self.last().and_then(FieldSegment::as_name))
    }

    /// The single name this path addresses, when it addresses exactly one.
    ///
    /// The common shape by a wide margin - one column, named - and the one a
    /// caller can answer without walking.
    #[must_use]
    pub fn as_name(&self) -> Option<&str> {
        match self.segments.as_ref() {
            [segment] => segment.as_name(),
            _ => None,
        }
    }

    /// The field this path reaches through `root`.
    ///
    /// Every step is typed by [`FieldSegment::apply_field`], and the result is
    /// named by [`Self::column_name`] - the alias where one is written, the
    /// last segment's own name otherwise, and the path's own text when it ends
    /// on a position that names nothing. The root path answers `root` itself.
    ///
    /// # Errors
    ///
    /// Returns an error naming the step that the schema cannot take.
    pub fn apply_field(&self, root: &Field) -> Result<Field> {
        let mut field = root.clone();
        for segment in self.segments.iter() {
            field = segment.apply_field(&field)?;
        }
        Ok(match self.column_name() {
            Some(name) => field.with_name(SmolStr::new(name)),
            None if self.is_root() => field,
            None => field.with_name(SmolStr::new(self.to_string())),
        })
    }

    /// The value this path reaches through `value`, typed as `root`.
    ///
    /// Absence at any step answers null, the way [`FieldSegment::apply_scalar`]
    /// does, and typing is asked of the schema rather than guessed from the
    /// value, so a struct spelled as a bare sequence is read in schema order.
    ///
    /// # Errors
    ///
    /// Returns an error when a step the schema cannot take is reached.
    pub fn apply_scalar(&self, root: &Field, value: &Scalar) -> Result<Scalar> {
        let mut field = root.clone();
        let mut held = value.clone();
        for segment in self.segments.iter() {
            let next = segment.apply_field(&field)?;
            held = segment.apply_scalar(&field, &held)?;
            field = next;
            if held.is_null() {
                break;
            }
        }
        Ok(held)
    }

    /// A deterministic hash of the complete path.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_segments(formatter, &self.segments)?;
        // An alias never sits on the root, so this never opens the rendering
        // with a space, and parsing it back is the exact inverse.
        if let Some(alias) = &self.alias {
            formatter.write_str(" as ")?;
            write_identifier(formatter, alias)?;
        }
        Ok(())
    }
}

/// Write a run of segments, the leading dot of a first name left off.
///
/// A one-name path renders as that name, which is what every caller writing
/// one spells; a later name keeps its dot so two names never run together.
pub(crate) fn write_segments(
    formatter: &mut fmt::Formatter<'_>,
    segments: &[FieldSegment],
) -> fmt::Result {
    for (index, segment) in segments.iter().enumerate() {
        match segment {
            FieldSegment::Field(name) if index == 0 => write_identifier(formatter, name)?,
            segment => write!(formatter, "{segment}")?,
        }
    }
    Ok(())
}

impl FromIterator<FieldSegment> for FieldPath {
    fn from_iter<I: IntoIterator<Item = FieldSegment>>(segments: I) -> Self {
        Self::new(segments)
    }
}

impl From<FieldSegment> for FieldPath {
    fn from(segment: FieldSegment) -> Self {
        Self::new([segment])
    }
}

impl AsRef<[FieldSegment]> for FieldPath {
    fn as_ref(&self) -> &[FieldSegment] {
        &self.segments
    }
}

impl<'a> IntoIterator for &'a FieldPath {
    type Item = &'a FieldSegment;
    type IntoIter = std::slice::Iter<'a, FieldSegment>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments.iter()
    }
}

impl FromStr for FieldPath {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        super::parser::parse_field_path(value)
    }
}

impl ::serde::Serialize for FieldPath {
    fn serialize<S: ::serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        // The canonical text, because that is the spelling every catalog,
        // metadata map and configuration file this path travels through
        // already holds.
        serializer.collect_str(self)
    }
}

impl<'de> ::serde::Deserialize<'de> for FieldPath {
    fn deserialize<D: ::serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let text = SmolStr::deserialize(deserializer)?;
        Self::from_str(&text).map_err(::serde::de::Error::custom)
    }
}

/// Write one identifier, quoting it only when the bare spelling would not come
/// back as itself.
pub(crate) fn write_identifier(formatter: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    if super::display::is_bare_identifier(name) {
        return formatter.write_str(name);
    }
    formatter.write_char('"')?;
    for character in name.chars() {
        if character == '"' {
            formatter.write_char('"')?;
        }
        formatter.write_char(character)?;
    }
    formatter.write_char('"')
}

/// Write one map key inside its brackets.
///
/// Total by construction: [`FieldSegment::key`] admits only the kinds this
/// writes, so rendering a path is always the exact inverse of parsing one.
fn write_key(formatter: &mut fmt::Formatter<'_>, key: &Literal) -> fmt::Result {
    if let Some(text) = key.value().as_str() {
        return super::display::write_text_literal(formatter, text);
    }
    formatter.write_str("''")
}

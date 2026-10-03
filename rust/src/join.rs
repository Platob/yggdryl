//! Joins: two series - held, chunked or streamed - matched on key terms,
//! through the crate's own `Term`, `Filter` and `Selector` vocabulary.
//!
//! [`JoinKind`] is DuckDB's six kinds, [`JoinOptions`] the facts beside
//! them, [`SerieSource`] what either side may be, and the engine here is what
//! [`Serie::join_with`], [`ChunkedSerie::join_with`] and
//! [`SerieReader::join_with`] run: the keys bound once against each side's
//! root and cast to their common datatype, the build side held - every chunk
//! settled under the spill bound - and hashed through Arrow's row format
//! where the keys ride it and through the values' own equality where they do
//! not, the probe side read one batch at a time, every output batch of at
//! most [`DEFAULT_RECORD_BATCH_ROW_SIZE`] rows settled as it is produced.
//! Nothing of the probe is collected: a stream probe holds one batch. Two
//! exceptions are stated where they are chosen: a probe batch whose keys all
//! fall outside the build keys' range is never hashed
//! ([`JoinOptions::prune`]), and a build side past the spill bound is joined
//! in partitions, the probe read whole ([`JoinOptions::spill`]).
//!
//! # Semantics
//!
//! A key row with an absent cell matches nothing on either side. Duplicates
//! multiply: every build match is emitted, in build order within a probe
//! row. `Semi` and `Anti` emit the left rows with and without a match and no
//! right column. `Left`, `Right` and `Full` emit the unmatched rows of the
//! named side with the other side's columns null. The output root is a
//! required record named after the left root whose children are the left
//! columns, then the right columns, a key stated as one bare column on both
//! sides appearing once under the left name when `coalesce` is on (left value
//! else right), a right column whose name collides with a left one taking
//! `suffix`, a column of a side the kind makes optional nullable, every
//! column keeping its own field and metadata otherwise. Rows come in probe
//! order, then the unmatched build rows in build order - partition by
//! partition where the build side is partitioned.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, RecordBatch, StructArray, UInt32Array, new_null_array,
};
use arrow_buffer::BooleanBufferBuilder;
use arrow_row::{Row, RowConverter, Rows, SortField};
use arrow_schema::{ArrowError, Fields};
use smol_str::{SmolStr, format_smolstr};

use crate::expression::{Bound, IntoJoinKeys, JoinKey, JoinKeys, Term};
use crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE;
use crate::serie::{Proof, land};
use crate::spill::SpillOptions;
use crate::{
    ChunkedSerie, DataType, Error, Field, Result, Scalar, Serie, SerieReader, SerieSource,
    StructType,
};

/// The suffix a right column takes when its name collides with a left one.
pub const DEFAULT_JOIN_SUFFIX: &str = "_right";

/// The largest distinct build key set pushed into a probe source's filter.
pub const DEFAULT_PUSHDOWN_KEYS: usize = 10_000;

/// Which rows a join keeps: DuckDB's words.
///
/// ```
/// use yggdryl::JoinKind;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!("outer".parse::<JoinKind>()?, JoinKind::Full);
/// assert_eq!("LEFT OUTER".parse::<JoinKind>()?, JoinKind::Left);
/// assert_eq!(JoinKind::Semi.to_string(), "semi");
/// assert!("cross".parse::<JoinKind>().is_err());
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum JoinKind {
    /// Rows whose keys match on both sides.
    Inner,
    /// Every left row, the right columns null where no right row matches.
    Left,
    /// Every right row, the left columns null where no left row matches.
    Right,
    /// Every row of both sides, the other side's columns null where nothing
    /// matches.
    Full,
    /// The left rows with a match, left columns only.
    Semi,
    /// The left rows without a match, left columns only.
    Anti,
}

impl JoinKind {
    /// Every kind, in canonical spelling.
    pub const ALL: [Self; 6] = [
        Self::Inner,
        Self::Left,
        Self::Right,
        Self::Full,
        Self::Semi,
        Self::Anti,
    ];

    /// The canonical word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inner => "inner",
            Self::Left => "left",
            Self::Right => "right",
            Self::Full => "full",
            Self::Semi => "semi",
            Self::Anti => "anti",
        }
    }

    /// Whether an unmatched left row is emitted.
    #[must_use]
    pub const fn keeps_unmatched_left(self) -> bool {
        matches!(self, Self::Left | Self::Full | Self::Anti)
    }

    /// Whether an unmatched right row is emitted.
    #[must_use]
    pub const fn keeps_unmatched_right(self) -> bool {
        matches!(self, Self::Right | Self::Full)
    }

    /// Whether the output carries the right side's columns.
    #[must_use]
    pub const fn emits_right(self) -> bool {
        !matches!(self, Self::Semi | Self::Anti)
    }

    /// Whether a matched left row is emitted once at most, the right side
    /// deciding only whether it is: the two filtering kinds.
    #[must_use]
    pub const fn is_filtering(self) -> bool {
        matches!(self, Self::Semi | Self::Anti)
    }
}

impl fmt::Display for JoinKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for JoinKind {
    type Err = Error;

    /// The six words, case folded, `outer` read as `full`, a trailing `outer`
    /// or `join` ignored: `left outer join` is `left`.
    fn from_str(text: &str) -> Result<Self> {
        let folded = text.trim().to_ascii_lowercase();
        let mut words: Vec<&str> = folded.split_whitespace().collect();
        while matches!(words.last(), Some(&"join" | &"outer")) && words.len() > 1 {
            words.pop();
        }
        match words.as_slice() {
            ["inner"] => Ok(Self::Inner),
            ["left"] => Ok(Self::Left),
            ["right"] => Ok(Self::Right),
            ["full" | "outer"] => Ok(Self::Full),
            ["semi"] => Ok(Self::Semi),
            ["anti"] => Ok(Self::Anti),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.how"),
                reason: crate::text::expected_got(
                    "one of `inner`, `left`, `right`, `full` (or `outer`), `semi`, `anti`",
                    format_args!("{text:?}"),
                ),
            }),
        }
    }
}

/// One side of a join.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum JoinSide {
    /// The side `join_with` is called on.
    Left,
    /// The side handed to it.
    Right,
}

impl JoinSide {
    /// The other side.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// The canonical word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

impl fmt::Display for JoinSide {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for JoinSide {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.build"),
                reason: crate::text::expected_got("`left` or `right`", format_args!("{text:?}")),
            }),
        }
    }
}

/// The facts beside a join's keys and kind.
///
/// ```
/// use yggdryl::{JoinOptions, JoinSide};
///
/// let options = JoinOptions::new().with_suffix("_r").with_build(Some(JoinSide::Left));
/// assert!(options.coalesce());
/// assert_eq!(options.suffix(), "_r");
/// assert_eq!(options.build(), Some(JoinSide::Left));
/// assert!(options.prune());
/// assert!(options.spill().is_none());
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinOptions {
    coalesce: bool,
    suffix: SmolStr,
    build: Option<JoinSide>,
    prune: bool,
    spill: Option<SpillOptions>,
    pushdown_keys: usize,
}

impl JoinOptions {
    /// Coalesce the keys, suffix a collision `_right`, pick the build side,
    /// prune, settle under the process default, push up to
    /// [`DEFAULT_PUSHDOWN_KEYS`] keys down.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            coalesce: true,
            suffix: SmolStr::new_static(DEFAULT_JOIN_SUFFIX),
            build: None,
            prune: true,
            spill: None,
            pushdown_keys: DEFAULT_PUSHDOWN_KEYS,
        }
    }

    /// Whether a key stated as the same bare column on both sides appears
    /// once, under the left name, left value else right.
    #[must_use]
    pub const fn coalesce(&self) -> bool {
        self.coalesce
    }

    /// The same options, coalescing or not.
    #[must_use]
    pub const fn with_coalesce(mut self, coalesce: bool) -> Self {
        self.coalesce = coalesce;
        self
    }

    /// What a right column whose name collides with a left one is suffixed
    /// with.
    #[must_use]
    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    /// The same options under another suffix.
    #[must_use]
    pub fn with_suffix(mut self, suffix: impl Into<SmolStr>) -> Self {
        self.suffix = suffix.into();
        self
    }

    /// Which side is held and hashed: `None` picks the held side over a
    /// stream, else the smaller by [`Serie::memory_size`], else the right.
    #[must_use]
    pub const fn build(&self) -> Option<JoinSide> {
        self.build
    }

    /// The same options holding `build`.
    #[must_use]
    pub const fn with_build(mut self, build: Option<JoinSide>) -> Self {
        self.build = build;
        self
    }

    /// Whether probe rows and batches the build keys cannot match are
    /// dropped before they are hashed: the probe source filtered by the
    /// build keys where a plan pushes them down, and, on the row format, a
    /// probe batch whose present keys all fall outside the range of the
    /// build keys - absent keys included - answered without hashing a row,
    /// standing alone where the kind keeps its unmatched rows. The rows a
    /// join answers are the same either way, in the same order.
    #[must_use]
    pub const fn prune(&self) -> bool {
        self.prune
    }

    /// The same options, pruning or not.
    #[must_use]
    pub const fn with_prune(mut self, prune: bool) -> Self {
        self.prune = prune;
        self
    }

    /// The bound the build side and every output batch settle under, `None`
    /// for the process default ([`SpillOptions::from_env`]).
    ///
    /// A build side keyed on the row format whose chunks hold more than the
    /// bound in all ([`Serie::memory_size`]) is joined in partitions - a
    /// grace hash join, one level. Both sides are scattered by the hash of
    /// their key rows, an absent key into the first partition, over
    /// `ceil(build bytes / bound)` partitions rounded up to a power of two,
    /// at least two and at most 64; a partition's rows are written to a
    /// spill file as they fill, so the pieces waiting across one side's
    /// partitions hold no more than the bound - or 64 KiB a partition where
    /// the bound's share is smaller. Then one partition is joined at a time,
    /// its rows settled under the bound, one hash table held.
    ///
    /// The probe is read whole before the first output batch - the bound
    /// says the build side does not fit, so the probe cannot be streamed
    /// past it - and the rows come partition by partition: the same rows a
    /// join held whole answers, in another order. A partition is not split
    /// again, so one key heavier than the bound is hashed whole. Keys hashed
    /// through their values' own equality have no partitioned path: their
    /// build side is hashed whole, whatever it weighs.
    #[must_use]
    pub const fn spill(&self) -> Option<&SpillOptions> {
        self.spill.as_ref()
    }

    /// The same options settling under `spill`.
    #[must_use]
    pub fn with_spill(mut self, spill: SpillOptions) -> Self {
        self.spill = Some(spill);
        self
    }

    /// The largest distinct build key set pushed into a probe source's
    /// filter, and the largest keyed `in` list a probe batch is pruned by.
    #[must_use]
    pub const fn pushdown_keys(&self) -> usize {
        self.pushdown_keys
    }

    /// The same options pushing up to `keys` keys down.
    #[must_use]
    pub const fn with_pushdown_keys(mut self, keys: usize) -> Self {
        self.pushdown_keys = keys;
        self
    }

    /// The bound these options settle under: the stated one, else the
    /// process default.
    fn spill_bound(&self) -> Result<&SpillOptions> {
        match &self.spill {
            Some(options) => Ok(options),
            None => SpillOptions::from_env(),
        }
    }

    /// Settle `serie` under these options' bound.
    pub(crate) fn settle(&self, mut serie: Serie) -> Result<Serie> {
        serie.spill(self.spill_bound()?)?;
        Ok(serie)
    }
}

impl Default for JoinOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Where one output column comes from.
#[derive(Clone, Copy, Debug)]
enum Column {
    /// Left child `i`.
    Left(usize),
    /// Right child `j`.
    Right(usize),
    /// Left child `i`, else right child `j`: a coalesced key.
    Coalesced(usize, usize),
}

/// One key, bound against both roots and cast to the datatype the two
/// sides share.
struct BoundKey {
    left: Bound,
    right: Bound,
    /// Whether a side was cast to the datatype the two share: a cast key
    /// may order otherwise than the column it was read from, so a merge
    /// trusts no declared order over it.
    cast: bool,
}

/// The join resolved once: the roots, the keys, the output and the sides.
pub(crate) struct JoinPlan {
    how: JoinKind,
    options: JoinOptions,
    left_root: Arc<Field>,
    right_root: Arc<Field>,
    keys: Vec<BoundKey>,
    output: Arc<Field>,
    columns: Vec<Column>,
    build: JoinSide,
    rung: KeyRung,
    /// Whether the two sides are merged rather than hashed: both roots
    /// declare the key order ascending, every key the same datatype on both
    /// sides, and the keys on the row-format rung - so the probe rows walk
    /// the build rows with one cursor and no table is built. Decided by
    /// [`Self::run`], which holds the sources.
    merge: bool,
}

/// How key rows are hashed and equated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyRung {
    /// Arrow's row format over the key buffers: bytes that equate as the
    /// values do.
    Buffers,
    /// The values' own equality, each key row built once.
    Values,
}

impl KeyRung {
    /// The word `explain` states.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Buffers => "row format",
            Self::Values => "values",
        }
    }
}

fn refuse(path: &str, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new(path),
        reason,
    }
}

/// The record root a side's rows are joined under, and the first-level
/// child fields it carries.
fn children(root: &Field) -> &[Field] {
    root.fields()
}

/// The Arrow fields a record root projects to.
fn struct_fields(root: &Field) -> Result<Fields> {
    match root.as_arrow_field_ref()?.data_type() {
        arrow_schema::DataType::Struct(fields) => Ok(fields.clone()),
        other => Err(refuse(
            root.name(),
            format_smolstr!("a record root projects to a struct, got {other}"),
        )),
    }
}

impl JoinPlan {
    /// Resolve a join: bind every key against its side's root, cast each
    /// pair to its common datatype, lay the output root out, and pick the
    /// build side.
    pub(crate) fn compile(
        left_root: Field,
        right_root: Field,
        keys: &JoinKeys,
        how: JoinKind,
        options: &JoinOptions,
        left_held: Option<usize>,
        right_held: Option<usize>,
    ) -> Result<Self> {
        if keys.is_empty() {
            return Err(refuse(
                left_root.name(),
                SmolStr::new_static("expected at least one key to join on, got an empty key list"),
            ));
        }
        let mut bound = Vec::with_capacity(keys.len());
        for key in keys {
            bound.push(bind_key(key, &left_root, &right_root)?);
        }
        let rung = key_rung(&bound)?;
        let (output, columns) = output_root(&left_root, &right_root, keys, how, options)?;
        let build = options.build.unwrap_or(match (left_held, right_held) {
            (Some(_), None) => JoinSide::Left,
            (None, Some(_)) => JoinSide::Right,
            (Some(left), Some(right)) if left < right => JoinSide::Left,
            _ => JoinSide::Right,
        });
        Ok(Self {
            how,
            options: options.clone(),
            left_root: Arc::new(left_root),
            right_root: Arc::new(right_root),
            keys: bound,
            output: Arc::new(output),
            columns,
            build,
            rung,
            merge: false,
        })
    }

    /// Whether both sides prove the key order the merge walks: each root's
    /// declared order ([`Serie::declared_order`]) opening with the join keys
    /// of its side, every key ascending, no key cast, and the row-format
    /// rung. A declaring stream counts: its batches are verified in order.
    fn merges(&self, left: &SerieSource, right: &SerieSource) -> Result<bool> {
        if self.rung != KeyRung::Buffers || self.keys.iter().any(|key| key.cast) {
            return Ok(false);
        }
        let declares = |source: &SerieSource, side: JoinSide| -> Result<bool> {
            let Some(declared) = source.declared_order()? else {
                return Ok(false);
            };
            if declared.len() < self.keys.len() {
                return Ok(false);
            }
            Ok(self.keys.iter().zip(&declared).all(|(key, ordering)| {
                let term = match side {
                    JoinSide::Left => key.left.term(),
                    JoinSide::Right => key.right.term(),
                };
                !ordering.is_descending() && ordering.term() == term
            }))
        };
        Ok(declares(left, JoinSide::Left)? && declares(right, JoinSide::Right)?)
    }

    /// The record every output row is typed by.
    pub(crate) fn output(&self) -> &Arc<Field> {
        &self.output
    }

    /// Which side is held and hashed.
    pub(crate) const fn build_side(&self) -> JoinSide {
        self.build
    }

    /// Which rung the keys are hashed on.
    pub(crate) const fn rung(&self) -> KeyRung {
        self.rung
    }

    /// The key arrays of one record batch of `side`, each cast to the
    /// common datatype, in key order.
    fn key_arrays(&self, side: JoinSide, batch: &RecordBatch) -> Result<Vec<ArrayRef>> {
        self.keys
            .iter()
            .map(|key| {
                let bound = match side {
                    JoinSide::Left => &key.left,
                    JoinSide::Right => &key.right,
                };
                bound.evaluate(batch).map_err(Into::into)
            })
            .collect()
    }

    /// The converter the key rows are read through on the row format rung.
    fn converter(&self) -> Result<RowConverter> {
        RowConverter::new(sort_fields(&self.keys)?).map_err(arrow_error)
    }

    /// Run the join: the build side held and hashed - partitioned where it
    /// passes the spill bound - the probe side read batch by batch, the
    /// output one record column per batch.
    pub(crate) fn run(mut self, left: SerieSource, right: SerieSource) -> Result<JoinOutput> {
        self.merge = self.merges(&left, &right)?;
        let (build, probe) = match self.build {
            JoinSide::Left => (left, right),
            JoinSide::Right => (right, left),
        };
        let plan = Arc::new(self);
        let mut chunks = Vec::new();
        for chunk in build.into_reader()? {
            let chunk = plan.options.settle(chunk?)?;
            if !chunk.is_empty() {
                chunks.push(chunk);
            }
        }
        let probe = probe.into_reader()?;
        let partitions = plan.partitions(&chunks)?;
        let (table, probe, grace) = if partitions == 1 {
            (
                BuildTable::new(&plan, chunks)?,
                Some(Probe::Stream(probe)),
                None,
            )
        } else {
            let grace = Grace::new(&plan, partitions, chunks, probe)?;
            (BuildTable::new(&plan, Vec::new())?, None, Some(grace))
        };
        Ok(JoinOutput {
            plan,
            table,
            probe,
            pending: VecDeque::new(),
            grace,
        })
    }

    /// How many partitions the build side is joined in: one where its chunks
    /// hold no more than the spill bound or its keys are hashed through
    /// their values, else the bound's share of them rounded up to a power of
    /// two, between two and [`MAX_GRACE_PARTITIONS`].
    fn partitions(&self, chunks: &[Serie]) -> Result<usize> {
        let bound = self.options.spill_bound()?;
        // A merge walks both sides in their order, which a scatter would lose.
        if self.rung != KeyRung::Buffers || self.merge || bound.is_never() {
            return Ok(1);
        }
        let held = chunks
            .iter()
            .map(|chunk| u64::try_from(chunk.memory_size()).unwrap_or(u64::MAX))
            .fold(0_u64, u64::saturating_add);
        if held <= bound.byte_size() {
            return Ok(1);
        }
        let share = held
            .div_ceil(bound.byte_size().max(1))
            .min(MAX_GRACE_PARTITIONS as u64);
        Ok(usize::try_from(share)
            .unwrap_or(MAX_GRACE_PARTITIONS)
            .next_power_of_two()
            .clamp(2, MAX_GRACE_PARTITIONS))
    }
}

/// Bind one key against both roots and cast each side to their common
/// datatype, refusing a pair that shares none by naming both.
fn bind_key(key: &JoinKey, left_root: &Field, right_root: &Field) -> Result<BoundKey> {
    let left = key.left().bind(left_root)?;
    let right = key.right().bind(right_root)?;
    let (left_dtype, right_dtype) = (left.field().dtype(), right.field().dtype());
    if left_dtype == right_dtype {
        return Ok(BoundKey {
            left,
            right,
            cast: false,
        });
    }
    let Some(common) = crate::expression::common_type(left_dtype, right_dtype) else {
        return Err(refuse(
            left_root.name(),
            format_smolstr!(
                "join key `{key}` compares {left_dtype} with {right_dtype}, which share no datatype"
            ),
        ));
    };
    let left = if *left_dtype == common {
        left
    } else {
        key.left().clone().cast(common.clone()).bind(left_root)?
    };
    let right = if *right_dtype == common {
        right
    } else {
        key.right().clone().cast(common).bind(right_root)?
    };
    Ok(BoundKey {
        left,
        right,
        cast: true,
    })
}

/// The rung every key of a join is hashed on: the row format where every
/// key's buffers equate as its values do and Arrow's row format has one for
/// its layout, the values' own equality otherwise.
fn key_rung(keys: &[BoundKey]) -> Result<KeyRung> {
    if keys
        .iter()
        .any(|key| !crate::serie::stored_order_is_value_order(key.left.field().dtype()))
    {
        return Ok(KeyRung::Values);
    }
    if RowConverter::supports_fields(&sort_fields(keys)?) {
        Ok(KeyRung::Buffers)
    } else {
        Ok(KeyRung::Values)
    }
}

/// The row format's field for every key, at the datatype both sides share.
fn sort_fields(keys: &[BoundKey]) -> Result<Vec<SortField>> {
    keys.iter()
        .map(|key| {
            Ok(SortField::new(
                key.left.field().as_arrow_field_ref()?.data_type().clone(),
            ))
        })
        .collect()
}

/// The output root and where each of its columns comes from.
fn output_root(
    left_root: &Field,
    right_root: &Field,
    keys: &JoinKeys,
    how: JoinKind,
    options: &JoinOptions,
) -> Result<(Field, Vec<Column>)> {
    let left = children(left_root);
    let right = children(right_root);
    if how.is_filtering() {
        let columns = (0..left.len()).map(Column::Left).collect();
        return Ok((left_root.clone().with_nullable(false), columns));
    }
    // A key stated as one bare column on both sides: left index, right index.
    let mut coalesced: Vec<(usize, usize)> = Vec::new();
    if options.coalesce {
        for key in keys {
            if let Some(name) = key.using_column()
                && let (Some(at_left), Some(at_right)) =
                    (left_root.index_of(name), right_root.index_of(name))
                && !coalesced.iter().any(|(held, _)| *held == at_left)
            {
                coalesced.push((at_left, at_right));
            }
        }
    }
    let left_optional = how.keeps_unmatched_right();
    let right_optional = how.keeps_unmatched_left();
    let mut fields = Vec::with_capacity(left.len() + right.len());
    let mut columns = Vec::with_capacity(left.len() + right.len());
    for (index, field) in left.iter().enumerate() {
        match coalesced.iter().find(|(held, _)| *held == index) {
            Some((_, at_right)) => {
                let nullable = field.is_nullable() || right[*at_right].is_nullable();
                fields.push(field.clone().with_nullable(nullable));
                columns.push(Column::Coalesced(index, *at_right));
            }
            None => {
                fields.push(
                    field
                        .clone()
                        .with_nullable(field.is_nullable() || left_optional),
                );
                columns.push(Column::Left(index));
            }
        }
    }
    for (index, field) in right.iter().enumerate() {
        if coalesced.iter().any(|(_, held)| *held == index) {
            continue;
        }
        let mut field = field
            .clone()
            .with_nullable(field.is_nullable() || right_optional);
        if fields.iter().any(|held| held.name() == field.name()) {
            let name = format_smolstr!("{}{}", field.name(), options.suffix());
            field = field.with_name(name);
        }
        fields.push(field);
        columns.push(Column::Right(index));
    }
    let dtype = DataType::from(StructType::from_fields(fields)?);
    let mut output = Field::new(left_root.name(), dtype, false);
    // The output carries no declaration the join did not prove: the roots'
    // own metadata - a `SORT:by`, a partition - describes rows this join
    // reordered or widened.
    output.set_metadata(std::iter::empty::<(&str, &str)>())?;
    Ok((output, columns))
}

/// One build row: its chunk and its row in it.
type BuildRef = (u32, u32);

/// The most partitions a build side past the spill bound is joined in.
const MAX_GRACE_PARTITIONS: usize = 64;

/// The fewest bytes a waiting partition's pieces are written to disk in:
/// one stream batch, so a small bound writes no file - and holds no
/// descriptor - per piece.
const MIN_SPOOL_BYTE_SIZE: usize = crate::DEFAULT_STREAM_BATCH_SIZE;

/// No next node or group: the end of a chain.
const END: u32 = u32::MAX;

/// One distinct key's group: the row the key is read at, the first and the
/// last node of its rows, and the next group hashed into the same bucket.
struct GroupHead {
    first: BuildRef,
    head: u32,
    tail: u32,
    next_group: u32,
}

/// The build rows grouped by key, as chains rather than a vector per key:
/// one node per present build row, in build order, pointing at the next row
/// of its key, and one head per distinct key. A build grows two vectors and
/// its map, so a side of unique keys allocates nothing per row.
struct Chains {
    groups: Vec<GroupHead>,
    nodes: Vec<(BuildRef, u32)>,
}

impl Chains {
    fn new() -> Self {
        Self {
            groups: Vec::new(),
            nodes: Vec::new(),
        }
    }

    /// Open the group of a new key read at `at`, chained before
    /// `next_group` in its bucket, answering the group.
    fn open(&mut self, at: BuildRef, next_group: u32) -> u32 {
        let node = self.nodes.len() as u32;
        self.nodes.push((at, END));
        self.groups.push(GroupHead {
            first: at,
            head: node,
            tail: node,
            next_group,
        });
        (self.groups.len() - 1) as u32
    }

    /// Append the row `at`, of a key already grouped, to `group`.
    fn push(&mut self, group: u32, at: BuildRef) {
        let node = self.nodes.len() as u32;
        self.nodes.push((at, END));
        let head = &mut self.groups[group as usize];
        self.nodes[head.tail as usize].1 = node;
        head.tail = node;
    }

    /// The rows of `group`, in build order; none for `None`.
    fn rows_of(&self, group: Option<u32>) -> Matches<'_> {
        Matches::Chain {
            nodes: &self.nodes,
            next: group.map_or(END, |group| self.groups[group as usize].head),
        }
    }
}

/// The build rows one probe row matches: walked off the chains, or - on a
/// merge - the run of build rows equal to the probe key from the cursor on,
/// across chunk edges.
#[derive(Clone, Copy)]
enum Matches<'a> {
    Chain {
        nodes: &'a [(BuildRef, u32)],
        next: u32,
    },
    Run {
        rows: &'a [Rows],
        key: Row<'a>,
        chunk: u32,
        at: u32,
    },
}

impl Matches<'_> {
    const NONE: Matches<'static> = Matches::Chain {
        nodes: &[],
        next: END,
    };

    fn is_empty(&self) -> bool {
        match *self {
            Self::Chain { next, .. } => next == END,
            Self::Run {
                rows,
                key,
                chunk,
                at,
                ..
            } => rows.get(chunk as usize).is_none_or(|current| {
                at as usize >= current.num_rows() || current.row(at as usize) != key
            }),
        }
    }
}

impl Iterator for Matches<'_> {
    type Item = BuildRef;

    fn next(&mut self) -> Option<BuildRef> {
        match self {
            Self::Chain { nodes, next } => {
                let (at, following) = *nodes.get(*next as usize)?;
                *next = following;
                Some(at)
            }
            Self::Run {
                rows,
                key,
                chunk,
                at,
            } => loop {
                let current = rows.get(*chunk as usize)?;
                if *at as usize >= current.num_rows() {
                    *chunk += 1;
                    *at = 0;
                    continue;
                }
                if current.row(*at as usize) != *key {
                    return None;
                }
                let found = (*chunk, *at);
                *at += 1;
                return Some(found);
            },
        }
    }
}

/// The least and the greatest present build key row, in the row format's
/// byte order. Rows match by their bytes, so a probe row outside the range
/// equals no build row, whatever the values' own order.
struct KeyRange {
    /// Where the least key is read, in the build's own key rows.
    min: BuildRef,
    /// Where the greatest key is read.
    max: BuildRef,
}

impl KeyRange {
    const fn new(at: BuildRef) -> Self {
        Self { min: at, max: at }
    }

    /// Widen the range to hold `row`, the key read at `at`: the range
    /// names rows the table already holds, so widening copies nothing.
    fn widen(&mut self, row: Row<'_>, at: BuildRef, rows: &[Rows], current: &Rows) {
        if row < rows_at(rows, current, self.min) {
            self.min = at;
        } else if row > rows_at(rows, current, self.max) {
            self.max = at;
        }
    }

    /// Whether `row` lies between the two, `rows` being every chunk's keys.
    fn contains(&self, row: Row<'_>, rows: &[Rows]) -> bool {
        let read = |at: BuildRef| rows[at.0 as usize].row(at.1 as usize);
        read(self.min) <= row && row <= read(self.max)
    }
}

/// The held side - or one grace partition of it - hashed by key.
struct BuildTable {
    /// The record chunks, each landed and settled.
    chunks: Vec<Serie>,
    /// Each chunk as the batch its columns are read from.
    batches: Vec<RecordBatch>,
    keys: KeyTable,
    /// One bit per build row, set where a probe row matched it: kept only
    /// where the kind emits the build side's unmatched rows or filters by
    /// them.
    matched: Option<Vec<BooleanBufferBuilder>>,
}

/// The key table of one rung.
enum KeyTable {
    Buffers {
        converter: RowConverter,
        rows: Vec<Rows>,
        /// The first group under one hash of the row bytes, the rest chained
        /// behind it.
        map: HashMap<u64, u32>,
        chains: Chains,
        /// The range of the present build keys, `None` where there is none:
        /// what a probe batch is pruned by.
        range: Option<KeyRange>,
    },
    Values {
        // `Scalar`'s hash reads canonical content only, never the
        // interior-mutable caches a datatype holds, so the key is stable.
        #[allow(clippy::mutable_key_type)]
        map: HashMap<Scalar, u32>,
        chains: Chains,
    },
    /// A merge: the build keys in their declared order, no table, and the
    /// cursor the probe rows - arriving in theirs - walk them with, never
    /// moving back.
    Sorted {
        converter: RowConverter,
        rows: Vec<Rows>,
        /// Each chunk's key arrays, read for the absent build keys the
        /// cursor steps over.
        arrays: Vec<Vec<ArrayRef>>,
        cursor: std::cell::Cell<BuildRef>,
    },
}

impl KeyTable {
    /// The build rows matching row `index` of a probe batch whose keys
    /// were read into `probe`.
    fn matches<'a>(&'a self, probe: &'a ProbeKeys, index: usize) -> Matches<'a> {
        match (self, probe) {
            (
                Self::Buffers {
                    rows, map, chains, ..
                },
                ProbeKeys::Buffers(converted),
            ) => {
                let row = converted.row(index);
                let mut cursor = map.get(&hashed(row.data())).copied();
                while let Some(group) = cursor {
                    let head = &chains.groups[group as usize];
                    if rows[head.first.0 as usize].row(head.first.1 as usize) == row {
                        return chains.rows_of(Some(group));
                    }
                    cursor = (head.next_group != END).then_some(head.next_group);
                }
                Matches::NONE
            }
            (Self::Values { map, chains }, ProbeKeys::Values(record)) => chains.rows_of(
                record
                    .scalar(index)
                    .ok()
                    .and_then(|key| map.get(&key).copied()),
            ),
            (
                Self::Sorted {
                    rows,
                    arrays,
                    cursor,
                    ..
                },
                ProbeKeys::Buffers(converted),
            ) => {
                let key = converted.row(index);
                // Step over the build rows the probe has passed - a lesser key,
                // or an absent one, which matches nothing - and stay on the
                // first row equal to or greater than the key, so a probe row
                // of the same key reads the same run.
                let (mut chunk, mut at) = cursor.get();
                while let Some(current) = rows.get(chunk as usize) {
                    if at as usize >= current.num_rows() {
                        chunk += 1;
                        at = 0;
                        continue;
                    }
                    if any_null(&arrays[chunk as usize], at as usize)
                        || current.row(at as usize) < key
                    {
                        at += 1;
                        continue;
                    }
                    break;
                }
                cursor.set((chunk, at));
                Matches::Run {
                    rows,
                    key,
                    chunk,
                    at,
                }
            }
            _ => Matches::NONE,
        }
    }
}

/// Mark build row `at` matched, where the kind tracks matches.
fn mark(matched: &mut Option<Vec<BooleanBufferBuilder>>, at: BuildRef) {
    if let Some(matched) = matched {
        matched[at.0 as usize].set_bit(at.1 as usize, true);
    }
}

/// An Arrow kernel's refusal, as the crate's error.
fn arrow_error(error: ArrowError) -> Error {
    Error::from(crate::arrow::Error::Arrow(error))
}

/// The hash of one key row's bytes.
fn hashed(bytes: &[u8]) -> u64 {
    twox_hash::XxHash3_64::oneshot(bytes)
}

/// The grace partition, of `count`, a present key row falls in.
fn partition_of(row: Row<'_>, count: usize) -> usize {
    (hashed(row.data()) % count as u64) as usize
}

/// Whether row `index` of any key array is absent: such a row matches
/// nothing.
fn any_null(keys: &[ArrayRef], index: usize) -> bool {
    keys.iter().any(|key| key.is_null(index))
}

/// The key record column of one batch, under a root named `key`.
fn key_record(keys: &[ArrayRef], plan: &JoinPlan, rows: usize) -> Result<Serie> {
    let fields: Vec<Field> = plan
        .keys
        .iter()
        .enumerate()
        .map(|(index, key)| {
            key.left
                .field()
                .clone()
                .with_name(format_smolstr!("k{index}"))
                .with_nullable(true)
        })
        .collect();
    let root = Field::new(
        "key",
        DataType::from(StructType::from_fields(fields)?),
        false,
    );
    let arrow_fields = struct_fields(&root)?;
    let array = StructArray::try_new_with_length(arrow_fields, keys.to_vec(), None, rows)
        .map_err(arrow_error)?;
    Ok(land(Arc::new(root), Arc::new(array), &Proof::Unproven)?)
}

impl BuildTable {
    /// Hash `chunks` - the build side's record chunks, or one grace
    /// partition's - each settled and none empty.
    fn new(plan: &JoinPlan, chunks: Vec<Serie>) -> Result<Self> {
        let side = plan.build;
        let batches = chunks
            .iter()
            .map(Serie::into_arrow_batch)
            .collect::<crate::arrow::Result<Vec<_>>>()?;
        let tracks = matches!(
            (plan.how, side),
            (JoinKind::Full, _)
                | (
                    JoinKind::Left | JoinKind::Semi | JoinKind::Anti,
                    JoinSide::Left
                )
                | (JoinKind::Right, JoinSide::Right)
        );
        let matched = tracks.then(|| {
            chunks
                .iter()
                .map(|chunk| {
                    let mut bits = BooleanBufferBuilder::new(chunk.len());
                    bits.append_n(chunk.len(), false);
                    bits
                })
                .collect()
        });
        let keys = match plan.rung {
            KeyRung::Buffers if plan.merge => {
                let converter = plan.converter()?;
                let mut rows = Vec::with_capacity(batches.len());
                let mut arrays = Vec::with_capacity(batches.len());
                for batch in &batches {
                    let keyed = plan.key_arrays(side, batch)?;
                    rows.push(converter.convert_columns(&keyed).map_err(arrow_error)?);
                    arrays.push(keyed);
                }
                KeyTable::Sorted {
                    converter,
                    rows,
                    arrays,
                    cursor: std::cell::Cell::new((0, 0)),
                }
            }
            KeyRung::Buffers => {
                let converter = plan.converter()?;
                let mut rows = Vec::with_capacity(batches.len());
                let mut map: HashMap<u64, u32> = HashMap::new();
                let mut chains = Chains::new();
                let mut range: Option<KeyRange> = None;
                for (chunk, batch) in batches.iter().enumerate() {
                    let arrays = plan.key_arrays(side, batch)?;
                    let converted = converter.convert_columns(&arrays).map_err(arrow_error)?;
                    for index in 0..batch.num_rows() {
                        if any_null(&arrays, index) {
                            continue;
                        }
                        let row = converted.row(index);
                        let at = (chunk as u32, index as u32);
                        let hash = hashed(row.data());
                        let bucket = map.get(&hash).copied();
                        let mut cursor = bucket;
                        let mut found = None;
                        while let Some(group) = cursor {
                            let head = &chains.groups[group as usize];
                            if rows_at(&rows, &converted, head.first) == row {
                                found = Some(group);
                                break;
                            }
                            cursor = (head.next_group != END).then_some(head.next_group);
                        }
                        match found {
                            Some(group) => chains.push(group, at),
                            None => {
                                // A new key is the only row that can widen
                                // the range.
                                range = Some(match range.take() {
                                    Some(mut range) => {
                                        range.widen(row, at, &rows, &converted);
                                        range
                                    }
                                    None => KeyRange::new(at),
                                });
                                let group = chains.open(at, bucket.unwrap_or(END));
                                map.insert(hash, group);
                            }
                        }
                    }
                    rows.push(converted);
                }
                KeyTable::Buffers {
                    converter,
                    rows,
                    map,
                    chains,
                    range,
                }
            }
            KeyRung::Values => {
                #[allow(clippy::mutable_key_type)]
                let mut map: HashMap<Scalar, u32> = HashMap::new();
                let mut chains = Chains::new();
                for (chunk, batch) in batches.iter().enumerate() {
                    let arrays = plan.key_arrays(side, batch)?;
                    let record = key_record(&arrays, plan, batch.num_rows())?;
                    for index in 0..batch.num_rows() {
                        if any_null(&arrays, index) {
                            continue;
                        }
                        let at = (chunk as u32, index as u32);
                        match map.get(&record.scalar(index)?).copied() {
                            Some(group) => chains.push(group, at),
                            None => {
                                let group = chains.open(at, END);
                                map.insert(record.scalar(index)?, group);
                            }
                        }
                    }
                }
                KeyTable::Values { map, chains }
            }
        };
        Ok(Self {
            chunks,
            batches,
            keys,
            matched,
        })
    }

    /// Whether the build keys' range excludes every present key of a probe
    /// batch whose keys were read into `probe`, so no row of it can match:
    /// one pass over the rows, stopping at the first one inside the range,
    /// none hashed. A batch of absent keys is excluded; the values rung
    /// keeps no range and answers `false`.
    fn excludes(&self, probe: &ProbeKeys, arrays: &[ArrayRef]) -> bool {
        let (
            KeyTable::Buffers {
                range, rows: held, ..
            },
            ProbeKeys::Buffers(rows),
        ) = (&self.keys, probe)
        else {
            return false;
        };
        let Some(range) = range else {
            return true;
        };
        (0..rows.num_rows())
            .all(|index| any_null(arrays, index) || !range.contains(rows.row(index), held))
    }

    /// Read the probe batch's keys on the table's rung.
    fn probe_keys(&self, arrays: &[ArrayRef], plan: &JoinPlan, rows: usize) -> Result<ProbeKeys> {
        match &self.keys {
            KeyTable::Buffers { converter, .. } | KeyTable::Sorted { converter, .. } => Ok(
                ProbeKeys::Buffers(converter.convert_columns(arrays).map_err(arrow_error)?),
            ),
            KeyTable::Values { .. } => Ok(ProbeKeys::Values(key_record(arrays, plan, rows)?)),
        }
    }
}

/// Row `at` of the build side's converted keys, which `rows` holds chunk by
/// chunk - the chunk being converted is `current` until it is pushed.
fn rows_at<'r>(rows: &'r [Rows], current: &'r Rows, at: BuildRef) -> Row<'r> {
    rows.get(at.0 as usize)
        .unwrap_or(current)
        .row(at.1 as usize)
}

/// One probe batch's keys, read on the table's rung.
enum ProbeKeys {
    Buffers(Rows),
    Values(Serie),
}

/// A build side past the spill bound, joined one partition at a time - a
/// grace hash join, one level: both sides scattered by the hash of their key
/// rows, each partition's rows written to disk as they fill, the probe read
/// whole on the first round.
struct Grace {
    /// Converts the key rows a partition is picked by.
    converter: RowConverter,
    /// What a waiting partition's rows are written under: the options'
    /// bound at zero bytes, so every written chunk lies in a spill file.
    spill: SpillOptions,
    /// The bytes a partition's waiting pieces reach before they are
    /// written: the bound's share of one partition, at least
    /// [`MIN_SPOOL_BYTE_SIZE`], so the pieces waiting across every partition
    /// of one side hold no more than the bound or that floor a partition.
    flush_at: usize,
    /// The build rows of every partition, emptied as each is joined.
    build: Vec<Spool>,
    /// The probe, until the first round reads it whole into `probe`.
    source: Option<SerieReader>,
    /// The probe rows of every partition, emptied as each is joined.
    probe: Vec<Spool>,
    /// The next partition to join.
    next: usize,
}

/// One side's rows of one grace partition: the chunks written to disk, and
/// the pieces still waiting to be.
#[derive(Default)]
struct Spool {
    written: Vec<Serie>,
    waiting: Vec<Serie>,
    waiting_bytes: usize,
}

impl Spool {
    /// Hold `piece`, writing the waiting pieces to disk as one chunk once
    /// they reach `flush_at` bytes.
    fn push(&mut self, piece: Serie, flush_at: usize, spill: &SpillOptions) -> Result<()> {
        self.waiting_bytes = self.waiting_bytes.saturating_add(piece.memory_size());
        self.waiting.push(piece);
        if self.waiting_bytes >= flush_at
            && let Some(mut chunk) = self.join_waiting()?
        {
            chunk.spill(spill)?;
            self.written.push(chunk);
        }
        Ok(())
    }

    /// The waiting pieces as one chunk, `None` where none waits.
    fn join_waiting(&mut self) -> Result<Option<Serie>> {
        let Some(first) = self.waiting.first() else {
            return Ok(None);
        };
        let root = Arc::new(first.require_field()?.clone());
        let pieces = std::mem::take(&mut self.waiting);
        self.waiting_bytes = 0;
        Ok(Some(ChunkedSerie::from_landed(root, pieces).into_serie()?))
    }

    /// Every chunk this side holds of the partition: the written ones, then
    /// the pieces still waiting as one more, joined now and so settled under
    /// `options` rather than written.
    fn into_chunks(mut self, options: &JoinOptions) -> Result<Vec<Serie>> {
        if let Some(chunk) = self.join_waiting()? {
            self.written.push(options.settle(chunk)?);
        }
        Ok(self.written)
    }
}

impl Grace {
    /// Scatter the build side's `chunks` over `count` partitions, the probe
    /// `source` left unread until the first round.
    fn new(plan: &JoinPlan, count: usize, chunks: Vec<Serie>, source: SerieReader) -> Result<Self> {
        let bound = plan.options.spill_bound()?;
        let mut grace = Self {
            converter: plan.converter()?,
            spill: bound.clone().with_byte_size(0),
            flush_at: usize::try_from(bound.byte_size() / count as u64)
                .unwrap_or(usize::MAX)
                .max(MIN_SPOOL_BYTE_SIZE),
            build: (0..count).map(|_| Spool::default()).collect(),
            source: Some(source),
            probe: (0..count).map(|_| Spool::default()).collect(),
            next: 0,
        };
        for chunk in chunks {
            grace.scatter(plan, plan.build, &chunk)?;
        }
        Ok(grace)
    }

    /// Scatter one record chunk of `side` over the partitions by the hash of
    /// its key rows - a row with an absent key into the first, where it
    /// matches nothing and still surfaces for a kind that keeps it.
    fn scatter(&mut self, plan: &JoinPlan, side: JoinSide, chunk: &Serie) -> Result<()> {
        let batch = chunk.into_arrow_batch()?;
        let arrays = plan.key_arrays(side, &batch)?;
        let rows = self
            .converter
            .convert_columns(&arrays)
            .map_err(arrow_error)?;
        let mut picked: Vec<Vec<u32>> = vec![Vec::new(); self.build.len()];
        for index in 0..batch.num_rows() {
            let at = if any_null(&arrays, index) {
                0
            } else {
                partition_of(rows.row(index), picked.len())
            };
            picked[at].push(index as u32);
        }
        let spools = if side == plan.build {
            &mut self.build
        } else {
            &mut self.probe
        };
        for (spool, order) in spools.iter_mut().zip(picked) {
            if order.is_empty() {
                continue;
            }
            let piece = if order.len() == chunk.len() {
                chunk.clone()
            } else {
                chunk.taken(&order)?
            };
            spool.push(piece, self.flush_at, &self.spill)?;
        }
        Ok(())
    }

    /// The next partition holding a row of either side: its build chunks
    /// and its probe chunks. The first call reads the probe whole and
    /// scatters it.
    fn next_round(&mut self, plan: &JoinPlan) -> Result<Option<(Vec<Serie>, Vec<Serie>)>> {
        if let Some(source) = self.source.take() {
            let side = plan.build.other();
            for chunk in source {
                self.scatter(plan, side, &chunk?)?;
            }
        }
        while self.next < self.build.len() {
            let at = self.next;
            self.next += 1;
            let build = std::mem::take(&mut self.build[at]).into_chunks(&plan.options)?;
            let probe = std::mem::take(&mut self.probe[at]).into_chunks(&plan.options)?;
            if !build.is_empty() || !probe.is_empty() {
                return Ok(Some((build, probe)));
            }
        }
        Ok(None)
    }
}

/// Where the probe rows of the round being joined come from.
enum Probe {
    /// The probe side itself, one batch pulled per step.
    Stream(SerieReader),
    /// One grace partition's probe chunks, written to disk.
    Spooled(std::vec::IntoIter<Serie>),
}

impl Iterator for Probe {
    type Item = Result<Serie>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Stream(reader) => reader.next().map(|chunk| chunk.map_err(Into::into)),
            Self::Spooled(chunks) => chunks.next().map(Ok),
        }
    }
}

/// The output of a join, one record column per batch, produced as the
/// probe is read.
pub(crate) struct JoinOutput {
    plan: Arc<JoinPlan>,
    /// The table of the round being joined: the whole build side, or one
    /// grace partition of it.
    table: BuildTable,
    /// The probe of the round being joined, `None` once it is drained.
    probe: Option<Probe>,
    /// Output batches produced and not yet handed out.
    pending: VecDeque<Serie>,
    /// The grace partitions not yet joined; `None` for a build side held
    /// whole.
    grace: Option<Grace>,
}

impl JoinOutput {
    /// The record every output row is typed by.
    pub(crate) fn root(&self) -> &Arc<Field> {
        self.plan.output()
    }

    /// Lay the output rows `pairs` names out: each a probe row and the
    /// build row it matched, `None` for an unmatched probe row.
    fn lay_out(&self, probe: &RecordBatch, pairs: &[(u32, Option<BuildRef>)]) -> Result<Serie> {
        let plan = &self.plan;
        let probe_indices = UInt32Array::from_iter_values(pairs.iter().map(|(row, _)| *row));
        let probe_columns: Vec<ArrayRef> = probe
            .columns()
            .iter()
            .map(|column| arrow_select::take::take(column.as_ref(), &probe_indices, None))
            .collect::<std::result::Result<_, _>>()
            .map_err(arrow_error)?;
        let build_columns = self.gather_build(pairs)?;
        let (left, right) = match plan.build {
            JoinSide::Left => (&build_columns, &probe_columns),
            JoinSide::Right => (&probe_columns, &build_columns),
        };
        self.compose(left, right, pairs.len())
    }

    /// The build side's columns at `pairs`' build rows, null where there is
    /// none.
    fn gather_build(&self, pairs: &[(u32, Option<BuildRef>)]) -> Result<Vec<ArrayRef>> {
        let batches = &self.table.batches;
        let Some(first) = batches.first() else {
            // No build row at all: every build column is absent.
            let root = match self.plan.build {
                JoinSide::Left => &self.plan.left_root,
                JoinSide::Right => &self.plan.right_root,
            };
            return Ok(struct_fields(root)?
                .iter()
                .map(|field| new_null_array(field.data_type(), pairs.len()))
                .collect());
        };
        let any_absent = pairs.iter().any(|(_, at)| at.is_none());
        let indices: Vec<(usize, usize)> = pairs
            .iter()
            .map(|(_, at)| match at {
                Some((chunk, row)) => (*chunk as usize, *row as usize),
                None => (batches.len(), 0),
            })
            .collect();
        let mut columns = Vec::with_capacity(first.num_columns());
        for column in 0..first.num_columns() {
            let mut sources: Vec<&dyn Array> = batches
                .iter()
                .map(|batch| batch.column(column).as_ref())
                .collect();
            let absent;
            if any_absent {
                absent = new_null_array(first.column(column).data_type(), 1);
                sources.push(absent.as_ref());
            }
            columns.push(
                arrow_select::interleave::interleave(&sources, &indices).map_err(arrow_error)?,
            );
        }
        Ok(columns)
    }

    /// Compose one output record out of the left and right columns of
    /// `rows` rows.
    fn compose(&self, left: &[ArrayRef], right: &[ArrayRef], rows: usize) -> Result<Serie> {
        let plan = &self.plan;
        let mut columns = Vec::with_capacity(plan.columns.len());
        for column in &plan.columns {
            columns.push(match column {
                Column::Left(index) => Arc::clone(&left[*index]),
                Column::Right(index) => Arc::clone(&right[*index]),
                Column::Coalesced(at_left, at_right) => {
                    let held = &left[*at_left];
                    if held.null_count() == 0 {
                        Arc::clone(held)
                    } else {
                        let present = BooleanArray::from(
                            held.nulls()
                                .map(|nulls| nulls.inner().clone())
                                .expect("a column with absent rows carries its validity"),
                        );
                        arrow_select::zip::zip(&present, held, &right[*at_right])
                            .map_err(arrow_error)?
                    }
                }
            });
        }
        let fields = struct_fields(&plan.output)?;
        let array =
            StructArray::try_new_with_length(fields, columns, None, rows).map_err(arrow_error)?;
        let landed = land(Arc::clone(&plan.output), Arc::new(array), &Proof::Proven)?;
        plan.options.settle(landed)
    }

    /// Queue `rows` of `side` standing alone, cut at
    /// [`DEFAULT_RECORD_BATCH_ROW_SIZE`]: the other side's columns null, or
    /// the rows themselves under a filtering kind. Nothing is gathered:
    /// every batch is a slice of `rows`, its buffers shared.
    fn one_sided(&mut self, side: JoinSide, rows: &Serie) -> Result<()> {
        let plan = Arc::clone(&self.plan);
        let mut offset = 0;
        while offset < rows.len() {
            let length = (rows.len() - offset).min(DEFAULT_RECORD_BATCH_ROW_SIZE);
            let piece = rows.slice(offset, length)?;
            offset += length;
            let out = if plan.how.is_filtering() {
                let landed = land(
                    Arc::clone(&plan.output),
                    piece.require_arrow_array()?,
                    &Proof::Proven,
                )?;
                plan.options.settle(landed)?
            } else {
                let held: Vec<ArrayRef> = piece.into_arrow_batch()?.columns().to_vec();
                let other = match side {
                    JoinSide::Left => &plan.right_root,
                    JoinSide::Right => &plan.left_root,
                };
                let absent: Vec<ArrayRef> = struct_fields(other)?
                    .iter()
                    .map(|field| new_null_array(field.data_type(), length))
                    .collect();
                let (left, right) = match side {
                    JoinSide::Left => (&held, &absent),
                    JoinSide::Right => (&absent, &held),
                };
                self.compose(left, right, length)?
            };
            self.pending.push_back(out);
        }
        Ok(())
    }

    /// Queue the rows `keep` marks of `rows`, a chunk of `side`, standing
    /// alone: the whole chunk shared where every row is kept.
    fn one_sided_kept(&mut self, side: JoinSide, rows: &Serie, keep: &[bool]) -> Result<()> {
        match keep.iter().filter(|kept| **kept).count() {
            0 => Ok(()),
            kept if kept == keep.len() => self.one_sided(side, rows),
            _ => self.one_sided(side, &rows.filtered(keep)?),
        }
    }

    /// Join one probe batch, queueing its output batches.
    fn probe_batch(&mut self, chunk: &Serie) -> Result<()> {
        let plan = Arc::clone(&self.plan);
        let probe_side = plan.build.other();
        let batch = chunk.into_arrow_batch()?;
        let arrays = plan.key_arrays(probe_side, &batch)?;
        let keys = self.table.probe_keys(&arrays, &plan, batch.num_rows())?;
        let keeps_unmatched_probe = match probe_side {
            JoinSide::Left => plan.how.keeps_unmatched_left(),
            JoinSide::Right => plan.how.keeps_unmatched_right(),
        };
        if plan.options.prune() && self.table.excludes(&keys, &arrays) {
            // No row of the batch can match: none is hashed, and the batch
            // stands alone where the kind keeps its unmatched rows.
            if keeps_unmatched_probe {
                self.one_sided(probe_side, chunk)?;
            }
            return Ok(());
        }
        if plan.how.is_filtering() && probe_side == JoinSide::Left {
            // The probe is the left side: keep its rows with (semi) or
            // without (anti) a match, left columns only.
            let keep: Vec<bool> = (0..batch.num_rows())
                .map(|index| {
                    let matched = !any_null(&arrays, index)
                        && !self.table.keys.matches(&keys, index).is_empty();
                    matched == matches!(plan.how, JoinKind::Semi)
                })
                .collect();
            return self.one_sided_kept(probe_side, chunk, &keep);
        }
        if plan.how.is_filtering() {
            // The build is the left side: a probe row only marks the left
            // rows it matches, which the drain emits once the probe ends.
            for index in 0..batch.num_rows() {
                if any_null(&arrays, index) {
                    continue;
                }
                for at in self.table.keys.matches(&keys, index) {
                    mark(&mut self.table.matched, at);
                }
            }
            return Ok(());
        }
        let mut pairs: Vec<(u32, Option<BuildRef>)> = Vec::new();
        let flush = |pairs: &mut Vec<(u32, Option<BuildRef>)>, output: &mut Self| -> Result<()> {
            if !pairs.is_empty() {
                let laid = output.lay_out(&batch, pairs)?;
                output.pending.push_back(laid);
                pairs.clear();
            }
            Ok(())
        };
        for index in 0..batch.num_rows() {
            let row = index as u32;
            let matches = if any_null(&arrays, index) {
                Matches::NONE
            } else {
                self.table.keys.matches(&keys, index)
            };
            if matches.is_empty() {
                if keeps_unmatched_probe {
                    pairs.push((row, None));
                }
            } else {
                for at in matches {
                    pairs.push((row, Some(at)));
                    mark(&mut self.table.matched, at);
                }
            }
            if pairs.len() >= DEFAULT_RECORD_BATCH_ROW_SIZE {
                flush(&mut pairs, self)?;
            }
        }
        flush(&mut pairs, self)?;
        Ok(())
    }

    /// Queue the round's unmatched build rows once its probe is drained:
    /// the build side's columns with the probe side's null, or the build
    /// rows alone - the matched ones for `semi` - for a filtering kind whose
    /// build is the left side.
    fn drain_build(&mut self) -> Result<()> {
        let Some(matched) = self.table.matched.take() else {
            return Ok(());
        };
        let side = self.plan.build;
        let want_matched = matches!(self.plan.how, JoinKind::Semi);
        let chunks = std::mem::take(&mut self.table.chunks);
        for (chunk, mut bits) in chunks.iter().zip(matched) {
            let bits = bits.finish();
            let keep: Vec<bool> = (0..chunk.len())
                .map(|row| bits.value(row) == want_matched)
                .collect();
            self.one_sided_kept(side, chunk, &keep)?;
        }
        Ok(())
    }

    /// Open the next grace partition: its table built, its probe chunks
    /// queued. `false` once every partition is joined, or for a build side
    /// held whole.
    fn next_round(&mut self) -> Result<bool> {
        let Some(grace) = &mut self.grace else {
            return Ok(false);
        };
        let Some((build, probe)) = grace.next_round(&self.plan)? else {
            self.grace = None;
            return Ok(false);
        };
        self.table = BuildTable::new(&self.plan, build)?;
        self.probe = Some(Probe::Spooled(probe.into_iter()));
        Ok(true)
    }
}

impl Iterator for JoinOutput {
    type Item = Result<Serie>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ready) = self.pending.pop_front() {
                return Some(Ok(ready));
            }
            let step = if let Some(probe) = &mut self.probe {
                match probe.next() {
                    Some(Ok(chunk)) => self.probe_batch(&chunk),
                    Some(Err(error)) => Err(error),
                    None => {
                        self.probe = None;
                        self.drain_build()
                    }
                }
            } else {
                match self.next_round() {
                    Ok(true) => Ok(()),
                    Ok(false) => return None,
                    Err(error) => Err(error),
                }
            };
            if let Err(error) = step {
                // Fused: nothing more is produced after a failure.
                self.probe = None;
                self.grace = None;
                self.pending.clear();
                return Some(Err(error));
            }
        }
    }
}

impl std::iter::FusedIterator for JoinOutput {}

/// Resolve and run one join over two sources.
pub(crate) fn join(
    left: SerieSource,
    right: SerieSource,
    by: impl IntoJoinKeys,
    how: JoinKind,
    options: &JoinOptions,
) -> Result<JoinOutput> {
    let keys = by.into_join_keys()?;
    let plan = JoinPlan::compile(
        left.root()?,
        right.root()?,
        &keys,
        how,
        options,
        left.memory_size(),
        right.memory_size(),
    )?;
    plan.run(left, right)
}

/// Whether a join pushes its held keys into the probe source's read: one
/// key, `options.prune()` on, and a kind that emits no unmatched probe row -
/// a `left` or `full` join probing the left, and `anti`, must see every
/// probe row. The one statement of the rule: [`pushdown_term`] applies it
/// with the held values in hand, and the plan's `explain` states it with
/// none.
pub(crate) fn pushes_down(
    keys: &JoinKeys,
    how: JoinKind,
    build: JoinSide,
    options: &JoinOptions,
) -> bool {
    let keeps_probe = match build {
        JoinSide::Right => how.keeps_unmatched_left(),
        JoinSide::Left => how.keeps_unmatched_right(),
    };
    options.prune() && !keeps_probe && keys.len() == 1
}

/// The term a probe source is filtered by before it is read: the probe key
/// `in` the distinct keys the held build side states, where [`pushes_down`]
/// says the join pushes - and `None` otherwise, or past
/// `options.pushdown_keys()` distinct values, where the plan's `execute`
/// reads the probe whole.
///
/// # Errors
///
/// A key that does not bind against its root, or a build chunk whose key
/// cannot be read.
pub(crate) fn pushdown_term(
    probe_root: &Field,
    build_root: &Field,
    keys: &JoinKeys,
    how: JoinKind,
    build: JoinSide,
    options: &JoinOptions,
    held: &ChunkedSerie,
) -> Result<Option<Term>> {
    if !pushes_down(keys, how, build, options) {
        return Ok(None);
    }
    let key = &keys.0[0];
    let (probe_term, build_term) = match build {
        JoinSide::Right => (key.left(), key.right()),
        JoinSide::Left => (key.right(), key.left()),
    };
    // The build key is bound against the build root alone: the literal the
    // probe is filtered by coerces into the probe key where it is compared.
    let bound = build_term.bind(build_root)?;
    let _ = probe_term.bind(probe_root)?;
    let mut columns = Vec::with_capacity(held.num_chunks());
    for chunk in held.chunks() {
        let batch = chunk.into_arrow_batch()?;
        columns.push(bound.evaluate(&batch).map_err(crate::Error::from)?);
    }
    let borrowed: Vec<&dyn Array> = columns.iter().map(AsRef::as_ref).collect();
    let values = arrow_select::concat::concat(&borrowed).map_err(arrow_error)?;
    let values =
        Serie::from_arrow_array(Some(bound.field()), values, crate::ArrowCastOptions::new())?;
    isin_term(probe_term, &values, options.pushdown_keys())
}

/// A term `in (...)` the distinct values of one key column, or `None` past
/// `limit` distinct values: the prune a probe batch is filtered by before it
/// is hashed.
pub(crate) fn isin_term(term: &Term, values: &Serie, limit: usize) -> Result<Option<Term>> {
    let distinct = values.into_unique()?;
    let present: Vec<Scalar> = distinct
        .iter()
        .filter(|value| !value.is_null())
        .map(|value| value.into_owned())
        .collect();
    if present.is_empty() || present.len() > limit {
        return Ok(None);
    }
    Ok(Some(
        term.clone().is_in(present.into_iter().map(Term::literal)),
    ))
}

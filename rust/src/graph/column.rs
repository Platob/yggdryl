//! The columns a graph event is stated in: one per fact the traits answer.
//!
//! Every generated schema of an event - a [text line](crate::text::TextLine)
//! read into a batch, a FIX message parsed out of it, a message the
//! lifecycle chained - states these nineteen under one name and one datatype
//! each, so the three join on them without a mapping: a message's
//! `srcuuids` are the `curruuid` of the lines it was read from, and a
//! chained message's `prevuuid` and `parentuuids` are the `curruuid` of the
//! messages before it. The names are the traits' own: what
//! [`Element`](super::Element) and [`Event`] read and write under
//! `get_`/`set_` is what a column is called.

use std::collections::BTreeMap;

use crate::{DataType, Field, Result, Scalar, State, TimeUnit, Timezone, Uuid};

use super::Event;

/// One column of the nineteen every graph event is stated in.
///
/// [`Self::ALL`] is the canonical order [`Self::fields`] and event-native
/// schemas use: **when** it happened - the instant, then the instants it is
/// read against - then **which**
/// event it is - its identity, the chain's, the codes, what it follows, its
/// place, what it descends from, what it was read from, the names it goes
/// by - and last the state it reached.
///
/// ```
/// use yggdryl::graph::{EventColumn, MarketEventData, Element, Event};
/// use yggdryl::{Scalar, Uuid};
///
/// # fn main() -> yggdryl::Result<()> {
/// let fields = EventColumn::fields()?;
/// assert_eq!(fields.len(), 19);
/// assert_eq!(fields[0].name(), "currunix");
/// assert_eq!(fields[8].name(), "curruuid");
/// assert_eq!(fields[18].name(), "state");
/// // What an event states under a column, and the same fact stated back.
/// let mut event = MarketEventData::at(1_700_000_000_000_000_000);
/// event.set_srcuuids(vec![Uuid::from_v8(7)]);
/// let sources = EventColumn::SrcUuids.fact(&event).expect("a source");
/// let mut again = MarketEventData::default();
/// EventColumn::SrcUuids.record(&mut again, &sources);
/// assert_eq!(again.get_srcuuids(), [Uuid::from_v8(7)]);
/// // Nothing stated is a null: an empty list, an empty code, no instant.
/// assert_eq!(EventColumn::ParentUuids.fact(&event), None);
/// assert_eq!(EventColumn::CrossCode.fact(&event), None);
/// assert_eq!(EventColumn::PrevUnix.fact(&event), None);
/// assert_eq!(EventColumn::of_name("SrcUuids"), Some(EventColumn::SrcUuids));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EventColumn {
    /// When the event happened, a nanosecond UTC clock; never absent.
    CurrUnix,
    /// When it was created, where that is known.
    CreaUnix,
    /// The latest execution clock its lifecycle reached, where known.
    ExecUnix,
    /// When it was recorded, where that is known.
    RecdUnix,
    /// The recording clock of the observation selected as the merge reference.
    RefRecdUnix,
    /// When it stops being good, where it does.
    ExprTime,
    /// When the event it follows happened, where it follows one.
    PrevUnix,
    /// The grid instant a walk read it as the snapshot of, where one did.
    SnapUnix,
    /// The event's identity; never absent.
    CurrUuid,
    /// The identity every event of one chain shares; the event's own where
    /// it names no cross code, so never absent.
    CrossUuid,
    /// The code naming the chain, as the event spells it; empty where none.
    CrossCode,
    /// The XXH3-64 the event's content digests to; never absent.
    CurrHashCode,
    /// The XXH3-64 of the cross code, zero where none; never absent.
    CrossHashCode,
    /// The identity of the event this one follows, where it follows one.
    PrevUuid,
    /// The event's place in its chain: how many came before it; none where
    /// none did.
    SeqNum,
    /// The identities this event descends from, sorted and unique.
    ParentUuids,
    /// The identities this event was read from: provenance, never lineage.
    SrcUuids,
    /// The names the event goes by, each under the scheme that issued it.
    Identifiers,
    /// The state the event reached, ranked so it sorts by lifecycle:
    /// `00UNKNOWN` where nothing states one, so never absent on a row an
    /// event wrote; null only where a row states none, because a state has
    /// no neutral member for an empty cell to read as.
    State,
}

impl EventColumn {
    /// Every column, in canonical event order.
    pub const ALL: [Self; 19] = [
        Self::CurrUnix,
        Self::CreaUnix,
        Self::ExecUnix,
        Self::RecdUnix,
        Self::RefRecdUnix,
        Self::ExprTime,
        Self::PrevUnix,
        Self::SnapUnix,
        Self::CurrUuid,
        Self::CrossUuid,
        Self::CrossCode,
        Self::CurrHashCode,
        Self::CrossHashCode,
        Self::PrevUuid,
        Self::SeqNum,
        Self::ParentUuids,
        Self::SrcUuids,
        Self::Identifiers,
        Self::State,
    ];

    /// The column's name: the fact's, as the traits spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CurrUnix => "currunix",
            Self::CreaUnix => "creaunix",
            Self::ExecUnix => "execunix",
            Self::RecdUnix => "recdunix",
            Self::RefRecdUnix => "refrecdunix",
            Self::ExprTime => "exprtime",
            Self::PrevUnix => "prevunix",
            Self::SnapUnix => "snapunix",
            Self::CurrUuid => "curruuid",
            Self::CrossUuid => "crossuuid",
            Self::CrossCode => "crosscode",
            Self::CurrHashCode => "currhashcode",
            Self::CrossHashCode => "crosshashcode",
            Self::PrevUuid => "prevuuid",
            Self::SeqNum => "seqnum",
            Self::ParentUuids => "parentuuids",
            Self::SrcUuids => "srcuuids",
            Self::Identifiers => "identifiers",
            Self::State => "state",
        }
    }

    /// The column's display, for a catalog.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::CurrUnix => "CurrUnix",
            Self::CreaUnix => "CreaUnix",
            Self::ExecUnix => "ExecUnix",
            Self::RecdUnix => "RecdUnix",
            Self::RefRecdUnix => "RefRecdUnix",
            Self::ExprTime => "ExprTime",
            Self::PrevUnix => "PrevUnix",
            Self::SnapUnix => "SnapUnix",
            Self::CurrUuid => "CurrUuid",
            Self::CrossUuid => "CrossUuid",
            Self::CrossCode => "CrossCode",
            Self::CurrHashCode => "CurrHashCode",
            Self::CrossHashCode => "CrossHashCode",
            Self::PrevUuid => "PrevUuid",
            Self::SeqNum => "SeqNum",
            Self::ParentUuids => "ParentUuids",
            Self::SrcUuids => "SrcUuids",
            Self::Identifiers => "Identifiers",
            Self::State => "State",
        }
    }

    /// What the column holds, for a catalog.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::CurrUnix => "When the event happened: the settled instant, UTC.",
            Self::CreaUnix => {
                "When the event was created, where that is known; the earliest its chain knows once followed."
            }
            Self::ExecUnix => {
                "The latest execution clock this lifecycle reached as of this event; an execution dates itself, following carries it, and duplicate statements keep their earliest observation."
            }
            Self::RecdUnix => {
                "When this event was recorded, where that is known; the earliest its statements know."
            }
            Self::RefRecdUnix => {
                "The recording clock of the observation selected as this event's merge reference; the latest its statements know."
            }
            Self::ExprTime => {
                "When the event stops being good, where it does; the latest its chain knows once followed."
            }
            Self::PrevUnix => "When the event this one follows happened, where it follows one.",
            Self::SnapUnix => {
                "The grid instant a walk read this event as the snapshot of; empty on every row no snapshot was taken of."
            }
            Self::CurrUuid => {
                "The event's identity: UUIDv7 ordered by millisecond and sequence, with a content payload seeded by its cross hash."
            }
            Self::CrossUuid => {
                "The identity every event of one chain shares, derived from the code they share; the event's own where it names none."
            }
            Self::CrossCode => {
                "The code every event of one chain shares, as the event spells it; empty where none."
            }
            Self::CurrHashCode => "The XXH3-64 of what the event states.",
            Self::CrossHashCode => {
                "The XXH3-64 of the cross code; zero where the event names none."
            }
            Self::PrevUuid => "The identity of the event this one follows, where it follows one.",
            Self::SeqNum => "The event's place in its chain: how many came before it.",
            Self::ParentUuids => {
                "The identities of the events this one descends from, sorted and unique."
            }
            Self::SrcUuids => {
                "The sorted unique identities of the elements this event was read from: provenance, never lineage - no walk moves it."
            }
            Self::Identifiers => {
                "The names this event goes by, each under the scheme that issued it, in sorted order."
            }
            Self::State => {
                "The state the event reached, ranked so it sorts by lifecycle; 00UNKNOWN where nothing states one, the furthest its chain knows once followed."
            }
        }
    }

    /// The one datatype the column is built and read at.
    ///
    /// The clocks are nanoseconds UTC, the identities the crate's own
    /// [`Uuid`], the codes `uint64`, the lists `list<uuid>` with the item
    /// named by the fact, the names a sorted `map<utf8, utf8>`, the state a
    /// [`State`] code.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the names' map does not
    /// build, which is a defect in this module rather than anything a
    /// caller did.
    pub fn datatype(self) -> Result<DataType> {
        let clock = || {
            DataType::DateTime(crate::DateTimeType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            })
        };
        Ok(match self {
            Self::CurrUnix
            | Self::CreaUnix
            | Self::ExecUnix
            | Self::RecdUnix
            | Self::RefRecdUnix
            | Self::ExprTime
            | Self::PrevUnix
            | Self::SnapUnix => clock(),
            Self::CurrUuid | Self::CrossUuid | Self::PrevUuid => DataType::Uuid,
            Self::CrossCode => DataType::utf8(),
            Self::CurrHashCode | Self::CrossHashCode | Self::SeqNum => DataType::UInt64,
            Self::ParentUuids => DataType::list(DataType::Uuid.required_field("parentuuid")),
            Self::SrcUuids => DataType::list(DataType::Uuid.required_field("srcuuid")),
            Self::Identifiers => DataType::map_of(DataType::utf8(), DataType::utf8(), true)?,
            Self::State => DataType::State,
        })
    }

    /// Whether the column may hold a null: the facts the traits answer as
    /// an option or as nothing - an empty code, list or map, a place of
    /// zero - may, and so may the state, which an event always answers -
    /// `00UNKNOWN` where nothing states one - but a row may leave unstated,
    /// a state having no neutral member for an empty cell to read as; the
    /// instant, the identities and the codes are never absent.
    #[must_use]
    pub const fn nullable(self) -> bool {
        !matches!(
            self,
            Self::CurrUnix
                | Self::CurrUuid
                | Self::CrossUuid
                | Self::CurrHashCode
                | Self::CrossHashCode
        )
    }

    /// The column as a field: its name, datatype and nullability, with
    /// its display and description for a catalog.
    ///
    /// # Errors
    ///
    /// Returns [`Self::datatype`]'s refusal.
    pub fn field(self) -> Result<Field> {
        let mut field = Field::new(self.name(), self.datatype()?, self.nullable());
        field.set_display(self.display())?;
        field.set_description(self.description())?;
        Ok(field)
    }

    /// Every column as a field, in [`Self::ALL`]'s order.
    ///
    /// # Errors
    ///
    /// Returns [`Self::datatype`]'s refusal.
    pub fn fields() -> Result<Vec<Field>> {
        Self::ALL.into_iter().map(Self::field).collect()
    }

    /// The column one name spells, whatever its case.
    #[must_use]
    pub fn of_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|column| crate::folds_equal(column.name(), name))
    }

    /// What an event states under this column, as the raw value the
    /// column's datatype types, or nothing where it states no fact: an
    /// absent instant, identity or place, an empty code, list or map.
    pub fn fact<E: Event + ?Sized>(self, event: &E) -> Option<Scalar> {
        let instant =
            |unix: i64| Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok();
        match self {
            Self::CurrUnix => instant(event.get_currunix()),
            Self::CreaUnix => event.get_creaunix().and_then(instant),
            Self::ExecUnix => event.get_execunix().and_then(instant),
            Self::RecdUnix => event.get_recdunix().and_then(instant),
            Self::RefRecdUnix => event.get_refrecdunix().and_then(instant),
            Self::ExprTime => event.get_exprtime().and_then(instant),
            Self::PrevUnix => event.get_prevunix().and_then(instant),
            Self::SnapUnix => event.get_snapunix().and_then(instant),
            Self::CurrUuid => Some(Scalar::Uuid(event.get_curruuid())),
            Self::CrossUuid => Some(Scalar::Uuid(event.get_crossuuid())),
            Self::CrossCode => {
                let code = event.get_crosscode();
                (!code.is_empty()).then(|| Scalar::from(code))
            }
            Self::CurrHashCode => Some(Scalar::from(event.get_currhashcode())),
            Self::CrossHashCode => Some(Scalar::from(event.get_crosshashcode())),
            Self::PrevUuid => event.get_prevuuid().map(Scalar::Uuid),
            Self::SeqNum => (event.get_seqnum() != 0).then(|| Scalar::from(event.get_seqnum())),
            Self::ParentUuids => uuids_fact(event.get_parentuuids()),
            Self::SrcUuids => uuids_fact(event.get_srcuuids()),
            Self::Identifiers => {
                let identifiers = event.get_identifiers();
                (!identifiers.is_empty()).then(|| {
                    Scalar::from_mapping(identifiers.iter().map(|(scheme, identifier)| {
                        (
                            Scalar::from(scheme.as_str()),
                            Scalar::from(identifier.as_str()),
                        )
                    }))
                    .ok()
                })?
            }
            Self::State => Some(Scalar::State(event.get_state().clone())),
        }
    }

    /// Records what one cell states on the event, through the traits: a
    /// null clears the fact, and a value the fact's type refuses is
    /// silence.
    pub fn record<E: Event + ?Sized>(self, event: &mut E, value: &Scalar) {
        let instant = || value.temporal_count_at(TimeUnit::Nanosecond);
        match self {
            Self::CurrUnix => {
                if let Some(unix) = instant() {
                    event.set_currunix(unix);
                }
            }
            Self::CreaUnix => event.set_creaunix(instant()),
            Self::ExecUnix => event.set_execunix(instant()),
            Self::RecdUnix => event.set_recdunix(instant()),
            Self::RefRecdUnix => event.set_refrecdunix(instant()),
            Self::ExprTime => event.set_exprtime(instant()),
            Self::PrevUnix => event.set_prevunix(instant()),
            Self::SnapUnix => event.set_snapunix(instant()),
            Self::CurrUuid => {
                if let Scalar::Uuid(uuid) = value {
                    event.set_curruuid(*uuid);
                }
            }
            Self::CrossUuid => {
                if let Scalar::Uuid(uuid) = value {
                    event.set_crossuuid(*uuid);
                }
            }
            Self::CrossCode => event.set_crosscode(
                value
                    .as_str()
                    .filter(|held| !held.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_default(),
            ),
            Self::CurrHashCode => {
                if let Some(code) = value.as_u64() {
                    event.set_currhashcode(code);
                }
            }
            Self::CrossHashCode => {
                if let Some(code) = value.as_u64() {
                    event.set_crosshashcode(code);
                }
            }
            Self::PrevUuid => event.set_prevuuid(match value {
                Scalar::Uuid(uuid) => Some(*uuid),
                _ => None,
            }),
            Self::SeqNum => event.set_seqnum(value.as_u64().unwrap_or(0)),
            Self::ParentUuids => event.set_parentuuids(uuids_of(value)),
            Self::SrcUuids => event.set_srcuuids(uuids_of(value)),
            Self::Identifiers => event.set_identifiers(identifiers_of(value)),
            Self::State => event.set_state(match value {
                Scalar::State(state) => state.clone(),
                other => other
                    .as_str()
                    .and_then(|text| State::read(text).ok())
                    .unwrap_or_else(State::unknown),
            }),
        }
    }
}

/// One list of identities as the raw value its `list<uuid>` column types,
/// or nothing where the list is empty.
fn uuids_fact(uuids: &[Uuid]) -> Option<Scalar> {
    (!uuids.is_empty()).then(|| Scalar::from_sequence(uuids.iter().copied().map(Scalar::Uuid)))
}

/// The identities one `list<uuid>` cell states, every other item passed
/// over; none for a cell stating no list.
fn uuids_of(value: &Scalar) -> Vec<Uuid> {
    value
        .as_sequence()
        .map(|held| {
            held.iter()
                .filter_map(|item| match item {
                    Scalar::Uuid(uuid) => Some(*uuid),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The names one `map<utf8, utf8>` cell states, every other entry passed
/// over; none for a cell stating no map.
fn identifiers_of(value: &Scalar) -> BTreeMap<String, String> {
    value
        .as_mapping()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(scheme, identifier)| {
                    Some((scheme.as_str()?.to_owned(), identifier.as_str()?.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

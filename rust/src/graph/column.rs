//! The columns a graph event is stated in: one per fact [`Event`] adds to
//! its element's.
//!
//! Every generated schema of an event states these nine right after the
//! six [`ElementColumn`](super::ElementColumn)s, under one name and one
//! datatype each - a [text line](crate::text::TextLine) read into a batch, a
//! FIX message parsed out of it, a message the lifecycle chained, a
//! `marketdata` row - so the rows join on them without a mapping: a chained
//! message's `prevuuid` is the `uuid` of the message before it. The
//! names are the trait's own: what [`Event`] reads and writes under
//! `get_`/`set_` is what a column is called.

use crate::{DataType, Field, Result, Scalar, State, TimeUnit, Timezone};

use super::Event;

/// One column of the nine every graph event adds to its element's.
///
/// [`Self::ALL`] is the canonical order [`Self::fields`] and every
/// generated schema use: **when** it happened - the transaction instant, then the
/// instants it is read against - then what it **follows** and where it
/// stands among the events of its instant, and last the **state** it
/// reached.
///
/// ```
/// use yggdryl::graph::{Event, EventColumn, OrderEvent};
///
/// # fn main() -> yggdryl::Result<()> {
/// let fields = EventColumn::fields()?;
/// assert_eq!(fields.len(), 9);
/// assert_eq!(fields[0].name(), "transunix");
/// assert_eq!(fields[6].name(), "prevuuid");
/// assert_eq!(fields[8].name(), "state");
/// // An identity is an element's fact, and when a market event executed a
/// // market fact: neither is an event's.
/// assert_eq!(EventColumn::of_name("uuid"), None);
/// assert_eq!(EventColumn::of_name("execunix"), None);
/// // What an event states under a column, and the same fact stated back.
/// let event = OrderEvent::at(1_700_000_000_000_000_000);
/// let instant = EventColumn::TransUnix.fact(&event).expect("an instant");
/// let mut again = OrderEvent::default();
/// EventColumn::TransUnix.record(&mut again, &instant);
/// assert_eq!(again.get_transunix(), 1_700_000_000_000_000_000);
/// // Nothing stated is a null: no predecessor, no earlier instant.
/// assert_eq!(EventColumn::PrevUuid.fact(&event), None);
/// assert_eq!(EventColumn::PrevUnix.fact(&event), None);
/// assert_eq!(EventColumn::of_name("PrevUnix"), Some(EventColumn::PrevUnix));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EventColumn {
    /// When the operation happened - the transaction instant, a nanosecond
    /// UTC clock; never absent.
    TransUnix,
    /// When it was created, where that is known.
    CreaUnix,
    /// When the message crossed the wire - the technical clock - where that
    /// is known.
    SendUnix,
    /// When it stops being good, where it does.
    ExprUnix,
    /// When the event it follows happened, where it follows one.
    PrevUnix,
    /// The grid instant a walk read it as the snapshot of, where one did.
    SnapUnix,
    /// The identity of the event this one follows, where it follows one.
    PrevUuid,
    /// Where the event stands in what it was read from, never null: a text
    /// line's row number under `start_rownum`, a parsed or walked event's
    /// place among the events of its instant.
    SeqNum,
    /// The state the event reached, the code of a lifecycle-sorted enum:
    /// `UNKNOWN` where nothing states one, so never absent on a row an
    /// event wrote; null only where a row states none, because a state has
    /// no neutral member for an empty cell to read as.
    State,
}

impl EventColumn {
    /// Every column, in canonical event order.
    pub const ALL: [Self; 9] = [
        Self::TransUnix,
        Self::CreaUnix,
        Self::SendUnix,
        Self::ExprUnix,
        Self::PrevUnix,
        Self::SnapUnix,
        Self::PrevUuid,
        Self::SeqNum,
        Self::State,
    ];

    /// The column's name: the fact's, as the trait spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TransUnix => "transunix",
            Self::CreaUnix => "creaunix",
            Self::SendUnix => "sendunix",
            Self::ExprUnix => "exprunix",
            Self::PrevUnix => "prevunix",
            Self::SnapUnix => "snapunix",
            Self::PrevUuid => "prevuuid",
            Self::SeqNum => "seqnum",
            Self::State => "state",
        }
    }

    /// The column's display, for a catalog.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::TransUnix => "Transaction Time",
            Self::CreaUnix => "Creation Time",
            Self::SendUnix => "Sending Time",
            Self::ExprUnix => "Expiry Time",
            Self::PrevUnix => "Previous Time",
            Self::SnapUnix => "Snapshot Time",
            Self::PrevUuid => "Previous UUID",
            Self::SeqNum => "Sequence Number",
            Self::State => "State",
        }
    }

    /// What the column holds, for a catalog.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::TransUnix => "When the operation happened: the settled transaction instant, UTC.",
            Self::CreaUnix => {
                "When the event was created, where that is known; the earliest its chain knows once followed."
            }
            Self::SendUnix => {
                "When the message crossed the wire, where that is known; the earliest its statements know."
            }
            Self::ExprUnix => {
                "When the event stops being good, where it does; the latest its chain knows once followed."
            }
            Self::PrevUnix => "When the event this one follows happened, where it follows one.",
            Self::SnapUnix => {
                "The grid instant a walk read this event as the snapshot of; empty on every row no snapshot was taken of."
            }
            Self::PrevUuid => "The identity of the event this one follows, where it follows one.",
            Self::SeqNum => {
                "Where the event stands in what it was read from, 0 for the first: a text line's row number under start_rownum, a parsed or walked event's place among the events of its instant."
            }
            Self::State => {
                "The state the event reached, as the code of a lifecycle-sorted enum; UNKNOWN where nothing states one, the furthest its chain knows once followed."
            }
        }
    }

    /// The one datatype the column is built and read at: the clocks
    /// nanoseconds UTC, the predecessor the crate's own
    /// [`Uuid`](crate::Uuid), the place `uint64`, the state a [`State`]
    /// code.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::TransUnix
            | Self::CreaUnix
            | Self::SendUnix
            | Self::ExprUnix
            | Self::PrevUnix
            | Self::SnapUnix => DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            },
            Self::PrevUuid => DataType::Uuid,
            Self::SeqNum => DataType::UInt64,
            Self::State => DataType::State,
        }
    }

    /// Whether the column may hold a null: the facts the trait answers as
    /// an option may, and so may the state, which an event always answers -
    /// `UNKNOWN` where nothing states one - but a row may leave unstated, a
    /// state having no neutral member for an empty cell to read as; the
    /// instant and the place among the events of it are never absent, the
    /// first place being zero.
    #[must_use]
    pub const fn nullable(self) -> bool {
        !matches!(self, Self::TransUnix | Self::SeqNum)
    }

    /// The column as a field: its name, datatype and nullability, with
    /// its display and description for a catalog.
    ///
    /// # Errors
    ///
    /// Returns an error when the display or the description cannot be set.
    pub fn field(self) -> Result<Field> {
        let mut field = Field::new(self.name(), self.datatype(), self.nullable());
        field.set_display(self.display())?;
        field.set_description(self.description())?;
        Ok(field)
    }

    /// Every column as a field, in [`Self::ALL`]'s order.
    ///
    /// # Errors
    ///
    /// Returns [`Self::field`]'s refusal.
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
    /// absent instant, predecessor or place.
    pub fn fact<E: Event + ?Sized>(self, event: &E) -> Option<Scalar> {
        let instant =
            |unix: i64| Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok();
        match self {
            Self::TransUnix => instant(event.get_transunix()),
            Self::CreaUnix => event.get_creaunix().and_then(instant),
            Self::SendUnix => event.get_sendunix().and_then(instant),
            Self::ExprUnix => event.get_exprunix().and_then(instant),
            Self::PrevUnix => event.get_prevunix().and_then(instant),
            Self::SnapUnix => event.get_snapunix().and_then(instant),
            Self::PrevUuid => event.get_prevuuid().map(Scalar::Uuid),
            Self::SeqNum => Some(Scalar::from(event.get_seqnum())),
            Self::State => Some(Scalar::State(*event.get_state())),
        }
    }

    /// Records what one cell states on the event, through the trait: a
    /// null clears the fact, and a value the fact's type refuses is
    /// silence.
    pub fn record<E: Event + ?Sized>(self, event: &mut E, value: &Scalar) {
        let instant = || value.temporal_count_at(TimeUnit::Nanosecond);
        match self {
            Self::TransUnix => {
                if let Some(unix) = instant() {
                    event.set_transunix(unix);
                }
            }
            Self::CreaUnix => event.set_creaunix(instant()),
            Self::SendUnix => event.set_sendunix(instant()),
            Self::ExprUnix => event.set_exprunix(instant()),
            Self::PrevUnix => event.set_prevunix(instant()),
            Self::SnapUnix => event.set_snapunix(instant()),
            Self::PrevUuid => event.set_prevuuid(match value {
                Scalar::Uuid(uuid) => Some(*uuid),
                _ => None,
            }),
            // The place read through its column's own value door, as the
            // digests are, so a table's `decimal(20, 0)` states the place
            // it stored; a null or a cell the door refuses is the first
            // place.
            Self::SeqNum => {
                event.set_seqnum(super::element_column::whole_u64(value).unwrap_or(0));
            }
            Self::State => event.set_state(match value {
                Scalar::State(state) => *state,
                other => <State as crate::EnumValue>::from_scalar_value(other).unwrap_or_default(),
            }),
        }
    }
}

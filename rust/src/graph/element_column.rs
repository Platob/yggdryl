//! The columns a graph element is stated in: one per fact [`Element`]
//! answers.
//!
//! Every generated schema of an element opens with these six, under one
//! name and one datatype each, before the [`EventColumn`](super::EventColumn)s
//! an event adds - a [text line](crate::text::TextLine) read into a batch, a
//! FIX message parsed out of it, a `marketdata` row - so the rows join on
//! them without a mapping: a message's `srcuuids` are the `curruuid` of the
//! lines it was read from. The names are the trait's own: what [`Element`]
//! reads and writes under `get_`/`set_` is what a column is called.

use crate::{DataType, Field, Result, Scalar, Uuid};

use super::Element;

/// One column of the six every graph element is stated in.
///
/// [`Self::ALL`] is the order [`Element`] declares them in and every
/// generated schema opens with: the element's identity, the chain's, the
/// code naming the chain, the two codes they digest to, and what the element
/// was read from.
///
/// ```
/// use yggdryl::graph::{Element, ElementColumn, OrderEvent};
/// use yggdryl::Uuid;
///
/// # fn main() -> yggdryl::Result<()> {
/// let fields = ElementColumn::fields()?;
/// assert_eq!(fields.len(), 6);
/// assert_eq!(fields[0].name(), "curruuid");
/// assert_eq!(fields[5].name(), "srcuuids");
/// // What an element states under a column, and the same fact stated back.
/// let mut element = OrderEvent::at(1_700_000_000_000_000_000);
/// element.set_srcuuids(vec![Uuid::from_v8(7)]);
/// let sources = ElementColumn::SrcUuids.fact(&element).expect("a source");
/// let mut again = OrderEvent::default();
/// ElementColumn::SrcUuids.record(&mut again, &sources);
/// assert_eq!(again.get_srcuuids(), [Uuid::from_v8(7)]);
/// // Nothing stated is a null: an empty code, an empty list.
/// assert_eq!(ElementColumn::CrossCode.fact(&OrderEvent::default()), None);
/// assert_eq!(ElementColumn::of_name("SrcUuids"), Some(ElementColumn::SrcUuids));
/// // When an element happened is an event's fact, not an element's.
/// assert_eq!(ElementColumn::of_name("currunix"), None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ElementColumn {
    /// The element's identity; never absent.
    CurrUuid,
    /// The identity every element of one chain shares; the element's own
    /// where it names no cross code, so never absent.
    CrossUuid,
    /// The code naming the chain, as the element spells it; empty where none.
    CrossCode,
    /// The XXH3-64 the element's content digests to; never absent.
    CurrHashCode,
    /// The XXH3-64 of the cross code, zero where none; never absent.
    CrossHashCode,
    /// The identities this element was read from: provenance, never its
    /// chain.
    SrcUuids,
}

impl ElementColumn {
    /// Every column, in the order [`Element`] declares them.
    pub const ALL: [Self; 6] = [
        Self::CurrUuid,
        Self::CrossUuid,
        Self::CrossCode,
        Self::CurrHashCode,
        Self::CrossHashCode,
        Self::SrcUuids,
    ];

    /// The column's name: the fact's, as the trait spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CurrUuid => "curruuid",
            Self::CrossUuid => "crossuuid",
            Self::CrossCode => "crosscode",
            Self::CurrHashCode => "currhashcode",
            Self::CrossHashCode => "crosshashcode",
            Self::SrcUuids => "srcuuids",
        }
    }

    /// The column's display, for a catalog.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::CurrUuid => "Current UUID",
            Self::CrossUuid => "Cross UUID",
            Self::CrossCode => "Cross Code",
            Self::CurrHashCode => "Current Hash Code",
            Self::CrossHashCode => "Cross Hash Code",
            Self::SrcUuids => "Source UUIDs",
        }
    }

    /// What the column holds, for a catalog.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::CurrUuid => {
                "The element's identity: UUIDv7 ordered by millisecond and sequence, with a content payload seeded by its cross hash."
            }
            Self::CrossUuid => {
                "The identity every element of one chain shares, derived from the code they share; the element's own where it names none."
            }
            Self::CrossCode => {
                "The code every element of one chain shares, as the element spells it; empty where none."
            }
            Self::CurrHashCode => "The XXH3-64 of what the element states.",
            Self::CrossHashCode => {
                "The XXH3-64 of the cross code; zero where the element names none."
            }
            Self::SrcUuids => {
                "The sorted unique identities of the elements this element was read from: provenance, never its chain - no walk moves it."
            }
        }
    }

    /// The one datatype the column is built and read at: the identities
    /// the crate's own [`Uuid`], the code `utf8`, the digests `uint64`, the
    /// sources `serie<uuid>` with the item named `srcuuid`.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::CurrUuid | Self::CrossUuid => DataType::Uuid,
            Self::CrossCode => DataType::utf8(),
            Self::CurrHashCode | Self::CrossHashCode => DataType::UInt64,
            Self::SrcUuids => DataType::serie(DataType::Uuid.required_field("srcuuid")),
        }
    }

    /// Whether the column may hold a null: the code and the sources, which
    /// an element answers as nothing where it states none, may; the
    /// identities and the codes it digests to are never absent.
    #[must_use]
    pub const fn nullable(self) -> bool {
        matches!(self, Self::CrossCode | Self::SrcUuids)
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

    /// What an element states under this column, as the raw value the
    /// column's datatype types, or nothing where it states no fact: an
    /// empty code or serie.
    pub fn fact<E: Element + ?Sized>(self, element: &E) -> Option<Scalar> {
        match self {
            Self::CurrUuid => Some(Scalar::Uuid(element.get_curruuid())),
            Self::CrossUuid => Some(Scalar::Uuid(element.get_crossuuid())),
            Self::CrossCode => {
                let code = element.get_crosscode();
                (!code.is_empty()).then(|| Scalar::from(code))
            }
            Self::CurrHashCode => Some(Scalar::from(element.get_currhashcode())),
            Self::CrossHashCode => Some(Scalar::from(element.get_crosshashcode())),
            Self::SrcUuids => uuids_fact(element.get_srcuuids()),
        }
    }

    /// Records what one cell states on the element, through the trait: a
    /// null clears the fact, and a value the fact's type refuses is
    /// silence.
    pub fn record<E: Element + ?Sized>(self, element: &mut E, value: &Scalar) {
        match self {
            Self::CurrUuid => {
                if let Scalar::Uuid(uuid) = value {
                    element.set_curruuid(*uuid);
                }
            }
            Self::CrossUuid => {
                if let Scalar::Uuid(uuid) = value {
                    element.set_crossuuid(*uuid);
                }
            }
            Self::CrossCode => element.set_crosscode(
                value
                    .as_str()
                    .filter(|held| !held.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_default(),
            ),
            Self::CurrHashCode => {
                if let Some(code) = value.as_u64() {
                    element.set_currhashcode(code);
                }
            }
            Self::CrossHashCode => {
                if let Some(code) = value.as_u64() {
                    element.set_crosshashcode(code);
                }
            }
            Self::SrcUuids => element.set_srcuuids(uuids_of(value)),
        }
    }
}

/// One run of identities as the raw value its `serie<uuid>` column types,
/// or nothing where the serie is empty.
fn uuids_fact(uuids: &[Uuid]) -> Option<Scalar> {
    (!uuids.is_empty()).then(|| Scalar::from_sequence(uuids.iter().copied().map(Scalar::Uuid)))
}

/// The identities one `serie<uuid>` cell states, every other item passed
/// over; none for a cell stating no serie.
fn uuids_of(value: &Scalar) -> Vec<Uuid> {
    value
        .as_serie()
        .map(|held| {
            held.iter()
                .filter_map(|item| match item.as_ref() {
                    Scalar::Uuid(uuid) => Some(*uuid),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

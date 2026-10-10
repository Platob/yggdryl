//! The columns a graph element is stated in: one per fact [`Element`]
//! answers.
//!
//! Every generated schema of an element opens with these six, under one
//! name and one datatype each, before the [`EventColumn`](super::EventColumn)s
//! an event adds - a [text line](crate::text::TextLine) read into a batch, a
//! FIX message parsed out of it, a `marketdata` row - so the rows join on
//! them without a mapping: a message's `srcuuids` are the `uuid` of the
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
/// use std::sync::Arc;
///
/// use yggdryl::graph::{Element, ElementColumn};
/// use yggdryl::text::{TextBytes, TextLine, TextOptions};
/// use yggdryl::Uuid;
///
/// # fn main() -> yggdryl::Result<()> {
/// let line = || TextLine::from_bytes(0, TextBytes::from_bytes(b"x")?, Arc::new(TextOptions::new()));
/// let fields = ElementColumn::fields()?;
/// assert_eq!(fields.len(), 6);
/// assert_eq!(fields[0].name(), "uuid");
/// assert_eq!(fields[5].name(), "srcuuids");
/// // What an element states under a column, and the same fact stated back.
/// let mut element = line()?;
/// element.set_srcuuids(vec![Uuid::from_v8(7)]);
/// let sources = ElementColumn::SrcUuids.fact(&element).expect("a source");
/// let mut again = line()?;
/// ElementColumn::SrcUuids.record(&mut again, &sources);
/// assert_eq!(again.get_srcuuids(), [Uuid::from_v8(7)]);
/// // A code is always stated, the empty text where the element names
/// // none; an empty list is a null.
/// assert_eq!(
///     ElementColumn::CrossCode.fact(&line()?),
///     Some(yggdryl::Scalar::from(""))
/// );
/// assert_eq!(ElementColumn::SrcUuids.fact(&line()?), None);
/// assert_eq!(ElementColumn::of_name("SrcUuids"), Some(ElementColumn::SrcUuids));
/// // When an element happened is an event's fact, not an element's.
/// assert_eq!(ElementColumn::of_name("transunix"), None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ElementColumn {
    /// The element's own UUID - its identity; never absent.
    Uuid,
    /// The identity every element of one chain shares; the element's own
    /// where it names no cross code, so never absent.
    CrossUuid,
    /// The code naming the chain, as the element spells it; empty where none.
    CrossCode,
    /// The element's hash code: the XXH3-64 its content digests to; never
    /// absent.
    HashCode,
    /// The XXH3-64 of the cross code, zero where none; never absent.
    CrossHashCode,
    /// The identities this element was read from: provenance, never its
    /// chain.
    SrcUuids,
}

impl ElementColumn {
    /// Every column, in the order [`Element`] declares them.
    pub const ALL: [Self; 6] = [
        Self::Uuid,
        Self::CrossUuid,
        Self::CrossCode,
        Self::HashCode,
        Self::CrossHashCode,
        Self::SrcUuids,
    ];

    /// The column's name: the fact's, as the trait spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Uuid => "uuid",
            Self::CrossUuid => "crossuuid",
            Self::CrossCode => "crosscode",
            Self::HashCode => "hashcode",
            Self::CrossHashCode => "crosshashcode",
            Self::SrcUuids => "srcuuids",
        }
    }

    /// The column's display, for a catalog.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::Uuid => "UUID",
            Self::CrossUuid => "Cross UUID",
            Self::CrossCode => "Cross Code",
            Self::HashCode => "Hash Code",
            Self::CrossHashCode => "Cross Hash Code",
            Self::SrcUuids => "Source UUIDs",
        }
    }

    /// What the column holds, for a catalog.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Uuid => {
                "The element's own UUID: UUIDv7 ordered by millisecond and sequence, with a content payload seeded by its cross hash."
            }
            Self::CrossUuid => {
                "The identity every element of one chain shares, derived from the code they share; the element's own where it names none."
            }
            Self::CrossCode => {
                "The code every element of one chain shares, as the element spells it: the empty text where it names none, never null."
            }
            Self::HashCode => "The XXH3-64 of what the element states.",
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
            Self::Uuid | Self::CrossUuid => DataType::Uuid,
            Self::CrossCode => DataType::utf8(),
            Self::HashCode | Self::CrossHashCode => DataType::UInt64,
            Self::SrcUuids => DataType::serie(DataType::Uuid.required_field("srcuuid")),
        }
    }

    /// Whether the column may hold a null: the sources, which an element
    /// answers as nothing where it states none, may; the identities, the
    /// cross code - the empty text where the element names none - and the
    /// codes it digests to are never absent.
    #[must_use]
    pub const fn nullable(self) -> bool {
        matches!(self, Self::SrcUuids)
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
    /// empty serie. The cross code is always stated, the empty text where
    /// the element names none.
    pub fn fact<E: Element + ?Sized>(self, element: &E) -> Option<Scalar> {
        match self {
            Self::Uuid => Some(Scalar::Uuid(element.get_uuid())),
            Self::CrossUuid => Some(Scalar::Uuid(element.get_crossuuid())),
            Self::CrossCode => Some(Scalar::from(element.get_crosscode())),
            Self::HashCode => Some(Scalar::from(element.get_hashcode())),
            Self::CrossHashCode => Some(Scalar::from(element.get_crosshashcode())),
            Self::SrcUuids => uuids_fact(element.get_srcuuids()),
        }
    }

    /// Records what one cell states on the element, through the trait: a
    /// null clears the fact, and a value the fact's type refuses is
    /// silence.
    pub fn record<E: Element + ?Sized>(self, element: &mut E, value: &Scalar) {
        match self {
            Self::Uuid => {
                if let Scalar::Uuid(uuid) = value {
                    element.set_uuid(*uuid);
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
            // A digest read as its bits or through its column's own value
            // door, so a cell a table stored in another layout - the
            // `decimal(20, 0)` an Iceberg table holds a `uint64` as, or the
            // `long` of its width - states the number it was; a cell the
            // door refuses states nothing.
            Self::HashCode => {
                if let Some(code) = digest_u64(value) {
                    element.set_hashcode(code);
                }
            }
            Self::CrossHashCode => {
                if let Some(code) = digest_u64(value) {
                    element.set_crosshashcode(code);
                }
            }
            Self::SrcUuids => element.set_srcuuids(uuids_of(value)),
        }
    }
}

/// The `uint64` one cell states, read through the datatype's own value
/// door: an integer of any width that fits, an enum's code, a decimal with
/// no fraction; `None` for a null or a value the door refuses.
pub(crate) fn whole_u64(value: &Scalar) -> Option<u64> {
    if value.is_null() {
        return None;
    }
    DataType::UInt64
        .scalar(value.clone())
        .ok()
        .and_then(|whole| whole.as_u64())
}

/// The digest one cell states: an `int64` cell's bits - the reading
/// `FIELD:representation=bits` states, which a digest column takes whatever
/// the schema it was read under says: an XXH3-64 is a bit pattern, a table
/// with no unsigned type stores it as the `long` of its width, and a
/// negative cell is no other `u64` - else [`whole_u64`].
pub(crate) fn digest_u64(value: &Scalar) -> Option<u64> {
    crate::integer::bits_reading(&DataType::UInt64, value)
        .and_then(|bits| bits.as_u64())
        .or_else(|| whole_u64(value))
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

//! The handful of facts every consumer wants, without each one walking groups.
//!
//! Who, what, how much, when. A monitor, a book and a risk system ask the same
//! four questions of a message, and each would otherwise derive the answers
//! from its own set of tags - which is how two systems come to disagree about
//! what one message said.
//!
//! # A lift answers only when the answer is unambiguous
//!
//! This is the whole design. One occurrence is one value and one value is what
//! a column can answer with; a tag that repeats belongs to a repeating group,
//! so no occurrence is the line's; a multi-leg order has no one symbol, and
//! saying so is the honest column. Two candidates therefore answer `None`,
//! never the first - and a caller wanting a specific one addresses it, through
//! [`party`](FixMsg::party) or
//! [`trd_reg_timestamp`](FixMsg::trd_reg_timestamp).
//!
//! # It is a table, not code per facet
//!
//! A facet is a name and an ordered list of sources; a source is a tag,
//! optionally conditioned on the message type - and on nothing else, because a
//! price means different things in an ExecutionReport and a Quote and the
//! message type is the one fact that says which. Adding a facet is a row.
//!
//! Nothing is stored. Every answer borrows from the row that is already there,
//! so two lifts of one message are the same walk twice with no cached state to
//! go stale, and a derived answer is indistinguishable from a stated one.

use std::sync::LazyLock;

use crate::{DataType, Field, Scalar, Version};

use super::FixId;
use super::msg::FixMsg;

/// One place a facet may be answered from.
///
/// The message type condition is the only one there is: no venue conditions,
/// no branch conditions, no value conditions. Those are an expression layer,
/// and this is a table.
pub(super) struct FixSource {
    /// The flat tag read.
    tag: i32,
    /// The message types this source speaks for; empty means all of them.
    msgtypes: &'static [&'static str],
}

impl FixSource {
    /// A source that speaks for every message type.
    const fn any(tag: i32) -> Self {
        Self { tag, msgtypes: &[] }
    }

    /// A source that speaks only for the listed message types.
    const fn typed(tag: i32, msgtypes: &'static [&'static str]) -> Self {
        Self { tag, msgtypes }
    }

    /// Whether this source speaks for a message of this type.
    fn applies(&self, msgtype: &str) -> bool {
        self.msgtypes.is_empty() || self.msgtypes.contains(&msgtype)
    }
}

/// One lifted facet: where it may come from, in priority order.
pub struct FixLift {
    facet: &'static str,
    sources: &'static [FixSource],
}

impl FixLift {
    /// The facet's name, which is also the name of its column.
    #[must_use]
    pub const fn facet(&self) -> &'static str {
        self.facet
    }

    /// The tags this facet may be answered from, in priority order.
    pub fn tags(&self) -> impl Iterator<Item = i32> + '_ {
        self.sources.iter().map(|source| source.tag)
    }
}

/// The message types carrying an order rather than a report of one.
///
/// Consulted only by the lane enrichment, which is about what a party is
/// *willing* to do. A fill is a report and projects nothing.
const ORDERS: &[&str] = &["D", "E", "F", "G", "AB", "AC"];

/// The message types carrying a quote rather than an order.
const QUOTES: &[&str] = &["S", "i", "W", "X", "b", "R"];

/// The message types carrying a fill.
const FILLS: &[&str] = &["8", "AE", "AK"];

/// The symbolic names whose side takes the bid lane.
///
/// Domain knowledge, written where a reviewer can check it: Orchestra does not
/// publish which side takes which lane. The names are resolved to wire values
/// through the dictionary's own code set and never matched as text - a
/// `SellShortExempt` beginning with `Sell` is a fact about English, and
/// reasoning from it is exactly what these listings exist to avoid.
const BID_LANE: &[&str] = &["Buy", "BuyMinus"];

/// The symbolic names whose side takes the ask lane.
///
/// Everything absent from both listings - `Cross`, `CrossShort`,
/// `CrossShortExempt`, `Undisclosed`, `AsDefined`, `Opposite`, and any code a
/// dialect adds - takes no lane. A cross is both sides at once and `Opposite`
/// means "whatever the other leg was", so neither can fill one.
const ASK_LANE: &[&str] = &[
    "Sell",
    "SellPlus",
    "SellShort",
    "SellShortExempt",
    "SellUndisclosed",
];

/// Every facet, in the order a batch writer's columns take.
///
/// Names obey the field-name rule - lowercase letters and digits, no
/// separators - so a lifted column sits beside a field name with no change of
/// style. Where a facet names one FIX field it *is* that field case-folded;
/// `askpx` and `asksize` are the two exceptions, because FIX spells that lane
/// `Offer` and every reader calls it the ask.
///
/// A source naming a tag no dictionary has is skipped rather than refused: a
/// facet is a best answer, not a schema.
static LIFTS: &[FixLift] = &[
    FixLift {
        facet: "id",
        sources: &[
            FixSource::typed(11, &["D", "E", "F", "G", "AB", "AC"]),
            FixSource::typed(37, FILLS),
            FixSource::typed(117, QUOTES),
            FixSource::typed(1003, &["AE", "AK"]),
            FixSource::any(11),
            FixSource::any(37),
            FixSource::any(117),
            FixSource::any(1003),
        ],
    },
    FixLift {
        facet: "secondaryid",
        sources: &[FixSource::any(526), FixSource::any(198), FixSource::any(41)],
    },
    FixLift {
        facet: "execid",
        sources: &[FixSource::any(17)],
    },
    FixLift {
        facet: "symbol",
        sources: &[FixSource::any(55), FixSource::any(48)],
    },
    FixLift {
        facet: "side",
        sources: &[FixSource::any(54)],
    },
    FixLift {
        facet: "price",
        sources: &[
            FixSource::typed(31, FILLS),
            FixSource::any(44),
            FixSource::any(31),
        ],
    },
    FixLift {
        facet: "bidpx",
        sources: &[FixSource::any(132)],
    },
    FixLift {
        facet: "askpx",
        sources: &[FixSource::any(133)],
    },
    FixLift {
        facet: "bidsize",
        sources: &[FixSource::any(134)],
    },
    FixLift {
        facet: "asksize",
        sources: &[FixSource::any(135)],
    },
    FixLift {
        facet: "quantity",
        sources: &[
            FixSource::typed(32, FILLS),
            FixSource::typed(38, ORDERS),
            FixSource::any(38),
            FixSource::any(32),
            FixSource::any(53),
            FixSource::any(14),
            FixSource::any(151),
        ],
    },
    FixLift {
        facet: "quantitytype",
        sources: &[FixSource::any(854), FixSource::any(465)],
    },
    FixLift {
        facet: "currency",
        sources: &[FixSource::any(15), FixSource::any(120)],
    },
    FixLift {
        facet: "transacttime",
        sources: &[FixSource::any(60), FixSource::any(52)],
    },
    FixLift {
        facet: "status",
        sources: &[FixSource::any(39), FixSource::any(150), FixSource::any(297)],
    },
    FixLift {
        facet: "sendingtime",
        sources: &[FixSource::any(52), FixSource::any(122)],
    },
    FixLift {
        facet: "seqnum",
        sources: &[FixSource::any(34)],
    },
    FixLift {
        facet: "sender",
        sources: &[FixSource::any(49), FixSource::any(115)],
    },
    FixLift {
        facet: "target",
        sources: &[FixSource::any(56), FixSource::any(128)],
    },
    FixLift {
        facet: "resent",
        sources: &[FixSource::any(43), FixSource::any(97)],
    },
    FixLift {
        facet: "text",
        sources: &[FixSource::any(58), FixSource::any(355)],
    },
    FixLift {
        facet: "reason",
        sources: &[
            FixSource::typed(373, &["3"]),
            FixSource::typed(380, &["j"]),
            FixSource::typed(103, FILLS),
            FixSource::typed(102, &["9"]),
            FixSource::any(373),
            FixSource::any(380),
            FixSource::any(103),
            FixSource::any(102),
        ],
    },
];

/// The rule one facet resolves by, when the table has one.
#[must_use]
pub fn fix_lift(facet: &str) -> Option<&'static FixLift> {
    LIFTS.iter().find(|lift| lift.facet == facet)
}

/// Every facet the table declares, in column order.
pub fn fix_lifts() -> impl Iterator<Item = &'static FixLift> {
    LIFTS.iter()
}

/// The side a bid lane implies, as the packed datatype types it.
///
/// The two lane listings say which *names* fill a lane, and the dictionary
/// translates a stated side's spelling before it is matched against them. A
/// *derived* side is the other direction and has no spelling to translate, so
/// it is the wire value FIX's side code set has given `Buy` since 2.7. A
/// dialect that renumbers its side code set still lifts a stated side through
/// its own dictionary; it simply gets no derived one, which is P9-R1 rather
/// than a gap.
static DERIVED_BID: LazyLock<Scalar> = LazyLock::new(|| packed_side("1"));

/// The side an ask lane implies, as the packed datatype types it.
static DERIVED_ASK: LazyLock<Scalar> = LazyLock::new(|| packed_side("2"));

/// One wire side value through the crate's own value contract.
fn packed_side(wire: &str) -> Scalar {
    DataType::Side
        .scalar(Scalar::from(wire))
        .unwrap_or(Scalar::Null)
}

/// One occurrence of the Parties group, addressed by the role it bears.
///
/// A party is never addressed by position: every message carries several
/// occurrences, and which one is third is not a fact about the trade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixParty<'msg> {
    id: Option<&'msg Scalar>,
    source: Option<&'msg Scalar>,
    role: Option<&'msg Scalar>,
    qualifier: Option<&'msg Scalar>,
}

impl<'msg> FixParty<'msg> {
    /// The party's identifier, `PartyID(448)`.
    #[must_use]
    pub const fn id(&self) -> Option<&'msg Scalar> {
        self.id
    }

    /// What scheme that identifier is in, `PartyIDSource(447)`.
    #[must_use]
    pub const fn source(&self) -> Option<&'msg Scalar> {
        self.source
    }

    /// The role borne, `PartyRole(452)`, as the row types it.
    #[must_use]
    pub const fn role(&self) -> Option<&'msg Scalar> {
        self.role
    }

    /// That role's qualifier, `PartyRoleQualifier(2376)`.
    #[must_use]
    pub const fn qualifier(&self) -> Option<&'msg Scalar> {
        self.qualifier
    }
}

/// The position one tag holds among a group item's members.
fn position_of(item: &Field, tag: i32) -> Option<usize> {
    item.dtype()
        .as_fields()?
        .iter()
        .position(|field| field.as_fix().tag().ok().flatten() == Some(tag))
}

/// One member of one occurrence by position, absent when it states nothing.
fn at(occurrence: &Scalar, position: Option<usize>) -> Option<&Scalar> {
    let held = occurrence.as_sequence()?.get(position?)?;
    (!held.is_null()).then_some(held)
}

/// The `item` field a group's List declares.
fn item_of(field: &Field) -> Option<&Field> {
    match field.dtype() {
        DataType::List(item) | DataType::LargeList(item) => Some(item),
        _ => None,
    }
}

impl FixMsg {
    /// The one value a facet names, or `None` when this message does not
    /// answer it unambiguously.
    ///
    /// Version-aware for free: it addresses tags, and this message's own
    /// fields already carry the names and types its version gave them, so a
    /// 4.2 message lifts `quantity` from tag 32 whether that tag is spelled
    /// `LastShares` or `LastQty`.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixReader, FixRegistry};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    /// let reader = FixReader::new(Arc::new(registry));
    /// let order = reader.text("8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|38=100|10=0|")?;
    ///
    /// assert_eq!(order.lifted("id").and_then(yggdryl::Scalar::as_str), Some("ORDER-1"));
    /// assert_eq!(order.lifted("symbol").and_then(yggdryl::Scalar::as_str), Some("AAPL"));
    /// // A buy order at a price is a party willing to pay it, which is a bid.
    /// assert_eq!(order.lifted("bidsize"), order.lifted("quantity"));
    /// assert_eq!(order.lifted("asksize"), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn lifted(&self, facet: &str) -> Option<&Scalar> {
        self.lift_resolved(facet).map(|(_, value)| value)
    }

    /// Which source answered a facet, so a fallback is visible.
    ///
    /// A latency computed from `SendingTime` when `TransactTime` was meant is
    /// not a latency, and a monitor cannot tell the two apart from the value
    /// alone. A derived answer names the tag it was derived *from*, which is
    /// the same question asked of a different lane.
    #[must_use]
    pub fn lift_source(&self, facet: &str) -> Option<FixId> {
        self.lift_resolved(facet).map(|(id, _)| id)
    }

    /// Every facet this message answers, for a batch writer.
    ///
    /// Yields in the table's own order, so two messages produce their columns
    /// in one order whatever each of them happens to carry.
    pub fn lift(&self) -> impl Iterator<Item = (&'static str, &Scalar)> {
        LIFTS
            .iter()
            .filter_map(|lift| Some((lift.facet, self.lifted(lift.facet)?)))
    }

    /// The party bearing one role, which is how a party is addressed.
    ///
    /// The role is matched through the code translation, so a caller asks for
    /// `"ClearingFirm"` rather than for `4`. `None` when no occurrence bears
    /// the role, and equally when several do.
    #[must_use]
    pub fn party(&self, role: &str) -> Option<FixParty<'_>> {
        let (item, occurrences) = self.group(453)?;
        let wanted = self.coded(452, item, role)?;
        // The members hold the same positions in every occurrence, so they are
        // located once rather than per occurrence: locating one reads a tag
        // out of each member's metadata, which is not a per-row cost.
        let at_role = position_of(item, 452)?;
        let at_id = position_of(item, 448);
        let at_source = position_of(item, 447);
        let at_qualifier = position_of(item, 2376);
        let mut found: Option<FixParty<'_>> = None;
        for occurrence in occurrences {
            if at(occurrence, Some(at_role)) != Some(&wanted) {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(FixParty {
                id: at(occurrence, at_id),
                source: at(occurrence, at_source),
                role: at(occurrence, Some(at_role)),
                qualifier: at(occurrence, at_qualifier),
            });
        }
        found
    }

    /// One regulatory timestamp by its type.
    ///
    /// Matched through the same code translation, so a caller asks for
    /// `"ExecutionTime"` rather than for `1`.
    #[must_use]
    pub fn trd_reg_timestamp(&self, kind: &str) -> Option<&Scalar> {
        let (item, occurrences) = self.group(768)?;
        let wanted = self.coded(770, item, kind)?;
        let at_kind = position_of(item, 770)?;
        let at_time = position_of(item, 769);
        let mut found = None;
        for occurrence in occurrences {
            if at(occurrence, Some(at_kind)) != Some(&wanted) {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = at(occurrence, at_time);
        }
        found
    }

    /// The source and the value of one facet, resolved once for both readers.
    fn lift_resolved(&self, facet: &str) -> Option<(FixId, &Scalar)> {
        let lift = fix_lift(facet)?;
        let msgtype = self.msgtype();
        let at = self.version();
        // The specification supersedes fields repeatedly - `QuantityType(465)`
        // by `QtyType(854)`, and so on - and the lineage already says which is
        // which. Every source this message's version has not deprecated is
        // tried before any it has. One rule, not a special case per pair.
        //
        // One pass, and deprecation is asked only of a source that actually
        // answered. Asking first and reading second walks the lineage of
        // every source in the table, and all but one of them had nothing to
        // say: the question is about the answer, not about the source.
        let mut superseded = None;
        for source in lift.sources {
            if !source.applies(msgtype) {
                continue;
            }
            let Some(value) = self.flat(source.tag) else {
                continue;
            };
            if !self.is_deprecated(source.tag, at) {
                return Some((self.identify(source.tag), value));
            }
            if superseded.is_none() {
                superseded = Some((self.identify(source.tag), value));
            }
        }
        superseded.or_else(|| self.derived(facet, msgtype))
    }

    /// One flat value a tag names, when exactly one occurrence carries it.
    ///
    /// A sequence is a tag that repeated, which belongs to a repeating group,
    /// so no occurrence is the message's; a null states nothing. Group members
    /// are never reached - a `price` inside a leg is that leg's, not the
    /// message's - because only the root's own children are read.
    fn flat(&self, tag: i32) -> Option<&Scalar> {
        let held = self.get_by_tag(tag)?;
        if held.is_null() || held.as_sequence().is_some() {
            return None;
        }
        Some(held)
    }

    /// This message's type, which is what the root is named.
    fn msgtype(&self) -> &str {
        self.as_field().name()
    }

    /// Whether the dictionary has deprecated a tag by this message's version.
    fn is_deprecated(&self, tag: i32, at: Option<Version>) -> bool {
        let Some(at) = at else {
            return false;
        };
        self.registry()
            .get_field_by_tag(tag)
            .is_some_and(|field| field.as_fix().deprecated_at(at))
    }

    /// The identity a tag names in this message's dialect.
    fn identify(&self, tag: i32) -> FixId {
        FixId::from_parts(self.branch(), tag).unwrap_or_else(|_| FixId::standard(tag))
    }

    /// One symbolic code spelling as the member field's own typed value.
    ///
    /// Two halves, because a group's members carry their tag and not the code
    /// set: the dictionary field translates the spelling to a wire value, and
    /// the member field types it, so the comparison against an occurrence is
    /// `Scalar` against `Scalar` and happens once rather than per occurrence.
    fn coded(&self, tag: i32, item: &Field, spelling: &str) -> Option<Scalar> {
        let field = self.registry().get_field_by_tag(tag)?;
        let view = field.as_fix();
        let wire = match self.version() {
            Some(at) => view.code_value_at(at, spelling),
            None => view.code_value(spelling),
        }
        .unwrap_or(spelling);
        let member = item.dtype().as_fields()?.get(position_of(item, tag)?)?;
        let value = crate::text::prepare_text(Scalar::from(wire), member)
            .unwrap_or_else(|_| Scalar::from(wire));
        member.scalar(value).ok()
    }

    /// A group's `item` field and the occurrences it holds.
    fn group(&self, tag: i32) -> Option<(&Field, &[Scalar])> {
        let index = self.index_of_tag(tag)?;
        let field = self.as_field().dtype().as_fields()?.get(index)?;
        let occurrences = self.get_by_tag(tag)?.as_sequence()?;
        Some((item_of(field)?, occurrences))
    }

    /// The facets a message answers without stating them.
    ///
    /// Enrichment fills and never overwrites: this runs only after every
    /// stated source has missed. A message carrying `BidPx` *and* `Side`
    /// answers both as stated, and a disagreement between them is reported
    /// through [`anomalies`](Self::anomalies) rather than resolved here.
    fn derived(&self, facet: &str, msgtype: &str) -> Option<(FixId, &Scalar)> {
        // One lane implies a side; two lanes imply nothing. A quote carrying
        // only `BidPx` is a party bidding, so the side is `Buy`; a two-sided
        // quote carries both lanes and no single side is the message's, which
        // is the case that makes this rule safe to have at all.
        if facet != "side" || !QUOTES.contains(&msgtype) {
            return self.lane_of(facet, msgtype);
        }
        match (self.flat(132).is_some(), self.flat(133).is_some()) {
            (true, false) => Some((self.identify(132), &DERIVED_BID)),
            (false, true) => Some((self.identify(133), &DERIVED_ASK)),
            _ => None,
        }
    }

    /// A lane facet filled from an order's stated side and price.
    ///
    /// A buy order at `P` is a party willing to pay `P`, which is a bid; a
    /// sell order at `P` is an offer. `LastPx(31)` never projects: a fill's
    /// price is a traded price, not a quote lane, and putting it on one would
    /// state a quote that never existed. The message type is what tells the
    /// two apart.
    fn lane_of(&self, facet: &str, msgtype: &str) -> Option<(FixId, &Scalar)> {
        let (lane, tag) = match facet {
            "bidpx" => (BID_LANE, 44),
            "askpx" => (ASK_LANE, 44),
            "bidsize" => (BID_LANE, 38),
            "asksize" => (ASK_LANE, 38),
            _ => return None,
        };
        if !ORDERS.contains(&msgtype) || !self.in_lane(lane) {
            return None;
        }
        Some((self.identify(tag), self.flat(tag)?))
    }

    /// Whether this message's stated side takes `lane`.
    ///
    /// The side's own value is named once and the name is matched against the
    /// listing, rather than each listed name being translated to a value and
    /// compared. Naming a value addresses one record; translating a name
    /// scans the whole code set to prove no second one is spelled the same,
    /// and this asked for that five times to decide one lane.
    ///
    /// It is still the canonical name that is compared, never a prefix of it.
    fn in_lane(&self, lane: &'static [&'static str]) -> bool {
        let Some(text) = self.flat(54).and_then(Scalar::as_str) else {
            return false;
        };
        let Some(field) = self.registry().get_field_by_tag(54) else {
            return false;
        };
        let view = field.as_fix();
        let named = match self.version() {
            Some(at) => view.code_name_at(at, text),
            None => view.code_name(text),
        };
        named.is_some_and(|name| {
            lane.iter()
                .any(|held| crate::types::folds_equal(held, name))
        })
    }
}

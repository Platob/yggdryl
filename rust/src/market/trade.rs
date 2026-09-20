//! The settled transaction an execution reports: the matched quantity at
//! its price, its identifier, its parties and its clocks.

use crate::graph::{Element, Event, MarketElement, MarketEvent, MarketEventData};
use crate::{DataType, Field, Result, Scalar, StructType};

use super::product::{Cells, MarketColumn, Product, column, feed, finished};

/// One party to a trade: who, in which role, under the scheme that names
/// them.
///
/// FIX's `Parties` occurrence, held as the text the venue stated: the role
/// is what the message spells for `PartyRole(452)`, a code or a word, the
/// identifier `PartyID(448)`, and the source `PartyIDSource(447)`, empty
/// where none was stated. Text rather than a code, because who names a
/// party and how is the venue's to say and no standard closes it.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Party {
    /// The role the party plays, as the message spells it.
    pub role: String,
    /// The party's identifier under the source that issued it.
    pub id: String,
    /// The scheme the identifier is issued under; empty where none.
    pub source: String,
}

/// What a trade answers beyond the market's facts: its clocks and its
/// parties, and what they imply.
///
/// A `get_`/`set_` pair reads and records a stated fact; a bare noun reads
/// what the stated facts imply, on every call, and stores nothing. The
/// matched price and quantity are [`MarketElement::get_px`] and
/// [`MarketElement::get_qty`], their worth [`MarketElement::notional`].
pub trait Trade: MarketEvent {
    /// The day the trade was done, as days since the Unix epoch, where the
    /// report states one.
    fn get_tradedate(&self) -> Option<i32>;

    /// Records the trade date; `None` states none.
    fn set_tradedate(&mut self, days: Option<i32>);

    /// The day the trade settles, as days since the Unix epoch, where the
    /// report states one.
    fn get_settldate(&self) -> Option<i32>;

    /// Records the settlement date; `None` states none.
    fn set_settldate(&mut self, days: Option<i32>);

    /// The parties to the trade, in the order the report states them.
    fn get_parties(&self) -> &[Party];

    /// Records the parties; the list is replaced whole.
    fn set_parties(&mut self, parties: Vec<Party>);

    /// The first party whose role is `role`, exactly as the report spells
    /// it; nothing where none is.
    fn party_by_role(&self, role: &str) -> Option<&Party> {
        self.get_parties().iter().find(|party| party.role == role)
    }

    /// How many days after the trade date it settles, where both are
    /// stated: `T+2` answers `2`.
    fn settlement_days(&self) -> Option<i32> {
        Some(self.get_settldate()? - self.get_tradedate()?)
    }
}

/// One statement of a trade: the settled transaction an execution reports.
///
/// The chain is the trade's own: `crosscode` is the identifier every
/// statement of one trade shares - the venue's `TrdMatchID`, else its
/// `TradeID`, its `TradeReportID` or the `ExecID` of the report - and
/// `crossuuid` the identity it derives, so the two sides' reports of one
/// match stand in one chain. `px` and `qty` are the matched price and
/// quantity, the side the reporting side's, the instrument under the codes
/// the market named it by, the parties who traded, and the clocks: when it
/// traded is `currunix`, the trade date and the settlement date its own.
///
/// A trade is a value of this crate: its identity is what its content
/// digests to - the parties and the dates among it - its sources the
/// messages it was read from, its chain the walk's to fill, and its row
/// the sixteen event columns, the market's facts, the two dates and the
/// parties, [`TradeData::field`].
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{Party, Product, Trade, TradeData};
/// use yggdryl::{Currency, Decimal18, Side};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut trade = TradeData::at(1_700_000_000_000_000_000);
/// trade.set_crosscode("M-1".to_owned());
/// trade.set_px("82.5".parse()?);
/// trade.set_qty(Decimal18::from_int(300));
/// trade.set_side(Side::read("1")?);
/// trade.set_currency(Currency::new("USD")?);
/// trade.set_tradedate(Some(19_676));
/// trade.set_settldate(Some(19_678));
/// trade.set_parties(vec![Party { role: "executingfirm".to_owned(), id: "SWXCCP".to_owned(), source: String::new() }]);
/// trade.finalize();
/// assert_eq!(trade.get_curruuid(), trade.time_uuid()?);
/// assert_eq!(trade.settlement_days(), Some(2));
/// assert_eq!(trade.party_by_role("executingfirm").map(|party| party.id.as_str()), Some("SWXCCP"));
/// assert_eq!(trade.notional(), Some(Decimal18::from_int(24_750)));
/// let field = TradeData::field()?;
/// assert_eq!(field.fields().last().map(yggdryl::Field::name), Some("parties"));
/// let again = TradeData::from_row(&field, &trade.into_row()?)?;
/// assert_eq!(again, trade);
/// // Another party is another statement.
/// let mut other = trade.clone();
/// other.set_parties(Vec::new());
/// other.finalize();
/// assert_ne!(other.get_curruuid(), trade.get_curruuid());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct TradeData {
    event: MarketEventData,
    tradedate: Option<i32>,
    settldate: Option<i32>,
    parties: Vec<Party>,
}

impl TradeData {
    /// A trade stated at `unix`, nanoseconds since the Unix epoch, stating
    /// nothing else yet.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self {
            event: MarketEventData::at(unix),
            tradedate: None,
            settldate: None,
            parties: Vec::new(),
        }
    }

    /// The row every trade publishes: [`Product::row_field`].
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal, which the fixed columns never
    /// raise.
    pub fn field() -> Result<Field> {
        Self::default().row_field()
    }

    /// The event the trade is.
    #[must_use]
    pub const fn event(&self) -> &MarketEventData {
        &self.event
    }

    /// The event, to write.
    pub(crate) const fn event_mut(&mut self) -> &mut MarketEventData {
        &mut self.event
    }

    /// The identity settled from what the trade states: the market's
    /// facts, the two dates and the parties.
    fn settle(&mut self) {
        self.fill_market();
        self.sync_cross();
        let mut state = self.digest_market_event();
        if let Some(days) = self.tradedate {
            feed(&mut state, "tradedate", &days.to_le_bytes());
        }
        if let Some(days) = self.settldate {
            feed(&mut state, "settldate", &days.to_le_bytes());
        }
        for party in &self.parties {
            feed(&mut state, "partyrole", party.role.as_bytes());
            feed(&mut state, "partyid", party.id.as_bytes());
            feed(&mut state, "partyidsource", party.source.as_bytes());
        }
        let code = finished(&state);
        self.finalized(code);
    }

    /// The dates and the parties the later statement states, else this
    /// one's.
    fn merge_own(&mut self, other: &Self, later: bool) -> bool {
        let mut changed = false;
        let stated = |this: Option<i32>, other: Option<i32>| {
            if later {
                other.or(this)
            } else {
                this.or(other)
            }
        };
        let tradedate = stated(self.tradedate, other.tradedate);
        if tradedate != self.tradedate {
            self.tradedate = tradedate;
            changed = true;
        }
        let settldate = stated(self.settldate, other.settldate);
        if settldate != self.settldate {
            self.settldate = settldate;
            changed = true;
        }
        let parties_lead = if later {
            !other.parties.is_empty()
        } else {
            self.parties.is_empty() && !other.parties.is_empty()
        };
        if parties_lead && other.parties != self.parties {
            self.parties.clone_from(&other.parties);
            changed = true;
        }
        changed
    }
}

impl Default for TradeData {
    /// A trade at the epoch, stating nothing.
    fn default() -> Self {
        Self::at(0)
    }
}

super::product::holds_market_event!(TradeData via event, event_mut);

impl Trade for TradeData {
    fn get_tradedate(&self) -> Option<i32> {
        self.tradedate
    }

    fn set_tradedate(&mut self, days: Option<i32>) {
        self.tradedate = days;
    }

    fn get_settldate(&self) -> Option<i32> {
        self.settldate
    }

    fn set_settldate(&mut self, days: Option<i32>) {
        self.settldate = days;
    }

    fn get_parties(&self) -> &[Party] {
        &self.parties
    }

    fn set_parties(&mut self, parties: Vec<Party>) {
        self.parties = parties;
    }
}

/// The datatype one party is stated in.
fn party_dtype() -> Result<DataType> {
    Ok(DataType::from(StructType::from_fields([
        column(
            "role",
            "Role",
            "The role the party plays, as the message spells it.",
            DataType::utf8(),
            false,
        )?,
        column(
            "id",
            "Id",
            "The party's identifier under the source that issued it.",
            DataType::utf8(),
            false,
        )?,
        column(
            "source",
            "Source",
            "The scheme the identifier is issued under; empty where none.",
            DataType::utf8(),
            true,
        )?,
    ])?))
}

impl Product for TradeData {
    const NAME: &'static str = "trade";

    const DISPLAY: &'static str = "Trade";

    const DESCRIPTION: &'static str = "One statement of a trade: the settled transaction an execution reports - the \
         matched quantity at its price, its identifier, its parties and its clocks.";

    const MARKET: &'static [MarketColumn] = &[
        MarketColumn::Px,
        MarketColumn::Qty,
        MarketColumn::Side,
        MarketColumn::Currency,
        MarketColumn::Unit,
        MarketColumn::SymbolTicker,
        MarketColumn::IsinCode,
        MarketColumn::CusipCode,
        MarketColumn::SedolCode,
        MarketColumn::BloombergCode,
        MarketColumn::CfiCode,
        MarketColumn::MicCode,
    ];

    const OWN: &'static [&'static str] = &["tradedate", "settldate", "parties"];

    fn own_fields(&self) -> Result<Vec<Field>> {
        Ok(vec![
            column(
                "tradedate",
                "TradeDate",
                "The day the trade was done; empty where the report states none.",
                DataType::date32(),
                true,
            )?,
            column(
                "settldate",
                "SettlDate",
                "The day the trade settles; empty where the report states none.",
                DataType::date32(),
                true,
            )?,
            column(
                "parties",
                "Parties",
                "The parties to the trade, each a role, an identifier and its source, in \
                 the order the report states them; empty where it states none.",
                DataType::list(party_dtype()?.required_field("party")),
                true,
            )?,
        ])
    }

    fn own_cells(&self, cells: &mut Vec<Scalar>) -> Result<()> {
        cells.push(self.tradedate.map_or(Scalar::Null, Scalar::date32));
        cells.push(self.settldate.map_or(Scalar::Null, Scalar::date32));
        cells.push(if self.parties.is_empty() {
            Scalar::Null
        } else {
            Scalar::from_sequence(self.parties.iter().map(|party| {
                Scalar::from_sequence([
                    Scalar::from(party.role.as_str()),
                    Scalar::from(party.id.as_str()),
                    if party.source.is_empty() {
                        Scalar::Null
                    } else {
                        Scalar::from(party.source.as_str())
                    },
                ])
            }))
        });
        Ok(())
    }

    fn from_cells(cells: &Cells<'_>) -> Result<Self> {
        let mut trade = Self::default();
        cells.record(&mut trade);
        trade.tradedate = days_of(cells.own(0));
        trade.settldate = days_of(cells.own(1));
        trade.parties = cells
            .own(2)
            .as_sequence()
            .map(|parties| parties.iter().filter_map(party_of).collect())
            .unwrap_or_default();
        Ok(trade)
    }
}

/// The day one `date32` cell states, or nothing for a cell stating no day.
fn days_of(value: &Scalar) -> Option<i32> {
    match value {
        Scalar::Date32(held) => Some(held.count()),
        _ => None,
    }
}

/// The party one occurrence states, read as the ordered row it
/// canonicalizes to or as the named input shape it may still be; nothing
/// for a cell stating no party.
fn party_of(value: &Scalar) -> Option<Party> {
    let text = |held: Option<&Scalar>| {
        held.and_then(Scalar::as_str)
            .map(str::to_owned)
            .unwrap_or_default()
    };
    if let Some(cells) = value.as_sequence() {
        return Some(Party {
            role: text(cells.first()),
            id: text(cells.get(1)),
            source: text(cells.get(2)),
        });
    }
    let named = value.as_struct()?;
    Some(Party {
        role: text(named.get("role")),
        id: text(named.get("id")),
        source: text(named.get("source")),
    })
}

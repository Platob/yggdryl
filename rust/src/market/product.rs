//! What the five products share: the columns a market fact is stated in,
//! the row every product publishes, the doors between a product and its
//! row, and the fold a stream of statements takes before it is walked.
//!
//! A product is a [`MarketEvent`] with facts of its own, and this module
//! is what makes five of them one vocabulary: [`MarketColumn`] names the
//! columns the market facts the traits answer are stated in, one name and
//! one datatype each, as [`EventColumn`] names the sixteen every event
//! opens with; [`Product`] is what a product answers about its row - the
//! market columns it publishes, the columns of its own after them, and
//! how it fills and reads them - and provides the row, the door into it
//! and the door out of it over those answers; [`Folded`] folds the
//! statements of one instant that are one product into one; and
//! [`arrow_reader`] and [`read_arrow_reader`] are the row doors every
//! product's Arrow twin composes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use crate::arrow::BatchReader;
use crate::graph::{Event, EventColumn, EventIterator, MarketElement, MarketEvent};
use crate::xxhash::Xxh3;
use crate::{
    Currency, DataType, Decimal18, Error, Field, Result, Scalar, Side, StructType, TimeInForce,
    Value,
};

/// One column a market fact is stated in: one per fact [`MarketElement`]
/// answers, under one name and one datatype each.
///
/// The name is the trait's own - what `get_px` reads is the `px` column -
/// and the datatype is the fact's, never the text a venue spelled it in:
/// every price and quantity a [`Decimal18`], the side, the currency, the
/// time in force and the instrument's identifiers the crate's own codes,
/// whether it can trade a boolean, the unit and the ticker text. Which of
/// them a product publishes is the product's to say, through
/// [`Product::MARKET`]; [`Self::fact`] and [`Self::record`] state one
/// through the trait, so a row and a value agree whichever product holds
/// them.
///
/// ```
/// use yggdryl::graph::{MarketElement, MarketEventData};
/// use yggdryl::market::MarketColumn;
/// use yggdryl::{Currency, DataType, Decimal18, Scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut event = MarketEventData::at(1_700_000_000_000_000_000);
/// event.set_px("82.5".parse()?);
/// event.set_currency(Currency::new("USD")?);
/// assert_eq!(MarketColumn::Px.name(), "px");
/// assert_eq!(MarketColumn::Px.datatype(), Decimal18::dtype());
/// assert_eq!(MarketColumn::Currency.datatype(), DataType::Currency);
/// assert_eq!(MarketColumn::Px.fact(&event), Some(Scalar::from("82.5".parse::<Decimal18>()?)));
/// // A quantity of nothing is stated as nothing, and a null clears a fact.
/// assert_eq!(MarketColumn::Qty.fact(&event), None);
/// MarketColumn::Px.record(&mut event, &Scalar::Null);
/// assert_eq!(event.get_px(), Decimal18::ZERO);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MarketColumn {
    /// The price the element is about; nothing where it is about none.
    Px,
    /// The currency the price is quoted in; `XXX` where none, never absent.
    Currency,
    /// The quantity the element is about; nothing where it is about none.
    Qty,
    /// The unit the quantity is counted in; nothing where the market says none.
    Unit,
    /// Which side of the market the element stood on; `UNKNOWN` where none,
    /// never absent.
    Side,
    /// How long the element stands, as the code FIX spells it.
    Tif,
    /// Whether the element can be traded right now, where the market says.
    Tradable,
    /// The ticker the instrument is known by, where it is known by one.
    SymbolTicker,
    /// The price the element averaged, where it states one.
    AvgPx,
    /// How much of the element's quantity is done, where it states it.
    CumQty,
    /// How much of it is still open, where it states it.
    LeavesQty,
    /// The price the element last traded at, where it states one.
    LastPx,
    /// The quantity the element last traded, where it states one.
    LastQty,
    /// The instrument's ISIN, where the market named it by one.
    IsinCode,
    /// The instrument's CUSIP, where the market named it by one.
    CusipCode,
    /// The instrument's SEDOL, where the market named it by one.
    SedolCode,
    /// The instrument's Bloomberg identifier, where the market named it by one.
    BloombergCode,
    /// The instrument's CFI classification, where the market stated it.
    CfiCode,
    /// The market the element traded on, as its ISO 10383 MIC.
    MicCode,
    /// The bid lane's price, where the element states one.
    BidPx,
    /// The currency the bid lane is quoted in, where the element states one.
    BidCurrency,
    /// The bid lane's quantity, where the element states one.
    BidQty,
    /// The unit the bid lane's quantity is counted in, where stated.
    BidUnit,
    /// The ask lane's price, where the element states one.
    AskPx,
    /// The currency the ask lane is quoted in, where the element states one.
    AskCurrency,
    /// The ask lane's quantity, where the element states one.
    AskQty,
    /// The unit the ask lane's quantity is counted in, where stated.
    AskUnit,
}

impl MarketColumn {
    /// The column's name: the fact's, as the trait spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Px => "px",
            Self::Currency => "currency",
            Self::Qty => "qty",
            Self::Unit => "unit",
            Self::Side => "side",
            Self::Tif => "tif",
            Self::Tradable => "tradable",
            Self::SymbolTicker => "symbolticker",
            Self::AvgPx => "avgpx",
            Self::CumQty => "cumqty",
            Self::LeavesQty => "leavesqty",
            Self::LastPx => "lastpx",
            Self::LastQty => "lastqty",
            Self::IsinCode => "isincode",
            Self::CusipCode => "cusipcode",
            Self::SedolCode => "sedolcode",
            Self::BloombergCode => "bloombergcode",
            Self::CfiCode => "cficode",
            Self::MicCode => "miccode",
            Self::BidPx => "bidpx",
            Self::BidCurrency => "bidcurrency",
            Self::BidQty => "bidqty",
            Self::BidUnit => "bidunit",
            Self::AskPx => "askpx",
            Self::AskCurrency => "askcurrency",
            Self::AskQty => "askqty",
            Self::AskUnit => "askunit",
        }
    }

    /// The column's display, for a catalog.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::Px => "Px",
            Self::Currency => "Currency",
            Self::Qty => "Qty",
            Self::Unit => "Unit",
            Self::Side => "Side",
            Self::Tif => "Tif",
            Self::Tradable => "Tradable",
            Self::SymbolTicker => "SymbolTicker",
            Self::AvgPx => "AvgPx",
            Self::CumQty => "CumQty",
            Self::LeavesQty => "LeavesQty",
            Self::LastPx => "LastPx",
            Self::LastQty => "LastQty",
            Self::IsinCode => "IsinCode",
            Self::CusipCode => "CusipCode",
            Self::SedolCode => "SedolCode",
            Self::BloombergCode => "BloombergCode",
            Self::CfiCode => "CfiCode",
            Self::MicCode => "MicCode",
            Self::BidPx => "BidPx",
            Self::BidCurrency => "BidCurrency",
            Self::BidQty => "BidQty",
            Self::BidUnit => "BidUnit",
            Self::AskPx => "AskPx",
            Self::AskCurrency => "AskCurrency",
            Self::AskQty => "AskQty",
            Self::AskUnit => "AskUnit",
        }
    }

    /// What the column holds, for a catalog.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Px => "The price the product is about, exact; empty where it is about none.",
            Self::Currency => "The currency the price is quoted in; XXX where none is stated.",
            Self::Qty => "The quantity the product is about, exact; empty where it is about none.",
            Self::Unit => "The unit the quantity is counted in; empty where the market says none.",
            Self::Side => "Which side of the market the product stood on; UNKNOWN where none.",
            Self::Tif => {
                "How long the product stands, as FIX spells its TimeInForce; empty where none."
            }
            Self::Tradable => {
                "Whether the instrument can be traded right now; empty where the market said nothing."
            }
            Self::SymbolTicker => "The ticker the instrument is known by; empty where none.",
            Self::AvgPx => "The volume-weighted price the product averaged; empty where none.",
            Self::CumQty => "How much of the quantity is done; empty where not stated.",
            Self::LeavesQty => "How much of the quantity is still open; empty where not stated.",
            Self::LastPx => "The price the product last traded at; empty where none.",
            Self::LastQty => "The quantity the product last traded; empty where none.",
            Self::IsinCode => "The instrument's ISIN; empty where the market named none.",
            Self::CusipCode => "The instrument's CUSIP; empty where the market named none.",
            Self::SedolCode => "The instrument's SEDOL; empty where the market named none.",
            Self::BloombergCode => {
                "The instrument's Bloomberg identifier; empty where the market named none."
            }
            Self::CfiCode => {
                "The instrument's CFI classification; empty where the market stated none."
            }
            Self::MicCode => "The market the product traded on, as its MIC; empty where none.",
            Self::BidPx => "The bid lane's price; empty where the lane states none.",
            Self::BidCurrency => "The currency the bid lane is quoted in; empty where none.",
            Self::BidQty => "The bid lane's quantity; empty where the lane states none.",
            Self::BidUnit => "The unit the bid lane's quantity is counted in; empty where none.",
            Self::AskPx => "The ask lane's price; empty where the lane states none.",
            Self::AskCurrency => "The currency the ask lane is quoted in; empty where none.",
            Self::AskQty => "The ask lane's quantity; empty where the lane states none.",
            Self::AskUnit => "The unit the ask lane's quantity is counted in; empty where none.",
        }
    }

    /// The one datatype the column is built and read at: every price and
    /// quantity `decimal128(38, 18)`, the codes the crate's own code leaves,
    /// the flag a boolean, the unit and the ticker `utf8`.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::Px
            | Self::Qty
            | Self::AvgPx
            | Self::CumQty
            | Self::LeavesQty
            | Self::LastPx
            | Self::LastQty
            | Self::BidPx
            | Self::BidQty
            | Self::AskPx
            | Self::AskQty => Decimal18::dtype(),
            Self::Currency | Self::BidCurrency | Self::AskCurrency => DataType::Currency,
            Self::Unit | Self::SymbolTicker | Self::BidUnit | Self::AskUnit => DataType::utf8(),
            Self::Side => DataType::Side,
            Self::Tif => DataType::TimeInForce,
            Self::Tradable => DataType::Boolean,
            Self::IsinCode => DataType::Isin,
            Self::CusipCode => DataType::Cusip,
            Self::SedolCode => DataType::Sedol,
            Self::BloombergCode => DataType::Bloomberg,
            Self::CfiCode => DataType::Cfi,
            Self::MicCode => DataType::Mic,
        }
    }

    /// Whether the column may hold a null: every fact the trait answers as
    /// an option or as nothing - a price or a quantity of nothing, an empty
    /// unit - may; the currency and the side, which every element answers -
    /// `XXX` and `UNKNOWN` where nothing states one - never are.
    #[must_use]
    pub const fn nullable(self) -> bool {
        !matches!(self, Self::Currency | Self::Side)
    }

    /// The column as a field: its name, datatype and nullability, with its
    /// display and description for a catalog.
    ///
    /// # Errors
    ///
    /// Returns the metadata grammar's refusal, which a fixed display and
    /// description never raise.
    pub fn field(self) -> Result<Field> {
        column(
            self.name(),
            self.display(),
            self.description(),
            self.datatype(),
            self.nullable(),
        )
    }

    /// What an element states under this column, as the raw value the
    /// column's datatype types, or nothing where it states no fact: a price
    /// or a quantity of nothing, an empty unit, an absent code.
    pub fn fact<E: MarketElement + ?Sized>(self, element: &E) -> Option<Scalar> {
        let stated = |held: Decimal18| (held != Decimal18::ZERO).then(|| Scalar::from(held));
        let text = |held: &str| (!held.is_empty()).then(|| Scalar::from(held));
        match self {
            Self::Px => stated(element.get_px()),
            Self::Currency => Some(Scalar::Currency(element.get_currency().clone())),
            Self::Qty => stated(element.get_qty()),
            Self::Unit => text(element.get_unit()),
            Self::Side => Some(Scalar::Side(element.get_side().clone())),
            Self::Tif => element
                .get_tif()
                .and_then(|held| TimeInForce::new(held).ok())
                .map(Scalar::TimeInForce),
            Self::Tradable => element.get_tradable().map(Scalar::from),
            Self::SymbolTicker => element.get_symbolticker().and_then(text),
            Self::AvgPx => element.get_avgpx().map(Scalar::from),
            Self::CumQty => element.get_cumqty().map(Scalar::from),
            Self::LeavesQty => element.get_leavesqty().map(Scalar::from),
            Self::LastPx => element.get_lastpx().map(Scalar::from),
            Self::LastQty => element.get_lastqty().map(Scalar::from),
            Self::IsinCode => element.get_isincode().cloned().map(Scalar::Isin),
            Self::CusipCode => element.get_cusipcode().cloned().map(Scalar::Cusip),
            Self::SedolCode => element.get_sedolcode().cloned().map(Scalar::Sedol),
            Self::BloombergCode => element.get_bloombergcode().cloned().map(Scalar::Bloomberg),
            Self::CfiCode => element.get_cficode().cloned().map(Scalar::Cfi),
            Self::MicCode => element.get_miccode().cloned().map(Scalar::Mic),
            Self::BidPx => element.get_bidpx().map(Scalar::from),
            Self::BidCurrency => element.get_bidcurrency().cloned().map(Scalar::Currency),
            Self::BidQty => element.get_bidqty().map(Scalar::from),
            Self::BidUnit => element.get_bidunit().and_then(text),
            Self::AskPx => element.get_askpx().map(Scalar::from),
            Self::AskCurrency => element.get_askcurrency().cloned().map(Scalar::Currency),
            Self::AskQty => element.get_askqty().map(Scalar::from),
            Self::AskUnit => element.get_askunit().and_then(text),
        }
    }

    /// Records what one cell states on the element, through the trait: a
    /// null clears the fact, and a value the fact's type refuses is
    /// silence.
    pub fn record<E: MarketElement + ?Sized>(self, element: &mut E, value: &Scalar) {
        let decimal = || Decimal18::from_scalar(value);
        let text = || {
            value
                .as_str()
                .filter(|held| !held.is_empty())
                .map(str::to_owned)
        };
        match self {
            Self::Px => element.set_px(decimal().unwrap_or(Decimal18::ZERO)),
            Self::Currency => {
                element.set_currency(code::<Currency>(value).unwrap_or_else(Currency::none));
            }
            Self::Qty => element.set_qty(decimal().unwrap_or(Decimal18::ZERO)),
            Self::Unit => element.set_unit(text().unwrap_or_default()),
            Self::Side => element.set_side(code::<Side>(value).unwrap_or_else(Side::unknown)),
            Self::Tif => element.set_tif(match value {
                Scalar::TimeInForce(held) => Some(held.as_str().to_owned()),
                other => other.as_str().map(str::to_owned),
            }),
            Self::Tradable => element.set_tradable(value.as_bool()),
            Self::SymbolTicker => element.set_symbolticker(text()),
            Self::AvgPx => element.set_avgpx(decimal()),
            Self::CumQty => element.set_cumqty(decimal()),
            Self::LeavesQty => element.set_leavesqty(decimal()),
            Self::LastPx => element.set_lastpx(decimal()),
            Self::LastQty => element.set_lastqty(decimal()),
            Self::IsinCode => element.set_isincode(code(value)),
            Self::CusipCode => element.set_cusipcode(code(value)),
            Self::SedolCode => element.set_sedolcode(code(value)),
            Self::BloombergCode => element.set_bloombergcode(code(value)),
            Self::CfiCode => element.set_cficode(code(value)),
            Self::MicCode => element.set_miccode(code(value)),
            Self::BidPx => element.set_bidpx(decimal()),
            Self::BidCurrency => element.set_bidcurrency(code(value)),
            Self::BidQty => element.set_bidqty(decimal()),
            Self::BidUnit => element.set_bidunit(text()),
            Self::AskPx => element.set_askpx(decimal()),
            Self::AskCurrency => element.set_askcurrency(code(value)),
            Self::AskQty => element.set_askqty(decimal()),
            Self::AskUnit => element.set_askunit(text()),
        }
    }
}

/// The code one cell states, where it states one of that code's own
/// datatype; text is not a code until the column's value contract has
/// read it, which a row canonicalized under the product's field has done.
fn code<C: Value + Clone>(value: &Scalar) -> Option<C> {
    C::from_scalar(value).cloned()
}

/// One column as a field: its name, datatype and nullability, with its
/// display and description for a catalog.
pub(crate) fn column(
    name: &'static str,
    display: &'static str,
    description: &'static str,
    datatype: DataType,
    nullable: bool,
) -> Result<Field> {
    let mut field = Field::new(name, datatype, nullable);
    field.set_display(display)?;
    field.set_description(description)?;
    Ok(field)
}

/// The row a product publishes: the sixteen event columns, then the
/// product's own, under a non-null Struct named for the product.
fn row_field(
    name: &'static str,
    display: &'static str,
    description: &'static str,
    own: Vec<Field>,
) -> Result<Field> {
    let mut fields = EventColumn::fields()?;
    fields.extend(own);
    column(
        name,
        display,
        description,
        DataType::from(StructType::from_fields(fields)?),
        false,
    )
}

/// The one null a cell that states nothing reads as.
static NULL: Scalar = Scalar::Null;

/// Where each column of a product's row sits in one schema: the sixteen
/// event columns, then the market columns the product publishes, then its
/// own, each found by name once so a stream of rows is read by position
/// and never by a lookup per cell.
///
/// A column the schema does not hold sits nowhere and reads as null, so a
/// row projected down to fewer columns still reads as the product it
/// states - what it left out is what it left out.
#[derive(Clone, Debug)]
pub struct RowPlan {
    event: [Option<usize>; 16],
    market: Vec<Option<usize>>,
    own: Vec<Option<usize>>,
}

impl RowPlan {
    /// Compiles the plan of one product's row under `field`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the field when it is not a
    /// Struct, because a row is a Struct and nothing else has columns.
    pub fn compile<P: Product>(field: &Field) -> Result<Self> {
        if !matches!(field.dtype(), DataType::Struct(_)) {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: crate::text::expected_got(
                    format_args!("a Struct row of {}", P::NAME),
                    field.dtype(),
                ),
            });
        }
        let mut event = [None; 16];
        for (slot, held) in event.iter_mut().zip(EventColumn::ALL) {
            *slot = field.index_of(held.name());
        }
        Ok(Self {
            event,
            market: P::MARKET
                .iter()
                .map(|held| field.index_of(held.name()))
                .collect(),
            own: P::OWN.iter().map(|name| field.index_of(name)).collect(),
        })
    }
}

/// One row read under a [`RowPlan`]: the cells by the column they sit in.
#[derive(Clone, Copy, Debug)]
pub struct Cells<'a> {
    plan: &'a RowPlan,
    row: &'a [Scalar],
}

impl<'a> Cells<'a> {
    /// One row's cells under its plan.
    #[must_use]
    pub const fn new(plan: &'a RowPlan, row: &'a [Scalar]) -> Self {
        Self { plan, row }
    }

    /// The cell at one position, or the null a column the row does not
    /// hold reads as.
    fn at(&self, position: Option<usize>) -> &'a Scalar {
        position.and_then(|at| self.row.get(at)).unwrap_or(&NULL)
    }

    /// Records the sixteen event columns and every market column the
    /// product publishes on `product`, through the traits, and fills what
    /// the product implies from them - the lane its side quotes - as
    /// [`MarketElement::fill_market`] fills it, so a product read out of
    /// its row is the product that wrote it: what the row leaves out is
    /// what its columns restate, and a derived fact is re-derived rather
    /// than stored twice.
    pub fn record<P: Product>(&self, product: &mut P) {
        for (column, position) in EventColumn::ALL.iter().zip(self.plan.event) {
            column.record(product, self.at(position));
        }
        for (column, position) in P::MARKET.iter().zip(&self.plan.market) {
            column.record(product, self.at(*position));
        }
        product.fill_market();
    }

    /// The cell of the product's own column at `index` in [`Product::OWN`],
    /// or null where the row does not hold it.
    #[must_use]
    pub fn own(&self, index: usize) -> &'a Scalar {
        self.at(self.plan.own.get(index).copied().flatten())
    }
}

/// What a product answers about its row, and the row it gets for it.
///
/// A product is a [`MarketEvent`] - every fact the three graph traits name,
/// read and written - with a row of its own: the sixteen event columns
/// first, [`EventColumn::ALL`], then the market columns it publishes,
/// [`Self::MARKET`], then the columns only it has, [`Self::OWN`]. The
/// three answers an implementor gives are which market columns, its own
/// columns as fields and cells, and how a row's cells become one of it;
/// [`Self::row_field`], [`Self::into_row`] and [`Self::from_row`] are
/// provided over them, so every product's row opens the same way and every
/// product's row joins the message table on `srcuuids` to `curruuid`
/// without a mapping.
pub trait Product: MarketEvent + Clone + Send + Sync + 'static {
    /// The name the product's row root and a table of it take.
    const NAME: &'static str;

    /// The product's display, for a catalog.
    const DISPLAY: &'static str;

    /// What the product is, for a catalog.
    const DESCRIPTION: &'static str;

    /// The market columns the product publishes, in the order they follow
    /// the sixteen.
    const MARKET: &'static [MarketColumn];

    /// The names of the columns only this product has, in the order they
    /// follow the market columns; empty for a product whose facts are
    /// exactly what the traits name.
    const OWN: &'static [&'static str];

    /// The columns only this product has, as fields, in [`Self::OWN`]'s
    /// order.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal of a column, which a fixed
    /// column never raises.
    fn own_fields(&self) -> Result<Vec<Field>>;

    /// This product's own cells, in [`Self::OWN`]'s order, pushed onto
    /// `cells`.
    ///
    /// # Errors
    ///
    /// Returns the value grammar's refusal of a cell.
    fn own_cells(&self, cells: &mut Vec<Scalar>) -> Result<()>;

    /// The product one row's cells state: the sixteen and the market
    /// columns through [`Cells::record`], its own through [`Cells::own`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the column a cell states
    /// what the product refuses.
    fn from_cells(cells: &Cells<'_>) -> Result<Self>;

    /// The row this product publishes: the sixteen event columns, the
    /// market columns it publishes, then its own, under a non-null Struct
    /// named [`Self::NAME`].
    ///
    /// # Errors
    ///
    /// Returns what [`Self::own_fields`] returns.
    fn row_field(&self) -> Result<Field> {
        let mut own = Vec::with_capacity(Self::MARKET.len() + Self::OWN.len());
        for held in Self::MARKET {
            own.push(held.field()?);
        }
        own.extend(self.own_fields()?);
        row_field(Self::NAME, Self::DISPLAY, Self::DESCRIPTION, own)
    }

    /// This product as one row of [`Self::row_field`]: every cell the raw
    /// value its column types, null where the product states no fact.
    ///
    /// Borrowed, as [`FixMsg::into_row`](crate::FixMsg::into_row) is: a
    /// row is another representation of the product, and a door writing a
    /// stream of them reads each product once without consuming it.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::own_cells`] returns.
    #[allow(clippy::wrong_self_convention)]
    fn into_row(&self) -> Result<Scalar> {
        let mut cells = Vec::with_capacity(16 + Self::MARKET.len() + Self::OWN.len());
        for column in EventColumn::ALL {
            cells.push(column.fact(self).unwrap_or(Scalar::Null));
        }
        for column in Self::MARKET {
            cells.push(column.fact(self).unwrap_or(Scalar::Null));
        }
        self.own_cells(&mut cells)?;
        Ok(Scalar::from_sequence(cells))
    }

    /// The product one row of `field` states, the row canonicalized under
    /// the field first so a code spelled as text and a number spelled at
    /// another scale read as what the column types.
    ///
    /// # Errors
    ///
    /// Returns the field's refusal of the row and [`RowPlan::compile`]'s
    /// of the field.
    fn from_row(field: &Field, row: &Scalar) -> Result<Self> {
        let row = field.canonicalize_value(row.clone())?;
        let plan = RowPlan::compile::<Self>(field)?;
        Self::from_cells(&Cells::new(&plan, row.as_sequence().unwrap_or_default()))
    }
}

/// Feeds one optional exact number to a digest under its name, where it
/// is stated.
pub(crate) fn feed_decimal(state: &mut Xxh3, name: &str, held: Option<Decimal18>) {
    if let Some(held) = held {
        crate::graph::element::feed(state, name, &held.units().to_le_bytes());
    }
}

/// Feeds one named fact to a digest.
pub(crate) fn feed(state: &mut Xxh3, name: &str, bytes: &[u8]) {
    crate::graph::element::feed(state, name, bytes);
}

/// The code a finished digest answers.
pub(crate) fn finished(state: &Xxh3) -> u64 {
    state.as_u64()
}

/// A stream of statements with every twin folded into the statement it
/// restates: two statements of one product - the same identity, which is
/// the same instant and the same content, read out of one message a
/// capture logged at two hops - become one, by [`Element::merge_with`](crate::graph::Element::merge_with),
/// so the product names both messages among its sources and is counted
/// once.
///
/// The fold is the product's own merge and nothing invented here: the
/// sources are unioned, and everything else two statements of one identity
/// already agree on. Statements are held while the stream stands at one
/// instant, because a twin shares its instant, and yielded in their order
/// once it has moved past it; a source error is yielded where it is met
/// and folds nothing. The stream is read in its own order, so a caller
/// hands it statements already sorted by instant, which is what the
/// lifecycle answers.
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{Folded, OrderData};
/// use yggdryl::{Decimal18, Uuid};
///
/// # fn main() -> yggdryl::Result<()> {
/// let statement = |source: u128| {
///     let mut order = OrderData::at(1_700_000_000_000_000_000);
///     order.set_crosscode("O-1".to_owned());
///     order.set_qty(Decimal18::from_int(100));
///     order.set_srcuuids(vec![Uuid::from_v8(source)]);
///     order.finalize();
///     order
/// };
/// let mut later = statement(9);
/// later.set_currunix(1_700_000_000_000_000_001);
/// later.finalize();
/// let folded: Vec<OrderData> = Folded::new([statement(1), statement(2), later].map(Ok))
///     .collect::<yggdryl::Result<_>>()?;
/// assert_eq!(folded.len(), 2, "a twin folds, a later statement stands");
/// assert_eq!(folded[0].get_srcuuids(), [Uuid::from_v8(1), Uuid::from_v8(2)]);
/// assert_eq!(folded[0].get_curruuid(), statement(1).get_curruuid());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Folded<E, I> {
    source: I,
    /// The statements of the instant the stream stands at, in their order.
    held: VecDeque<E>,
    /// The first statement of a later instant, pulled while the held ones
    /// were still open to a twin.
    later: Option<E>,
    done: bool,
}

impl<E, I> Folded<E, I>
where
    E: Event + Clone,
    I: Iterator<Item = Result<E>>,
{
    /// Opens the fold over `statements`, in the order they come.
    pub fn new(statements: impl IntoIterator<IntoIter = I>) -> Self {
        Self {
            source: statements.into_iter(),
            held: VecDeque::new(),
            later: None,
            done: false,
        }
    }

    /// Holds one statement: folded into the held statement of its identity
    /// where one is held, held beside the others of its instant, or kept
    /// as the first of a later instant.
    fn fold(&mut self, statement: E) {
        let Some(instant) = self.held.front().map(Event::get_currunix) else {
            self.held.push_back(statement);
            return;
        };
        if statement.get_currunix() != instant {
            self.later = Some(statement);
            return;
        }
        let identity = statement.get_curruuid();
        if let Some(at) = self
            .held
            .iter()
            .position(|held| held.get_curruuid() == identity)
        {
            // Another statement of a held one: what it adds is folded in,
            // and a statement adding nothing is already what the held one
            // says.
            if let Some(merged) = self.held[at].clone().merge_with(&statement) {
                self.held[at] = merged;
            }
            return;
        }
        self.held.push_back(statement);
    }
}

impl<E, I> Iterator for Folded<E, I>
where
    E: Event + Clone,
    I: Iterator<Item = Result<E>>,
{
    type Item = Result<E>;

    fn next(&mut self) -> Option<Result<E>> {
        loop {
            if self.held.is_empty() {
                if let Some(later) = self.later.take() {
                    self.held.push_back(later);
                }
            }
            // The held statements are closed to twins once the stream has
            // moved past their instant, or ended.
            if self.later.is_some() || self.done {
                if let Some(front) = self.held.pop_front() {
                    return Some(Ok(front));
                }
                if self.done {
                    return None;
                }
            }
            match self.source.next() {
                None => self.done = true,
                Some(Err(error)) => return Some(Err(error)),
                Some(Ok(statement)) => self.fold(statement),
            }
        }
    }
}

impl<E, I> std::iter::FusedIterator for Folded<E, I>
where
    E: Event + Clone,
    I: Iterator<Item = Result<E>>,
{
}

/// The errors a stream met, kept aside while a walk reads past them.
pub(crate) type Failures = Arc<Mutex<VecDeque<Error>>>;

/// The failure a stream met first, where it met one.
pub(crate) fn failed(failures: &Failures) -> Option<Error> {
    failures
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
}

/// A fallible stream read as its elements, its failures kept aside.
pub(crate) struct Sieve<I> {
    source: I,
    failures: Failures,
}

impl<I> Sieve<I> {
    /// Opens the sieve over `source`, its failures kept in `failures`.
    pub(crate) const fn new(source: I, failures: Failures) -> Self {
        Self { source, failures }
    }
}

impl<E, I> Iterator for Sieve<I>
where
    I: Iterator<Item = Result<E>>,
{
    type Item = E;

    fn next(&mut self) -> Option<E> {
        loop {
            match self.source.next()? {
                Ok(element) => return Some(element),
                Err(error) => self
                    .failures
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_back(error),
            }
        }
    }
}

/// A fallible stream of statements walked: the one
/// [`EventIterator`] over the statements in the order they come, so each
/// is stated as the statement after the live one it follows - its
/// predecessor's identity and instant, its place in its chain, the
/// predecessor's lineage as its parents and the lifecycle carried forward -
/// with a source error yielded where it is met and never advancing the
/// walk.
///
/// The walk reads elements and not results, so an error is kept aside
/// while the walk reads past it and yielded before the statement the walk
/// pulled past it, which keeps the order the source had. `sorted` is the
/// caller's word that the statements arrive in their own order, which a
/// stream read out of the [lifecycle](crate::FixCodec::lifecycle) does;
/// otherwise the walk collects and sorts them first.
///
/// ```
/// use yggdryl::graph::{Element, Event};
/// use yggdryl::market::{OrderData, Walked};
/// use yggdryl::State;
///
/// # fn main() -> yggdryl::Result<()> {
/// let statement = |unix: i64, state: &str| {
///     let mut order = OrderData::at(unix);
///     order.set_crosscode("O-1".to_owned());
///     order.set_state(State::read(state)?);
///     order.finalize();
///     Ok::<_, yggdryl::Error>(order)
/// };
/// let walked: Vec<OrderData> = Walked::new(
///     [statement(10, "New"), statement(20, "PartiallyFilled"), statement(30, "Filled")],
///     true,
/// )
/// .collect::<yggdryl::Result<_>>()?;
/// assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
/// assert_eq!(walked[2].get_seqnum(), 2);
/// assert_eq!(walked[2].get_parentuuids(), [walked[0].get_curruuid(), walked[1].get_curruuid()]);
/// # Ok(())
/// # }
/// ```
pub struct Walked<E, I> {
    walk: EventIterator<E, Sieve<I>>,
    failures: Failures,
    /// The statement the walk pulled while a failure was met, owed after it.
    pending: Option<E>,
}

impl<E, I> std::fmt::Debug for Walked<E, I> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Walked").finish_non_exhaustive()
    }
}

impl<E, I> Walked<E, I>
where
    E: Event + Clone,
    I: Iterator<Item = Result<E>>,
{
    /// Opens the walk over `statements`; `sorted` states that they arrive
    /// in their own order already.
    pub fn new(statements: impl IntoIterator<IntoIter = I>, sorted: bool) -> Self {
        let failures: Failures = Arc::new(Mutex::new(VecDeque::new()));
        let sieve = Sieve::new(statements.into_iter(), Arc::clone(&failures));
        Self {
            walk: EventIterator::new(sieve, sorted),
            failures,
            pending: None,
        }
    }
}

impl<E, I> Iterator for Walked<E, I>
where
    E: Event + Clone,
    I: Iterator<Item = Result<E>>,
{
    type Item = Result<E>;

    fn next(&mut self) -> Option<Result<E>> {
        if let Some(error) = failed(&self.failures) {
            return Some(Err(error));
        }
        if let Some(statement) = self.pending.take() {
            return Some(Ok(statement));
        }
        let next = self.walk.next();
        if let Some(error) = failed(&self.failures) {
            self.pending = next;
            return Some(Err(error));
        }
        next.map(Ok)
    }
}

impl<E, I> std::iter::FusedIterator for Walked<E, I>
where
    E: Event + Clone,
    I: Iterator<Item = Result<E>>,
{
}

/// A stream of products as a stream of batches of their rows under
/// `field`, each row through [`Product::into_row`], the batches closing
/// on `batch_row_size` rows or `batch_byte_size` bytes, whichever binds
/// first; a zero of either is no bound of that kind.
///
/// An error item yields the completed prefix, then the error, and fuses
/// the reader.
///
/// # Errors
///
/// Returns the Arrow layer's refusal when `field` does not make an Arrow
/// schema.
pub fn arrow_reader<P, I>(
    field: &Field,
    products: I,
    batch_row_size: usize,
    batch_byte_size: u64,
) -> Result<BatchReader>
where
    P: Product,
    I: IntoIterator<Item = Result<P>>,
    I::IntoIter: Send + 'static,
{
    let rows = products
        .into_iter()
        .map(|held| held.and_then(|product| product.into_row()));
    Ok(crate::arrow::rows::result_reader(
        field,
        rows,
        (batch_row_size > 0).then_some(batch_row_size),
        (batch_byte_size > 0).then_some(batch_byte_size),
        None,
        None,
    )?)
}

/// The products a stream of batches of their rows holds, each row through
/// [`Product::from_cells`] under a plan compiled once from the stream's
/// schema; one batch is held at a time, and the source reader's own
/// failure is an error item that fuses the stream.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the source's schema does not
/// make a root field, and [`RowPlan::compile`]'s when that field is not a
/// row.
pub fn read_arrow_reader<P: Product>(
    source: BatchReader,
) -> Result<impl Iterator<Item = Result<P>> + Send + use<P>> {
    let field = Field::from_arrow_schema(P::NAME, source.schema().as_ref())?;
    let plan = RowPlan::compile::<P>(&field)?;
    Ok(Rows {
        source,
        plan,
        rows: VecDeque::new(),
        done: false,
        read: std::marker::PhantomData,
    })
}

/// The row door out: one batch held at a time, read row by row.
struct Rows<P> {
    source: BatchReader,
    plan: RowPlan,
    rows: VecDeque<Scalar>,
    done: bool,
    read: std::marker::PhantomData<fn() -> P>,
}

impl<P: Product> Iterator for Rows<P> {
    type Item = Result<P>;

    fn next(&mut self) -> Option<Result<P>> {
        loop {
            if let Some(row) = self.rows.pop_front() {
                let cells = Cells::new(&self.plan, row.as_sequence().unwrap_or_default());
                return Some(P::from_cells(&cells));
            }
            if self.done {
                return None;
            }
            match self.source.next() {
                None => self.done = true,
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(Error::from(error)));
                }
                Some(Ok(batch)) => match crate::arrow::batch_to_value(&batch) {
                    Ok(rows) => self
                        .rows
                        .extend(rows.as_sequence().unwrap_or_default().iter().cloned()),
                    Err(error) => {
                        self.done = true;
                        return Some(Err(Error::from(error)));
                    }
                },
            }
        }
    }
}

impl<P: Product> std::iter::FusedIterator for Rows<P> {}

/// The three graph traits, answered from the [`MarketEventData`](crate::graph::MarketEventData)
/// a product reaches through `$event` and `$event_mut`, its two accessors.
///
/// What every product delegates - forty accessors and their setters, the
/// order by instant - is written once here, and the four readings a walk
/// takes by value are hooks with one default each: `finalize` is the
/// product's own `settle`, which digests the event's facts and its own
/// and hands the code to [`Event::finalized`]; `with_previous` is the
/// timed market reading, [`MarketEvent::following_market`]; `merge_with`
/// is the product's own `merge_own` - what its own facts take from
/// another statement of itself, the later statement leading where `later`
/// says it is, answering whether any moved - then the timed market merge;
/// and `restating` is the market restatement. A product that means
/// something else by one of them names its own reading in that hook, in
/// this order: `finalize = ..., with_previous = ..., merge_with = ...,
/// restating = ...`, each a path or closure over the same arguments the
/// trait method takes.
macro_rules! holds_market_event {
    ($product:ident via $event:ident, $event_mut:ident
        $(, finalize = $finalize:expr)?
        $(, with_previous = $following:expr)?
        $(, merge_with = $merging:expr)?
        $(, restating = $restating:expr)?
    ) => {
        impl $crate::graph::Element for $product {
            fn get_curruuid(&self) -> $crate::Uuid {
                self.$event().get_curruuid()
            }

            fn set_curruuid(&mut self, curruuid: $crate::Uuid) {
                self.$event_mut().set_curruuid(curruuid);
            }

            fn get_crossuuid(&self) -> $crate::Uuid {
                self.$event().get_crossuuid()
            }

            fn set_crossuuid(&mut self, crossuuid: $crate::Uuid) {
                self.$event_mut().set_crossuuid(crossuuid);
            }

            fn get_crosscode(&self) -> &str {
                self.$event().get_crosscode()
            }

            fn set_crosscode(&mut self, crosscode: String) {
                self.$event_mut().set_crosscode(crosscode);
            }

            fn get_currhashcode(&self) -> u64 {
                self.$event().get_currhashcode()
            }

            fn set_currhashcode(&mut self, hashcode: u64) {
                self.$event_mut().set_currhashcode(hashcode);
            }

            fn get_crosshashcode(&self) -> u64 {
                self.$event().get_crosshashcode()
            }

            fn set_crosshashcode(&mut self, crosshashcode: u64) {
                self.$event_mut().set_crosshashcode(crosshashcode);
            }

            fn get_identifiers(&self) -> &::std::collections::BTreeMap<String, String> {
                self.$event().get_identifiers()
            }

            fn set_identifiers(
                &mut self,
                identifiers: ::std::collections::BTreeMap<String, String>,
            ) {
                self.$event_mut().set_identifiers(identifiers);
            }

            fn get_parentuuids(&self) -> &[$crate::Uuid] {
                self.$event().get_parentuuids()
            }

            fn set_parentuuids(&mut self, parents: Vec<$crate::Uuid>) {
                self.$event_mut().set_parentuuids(parents);
            }

            fn get_srcuuids(&self) -> &[$crate::Uuid] {
                self.$event().get_srcuuids()
            }

            fn set_srcuuids(&mut self, sources: Vec<$crate::Uuid>) {
                self.$event_mut().set_srcuuids(sources);
            }

            /// A product's order is its instant.
            fn is_after(&self, other: &Self) -> bool {
                self.$event().is_after(other.$event())
            }

            /// The identity settled again from what the product now states.
            fn finalize(&mut self) {
                $crate::market::product::holds_market_event!(@finalize self $(, $finalize)?);
            }

            /// The timed market reading, unless the product names its own:
            /// a product descends from the whole lineage of the one it
            /// follows, the predecessor last, and takes the step before it
            /// and what the chain is about.
            fn with_previous(self, previous: &Self) -> Option<Self> {
                $crate::market::product::holds_market_event!(@following self, previous $(, $following)?)
            }

            /// What the product's own facts take from the other statement,
            /// then the timed market merge, unless the product names its
            /// own.
            fn merge_with(self, other: &Self) -> Option<Self> {
                $crate::market::product::holds_market_event!(@merging self, other $(, $merging)?)
            }
        }

        impl $crate::graph::Event for $product {
            /// The timed restatement, and then the market's, unless the
            /// product names its own: a twin takes the live product's place
            /// in its chain, the step before it included, and what that
            /// chain is about where this statement said nothing of it.
            fn restating(self, live: &Self) -> Self {
                $crate::market::product::holds_market_event!(@restating self, live $(, $restating)?)
            }

            fn get_currunix(&self) -> i64 {
                self.$event().get_currunix()
            }

            fn set_currunix(&mut self, unix: i64) {
                self.$event_mut().set_currunix(unix);
            }

            fn get_state(&self) -> &$crate::State {
                self.$event().get_state()
            }

            fn set_state(&mut self, state: $crate::State) {
                self.$event_mut().set_state(state);
            }

            fn get_seqnum(&self) -> u64 {
                self.$event().get_seqnum()
            }

            fn set_seqnum(&mut self, seqnum: u64) {
                self.$event_mut().set_seqnum(seqnum);
            }

            fn get_creaunix(&self) -> Option<i64> {
                self.$event().get_creaunix()
            }

            fn set_creaunix(&mut self, unix: Option<i64>) {
                self.$event_mut().set_creaunix(unix);
            }

            fn get_expirunix(&self) -> Option<i64> {
                self.$event().get_expirunix()
            }

            fn set_expirunix(&mut self, unix: Option<i64>) {
                self.$event_mut().set_expirunix(unix);
            }

            fn get_prevunix(&self) -> Option<i64> {
                self.$event().get_prevunix()
            }

            fn set_prevunix(&mut self, unix: Option<i64>) {
                self.$event_mut().set_prevunix(unix);
            }

            fn get_prevuuid(&self) -> Option<$crate::Uuid> {
                self.$event().get_prevuuid()
            }

            fn set_prevuuid(&mut self, uuid: Option<$crate::Uuid>) {
                self.$event_mut().set_prevuuid(uuid);
            }

            fn get_snapunix(&self) -> Option<i64> {
                self.$event().get_snapunix()
            }

            fn set_snapunix(&mut self, unix: Option<i64>) {
                self.$event_mut().set_snapunix(unix);
            }
        }

        impl $crate::graph::MarketElement for $product {
            fn get_px(&self) -> $crate::Decimal18 {
                self.$event().get_px()
            }

            fn set_px(&mut self, px: $crate::Decimal18) {
                self.$event_mut().set_px(px);
            }

            fn get_currency(&self) -> &$crate::Currency {
                self.$event().get_currency()
            }

            fn set_currency(&mut self, currency: $crate::Currency) {
                self.$event_mut().set_currency(currency);
            }

            fn get_qty(&self) -> $crate::Decimal18 {
                self.$event().get_qty()
            }

            fn set_qty(&mut self, qty: $crate::Decimal18) {
                self.$event_mut().set_qty(qty);
            }

            fn get_unit(&self) -> &str {
                self.$event().get_unit()
            }

            fn set_unit(&mut self, unit: String) {
                self.$event_mut().set_unit(unit);
            }

            fn get_side(&self) -> &$crate::Side {
                self.$event().get_side()
            }

            fn set_side(&mut self, side: $crate::Side) {
                self.$event_mut().set_side(side);
            }

            fn get_isincode(&self) -> Option<&$crate::Isin> {
                self.$event().get_isincode()
            }

            fn set_isincode(&mut self, isincode: Option<$crate::Isin>) {
                self.$event_mut().set_isincode(isincode);
            }

            fn get_cusipcode(&self) -> Option<&$crate::Cusip> {
                self.$event().get_cusipcode()
            }

            fn set_cusipcode(&mut self, cusipcode: Option<$crate::Cusip>) {
                self.$event_mut().set_cusipcode(cusipcode);
            }

            fn get_sedolcode(&self) -> Option<&$crate::Sedol> {
                self.$event().get_sedolcode()
            }

            fn set_sedolcode(&mut self, sedolcode: Option<$crate::Sedol>) {
                self.$event_mut().set_sedolcode(sedolcode);
            }

            fn get_bloombergcode(&self) -> Option<&$crate::Bloomberg> {
                self.$event().get_bloombergcode()
            }

            fn set_bloombergcode(&mut self, bloombergcode: Option<$crate::Bloomberg>) {
                self.$event_mut().set_bloombergcode(bloombergcode);
            }

            fn get_cficode(&self) -> Option<&$crate::Cfi> {
                self.$event().get_cficode()
            }

            fn set_cficode(&mut self, cficode: Option<$crate::Cfi>) {
                self.$event_mut().set_cficode(cficode);
            }

            fn get_miccode(&self) -> Option<&$crate::Mic> {
                self.$event().get_miccode()
            }

            fn set_miccode(&mut self, miccode: Option<$crate::Mic>) {
                self.$event_mut().set_miccode(miccode);
            }

            fn get_lastpx(&self) -> Option<$crate::Decimal18> {
                self.$event().get_lastpx()
            }

            fn set_lastpx(&mut self, px: Option<$crate::Decimal18>) {
                self.$event_mut().set_lastpx(px);
            }

            fn get_lastqty(&self) -> Option<$crate::Decimal18> {
                self.$event().get_lastqty()
            }

            fn set_lastqty(&mut self, qty: Option<$crate::Decimal18>) {
                self.$event_mut().set_lastqty(qty);
            }

            fn get_tif(&self) -> Option<&str> {
                self.$event().get_tif()
            }

            fn set_tif(&mut self, tif: Option<String>) {
                self.$event_mut().set_tif(tif);
            }

            fn get_tradable(&self) -> Option<bool> {
                self.$event().get_tradable()
            }

            fn set_tradable(&mut self, tradable: Option<bool>) {
                self.$event_mut().set_tradable(tradable);
            }

            fn get_symbolticker(&self) -> Option<&str> {
                self.$event().get_symbolticker()
            }

            fn set_symbolticker(&mut self, ticker: Option<String>) {
                self.$event_mut().set_symbolticker(ticker);
            }

            fn get_avgpx(&self) -> Option<$crate::Decimal18> {
                self.$event().get_avgpx()
            }

            fn set_avgpx(&mut self, px: Option<$crate::Decimal18>) {
                self.$event_mut().set_avgpx(px);
            }

            fn get_cumqty(&self) -> Option<$crate::Decimal18> {
                self.$event().get_cumqty()
            }

            fn set_cumqty(&mut self, qty: Option<$crate::Decimal18>) {
                self.$event_mut().set_cumqty(qty);
            }

            fn get_leavesqty(&self) -> Option<$crate::Decimal18> {
                self.$event().get_leavesqty()
            }

            fn set_leavesqty(&mut self, qty: Option<$crate::Decimal18>) {
                self.$event_mut().set_leavesqty(qty);
            }

            fn get_prevpx(&self) -> Option<$crate::Decimal18> {
                self.$event().get_prevpx()
            }

            fn set_prevpx(&mut self, px: Option<$crate::Decimal18>) {
                self.$event_mut().set_prevpx(px);
            }

            fn get_prevqty(&self) -> Option<$crate::Decimal18> {
                self.$event().get_prevqty()
            }

            fn set_prevqty(&mut self, qty: Option<$crate::Decimal18>) {
                self.$event_mut().set_prevqty(qty);
            }

            fn get_bidpx(&self) -> Option<$crate::Decimal18> {
                self.$event().get_bidpx()
            }

            fn set_bidpx(&mut self, px: Option<$crate::Decimal18>) {
                self.$event_mut().set_bidpx(px);
            }

            fn get_bidcurrency(&self) -> Option<&$crate::Currency> {
                self.$event().get_bidcurrency()
            }

            fn set_bidcurrency(&mut self, currency: Option<$crate::Currency>) {
                self.$event_mut().set_bidcurrency(currency);
            }

            fn get_bidqty(&self) -> Option<$crate::Decimal18> {
                self.$event().get_bidqty()
            }

            fn set_bidqty(&mut self, qty: Option<$crate::Decimal18>) {
                self.$event_mut().set_bidqty(qty);
            }

            fn get_bidunit(&self) -> Option<&str> {
                self.$event().get_bidunit()
            }

            fn set_bidunit(&mut self, unit: Option<String>) {
                self.$event_mut().set_bidunit(unit);
            }

            fn get_askpx(&self) -> Option<$crate::Decimal18> {
                self.$event().get_askpx()
            }

            fn set_askpx(&mut self, px: Option<$crate::Decimal18>) {
                self.$event_mut().set_askpx(px);
            }

            fn get_askcurrency(&self) -> Option<&$crate::Currency> {
                self.$event().get_askcurrency()
            }

            fn set_askcurrency(&mut self, currency: Option<$crate::Currency>) {
                self.$event_mut().set_askcurrency(currency);
            }

            fn get_askqty(&self) -> Option<$crate::Decimal18> {
                self.$event().get_askqty()
            }

            fn set_askqty(&mut self, qty: Option<$crate::Decimal18>) {
                self.$event_mut().set_askqty(qty);
            }

            fn get_askunit(&self) -> Option<&str> {
                self.$event().get_askunit()
            }

            fn set_askunit(&mut self, unit: Option<String>) {
                self.$event_mut().set_askunit(unit);
            }
        }
    };
    (@finalize $this:expr) => {
        $this.settle()
    };
    (@finalize $this:expr, $finalize:expr) => {
        $finalize($this)
    };
    (@following $this:expr, $previous:expr) => {
        $crate::graph::MarketEvent::following_market($this, $previous)
    };
    (@following $this:expr, $previous:expr, $following:expr) => {
        $following($this, $previous)
    };
    (@merging $this:expr, $other:expr) => {{
        let mut this = $this;
        let later = $crate::graph::Event::get_currunix($other)
            > $crate::graph::Event::get_currunix(&this);
        if this.merge_own($other, later) {
            // The product's own facts moved, so the fold is a change
            // whatever the market's fold answers; the copy stands in where
            // the market's fold answers nothing, because that fold consumes
            // what it refuses.
            let held = this.clone();
            Some(
                $crate::graph::MarketEvent::merging_market_event(this, $other).unwrap_or_else(
                    || {
                        let mut held = held;
                        $crate::graph::Element::finalize(&mut held);
                        held
                    },
                ),
            )
        } else {
            $crate::graph::MarketEvent::merging_market_event(this, $other)
        }
    }};
    (@merging $this:expr, $other:expr, $merging:expr) => {
        $merging($this, $other)
    };
    (@restating $this:expr, $live:expr) => {
        $crate::graph::element::restating_market($this, $live)
    };
    (@restating $this:expr, $live:expr, $restating:expr) => {
        $restating($this, $live)
    };
}

pub(crate) use holds_market_event;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::MarketEventData;
    use crate::{Bloomberg, Cfi, Cusip, Isin, Mic, Sedol};

    #[test]
    fn every_market_column_states_back_what_it_read() {
        let mut event = MarketEventData::at(7);
        event.set_px("82.5".parse().expect("a price"));
        event.set_currency(Currency::new("USD").expect("a currency"));
        event.set_qty(Decimal18::from_int(1_000));
        event.set_unit("bbl".to_owned());
        event.set_side(Side::read("1").expect("a side"));
        event.set_tif("1".to_owned().into());
        event.set_tradable(Some(true));
        event.set_symbolticker(Some("AAPL".to_owned()));
        event.set_avgpx(Some(Decimal18::from_int(82)));
        event.set_cumqty(Some(Decimal18::from_int(500)));
        event.set_leavesqty(Some(Decimal18::from_int(500)));
        event.set_lastpx(Some(Decimal18::from_int(83)));
        event.set_lastqty(Some(Decimal18::from_int(10)));
        event.set_isincode(Some(Isin::new("US0378331005").expect("an ISIN")));
        event.set_cusipcode(Some(Cusip::new("037833100").expect("a CUSIP")));
        event.set_sedolcode(Some(Sedol::new("2046251").expect("a SEDOL")));
        event.set_bloombergcode(Some(Bloomberg::new("BBG000B9XRY4").expect("a FIGI")));
        event.set_cficode(Some(Cfi::new("ESVUFR").expect("a CFI")));
        event.set_miccode(Some(Mic::new("XNAS").expect("a MIC")));
        event.set_bidpx(Some(Decimal18::from_int(81)));
        event.set_bidcurrency(Some(Currency::new("USD").expect("a currency")));
        event.set_bidqty(Some(Decimal18::from_int(5)));
        event.set_bidunit(Some("bbl".to_owned()));
        event.set_askpx(Some(Decimal18::from_int(84)));
        event.set_askcurrency(Some(Currency::new("EUR").expect("a currency")));
        event.set_askqty(Some(Decimal18::from_int(6)));
        event.set_askunit(Some("t".to_owned()));
        let mut again = MarketEventData::at(7);
        for column in ALL_MARKET {
            let fact = column.fact(&event).expect("every fact is stated");
            column
                .datatype()
                .required_field(column.name())
                .scalar(fact.clone())
                .expect("the fact fits the column");
            column.record(&mut again, &fact);
        }
        assert_eq!(again, event);
    }

    #[test]
    fn a_null_clears_and_nothing_stated_is_none() {
        let mut event = MarketEventData::at(7);
        assert_eq!(MarketColumn::Px.fact(&event), None);
        assert_eq!(MarketColumn::Unit.fact(&event), None);
        assert_eq!(
            MarketColumn::Side.fact(&event),
            Some(Scalar::Side(Side::unknown()))
        );
        assert_eq!(
            MarketColumn::Currency.fact(&event),
            Some(Scalar::Currency(Currency::none()))
        );
        event.set_px(Decimal18::from_int(3));
        event.set_isincode(Some(Isin::new("US0378331005").expect("an ISIN")));
        MarketColumn::Px.record(&mut event, &Scalar::Null);
        MarketColumn::IsinCode.record(&mut event, &Scalar::Null);
        assert_eq!(event.get_px(), Decimal18::ZERO);
        assert_eq!(event.get_isincode(), None);
        assert!(!MarketColumn::Side.nullable() && !MarketColumn::Currency.nullable());
        assert!(MarketColumn::Px.nullable());
    }

    /// Every market column, for the round trip above.
    const ALL_MARKET: [MarketColumn; 27] = [
        MarketColumn::Px,
        MarketColumn::Currency,
        MarketColumn::Qty,
        MarketColumn::Unit,
        MarketColumn::Side,
        MarketColumn::Tif,
        MarketColumn::Tradable,
        MarketColumn::SymbolTicker,
        MarketColumn::AvgPx,
        MarketColumn::CumQty,
        MarketColumn::LeavesQty,
        MarketColumn::LastPx,
        MarketColumn::LastQty,
        MarketColumn::IsinCode,
        MarketColumn::CusipCode,
        MarketColumn::SedolCode,
        MarketColumn::BloombergCode,
        MarketColumn::CfiCode,
        MarketColumn::MicCode,
        MarketColumn::BidPx,
        MarketColumn::BidCurrency,
        MarketColumn::BidQty,
        MarketColumn::BidUnit,
        MarketColumn::AskPx,
        MarketColumn::AskCurrency,
        MarketColumn::AskQty,
        MarketColumn::AskUnit,
    ];
}

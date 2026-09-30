//! `Candle`: one OHLC per book cross code and time bucket over a sorted
//! stream of books - the best bid, the best ask, their midpoint and the
//! spread each as an open, a high, a low and a close - with its datatype,
//! its field, its scalar, the options that bucket instants in a zone and
//! the walk that folds books into candles.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::iter::FusedIterator;

use smol_str::{SmolStr, format_smolstr};

use super::market::unsided_crosscode;
use super::{BookEvent, Element, Event, ExecutionEvent, Market, Operation};
use crate::arrow::BatchReader;
use crate::text::expected_got;
use crate::{
    DataType, Decimal, Error, Field, Result, Scalar, Side, StructType, TimeUnit, Timezone,
};

/// Nanoseconds in one second: what splits an instant into the seconds a
/// zone converts and the remainder it leaves alone.
const NANOS: i64 = 1_000_000_000;

/// Seconds in one day: how far from an instant a zone's other offset is
/// probed for, since no zone changes its offset twice in a day.
const DAY: i64 = 86_400;

/// The units an interval is spelled in, the widest first, each as its
/// suffix and its nanoseconds.
const UNITS: [(&str, i64); 8] = [
    ("w", 7 * 24 * 3_600 * NANOS),
    ("d", 24 * 3_600 * NANOS),
    ("h", 3_600 * NANOS),
    ("m", 60 * NANOS),
    ("s", NANOS),
    ("ms", 1_000_000),
    ("us", 1_000),
    ("ns", 1),
];

/// The cells a candle states, in the order its datatype declares them.
const NAMES: [&str; 25] = [
    "crosscode",
    "ticker",
    "start",
    "end",
    "bidopen",
    "bidhigh",
    "bidlow",
    "bidclose",
    "askopen",
    "askhigh",
    "asklow",
    "askclose",
    "midopen",
    "midhigh",
    "midlow",
    "midclose",
    "spreadopen",
    "spreadhigh",
    "spreadlow",
    "spreadclose",
    "bidqty",
    "askqty",
    "books",
    "executions",
    "volume",
];

/// The path a refusal of the stream's order or range is located at.
const BOOK_UNIX: &str = "$.book.currunix";

/// The alternate identifiers an execution names the trade it reports by,
/// each a kind of name of its own - a `TRADEID` and an `EXECID` spelled
/// alike name two trades: FIX's `TradeID(1003)` and `TradeReportID(571)`,
/// which every side a trade report's parse splits off keeps, and a trading
/// venue's transaction identification code, which every side of one venue
/// trade states; then `ExecID(17)`, which every statement of one fill
/// states - a side split off a trade report states its `SideExecID(1427)`
/// there.
const TRADE_NAMES: [&str; 4] = ["TRADEID", "TRADEREPORTID", "TVTIC", "EXECID"];

/// The kind of name an execution stating none of [`TRADE_NAMES`] goes by:
/// the base of its cross code, which the two sides of one identifier share.
const BASE_CODE: usize = TRADE_NAMES.len();

/// How many kinds of name a trade is found by: [`TRADE_NAMES`], then the
/// base code.
const NAME_KINDS: usize = BASE_CODE + 1;

/// Of [`TRADE_NAMES`], the kinds each of which names one trade and no
/// other - a trade identifier and a venue's transaction code - so two
/// trades stating different names of one of these kinds are two, whatever
/// else they share.
const DISTINCT: [usize; 2] = [0, 2];

/// One reading's open, high, low and close over a bucket.
///
/// The open is the first value the bucket saw and the close the last; the
/// high and the low are the greatest and the least, so a bucket that saw
/// one value states it four times.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Ohlc {
    /// The first value of the bucket.
    pub open: Decimal,
    /// The greatest value of the bucket.
    pub high: Decimal,
    /// The least value of the bucket.
    pub low: Decimal,
    /// The last value of the bucket.
    pub close: Decimal,
}

impl Ohlc {
    /// The reading one value opens: every value that one.
    #[must_use]
    pub const fn at(value: Decimal) -> Self {
        Self {
            open: value,
            high: value,
            low: value,
            close: value,
        }
    }

    /// Folds one more value: the close, and the high or the low it moves.
    pub fn fold(&mut self, value: Decimal) {
        self.high = self.high.max(value);
        self.low = self.low.min(value);
        self.close = value;
    }

    /// Folds a book's reading into the bucket's: nothing for a book that
    /// states none, the opening reading for the first that does.
    fn folded(slot: &mut Option<Self>, value: Option<Decimal>) {
        match (slot.as_mut(), value) {
            (_, None) => {}
            (Some(reading), Some(value)) => reading.fold(value),
            (None, Some(value)) => *slot = Some(Self::at(value)),
        }
    }

    /// The four cells this reading states, in `open, high, low, close`
    /// order; four nulls for no reading.
    fn cells(reading: Option<Self>) -> [Scalar; 4] {
        match reading {
            Some(reading) => [
                Scalar::from(reading.open),
                Scalar::from(reading.high),
                Scalar::from(reading.low),
                Scalar::from(reading.close),
            ],
            None => [Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null],
        }
    }
}

/// One OHLC of one book over one bucket: what the books of one cross code
/// whose instants fell in `[start, end)` read at their best bid, their best
/// ask, their midpoint and their spread, the quantities resting at the
/// touch when the bucket closed, and what traded in it.
///
/// A candle is a value of its own rather than a datatype: its datatype is
/// the struct [`Self::field`] declares, its scalar the named struct
/// [`Self::into_scalar`] hands over, read back by [`Self::from_scalar`] in
/// that shape or in the ordered row the field's own value door answers, and
/// [`Self::arrow_reader`] lays candles out as batches under that field.
/// [`CandleIterator`] is what makes them.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Candle {
    /// The book's cross code: [`Element::get_crosscode`].
    pub crosscode: SmolStr,
    /// The book's ticker, where it states one: [`Market::get_ticker`].
    pub ticker: Option<SmolStr>,
    /// The bucket's start, nanoseconds since the epoch, UTC.
    pub start: i64,
    /// The bucket's end, exclusive, nanoseconds since the epoch, UTC.
    pub end: i64,
    /// Over [`BookEvent::best_price`] on the bid of every book stating one.
    pub bid: Option<Ohlc>,
    /// Over [`BookEvent::best_price`] on the ask of every book stating one.
    pub ask: Option<Ohlc>,
    /// Over [`BookEvent::bbo_midpoint`] of every book stating one.
    pub mid: Option<Ohlc>,
    /// Over [`BookEvent::spread`] of every book stating one.
    pub spread: Option<Ohlc>,
    /// The last book's [`BookEvent::best_quantity`] on the bid.
    pub bidqty: Option<Decimal>,
    /// The last book's [`BookEvent::best_quantity`] on the ask.
    pub askqty: Option<Decimal>,
    /// How many books folded into the bucket.
    pub books: u64,
    /// How many executions the folded books carried, a trade they carried
    /// twice counted twice.
    pub executions: u64,
    /// What traded in the bucket: the exact sum, over the trades those
    /// executions report, of the quantity each traded - an execution's
    /// [`Market::get_lastqty`], what its fill traded, one stating none
    /// adding nothing, never the order's [`Market::get_quantity`] - each
    /// trade counted once, at the largest last quantity any of its
    /// executions states.
    ///
    /// An execution names the trade it reports by what its
    /// [`Operation::get_altids`] state under `TRADEID`, `TRADEREPORTID`,
    /// `TVTIC` and `EXECID` - FIX's `TradeID(1003)` and
    /// `TradeReportID(571)`, which every side a trade report's parse splits
    /// off keeps, a venue's transaction code, which every side of one venue
    /// trade states, and `ExecID(17)`, which every statement of one fill
    /// states (`SideExecID(1427)` on a side split off a trade report) - each
    /// a kind of name of its own; one stating none of them by the base of
    /// its cross code, which the two sides of one identifier share
    /// (`BUYS:X`, `SELL:X`). Executions sharing a name report one trade, and
    /// so do the trades one execution names together, save that two trades
    /// stating different `TRADEID`s, or different `TVTIC`s, stay two. So the
    /// two sides of a trade report and a fill delivered twice count once,
    /// even where one statement names what another does not, while fills
    /// naming nothing in common - even at one instant, price and quantity -
    /// are trades of their own.
    ///
    /// A candle knows the trades its own bucket names and those the bucket
    /// just before it named for the same cross code: a trade named again in
    /// the next bucket adds there only what it states past what was counted,
    /// so one restated bucket after bucket counts once in all, while one
    /// named again only after a bucket that did not name it counts again.
    pub volume: Decimal,
}

impl Candle {
    /// The required struct `candle`: `crosscode: utf8 not null`, `ticker:
    /// utf8`, `start` and `end` as `datetime64(ns, UTC) not null`, then the
    /// four cells of each reading - `bidopen`, `bidhigh`, `bidlow`,
    /// `bidclose`, and the same under `ask`, `mid` and `spread` - as a
    /// nullable `decimal` each, `bidqty` and `askqty` the same, `books` and
    /// `executions` as `uint64 not null` and `volume` as `decimal not
    /// null`.
    ///
    /// ```
    /// use yggdryl::graph::Candle;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = Candle::field()?;
    /// assert_eq!(field.name(), "candle");
    /// assert!(!field.is_nullable());
    /// assert_eq!(field.field_len(), 25);
    /// assert_eq!(field.fields()[2].name(), "start");
    /// assert_eq!(field.fields()[2].dtype().to_string(), "datetime64(ns,\"UTC\")");
    /// assert!(field.fields()[4].is_nullable());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a field cannot be built.
    pub fn field() -> Result<Field> {
        let instant = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?;
        let mut fields = Vec::with_capacity(NAMES.len());
        fields.push(DataType::utf8().required_field(NAMES[0]));
        fields.push(DataType::utf8().nullable_field(NAMES[1]));
        fields.push(instant.clone().required_field(NAMES[2]));
        fields.push(instant.required_field(NAMES[3]));
        fields.extend(
            NAMES[4..22]
                .iter()
                .map(|name| DataType::Decimal.nullable_field(*name)),
        );
        fields.push(DataType::UInt64.required_field(NAMES[22]));
        fields.push(DataType::UInt64.required_field(NAMES[23]));
        fields.push(DataType::Decimal.required_field(NAMES[24]));
        Ok(DataType::Struct(StructType::from_fields(fields)?).required_field("candle"))
    }

    /// The candle as the named struct of its cells, an absent ticker,
    /// reading or quantity a null.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        let instant = |unix: i64| {
            Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC)
                .expect("the nanosecond is a clock resolution")
        };
        let optional = |value: Option<Decimal>| value.map_or(Scalar::Null, Scalar::from);
        let cells = [
            Scalar::from(self.crosscode.clone()),
            self.ticker.clone().map_or(Scalar::Null, Scalar::from),
            instant(self.start),
            instant(self.end),
        ]
        .into_iter()
        .chain(Ohlc::cells(self.bid))
        .chain(Ohlc::cells(self.ask))
        .chain(Ohlc::cells(self.mid))
        .chain(Ohlc::cells(self.spread))
        .chain([
            optional(self.bidqty),
            optional(self.askqty),
            Scalar::from(self.books),
            Scalar::from(self.executions),
            Scalar::from(self.volume),
        ]);
        Scalar::from_struct(NAMES.iter().copied().zip(cells)).expect("twenty-five distinct names")
    }

    /// Reads a candle back from the named struct [`Self::into_scalar`]
    /// answers or from the ordered row [`Self::field`]'s value door
    /// canonicalizes it to. The value passes that one door first, so a cell
    /// is read exactly as a `candle` column would hold it - a text or a
    /// number the decimal datatype restates is read as it restates it, an
    /// instant at its own unit and zone is read at nanoseconds UTC - except
    /// that a name the struct lacks is a null rather than the default the
    /// door fills a required cell with.
    ///
    /// # Errors
    ///
    /// The refusal [`Self::field`]'s [`Field::scalar`] answers, located
    /// under `$.candle`: a null or missing cross code, start, end, count or
    /// volume, a cell of another datatype, a name the struct should not
    /// hold, a row of another width or a value of another shape; and a
    /// reading stating some of its four cells and not the others, since a
    /// bucket that saw a value saw an open, a high, a low and a close.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        let value = match value.as_struct() {
            Some(fields) if NAMES.iter().any(|name| !fields.contains_key(*name)) => {
                let absent = NAMES
                    .iter()
                    .filter(|name| !fields.contains_key(**name))
                    .map(|name| (SmolStr::new_static(name), Scalar::Null));
                Scalar::from_struct(
                    fields
                        .iter()
                        .map(|(name, cell)| (name.clone(), cell.clone()))
                        .chain(absent),
                )?
            }
            _ => value.clone(),
        };
        let row = Self::field()?.scalar(value)?;
        let cells = row.sequence_rows();
        let Some(cells) = cells.as_deref().filter(|cells| cells.len() == NAMES.len()) else {
            return Err(unread(&row));
        };
        let unread = || unread(&row);
        let crosscode = cells[0].as_str().ok_or_else(unread)?;
        let ticker = match &cells[1] {
            Scalar::Null => None,
            cell => Some(SmolStr::new(cell.as_str().ok_or_else(unread)?)),
        };
        let instant = |cell: &Scalar| cell.as_datetime64().map(|(count, ..)| count);
        let count = |cell: &Scalar| cell.as_u128().and_then(|count| u64::try_from(count).ok());
        Ok(Self {
            crosscode: SmolStr::new(crosscode),
            ticker,
            start: instant(&cells[2]).ok_or_else(unread)?,
            end: instant(&cells[3]).ok_or_else(unread)?,
            bid: reading(&cells[4..8]).ok_or_else(unread)?,
            ask: reading(&cells[8..12]).ok_or_else(unread)?,
            mid: reading(&cells[12..16]).ok_or_else(unread)?,
            spread: reading(&cells[16..20]).ok_or_else(unread)?,
            bidqty: decimal(&cells[20]).ok_or_else(unread)?,
            askqty: decimal(&cells[21]).ok_or_else(unread)?,
            books: count(&cells[22]).ok_or_else(unread)?,
            executions: count(&cells[23]).ok_or_else(unread)?,
            volume: decimal(&cells[24]).ok_or_else(unread)?.ok_or_else(unread)?,
        })
    }

    /// Streams candles into bounded Arrow record batches under
    /// [`Self::field`], one row per candle.
    ///
    /// The source is not pulled until the returned reader is pulled. A
    /// `None` row size uses the crate default and zero normalizes to one. A
    /// source error follows the completed prefix and fuses the reader.
    ///
    /// ```
    /// use yggdryl::graph::{Candle, Ohlc};
    /// use yggdryl::{ArrowCastOptions, Decimal, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let candle = Candle {
    ///     crosscode: "ACME".into(),
    ///     ticker: Some("ACME".into()),
    ///     start: 60_000_000_000,
    ///     end: 120_000_000_000,
    ///     bid: Some(Ohlc::at("99.5".parse()?)),
    ///     ask: None,
    ///     mid: None,
    ///     spread: None,
    ///     bidqty: Some(Decimal::from_int(300)),
    ///     askqty: None,
    ///     books: 1,
    ///     executions: 0,
    ///     volume: Decimal::ZERO,
    /// };
    /// let mut batches = Candle::arrow_reader([Ok(candle.clone())], None)?;
    /// let batch = batches.next().expect("one batch")?;
    /// assert_eq!(batch.num_rows(), 1);
    /// let rows = Serie::from_arrow_batch(Some(&Candle::field()?), &batch, ArrowCastOptions::default())?;
    /// assert_eq!(Candle::from_scalar(&rows.scalar(0)?)?, candle);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the row field cannot be built.
    pub fn arrow_reader<I>(candles: I, batch_row_size: Option<usize>) -> Result<BatchReader>
    where
        I: IntoIterator<Item = Result<Self>>,
        I::IntoIter: Send + 'static,
    {
        let field = Self::field()?;
        let rows = candles
            .into_iter()
            .map(|candle| candle.map(|candle| candle.into_scalar()));
        Ok(crate::arrow::rows::result_reader(
            &field,
            rows,
            batch_row_size,
            None,
            None,
            None,
        )?)
    }
}

/// The reading four cells state: none when all four are null, one when all
/// four are decimals, nothing readable otherwise.
fn reading(cells: &[Scalar]) -> Option<Option<Ohlc>> {
    match cells {
        [Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null] => Some(None),
        [
            Scalar::Decimal(open),
            Scalar::Decimal(high),
            Scalar::Decimal(low),
            Scalar::Decimal(close),
        ] => Some(Some(Ohlc {
            open: *open,
            high: *high,
            low: *low,
            close: *close,
        })),
        _ => None,
    }
}

/// The decimal a cell states, none for a null, nothing readable otherwise.
fn decimal(cell: &Scalar) -> Option<Option<Decimal>> {
    match cell {
        Scalar::Null => Some(None),
        Scalar::Decimal(value) => Some(Some(*value)),
        _ => None,
    }
}

/// The value door answered a row this reading does not know: the one
/// contract and this reading disagree, which no caller value can cause -
/// except a reading stating some of its four cells, which the door lets
/// through and this reading refuses.
fn unread(row: &Scalar) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.candle"),
        reason: expected_got(
            "the canonical candle row, each reading four decimals or four nulls",
            row.kind(),
        ),
    }
}

/// How instants are bucketed: the interval and the zone whose wall clock the
/// buckets align to.
///
/// A bucket is `interval` nanoseconds of the zone's wall clock: a daily
/// candle opens at local midnight, an hourly one on the local hour, so
/// hourly candles follow a saving-time change - the hour a spring-forward
/// skips yields no candle, the hour a fall-back repeats is one candle - and
/// a daily candle spans twenty-three or twenty-five hours on the day of one.
/// Every bucket's edges are the earliest instants whose wall clock reads at
/// or after the local edges, so the edges rise with the instants and a
/// sorted stream never re-enters a bucket it left.
///
/// ```
/// use yggdryl::Timezone;
/// use yggdryl::graph::CandleOptions;
///
/// # fn main() -> yggdryl::Result<()> {
/// let options = CandleOptions::from_spelling("5m")?.with_timezone(Timezone::from_str("Europe/Zurich")?);
/// assert_eq!(options.interval(), 300_000_000_000);
/// assert_eq!(options.spelling(), "5m");
/// assert_eq!(options.timezone().as_str(), "Europe/Zurich");
/// assert_eq!(CandleOptions::new(90_000_000_000)?.spelling(), "90s");
/// assert!(CandleOptions::new(0).is_err());
/// assert!(CandleOptions::from_spelling("1x").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CandleOptions {
    interval: i64,
    timezone: Timezone,
}

impl CandleOptions {
    /// Buckets of `interval` nanoseconds aligned to UTC.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.interval` for an interval that
    /// is not positive.
    pub fn new(interval: i64) -> Result<Self> {
        if interval <= 0 {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.interval"),
                reason: expected_got("a positive count of nanoseconds", interval),
            });
        }
        Ok(Self {
            interval,
            timezone: Timezone::UTC,
        })
    }

    /// The same buckets aligned to `timezone`'s wall clock.
    #[must_use]
    pub fn with_timezone(mut self, timezone: Timezone) -> Self {
        self.timezone = timezone;
        self
    }

    /// The bucket width in nanoseconds of the zone's wall clock.
    #[must_use]
    pub fn interval(&self) -> i64 {
        self.interval
    }

    /// The zone the buckets align to.
    #[must_use]
    pub fn timezone(&self) -> &Timezone {
        &self.timezone
    }

    /// Reads an interval spelled as a count and a unit - `30s`, `1m`, `5m`,
    /// `1h`, `1d`, `1w`, and `ms`, `us` and `ns` below the second - aligned
    /// to UTC.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.interval` for text that is not
    /// a positive count followed by one of those units, or an interval past
    /// `i64` nanoseconds.
    pub fn from_spelling(text: &str) -> Result<Self> {
        let refused = || Error::InvalidRecord {
            path: SmolStr::new_static("$.interval"),
            reason: expected_got(
                "a count and a unit (`30s`, `1m`, `5m`, `1h`, `1d`, `1w`)",
                format_smolstr!("{text:?}"),
            ),
        };
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        let (count, unit) = text.split_at(digits);
        let count: i64 = count.parse().map_err(|_| refused())?;
        let (_, nanos) = UNITS
            .iter()
            .find(|(suffix, _)| *suffix == unit)
            .ok_or_else(refused)?;
        let interval = count.checked_mul(*nanos).filter(|interval| *interval > 0);
        Self::new(interval.ok_or_else(refused)?)
    }

    /// The interval as its count and the widest unit that divides it
    /// exactly: what [`Self::from_spelling`] reads back.
    #[must_use]
    pub fn spelling(&self) -> String {
        let (suffix, nanos) = UNITS
            .iter()
            .find(|(_, nanos)| self.interval % nanos == 0)
            .expect("one nanosecond divides every interval");
        format!("{}{suffix}", self.interval / nanos)
    }

    /// The wall clock the zone reads at `unix`, nanoseconds, as if UTC.
    fn local(&self, unix: i64) -> Result<i64> {
        let local = self.timezone.into_local(unix.div_euclid(NANOS))?;
        local
            .checked_mul(NANOS)
            .and_then(|local| local.checked_add(unix.rem_euclid(NANOS)))
            .ok_or_else(|| range(unix))
    }

    /// The offset the zone reads `epoch` under, seconds east of UTC, through
    /// the one refusal the zone answers for rules this build lacks.
    fn offset(&self, epoch: i64) -> Result<i64> {
        Ok(self.timezone.into_local(epoch)? - epoch)
    }

    /// The earliest instant whose wall clock reads at or after `local`
    /// seconds, and whether it reads exactly that.
    ///
    /// A wall clock a fall-back repeats is read first under the greater
    /// offset before the transition, which the probe a day earlier finds; a
    /// wall clock a spring-forward skips is first exceeded at the
    /// transition, found between the last instant under the old offset and
    /// the one the zone settles the reading at.
    fn boundary(&self, local: i64) -> Result<(i64, bool)> {
        let settled = self.timezone.into_utc(local)?;
        let mut earliest = settled;
        for probe in [settled.checked_sub(DAY), settled.checked_add(DAY)] {
            let probe = probe.ok_or_else(|| range(settled))?;
            let instant = local - self.offset(probe)?;
            if instant < earliest && self.timezone.into_local(instant)? == local {
                earliest = instant;
            }
        }
        if self.timezone.into_local(earliest)? == local {
            return Ok((earliest, true));
        }
        let after = self.offset(earliest)?;
        let (mut before, mut past) = (
            earliest.checked_sub(DAY).ok_or_else(|| range(earliest))?,
            earliest,
        );
        while past - before > 1 {
            let middle = before + (past - before) / 2;
            if self.offset(middle)? == after {
                past = middle;
            } else {
                before = middle;
            }
        }
        Ok((past, false))
    }

    /// The earliest instant, nanoseconds, whose wall clock reads at or after
    /// the local edge `local`, nanoseconds.
    fn edge(&self, local: i64) -> Result<i64> {
        let (instant, exact) = self.boundary(local.div_euclid(NANOS))?;
        let remainder = if exact { local.rem_euclid(NANOS) } else { 0 };
        instant
            .checked_mul(NANOS)
            .and_then(|instant| instant.checked_add(remainder))
            .ok_or_else(|| range(local))
    }

    /// The bucket `unix` falls in: the greatest bucket index whose start is
    /// at or before the instant, its end the next index's start.
    ///
    /// The starts never fall as the index rises, so the search opens at the
    /// index of the instant's own wall clock, which is the answer everywhere
    /// but in a fall-back's repeated readings, whose bucket is the one the
    /// first pass reached. It gallops away from there by doubling steps
    /// until the instant lies between two starts, then bisects: `O(log n)`
    /// edge solves for a gap of `n` intervals, whatever the interval and the
    /// zone.
    fn bucket(&self, unix: i64) -> Result<Bucket> {
        let start = |index: i64| -> Result<(i64, i64)> {
            let local = index
                .checked_mul(self.interval)
                .ok_or_else(|| range(unix))?;
            Ok((index, self.edge(local)?))
        };
        let guess = start(self.local(unix)?.div_euclid(self.interval))?;
        // `low` starts at or before the instant, `high` after it.
        let (mut low, mut high) = if guess.1 <= unix {
            let mut low = guess;
            let mut step = 1_i64;
            loop {
                let probe = start(guess.0.checked_add(step).ok_or_else(|| range(unix))?)?;
                if probe.1 > unix {
                    break (low, probe);
                }
                low = probe;
                step = step.checked_mul(2).ok_or_else(|| range(unix))?;
            }
        } else {
            let mut high = guess;
            let mut step = 1_i64;
            loop {
                let probe = start(guess.0.checked_sub(step).ok_or_else(|| range(unix))?)?;
                if probe.1 <= unix {
                    break (probe, high);
                }
                high = probe;
                step = step.checked_mul(2).ok_or_else(|| range(unix))?;
            }
        };
        while high.0 - low.0 > 1 {
            let middle = start(low.0 + (high.0 - low.0) / 2)?;
            if middle.1 <= unix {
                low = middle;
            } else {
                high = middle;
            }
        }
        Ok(Bucket {
            start: low.1,
            end: high.1,
        })
    }
}

/// An instant a zone cannot read or a bucket cannot hold in `i64`
/// nanoseconds.
fn range(unix: i64) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static(BOOK_UNIX),
        reason: expected_got("an instant within i64 nanoseconds of the epoch", unix),
    }
}

/// One bucket's edges, nanoseconds UTC, the end exclusive.
#[derive(Clone, Copy, Debug)]
struct Bucket {
    start: i64,
    end: i64,
}

impl Bucket {
    fn contains(self, unix: i64) -> bool {
        self.start <= unix && unix < self.end
    }
}

/// One trade as a fold knows it: what its statements stated and what the
/// bucket before counted of it.
#[derive(Debug)]
struct Slot {
    /// The slot this one was found to be; its own index while it stands.
    parent: usize,
    /// The largest last quantity any statement of the trade states, where
    /// one does.
    stated: Option<Decimal>,
    /// What the buckets before the open one counted of it: zero for a
    /// trade first stated in the open bucket.
    counted: Decimal,
    /// Whether a statement of the open bucket named it.
    named: bool,
    /// The [`DISTINCT`] names it states, where it does, in that order.
    distinct: [Option<SmolStr>; DISTINCT.len()],
}

impl Slot {
    /// A trade first named in the open bucket, at index `at`.
    const fn opened(at: usize) -> Self {
        Self {
            parent: at,
            stated: None,
            counted: Decimal::ZERO,
            named: true,
            distinct: [None, None],
        }
    }

    /// What the trade adds to the open bucket's volume: what it states past
    /// what the buckets before counted, never less than nothing; none where
    /// that difference spells past `decimal`.
    fn adds(&self) -> Option<Decimal> {
        match self.stated {
            Some(stated) if stated > self.counted => stated.checked_sub(self.counted),
            _ => Some(Decimal::ZERO),
        }
    }
}

/// The trades one cross code's executions reported, as [`Candle::volume`]
/// names them: those of the open bucket, and those the bucket just before
/// it counted, carried in.
///
/// A name is looked up borrowed, so a statement of a trade whose names are
/// all known allocates nothing; a name first seen is held once, and past
/// `SmolStr`'s inline width that costs one allocation.
#[derive(Debug, Default)]
struct Trades {
    /// Each name a statement stated, one map per kind - [`TRADE_NAMES`],
    /// then the base code - to a slot of the trade it names.
    names: [HashMap<SmolStr, usize>; NAME_KINDS],
    /// One slot per trade, a trade found to be another pointing at it.
    slots: Vec<Slot>,
}

impl Trades {
    /// Whether no trade is held.
    fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The slot `slot` was found to be, the path to it shortened.
    fn root(&mut self, slot: usize) -> usize {
        let mut root = slot;
        while self.slots[root].parent != root {
            root = self.slots[root].parent;
        }
        let mut at = slot;
        while self.slots[at].parent != root {
            let next = self.slots[at].parent;
            self.slots[at].parent = root;
            at = next;
        }
        root
    }

    /// Counts one statement of a trade, which names it by `names` - one per
    /// kind - and states `quantity`: answers what the trades it reaches
    /// added to the open bucket before it and what the trade they are adds
    /// after it; none where a sum spells past `decimal`.
    ///
    /// The trades its names reach are one, taken in the order of the kinds,
    /// save one stating a [`DISTINCT`] name that differs from the
    /// statement's or from those already taken, which stays apart and keeps
    /// the name that reached it.
    fn state(
        &mut self,
        names: [Option<&str>; NAME_KINDS],
        quantity: Option<Decimal>,
    ) -> Option<(Decimal, Decimal)> {
        let mut reached = [None; NAME_KINDS];
        for (kind, name) in names.iter().enumerate() {
            if let Some(&slot) = name.and_then(|name| self.names[kind].get(name)) {
                reached[kind] = Some(self.root(slot));
            }
        }
        let mut joined = [0; NAME_KINDS];
        let mut count = 0;
        let mut distinct = DISTINCT.map(|kind| names[kind]);
        for root in reached.into_iter().flatten() {
            if joined[..count].contains(&root) {
                continue;
            }
            let held = &self.slots[root].distinct;
            let agrees = distinct.iter().zip(held).all(|pair| match pair {
                (Some(ours), Some(theirs)) => *ours == theirs.as_str(),
                _ => true,
            });
            if agrees {
                for (ours, theirs) in distinct.iter_mut().zip(held) {
                    if ours.is_none() {
                        *ours = theirs.as_deref();
                    }
                }
                joined[count] = root;
                count += 1;
            }
        }
        let before = joined[..count]
            .iter()
            .try_fold(Decimal::ZERO, |sum, root| {
                sum.checked_add(self.slots[*root].adds()?)
            })?;
        let trade = if count == 0 {
            self.slots.push(Slot::opened(self.slots.len()));
            self.slots.len() - 1
        } else {
            for other in joined[1..count].iter().copied() {
                self.merge(joined[0], other)?;
            }
            joined[0]
        };
        for (at, kind) in DISTINCT.iter().enumerate() {
            if let (Some(name), None) = (names[*kind], &self.slots[trade].distinct[at]) {
                self.slots[trade].distinct[at] = Some(SmolStr::new(name));
            }
        }
        let slot = &mut self.slots[trade];
        slot.stated = slot.stated.max(quantity);
        slot.named = true;
        let after = slot.adds()?;
        for (map, name) in self.names.iter_mut().zip(names) {
            if let Some(name) = name.filter(|name| !map.contains_key(*name)) {
                map.insert(SmolStr::new(name), trade);
            }
        }
        Some((before, after))
    }

    /// Makes the trade at `other` the one at `into`: the larger stated
    /// quantity, what the buckets before counted of both, the names either
    /// states; none where that count spells past `decimal`.
    fn merge(&mut self, into: usize, other: usize) -> Option<()> {
        let taken = std::mem::replace(&mut self.slots[other], Slot::opened(into));
        let slot = &mut self.slots[into];
        slot.stated = slot.stated.max(taken.stated);
        slot.counted = slot.counted.checked_add(taken.counted)?;
        slot.named |= taken.named;
        for (held, name) in slot.distinct.iter_mut().zip(taken.distinct) {
            if held.is_none() {
                *held = name;
            }
        }
        Some(())
    }

    /// The trades the next bucket carries in once the open one closes: each
    /// a statement of the closing bucket named, counted at what it and the
    /// buckets before counted of it; none where that count spells past
    /// `decimal`. A trade carried in and named by nothing here is let go,
    /// so what is held is one bucket's trades beside the open one's.
    fn carried(mut self) -> Option<Self> {
        let roots: Vec<usize> = (0..self.slots.len()).map(|slot| self.root(slot)).collect();
        let mut moved = vec![None; self.slots.len()];
        let mut slots = Vec::new();
        for (at, slot) in self.slots.into_iter().enumerate() {
            if roots[at] != at || !slot.named {
                continue;
            }
            let counted = slot.counted.checked_add(slot.adds()?)?;
            moved[at] = Some(slots.len());
            slots.push(Slot {
                parent: slots.len(),
                stated: Some(counted),
                counted,
                named: false,
                distinct: slot.distinct,
            });
        }
        let mut names = self.names;
        for map in &mut names {
            map.retain(|_, slot| match moved[roots[*slot]] {
                Some(to) => {
                    *slot = to;
                    true
                }
                None => false,
            });
        }
        Some(Self { names, slots })
    }
}

/// The refusal of a volume `sum` spells past `decimal`.
fn past_decimal(sum: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.candle.volume"),
        reason: expected_got("a volume within decimal", sum),
    }
}

/// The candle of one cross code as it is folded.
#[derive(Debug)]
struct Fold {
    ticker: Option<SmolStr>,
    bid: Option<Ohlc>,
    ask: Option<Ohlc>,
    mid: Option<Ohlc>,
    spread: Option<Ohlc>,
    bidqty: Option<Decimal>,
    askqty: Option<Decimal>,
    books: u64,
    executions: u64,
    volume: Decimal,
    /// The trades the bucket's books reported, beside those the bucket
    /// just before counted: held so a later statement of one adds only what
    /// it states past what was counted.
    trades: Trades,
}

impl Fold {
    /// An empty fold stating the ticker of the first book it will see, over
    /// the trades the bucket just before carries in.
    fn opened(book: &BookEvent, trades: Trades) -> Self {
        Self {
            ticker: book.get_ticker().map(SmolStr::new),
            bid: None,
            ask: None,
            mid: None,
            spread: None,
            bidqty: None,
            askqty: None,
            books: 0,
            executions: 0,
            volume: Decimal::ZERO,
            trades,
        }
    }

    /// Folds one more book: its readings, its touch quantities, its
    /// executions and what they traded.
    fn fold(&mut self, book: &BookEvent) -> Result<()> {
        Ohlc::folded(&mut self.bid, book.best_price(Side::Buy));
        Ohlc::folded(&mut self.ask, book.best_price(Side::Sell));
        Ohlc::folded(&mut self.mid, book.bbo_midpoint());
        Ohlc::folded(&mut self.spread, book.spread());
        self.bidqty = book.best_quantity(Side::Buy);
        self.askqty = book.best_quantity(Side::Sell);
        self.books += 1;
        self.executions += book.executions().len() as u64;
        book.executions()
            .iter()
            .try_for_each(|execution| self.traded(execution))
    }

    /// Counts what `execution` traded into the volume: the first statement
    /// of its trade adds its last quantity, a later one what it states past
    /// the largest counted before it, any other nothing.
    fn traded(&mut self, execution: &ExecutionEvent) -> Result<()> {
        let altids = execution.get_altids();
        let mut names = [None; NAME_KINDS];
        for (name, key) in names.iter_mut().zip(TRADE_NAMES) {
            *name = altids.get(key);
        }
        if names.iter().all(Option::is_none) {
            names[BASE_CODE] = Some(unsided_crosscode(execution.get_crosscode()));
        }
        let volume = self.volume;
        let quantity = execution.get_lastqty();
        let (before, after) = self.trades.state(names, quantity).ok_or_else(|| {
            past_decimal(format_smolstr!(
                "{volume} + {}",
                quantity.unwrap_or(Decimal::ZERO)
            ))
        })?;
        self.volume = volume
            .checked_sub(before)
            .and_then(|volume| volume.checked_add(after))
            .ok_or_else(|| past_decimal(format_smolstr!("{volume} - {before} + {after}")))?;
        Ok(())
    }

    /// The candle this fold states over `bucket`, and the trades the bucket
    /// after it carries in; none where a count spells past `decimal`, which
    /// the volume already refused.
    fn into_candle(self, crosscode: SmolStr, bucket: Bucket) -> (Candle, Option<Trades>) {
        let candle = Candle {
            crosscode,
            ticker: self.ticker,
            start: bucket.start,
            end: bucket.end,
            bid: self.bid,
            ask: self.ask,
            mid: self.mid,
            spread: self.spread,
            bidqty: self.bidqty,
            askqty: self.askqty,
            books: self.books,
            executions: self.executions,
            volume: self.volume,
        };
        (candle, self.trades.carried())
    }
}

/// Candles from a sorted stream of books, one per cross code and bucket.
///
/// Books must arrive sorted by [`Event::get_currunix`]; a regression is
/// refused at `$.book.currunix`. The candles of a bucket are emitted, in
/// cross-code order, when the stream moves past the bucket and at its end;
/// an empty bucket yields no candle. The open bucket holds one fold per
/// cross code, each with the trades its books reported ([`Candle::volume`]),
/// until the bucket closes; the trades a closed bucket counted are held
/// until the next bucket closes, for the cross codes it folds. An error - the source's, a regression,
/// an instant the zone cannot read, a volume past `decimal` - ends the walk
/// after the candles of every bucket the stream moved past: the bucket that
/// was open is dropped rather than emitted incomplete, the error is the last
/// item, and the iterator fuses.
///
/// ```
/// use yggdryl::graph::{BookIterator, CandleIterator, CandleOptions, Element, Event, Market, MarketData, QuoteEvent};
/// use yggdryl::{Decimal, Side, State};
///
/// # fn main() -> yggdryl::Result<()> {
/// let quote = |unix: i64, code: &str, side: &str, price: &str| -> yggdryl::Result<MarketData> {
///     let mut quote = QuoteEvent::at(unix);
///     quote.set_crosscode(code.to_owned());
///     quote.set_ticker(Some("ACME".into()));
///     quote.set_side(Side::read(side).expect("a shipped side"));
///     quote.set_price(Some(price.parse()?));
///     quote.set_quantity(Some(Decimal::from_int(10)));
///     quote.set_state(State::New);
///     quote.finalize();
///     Ok(MarketData::from(quote))
/// };
/// let second = 1_000_000_000;
/// let quotes = vec![
///     quote(10 * second, "B", "Buy", "100")?,
///     quote(10 * second, "A", "Sell", "101")?,
///     quote(20 * second, "B", "Buy", "102")?,
///     quote(70 * second, "A", "Sell", "104")?,
/// ];
/// let books = BookIterator::new(quotes.into_iter(), 0)?;
/// let candles = CandleIterator::new(books, CandleOptions::from_spelling("1m")?)
///     .collect::<yggdryl::Result<Vec<_>>>()?;
/// assert_eq!(candles.len(), 2);
/// let first = &candles[0];
/// assert_eq!((first.start, first.end), (0, 60 * second));
/// assert_eq!(first.crosscode, "ACME");
/// let bid = first.bid.expect("two books stated a bid");
/// assert_eq!((bid.open, bid.close), (Decimal::from_int(100), Decimal::from_int(102)));
/// assert_eq!(first.spread.map(|spread| spread.close), Some(Decimal::from_int(-1)));
/// assert_eq!(first.books, 2);
/// assert_eq!(candles[1].start, 60 * second);
/// assert_eq!(candles[1].ask.map(|ask| ask.open), Some(Decimal::from_int(104)));
/// # Ok(())
/// # }
/// ```
pub struct CandleIterator<I> {
    books: I,
    options: CandleOptions,
    bucket: Option<Bucket>,
    open: BTreeMap<SmolStr, Fold>,
    /// The trades the bucket that closed last carries into the one after
    /// it, per cross code, and where that bucket ended.
    carried: (BTreeMap<SmolStr, Trades>, Option<i64>),
    pending: VecDeque<Candle>,
    /// The refusal that ended the walk, answered once `pending` is drained.
    failed: Option<Error>,
    last_unix: Option<i64>,
    done: bool,
}

impl<I> CandleIterator<I>
where
    I: Iterator<Item = Result<BookEvent>>,
{
    /// Opens a candle walk over books already sorted by their instant.
    #[must_use]
    pub fn new(books: I, options: CandleOptions) -> Self {
        Self {
            books,
            options,
            bucket: None,
            open: BTreeMap::new(),
            carried: (BTreeMap::new(), None),
            pending: VecDeque::new(),
            failed: None,
            last_unix: None,
            done: false,
        }
    }

    /// The options the walk buckets by.
    #[must_use]
    pub fn options(&self) -> &CandleOptions {
        &self.options
    }

    /// Emits the open bucket's candles, in cross-code order, holding the
    /// trades each carries into the bucket after it.
    fn flush(&mut self) {
        let Some(bucket) = self.bucket.take() else {
            return;
        };
        let mut carried = BTreeMap::new();
        for (crosscode, fold) in std::mem::take(&mut self.open) {
            let (candle, trades) = fold.into_candle(crosscode, bucket);
            if let Some(trades) = trades.filter(|trades| !trades.is_empty()) {
                carried.insert(candle.crosscode.clone(), trades);
            }
            self.pending.push_back(candle);
        }
        self.carried = (carried, Some(bucket.end));
    }

    /// Folds one book into its bucket, closing the bucket before it.
    fn fold(&mut self, book: &BookEvent) -> Result<()> {
        let unix = book.get_currunix();
        if let Some(previous) = self.last_unix.filter(|previous| unix < *previous) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static(BOOK_UNIX),
                reason: expected_got(format_smolstr!("an instant at or after {previous}"), unix),
            });
        }
        self.last_unix = Some(unix);
        if !self.bucket.is_some_and(|bucket| bucket.contains(unix)) {
            self.flush();
            let bucket = self.options.bucket(unix)?;
            // Only the bucket just before carries its trades in.
            if self.carried.1 != Some(bucket.start) {
                self.carried.0.clear();
            }
            self.bucket = Some(bucket);
        }
        let crosscode = book.get_crosscode();
        match self.open.get_mut(crosscode) {
            Some(fold) => fold.fold(book),
            None => {
                let trades = self.carried.0.remove(crosscode).unwrap_or_default();
                let mut fold = Fold::opened(book, trades);
                fold.fold(book)?;
                self.open.insert(SmolStr::new(crosscode), fold);
                Ok(())
            }
        }
    }
}

impl<I> Iterator for CandleIterator<I>
where
    I: Iterator<Item = Result<BookEvent>>,
{
    type Item = Result<Candle>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(candle) = self.pending.pop_front() {
                return Some(Ok(candle));
            }
            if let Some(error) = self.failed.take() {
                return Some(Err(error));
            }
            if self.done {
                return None;
            }
            let Some(next) = self.books.next() else {
                self.done = true;
                self.flush();
                continue;
            };
            // A book past the open bucket emits it before its own bucket
            // and fold are read, so the candles it completed are answered
            // before a refusal of either.
            let folded = next.and_then(|book| self.fold(&book));
            if let Err(error) = folded {
                self.done = true;
                self.open.clear();
                self.carried.0.clear();
                self.bucket = None;
                self.failed = Some(error);
            }
        }
    }
}

impl<I> FusedIterator for CandleIterator<I> where I: Iterator<Item = Result<BookEvent>> {}

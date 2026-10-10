//! The typed body facts of an instrument no agency numbers - what the
//! `class:body` cross code of an FX forward, a swap, an option or a future
//! is written from, each held once as its canonical value and spelled by the
//! one speller (`Instrument::write_crosscode`), never parsed back per row.
//!
//! [`Settle`] is when an FX pair settles: a date, or the FIX `SettlType(63)`
//! text a tenor spells; [`Expiry`] is when a derivative expires: a day, or a
//! contract month with an optional week; [`Exercise`] is an option's style;
//! [`Characteristics`] holds them beside the strike, the multiplier and the
//! style, every field nullable, a cash security stating none.

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use yggdryl::implementer::expected_got;
use yggdryl::{DataType, Date32, Decimal, Error, Field, Result, Scalar};

/// The column names of [`Characteristics::dtype`], in order.
const NAMES: [&str; 6] = [
    "settle",
    "settle2",
    "expiry",
    "strikepx",
    "multiplier",
    "exercise",
];

/// The most bytes a tenor spells: FIX's `SettlType(63)` values are one to
/// three bytes.
const MAX_TENOR_WIDTH: usize = 3;

/// A refusal located on `name`.
fn refused(name: &str, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason,
    }
}

/// When an FX pair settles, as its cross code spells it: the ISO date of a
/// stated `SettlDate(64)` or `SettlDate2(193)`, else the FIX `SettlType(63)`
/// text a venue's symbol spells - `0`, `1`, `2`, `C`, `W1`, `B`, `M3`,
/// `Y1`. The two never collide: a tenor is one to three bytes with no `-`,
/// a date ten with two.
///
/// ```
/// use yggdryl_market::Settle;
///
/// let date: Settle = "2027-01-15".parse().unwrap();
/// assert_eq!(date.to_string(), "2027-01-15");
/// let tenor: Settle = "m3".parse().unwrap();
/// assert_eq!(tenor.to_string(), "M3");
/// assert!("2027-01-15T00:00".parse::<Settle>().is_err());
/// assert!("TOOLONG".parse::<Settle>().is_err());
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Settle {
    /// A stated settlement date.
    Date(Date32),
    /// A tenor, as FIX's `SettlType(63)` spells it, upper case.
    Tenor(SmolStr),
}

impl Settle {
    /// The settlement `text` spells: a date, else a tenor.
    ///
    /// # Errors
    ///
    /// A text that is neither a calendar day nor one to three ASCII letters
    /// or digits.
    pub fn from_text(text: &str) -> Result<Self> {
        if let Ok(day) = Date32::from_text(text) {
            return Ok(Self::Date(day));
        }
        let bytes = text.as_bytes();
        if (1..=MAX_TENOR_WIDTH).contains(&bytes.len())
            && bytes.iter().all(u8::is_ascii_alphanumeric)
        {
            return Ok(Self::Tenor(SmolStr::new(text.to_ascii_uppercase())));
        }
        Err(refused(
            NAMES[0],
            format_smolstr!(
                "expected a settlement date or a one-to-three-letter tenor, got {text:?}"
            ),
        ))
    }
}

impl fmt::Display for Settle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date(day) => write!(formatter, "{day}"),
            Self::Tenor(tenor) => formatter.write_str(tenor),
        }
    }
}

impl std::str::FromStr for Settle {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::from_text(text)
    }
}

/// When a derivative expires, as its cross code spells it: the ISO date of
/// `MaturityDate(541)` for an option, or the contract month of
/// `MaturityMonthYear(200)` - `YYYY-MM`, `YYYY-MMwN` for a weekly listing -
/// for a future, whose identity is its month.
///
/// ```
/// use yggdryl_market::Expiry;
///
/// let day: Expiry = "2026-12-18".parse().unwrap();
/// assert_eq!(day.to_string(), "2026-12-18");
/// assert_eq!(day.month().to_string(), "2026-12");
/// let week: Expiry = "202612w3".parse().unwrap();
/// assert_eq!(week.to_string(), "2026-12w3");
/// assert!("2026-13".parse::<Expiry>().is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Expiry {
    /// A stated expiry day.
    Day(Date32),
    /// A contract month, with the week of a weekly listing.
    Month {
        /// The year.
        year: u16,
        /// The month, one to twelve.
        month: u8,
        /// The week of the month a weekly listing states, one to five.
        week: Option<u8>,
    },
}

impl Expiry {
    /// The expiry `text` spells: a day in either ISO spelling, else a month
    /// `YYYY-MM` or `YYYYMM`, either closed by `wN` for a week.
    ///
    /// # Errors
    ///
    /// A text that spells neither.
    pub fn from_text(text: &str) -> Result<Self> {
        if let Ok(day) = Date32::from_text(text) {
            return Ok(Self::Day(day));
        }
        let unread = || {
            refused(
                NAMES[2],
                format_smolstr!("expected an expiry day, month or week, got {text:?}"),
            )
        };
        let (month_text, week) = match text.split_once('w') {
            Some((month, week)) => {
                let week: u8 = week.parse().map_err(|_| unread())?;
                if !(1..=5).contains(&week) {
                    return Err(unread());
                }
                (month, Some(week))
            }
            None => (text, None),
        };
        let digits: String = month_text.chars().filter(|c| *c != '-').collect();
        let (year, month) = match (month_text.len(), digits.len()) {
            (7, 6) if month_text.as_bytes()[4] == b'-' => (&digits[..4], &digits[4..]),
            (6, 6) => (&digits[..4], &digits[4..]),
            _ => return Err(unread()),
        };
        let year: u16 = year.parse().map_err(|_| unread())?;
        let month: u8 = month.parse().map_err(|_| unread())?;
        if !(1..=12).contains(&month) {
            return Err(unread());
        }
        Ok(Self::Month { year, month, week })
    }

    /// The month this expiry falls in: a month as it is, a day's month.
    #[must_use]
    pub fn month(self) -> Self {
        match self {
            Self::Month { .. } => self,
            Self::Day(day) => {
                let (year, month, _) =
                    yggdryl::implementer::civil_from_days(i64::from(day.count()));
                Self::Month {
                    year: u16::try_from(year).unwrap_or_default(),
                    month: u8::try_from(month).unwrap_or_default(),
                    week: None,
                }
            }
        }
    }
}

impl fmt::Display for Expiry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Day(day) => write!(formatter, "{day}"),
            Self::Month { year, month, week } => {
                write!(formatter, "{year:04}-{month:02}")?;
                if let Some(week) = week {
                    write!(formatter, "w{week}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::str::FromStr for Expiry {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::from_text(text)
    }
}

/// An option's exercise style, FIX's `ExerciseStyle(1194)`: a fact of the
/// instrument, never a key byte.
///
/// ```
/// use yggdryl_market::Exercise;
///
/// assert_eq!(Exercise::from_fix("1"), Some(Exercise::American));
/// assert_eq!("european".parse::<Exercise>().unwrap(), Exercise::European);
/// assert_eq!(Exercise::Bermuda.as_str(), "Bermuda");
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Exercise {
    /// Exercised at expiry alone.
    European,
    /// Exercised any day to expiry.
    American,
    /// Exercised on stated days.
    Bermuda,
}

impl Exercise {
    /// Every style, in FIX code order.
    pub const ALL: [Self; 3] = [Self::European, Self::American, Self::Bermuda];

    /// The style a FIX `ExerciseStyle(1194)` code names.
    #[must_use]
    pub fn from_fix(code: &str) -> Option<Self> {
        match code {
            "0" => Some(Self::European),
            "1" => Some(Self::American),
            "2" => Some(Self::Bermuda),
            _ => None,
        }
    }

    /// The style's name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::European => "European",
            Self::American => "American",
            Self::Bermuda => "Bermuda",
        }
    }

    /// The FIX `ExerciseStyle(1194)` code.
    #[must_use]
    pub const fn fix_code(self) -> &'static str {
        match self {
            Self::European => "0",
            Self::American => "1",
            Self::Bermuda => "2",
        }
    }
}

impl fmt::Display for Exercise {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for Exercise {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|style| style.as_str().eq_ignore_ascii_case(text))
            .or_else(|| Self::from_fix(text))
            .ok_or_else(|| {
                refused(
                    NAMES[5],
                    format_smolstr!("expected European, American or Bermuda, got {text:?}"),
                )
            })
    }
}

/// The typed facts the body of a `class:body` cross code is written from,
/// beside the two a fact of the instrument but no key byte - the multiplier
/// and the exercise style. Every field nullable; a cash security holds the
/// default, stating none.
///
/// ```
/// use yggdryl::Decimal;
/// use yggdryl_market::{Characteristics, Exercise};
///
/// let option = Characteristics::default()
///     .with_expiry(Some("2026-12-18".parse().unwrap()))
///     .with_strikepx(Some(Decimal::parse("200").unwrap()))
///     .with_exercise(Some(Exercise::American));
/// assert_eq!(option.expiry().map(|expiry| expiry.to_string()), Some("2026-12-18".to_owned()));
/// assert_eq!(Characteristics::from_scalar(&option.into_scalar()).unwrap(), option);
/// assert!(Characteristics::default().is_default());
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Characteristics {
    settle: Option<Settle>,
    settle2: Option<Settle>,
    expiry: Option<Expiry>,
    strikepx: Option<Decimal>,
    multiplier: Option<Decimal>,
    exercise: Option<Exercise>,
}

impl Characteristics {
    /// The datatype the characteristics are: `struct<settle: utf8,
    /// settle2: utf8, expiry: utf8, strikepx: decimal, multiplier: decimal,
    /// exercise: utf8>`, each nullable - the settles and the expiry as the
    /// text the cross code spells, since each is a date or a tenor, a day,
    /// a month or a week, which one typed column cannot hold.
    #[must_use]
    pub fn dtype() -> DataType {
        DataType::Struct(yggdryl::implementer::struct_type_from_unique_fields(vec![
            DataType::utf8().nullable_field(NAMES[0]),
            DataType::utf8().nullable_field(NAMES[1]),
            DataType::utf8().nullable_field(NAMES[2]),
            DataType::Decimal.nullable_field(NAMES[3]),
            DataType::Decimal.nullable_field(NAMES[4]),
            DataType::utf8().nullable_field(NAMES[5]),
        ]))
    }

    /// The nullable field `characteristics` an instrument row holds.
    #[must_use]
    pub fn field() -> Field {
        Field::new("characteristics", Self::dtype(), true)
    }

    /// When the pair settles - the near leg of a swap.
    #[must_use]
    pub fn settle(&self) -> Option<&Settle> {
        self.settle.as_ref()
    }

    /// When the far leg of a swap settles.
    #[must_use]
    pub fn settle2(&self) -> Option<&Settle> {
        self.settle2.as_ref()
    }

    /// When the derivative expires.
    #[must_use]
    pub fn expiry(&self) -> Option<Expiry> {
        self.expiry
    }

    /// The strike price.
    #[must_use]
    pub fn strikepx(&self) -> Option<Decimal> {
        self.strikepx
    }

    /// The contract multiplier, FIX's `ContractMultiplier(231)`.
    #[must_use]
    pub fn multiplier(&self) -> Option<Decimal> {
        self.multiplier
    }

    /// The exercise style.
    #[must_use]
    pub fn exercise(&self) -> Option<Exercise> {
        self.exercise
    }

    /// Whether every field is unstated: a cash security's.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The characteristics with the settle.
    #[must_use]
    pub fn with_settle(mut self, settle: Option<Settle>) -> Self {
        self.settle = settle;
        self
    }

    /// The characteristics with the far settle.
    #[must_use]
    pub fn with_settle2(mut self, settle: Option<Settle>) -> Self {
        self.settle2 = settle;
        self
    }

    /// The characteristics with the expiry.
    #[must_use]
    pub fn with_expiry(mut self, expiry: Option<Expiry>) -> Self {
        self.expiry = expiry;
        self
    }

    /// The characteristics with the strike.
    #[must_use]
    pub fn with_strikepx(mut self, strikepx: Option<Decimal>) -> Self {
        self.strikepx = strikepx;
        self
    }

    /// The characteristics with the multiplier.
    #[must_use]
    pub fn with_multiplier(mut self, multiplier: Option<Decimal>) -> Self {
        self.multiplier = multiplier;
        self
    }

    /// The characteristics with the exercise style.
    #[must_use]
    pub fn with_exercise(mut self, exercise: Option<Exercise>) -> Self {
        self.exercise = exercise;
        self
    }

    /// The characteristics as the named struct of their six cells, an
    /// unstated one a null.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        let text = |value: Option<String>| value.map_or(Scalar::Null, Scalar::from);
        Scalar::from_struct([
            (NAMES[0], text(self.settle.as_ref().map(Settle::to_string))),
            (NAMES[1], text(self.settle2.as_ref().map(Settle::to_string))),
            (NAMES[2], text(self.expiry.map(|expiry| expiry.to_string()))),
            (NAMES[3], self.strikepx.map_or(Scalar::Null, Scalar::from)),
            (NAMES[4], self.multiplier.map_or(Scalar::Null, Scalar::from)),
            (
                NAMES[5],
                self.exercise
                    .map_or(Scalar::Null, |style| Scalar::from(style.as_str())),
            ),
        ])
        .expect("six distinct names")
    }

    /// The characteristics as the ordered run of their six cells in
    /// [`Self::dtype`]'s order - what a snapshot streams.
    // Built from borrowed facts, as `into_scalar` beside it is.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn into_row(&self) -> Scalar {
        let text = |value: Option<String>| value.map_or(Scalar::Null, Scalar::from);
        Scalar::from_sequence([
            text(self.settle.as_ref().map(Settle::to_string)),
            text(self.settle2.as_ref().map(Settle::to_string)),
            text(self.expiry.map(|expiry| expiry.to_string())),
            self.strikepx.map_or(Scalar::Null, Scalar::from),
            self.multiplier.map_or(Scalar::Null, Scalar::from),
            self.exercise
                .map_or(Scalar::Null, |style| Scalar::from(style.as_str())),
        ])
    }

    /// Reads the characteristics back from the named struct
    /// [`Self::into_scalar`] answers - a name it lacks a null - or from the
    /// ordered row [`Self::dtype`]'s value door canonicalizes it to, a null
    /// whole the default.
    ///
    /// # Errors
    ///
    /// The refusal [`Self::field`]'s [`Field::scalar`] answers, and a settle,
    /// an expiry or an exercise text that spells none.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        if matches!(value, Scalar::Null) {
            return Ok(Self::default());
        }
        let value = match value.as_struct() {
            Some(fields) => {
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
            None => value.clone(),
        };
        let row = Self::field().scalar(value)?;
        let unread = || Error::InvalidRecord {
            path: SmolStr::new_static("$.characteristics"),
            reason: expected_got("the canonical characteristics row", row.kind()),
        };
        let cells = row.sequence_rows().ok_or_else(unread)?;
        let [settle, settle2, expiry, strikepx, multiplier, exercise] = cells.as_ref() else {
            return Err(unread());
        };
        let decimal = |cell: &Scalar| match cell {
            Scalar::Null => Ok(None),
            Scalar::Decimal(held) => Ok(Some(*held)),
            _ => Err(unread()),
        };
        Ok(Self {
            settle: settle.as_str().map(Settle::from_text).transpose()?,
            settle2: settle2.as_str().map(Settle::from_text).transpose()?,
            expiry: expiry.as_str().map(Expiry::from_text).transpose()?,
            strikepx: decimal(strikepx)?,
            multiplier: decimal(multiplier)?,
            exercise: exercise.as_str().map(str::parse).transpose()?,
        })
    }

    /// Feeds each stated characteristic to `feed`, named, as its canonical
    /// bytes: a settle or an expiry its text, a decimal its units, a style
    /// its name.
    pub(crate) fn feed(&self, mut feed: impl FnMut(&str, &[u8])) {
        if let Some(settle) = &self.settle {
            feed(NAMES[0], settle.to_string().as_bytes());
        }
        if let Some(settle) = &self.settle2 {
            feed(NAMES[1], settle.to_string().as_bytes());
        }
        if let Some(expiry) = self.expiry {
            feed(NAMES[2], expiry.to_string().as_bytes());
        }
        if let Some(strike) = self.strikepx {
            feed(NAMES[3], &strike.units().to_le_bytes());
        }
        if let Some(multiplier) = self.multiplier {
            feed(NAMES[4], &multiplier.units().to_le_bytes());
        }
        if let Some(style) = self.exercise {
            feed(NAMES[5], style.as_str().as_bytes());
        }
    }
}

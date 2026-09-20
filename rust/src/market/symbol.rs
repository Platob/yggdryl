//! The instrument a statement is about, as the one text every book of it
//! is keyed by.

use std::fmt;

use smol_str::SmolStr;

use crate::graph::MarketElement;

/// The key a book is read under: the text that names an instrument across
/// the venues that trade it, or [`Symbol::GLOBAL`] for no instrument.
///
/// A statement names its instrument under whichever codes the venue
/// spelled - an ISIN, a ticker, a CUSIP, a SEDOL, a Bloomberg identifier -
/// and [`Symbol::of`] reads one key off them, the strongest first: the
/// ISIN, because it names the issue across every venue that trades it;
/// else the ticker, what a venue spells; else the three that are a
/// nation's or a vendor's. A statement naming none keys the global
/// symbol, which is also the one symbol a global book - every statement
/// of a stream in one ladder - is read under. The text orders bytewise,
/// which is the order books of one instant are yielded in, and a
/// [`BookData`](super::BookData)'s key is its `crosscode`, the symbol it
/// was read under; `Symbol::of` is the rule for a statement.
///
/// ```
/// use yggdryl::graph::{MarketElement, MarketEventData};
/// use yggdryl::market::Symbol;
/// use yggdryl::Isin;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut order = MarketEventData::at(1_700_000_000_000_000_000);
/// assert_eq!(Symbol::of(&order), Symbol::GLOBAL, "no instrument named");
/// order.set_symbolticker(Some("AAPL".to_owned()));
/// assert_eq!(Symbol::of(&order).as_str(), "AAPL");
/// order.set_isincode(Some(Isin::new("US0378331005")?));
/// assert_eq!(Symbol::of(&order).as_str(), "US0378331005", "the ISIN leads");
/// assert_eq!(Symbol::new("  "), Symbol::default(), "blank text is the global symbol");
/// assert!(Symbol::new("AAPL") < Symbol::new("MSFT"), "ordered by text");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Symbol(SmolStr);

impl Symbol {
    /// The symbol of no instrument: what a statement naming none keys,
    /// and the one symbol a global book is read under. Spelled as the word,
    /// because no standard reserves a code for it; a venue ticker spelled
    /// `GLOBAL` keys the global book, exactly as a currency spelled `XXX`
    /// is none.
    pub const GLOBAL: Self = Self(SmolStr::new_static("GLOBAL"));

    /// A symbol spelled by the caller, trimmed; blank text is
    /// [`Self::GLOBAL`].
    #[must_use]
    pub fn new(text: &str) -> Self {
        let text = text.trim();
        if text.is_empty() {
            Self::GLOBAL
        } else {
            Self(SmolStr::new(text))
        }
    }

    /// The symbol an element names: its ISIN, else the ticker it is known
    /// by, else its CUSIP, else its SEDOL, else its Bloomberg identifier,
    /// else [`Self::GLOBAL`]. A blank ticker names nothing.
    #[must_use]
    pub fn of<E: MarketElement + ?Sized>(element: &E) -> Self {
        element
            .get_isincode()
            .map(|held| held.as_str())
            .or_else(|| element.get_symbolticker().map(str::trim))
            .filter(|held| !held.is_empty())
            .or_else(|| element.get_cusipcode().map(|held| held.as_str()))
            .or_else(|| element.get_sedolcode().map(|held| held.as_str()))
            .or_else(|| element.get_bloombergcode().map(|held| held.as_str()))
            .map_or(Self::GLOBAL, Self::new)
    }

    /// The text the symbol is.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether this is the symbol of no instrument.
    #[must_use]
    pub fn is_global(&self) -> bool {
        *self == Self::GLOBAL
    }
}

impl Default for Symbol {
    /// The global symbol.
    fn default() -> Self {
        Self::GLOBAL
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

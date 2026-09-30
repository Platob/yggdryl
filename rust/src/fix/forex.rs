//! Foreign exchange detection: the currency pair a message's `Symbol(55)`
//! names, read once per distinct spelling, and the cells the pair implies.
//!
//! A venue quoting a currency pair routinely states nothing but the symbol:
//! `55=EUR/USD` and a price, with no `SecurityType(167)`, no `Product(460)`,
//! no classification and no currency. The symbol already says all of them,
//! so detection reads it - through [`FxSymbol::from_symbol`], the one
//! detector - and fills what the message left absent, exactly as a
//! [derivation](super::enrich) fills what a message implies: only where a
//! cell is absent, or where detection wrote it itself.
//!
//! The pair is *derived*, never stated: it enters the message's derived
//! identifier overlay under the crate's `FOREX` key, so a stated identifier
//! still replaces it and a changed symbol derives it again. The `forexcode`
//! cell is a view of that key, so a row written from a detected message
//! carries the pair and a reader of that row states it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use smallvec::SmallVec;
use smol_str::SmolStr;

use super::FixMsg;
use crate::xxhash::Xxh64;
use crate::{Cfi, FxSymbol, FxTenor, Result, Scalar};

/// Every symbol one registry's messages have spelled, each read once into
/// the pair it names - or into none.
///
/// Held by the registry, so every codec and door reading that registry
/// shares the answers; a symbol names the pair it names whatever the
/// dictionary holds, so no field change forgets them. Keyed by
/// the trimmed symbol text, success and failure alike, and bounded at
/// [`Self::CAPACITY`] entries past which an answer is still given and simply
/// not remembered, the rule [`Memo`](super::memo::Memo) states for the
/// dictionary's own answers: a capture spells a few hundred distinct
/// symbols, the bound keeps the table under 512 KiB, and a capture spelling
/// more is spelling junk the parse still answers.
///
/// A hit clones an [`FxSymbol`] whose texts are inline - a pair is seven
/// bytes, a settlement type at most two - so a hit allocates nothing.
pub struct FxMemo {
    symbols: Mutex<HashMap<SmolStr, Option<FxSymbol>, Xxh64>>,
    /// How many symbols were parsed rather than answered from the table.
    parses: AtomicU64,
}

impl FxMemo {
    /// The most symbols the table remembers.
    pub const CAPACITY: usize = 4_096;

    /// A memo remembering nothing yet.
    pub(super) fn new() -> Self {
        Self {
            symbols: Mutex::new(HashMap::with_hasher(Xxh64::new())),
            parses: AtomicU64::new(0),
        }
    }

    /// The pair `text` names with what it says about the tenor, or `None`
    /// where it names no pair: remembered after the first ask while the
    /// table holds fewer than [`Self::CAPACITY`] symbols.
    pub(super) fn symbol(&self, text: &str) -> Option<FxSymbol> {
        let text = text.trim_matches(|c: char| c.is_ascii_whitespace());
        let mut symbols = self.symbols.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = symbols.get(text) {
            return held.clone();
        }
        self.parses.fetch_add(1, Ordering::Relaxed);
        let answer = FxSymbol::from_symbol(text);
        if symbols.len() < Self::CAPACITY {
            symbols.insert(SmolStr::new(text), answer.clone());
        }
        answer
    }
}

/// The FIX `SecurityType(167)` codes that are foreign exchange: a stated
/// type among them leaves a symbol to name the pair, any other names an
/// instrument that is not one.
const FX_SECURITY_TYPES: [&str; 8] = [
    "FOR", "FXNDF", "FXSPOT", "FXFWD", "FXSWAP", "FXNDS", "FXBN", "FXDN",
];

/// The cells detection fills, each with the bit of [`FixMsg`]'s detected
/// word saying detection wrote it: `SecurityType(167)`, `Product(460)`,
/// `CFICode(461)`, `Currency(15)`, `SettlCurrency(120)` and
/// `SettlType(63)`.
const CELLS: [(i32, u8); 6] = [
    (167, 1),
    (460, 1 << 1),
    (461, 1 << 2),
    (15, 1 << 3),
    (120, 1 << 4),
    (63, 1 << 5),
];

/// Every tag detection reads: the symbol, the cells it fills and
/// `SettlDate(64)`, which leaves a settlement type to the message. A write
/// to one of them on a message detection answered for runs it again.
pub(super) const READS: [i32; 8] = [55, 167, 460, 461, 15, 120, 63, 64];

/// The detected-word bit of one cell, `0` for a tag detection never fills.
pub(super) fn detected_bit(tag: i32) -> u8 {
    CELLS
        .iter()
        .find_map(|(held, bit)| (*held == tag).then_some(*bit))
        .unwrap_or(0)
}

impl FixMsg {
    /// Detects the currency pair `Symbol(55)` names and fills what it
    /// implies, where the message states no other class.
    ///
    /// Skipped where the row stated its `forexcode`: that is the row's word.
    /// A symbol naming no pair, or a message stating a class that is not
    /// foreign exchange - a `SecurityType(167)` outside the FX codes, a
    /// `Product(460)` other than currency (commodity too for a metal), a
    /// `CFICode(461)` that is neither all `X` nor an FX group - detects
    /// nothing, and takes back whatever an earlier detection of this message
    /// derived. A detected pair enters the derived identifier overlay under
    /// `FOREX`, and the cells below are written only where absent or where
    /// detection wrote them before:
    ///
    /// - `SecurityType(167)`: `FXSPOT` for a spot or unstated tenor, `FXFWD`
    ///   for a forward; nothing for a metal.
    /// - `Product(460)`: `4`, a metal `2`.
    /// - `CFICode(461)`, also where it is all `X`: `IFXXXP` for `FXSPOT`,
    ///   `JFTXFP` for `FXFWD`, `ITKXXX` for a metal.
    /// - `Currency(15)`: the base leg; a stated currency that is neither leg
    ///   leaves both currencies alone.
    /// - `SettlCurrency(120)`: the other leg, where `Currency(15)` is one.
    /// - `SettlType(63)`: what the symbol's suffix spells, where neither it
    ///   nor `SettlDate(64)` is stated.
    ///
    /// Written through the lenient unsettled row write the derivations use,
    /// so a value the dictionary's field refuses is silence. Answers whether
    /// detection moved anything - a cell written or taken back, the derived
    /// pair set or dropped, a cell it owns - which a message a pass already
    /// read never has.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the written children do not
    /// make a root.
    pub(super) fn derive_forex(&mut self, memo: &FxMemo) -> Result<bool> {
        if self.states_forex() {
            return Ok(false);
        }
        let owned = self.detected_fx();
        // A cell detection wrote is its own, and reads as absent to it.
        let stated = |tag: i32| {
            if owned & detected_bit(tag) != 0 {
                return None;
            }
            self.get_by_tag(tag)
                .as_ref()
                .and_then(super::msg::scalar_text)
        };
        let written = self.get_by_tag(55);
        let detected = written
            .as_ref()
            .and_then(Scalar::as_str)
            .map(str::trim)
            .filter(|held| !held.is_empty() && *held != "[N/A]" && *held != "[N/A")
            .and_then(|held| memo.symbol(held))
            .filter(|symbol| admits(symbol, &stated));
        let Some(symbol) = detected else {
            if owned == 0 && !self.derives_pair() {
                return Ok(false);
            }
            // What an earlier detection derived no longer derives.
            let retracted: SmallVec<[(i32, Scalar); 6]> = CELLS
                .iter()
                .filter(|(_, bit)| owned & bit != 0)
                .map(|(tag, _)| (*tag, Scalar::Null))
                .collect();
            self.set_derived_pair(None);
            if !retracted.is_empty() {
                self.set_each(retracted)?;
            }
            self.set_detected_fx(0);
            return Ok(true);
        };
        let forex = &symbol.forex;
        let metal = forex.is_metal();
        let (base, quote) = (forex.base(), forex.quote());
        let security = (!metal).then_some(match symbol.tenor {
            FxTenor::Spot | FxTenor::Unstated => "FXSPOT",
            FxTenor::Forward => "FXFWD",
        });
        let stated_security = stated(167);
        let classification = match stated_security.as_deref().or(security) {
            _ if metal => Cfi::is_detailed("ITKXXX").then_some("ITKXXX"),
            Some(held) if held.eq_ignore_ascii_case("FXSPOT") => Some("IFXXXP"),
            Some(held) if held.eq_ignore_ascii_case("FXFWD") => Some("JFTXFP"),
            _ => None,
        };
        let stated_cfi = stated(461).filter(|held| !held.bytes().all(|byte| byte == b'X'));
        // A stated currency that is neither leg is another reading of the
        // message, and both currencies stay the message's.
        let stated_currency = stated(15);
        let leg = |held: &str| held == base.as_str() || held == quote.as_str();
        let blocked = stated_currency.as_deref().is_some_and(|held| !leg(held));
        let currency = stated_currency.as_deref().unwrap_or(base.as_str());
        let settlcurrency = if currency == base.as_str() {
            quote.as_str()
        } else {
            base.as_str()
        };
        // A settlement date is a clock, stated whatever it spells.
        let settles = self.get_by_tag(64).is_none_or(|held| held.is_null());
        let wanted = |tag: i32| -> Option<Scalar> {
            match tag {
                167 => security.map(Scalar::from),
                460 => Some(Scalar::from(if metal { 2_i32 } else { 4 })),
                461 => classification.map(Scalar::from),
                15 => Some(Scalar::from(base.as_str())),
                120 => (!blocked).then(|| Scalar::from(settlcurrency)),
                63 => symbol
                    .settltype
                    .as_deref()
                    .filter(|_| settles)
                    .map(Scalar::from),
                _ => None,
            }
        };
        let mut writes: SmallVec<[(i32, Scalar); 6]> = SmallVec::new();
        let mut detected_fx = 0_u8;
        for (tag, bit) in CELLS {
            let own = owned & bit != 0;
            // 461 all `X` is no classification, so detection may replace it.
            let open = own
                || if tag == 461 {
                    stated_cfi.is_none()
                } else {
                    stated(tag).is_none()
                };
            if !open {
                continue;
            }
            match wanted(tag) {
                Some(value) => {
                    detected_fx |= bit;
                    let held = self
                        .get_by_tag(tag)
                        .as_ref()
                        .and_then(super::msg::scalar_text);
                    if held.as_deref() != super::msg::scalar_text(&value).as_deref() {
                        writes.push((tag, value));
                    }
                }
                None if own => writes.push((tag, Scalar::Null)),
                None => {}
            }
        }
        let paired = !self.derives_pair_of(forex.as_str());
        if paired {
            self.set_derived_pair(Some(forex.as_str()));
        }
        let wrote = !writes.is_empty();
        if wrote {
            self.set_each(writes)?;
        }
        self.set_detected_fx(detected_fx);
        Ok(paired || wrote || detected_fx != owned)
    }
}

/// Whether the message states no class other than foreign exchange for the
/// pair `symbol` names, `stated` answering what it states under a tag.
fn admits(symbol: &FxSymbol, stated: &impl Fn(i32) -> Option<SmolStr>) -> bool {
    let metal = symbol.forex.is_metal();
    let security = stated(167).is_none_or(|held| {
        FX_SECURITY_TYPES
            .iter()
            .any(|code| held.eq_ignore_ascii_case(code))
    });
    let product = stated(460).is_none_or(|held| held == "4" || (metal && held == "2"));
    let classified = stated(461).is_none_or(|held| {
        let held = held.as_bytes();
        let group = |open: &[u8]| held.len() >= 2 && held[..2].eq_ignore_ascii_case(open);
        held.iter().all(|byte| byte.eq_ignore_ascii_case(&b'X'))
            || group(b"IF")
            || group(b"JF")
            || group(b"SF")
            || group(b"HF")
            || (metal && group(b"IT"))
    });
    security && product && classified
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/forex.rs` pins and a caller cannot reach.
    //!
    //! The memo is invisible except through the parses it does *not*
    //! repeat, so the pin counts them. `fix::forex` is a private module of a
    //! published one, so these being `pub` reaches nobody: this door is the
    //! only path to them, and it exists under the `internals` feature alone.
    use std::sync::atomic::Ordering;

    use crate::{FixMsg, FxSymbol, Result};

    pub use super::FxMemo;

    /// Runs FX detection over `message` against `memo`.
    ///
    /// # Errors
    ///
    /// Returns what detection's row write returns.
    pub fn derive(message: &mut FixMsg, memo: &FxMemo) -> Result<()> {
        message.derive_forex(memo).map(|_| ())
    }

    /// A memo remembering nothing yet.
    #[must_use]
    pub fn memo() -> FxMemo {
        FxMemo::new()
    }

    /// What `memo` answers for `text`.
    #[must_use]
    pub fn symbol(memo: &FxMemo, text: &str) -> Option<FxSymbol> {
        memo.symbol(text)
    }

    /// How many symbols `memo` parsed rather than answered from its table.
    #[must_use]
    pub fn parses(memo: &FxMemo) -> u64 {
        memo.parses.load(Ordering::Relaxed)
    }

    /// How many symbols `memo` remembers.
    #[must_use]
    pub fn remembered(memo: &FxMemo) -> usize {
        memo.symbols
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

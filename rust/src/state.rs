//! What state one thing is in, ranked so the bytes sort by lifecycle.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::folded_spelling;
use crate::code::{CodeValue, code_leaf, code_value};
use crate::typed::define_field_types;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(State, STATE_WIDTH);

impl State {
    /// The state stated as none: `00UNKNOWN`, rank zero, which every other
    /// state is further along than.
    #[must_use]
    pub fn unknown() -> Self {
        Self(SmolStr::new_static("00UNKNOWN"))
    }

    /// The better of two states: this one, unless it reached none - rank
    /// `00` - or the other reached further.
    fn merged(self, other: &Self) -> Self {
        match (self.rank(), other.rank()) {
            (None | Some(0), _) => other.clone(),
            (Some(this), Some(that)) if that > this => other.clone(),
            _ => self,
        }
    }

    /// The rank a stored state opens with, first to terminal.
    ///
    /// The two leading digits read as the number they spell, `0` to `99`, or
    /// `None` where the value does not open with two digits. A sort of the
    /// raw column is already in this order, because the digits lead and are
    /// fixed at two.
    #[must_use]
    pub fn rank(&self) -> Option<u8> {
        match self.as_str().as_bytes() {
            [tens @ b'0'..=b'9', ones @ b'0'..=b'9', ..] => {
                Some((tens - b'0') * 10 + (ones - b'0'))
            }
            _ => None,
        }
    }

    /// Whether this state can still change.
    ///
    /// Every rank below `80`, the first terminal band. A reader asking "is
    /// this still going" asks this rather than listing names.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.rank().is_some_and(|rank| rank < 80)
    }

    /// Whether this state ended having done what was asked: rank `80`-`89`.
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self.rank(), Some(80..=89))
    }

    /// Whether this state ended because someone stopped it: rank `90`-`94`.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        matches!(self.rank(), Some(90..=94))
    }

    /// Whether this state ended because it could not be done: rank `95`-`99`.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self.rank(), Some(95..=99))
    }

    /// The state one spelling names, refused where none does.
    ///
    /// [`Self::from_spelling`] as the value contract reads it: a column typed
    /// `state` holds ranked values only, so text that names no state leaves
    /// the column null rather than storing a value nothing can rank.
    ///
    /// # Errors
    ///
    /// Returns an error naming the spelling.
    pub fn read(spelling: &str) -> Result<Self> {
        Self::from_spelling(spelling).ok_or_else(|| crate::Error::InvalidDataType {
            kind: "state",
            reason: smol_str::format_smolstr!(
                "expected a state code, name or stored value, got {spelling:?}"
            ),
        })
    }

    /// The state one spelling names, or `None` where none does.
    ///
    /// Four vocabularies reach one value, because they name one thing:
    ///
    /// - a FIX `OrdStatus(39)` or `ExecType(150)` wire code - `0`, `1`, `F`;
    /// - the specification's own name for it - `PartiallyFilled`, `DoneForDay`;
    /// - the word a scheduler uses - `running`, `succeeded`, `timed out`;
    /// - the short name a FIX bridge logs - `PartFill`, `PendNew`, `DoneDay`.
    ///
    /// Names fold the way every other name in this crate folds: ASCII case
    /// insensitive, with `_`, `-` and spaces ignored, so `DoneForDay`,
    /// `done_for_day` and `DONE FOR DAY` are one spelling. A wire code does
    /// **not** fold, because `A` and `a` are different codes in FIX and a
    /// folded lookup would answer the wrong state for one of them.
    ///
    /// ```
    /// use yggdryl::State;
    ///
    /// // The wire code, the specification's name and the scheduler's word.
    /// assert_eq!(State::from_spelling("1").unwrap().as_str(), "40PARTFILL");
    /// assert_eq!(State::from_spelling("PartiallyFilled").unwrap().as_str(), "40PARTFILL");
    /// assert_eq!(State::from_spelling("running").unwrap().as_str(), "30RUNNING");
    ///
    /// // The stored bytes sort from the first state to the terminal ones,
    /// // which is the whole reason the rank leads.
    /// let mut held = ["80FILLED", "20NEW", "95REJECTED", "40PARTFILL"];
    /// held.sort_unstable();
    /// assert_eq!(held, ["20NEW", "40PARTFILL", "80FILLED", "95REJECTED"]);
    ///
    /// // And the three endings are told apart without reading the name.
    /// assert!(State::from_spelling("New").unwrap().is_live());
    /// assert!(State::from_spelling("Filled").unwrap().is_done());
    /// assert!(State::from_spelling("Rejected").unwrap().is_failed());
    /// ```
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        // A stored value names itself, which is what makes reading one back
        // free and the whole mapping idempotent.
        if crate::StringEnum::STATES.contains(&spelling) {
            return Self::new(spelling).ok();
        }
        if let Some(held) = STATE_CODES
            .iter()
            .find(|(code, _)| *code == spelling)
            .map(|(_, state)| *state)
        {
            return Self::new(held).ok();
        }
        let folded = folded_spelling(spelling);
        STATE_NAMES
            .iter()
            .find(|(name, _)| *name == folded.as_str())
            .map(|(_, state)| *state)
            .and_then(|held| Self::new(held).ok())
    }
}

/// FIX's `OrdStatus(39)` and `ExecType(150)` wire codes, unfolded.
///
/// The two code sets agree on every value they share, which is why one table
/// answers both: `0` is New in each, `1` PartiallyFilled, `2` Filled. Where
/// only `ExecType` defines a value - `F` Trade, `L` Triggered - the state is
/// what that report says the order is doing.
static STATE_CODES: &[(&str, &str)] = &[
    ("0", "20NEW"),
    ("1", "40PARTFILL"),
    ("2", "80FILLED"),
    ("3", "80DONEDAY"),
    ("4", "90CANCELED"),
    ("5", "70REPLACED"),
    ("6", "60PENDCXL"),
    ("7", "50STOPPED"),
    ("8", "95REJECTED"),
    ("9", "50SUSPEND"),
    ("A", "10PENDNEW"),
    ("B", "80CALCULAT"),
    ("C", "95EXPIRED"),
    ("D", "20ACCEPTED"),
    ("E", "60PENDRPL"),
    ("F", "40TRADE"),
    ("G", "40TRDCORR"),
    ("H", "40TRDCXL"),
    ("I", "30STATUS"),
    ("J", "40TRDHOLD"),
    ("K", "80TRDRELS"),
    ("L", "30TRIGGER"),
];

/// Every name that reaches a state, folded: FIX's, a scheduler's, and the
/// short names a FIX bridge logs - `PartFill`, `PendNew`, `DoneDay`,
/// `Cancel`, `Reject`.
static STATE_NAMES: &[(&str, &str)] = &[
    ("accepted", "20ACCEPTED"),
    ("acceptedforbidding", "20ACCEPTED"),
    ("calculated", "80CALCULAT"),
    ("cancel", "90CANCELED"),
    ("canceled", "90CANCELED"),
    ("cancelled", "90CANCELED"),
    ("complete", "80COMPLETE"),
    ("completed", "80COMPLETE"),
    ("doneday", "80DONEDAY"),
    ("doneforday", "80DONEDAY"),
    ("expired", "95EXPIRED"),
    ("failed", "95FAILED"),
    ("failure", "95FAILED"),
    ("filled", "80FILLED"),
    ("inprogress", "40INPROGR"),
    ("new", "20NEW"),
    ("orderstatus", "30STATUS"),
    ("partfill", "40PARTFILL"),
    ("partfilled", "40PARTFILL"),
    ("partiallyfilled", "40PARTFILL"),
    ("paused", "50PAUSED"),
    ("pendcancel", "60PENDCXL"),
    ("pending", "10PENDING"),
    ("pendingcancel", "60PENDCXL"),
    ("pendingnew", "10PENDNEW"),
    ("pendingreplace", "60PENDRPL"),
    ("pendnew", "10PENDNEW"),
    ("pendreplace", "60PENDRPL"),
    ("queued", "10QUEUED"),
    ("reject", "95REJECTED"),
    ("rejected", "95REJECTED"),
    ("replaced", "70REPLACED"),
    ("restated", "70RESTATED"),
    ("running", "30RUNNING"),
    ("starting", "20STARTING"),
    ("stopped", "50STOPPED"),
    ("submitted", "20SUBMITTD"),
    ("succeeded", "80SUCCESS"),
    ("success", "80SUCCESS"),
    ("suspended", "50SUSPEND"),
    ("timedout", "95TIMEOUT"),
    ("timeout", "95TIMEOUT"),
    ("trade", "40TRADE"),
    ("tradecancel", "40TRDCXL"),
    ("tradecorrect", "40TRDCORR"),
    ("tradehasbeenreleasedtoclearing", "80TRDRELS"),
    ("tradeinaclearinghold", "40TRDHOLD"),
    ("triggeredoractivatedbysystem", "30TRIGGER"),
    ("unknown", "00UNKNOWN"),
];

code_value!(State, State, STATE_WIDTH, merge = State::merged);

/// The Arrow extension name of a thing's state.
pub(crate) const STATE_EXTENSION_NAME: &str = "yggdryl.state";

/// The most bytes a thing's state may be.
///
/// Two decimal digits of rank and up to eight name bytes. Ten because the
/// rank has to leave room for a name a person can read, and because widening
/// later would change a discriminant, which is a wire contract.
pub(crate) const STATE_WIDTH: usize = 10;

// /// A field declared as a thing's state.
define_field_types!(StateType, State);

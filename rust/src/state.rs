//! What state one thing is in: a lifecycle-sorted enum, stored as an `int32`.

use std::fmt;
use std::sync::LazyLock;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use crate::code::folded_spelling;
use crate::typed::define_field_types;
use crate::{DataType, Result, Scalar, Value};

/// Declares the members once: the variant, the code, the stored name and what
/// the state means.
macro_rules! states {
    ($($(#[$meta:meta])* $variant:ident = $code:literal as $name:literal: $doc:literal,)+) => {
        /// What state one thing is in, from asked for to ended.
        ///
        /// One vocabulary over two worlds. FIX names an order's state twice -
        /// `OrdStatus` says where the order stands and `ExecType` says what
        /// the report is - its post-trade messages name a report's, an
        /// allocation's and a confirmation's states again, and a scheduler
        /// names a job's state in ordinary English. They are the same shape:
        /// a thing is asked for, acknowledged, it works, and it ends one of
        /// three ways. A capture and the pipeline that reads it should not
        /// need several vocabularies and a join to answer "what happened".
        ///
        /// # The code is the rank
        ///
        /// A state is stored as the `int32` code of its member, and the
        /// hundreds of the code are its rank, so the stored integers sort
        /// from the first state to the terminal ones in every format the
        /// column crosses - a Parquet row group's min and max, an Iceberg
        /// predicate, an `ORDER BY` in whatever reads the file. The units are
        /// the member's place inside its rank: a state added to a rank takes
        /// the next free number of it, so nothing already stored moves.
        ///
        /// | rank | codes | meaning |
        /// | --- | --- | --- |
        /// | `0` | `0` | stated, but not a state anything reached |
        /// | `10` | `1000`-`1099` | asked for, not yet acknowledged |
        /// | `20` | `2000`-`2099` | acknowledged, not yet working |
        /// | `30` | `3000`-`3099` | working |
        /// | `40` | `4000`-`4099` | working, and something has happened |
        /// | `50` | `5000`-`5099` | halted, and able to resume |
        /// | `60` | `6000`-`6099` | a change is outstanding |
        /// | `70` | `7000`-`7099` | changed, and the new thing carries on |
        /// | `80` | `8000`-`8999` | ended, having done what was asked |
        /// | `90` | `9000`-`9499` | ended, because someone stopped it |
        /// | `95` | `9500`-`9999` | ended, because it could not be done |
        ///
        /// The three endings are ranked apart deliberately: "did it finish"
        /// and "did it work" are different questions, and a single terminal
        /// rank would answer neither without reading the name. Each ending
        /// owns a band - [`Self::is_done`], [`Self::is_cancelled`] and
        /// [`Self::is_failed`] read it - and the ranks between two shipped
        /// ones are the places a state that belongs between them takes.
        ///
        /// ```
        /// use yggdryl::State;
        ///
        /// // The wire code, the specification's name and the scheduler's word.
        /// assert_eq!(State::from_spelling("1"), Some(State::PartiallyFilled));
        /// assert_eq!(State::from_spelling("PartiallyFilled"), Some(State::PartiallyFilled));
        /// assert_eq!(State::from_spelling("running"), Some(State::Running));
        ///
        /// // The stored codes sort from the first state to the terminal ones.
        /// let mut held = [State::Filled, State::New, State::Rejected, State::PartiallyFilled];
        /// held.sort_unstable();
        /// assert_eq!(held.map(State::code), [2001, 4001, 8003, 9502]);
        /// assert_eq!(State::from_code(4001), Some(State::PartiallyFilled));
        /// assert_eq!(State::PartiallyFilled.as_str(), "PARTIALLY_FILLED");
        ///
        /// // And the three endings are told apart without reading the name.
        /// assert!(State::New.is_live());
        /// assert!(State::Filled.is_done());
        /// assert!(State::Rejected.is_failed());
        /// ```
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(i32)]
        #[non_exhaustive]
        pub enum State {
            $(#[doc = $doc] $(#[$meta])* $variant = $code,)+
        }

        impl State {
            /// Every member, in code order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// The stored name: upper case, words joined by `_`.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            /// What this state means, in a sentence.
            #[must_use]
            pub const fn description(self) -> &'static str {
                match self {
                    $(Self::$variant => $doc,)+
                }
            }

            /// The member one stored code names, or `None` where none does.
            #[must_use]
            pub const fn from_code(code: i32) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// The member one stored name names, exactly as [`Self::as_str`]
            /// spells it, or `None` where none does.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

states! {
    #[default]
    Unknown = 0 as "UNKNOWN": "Stated, but not a state anything reached; every other state is further along than it.",
    Pending = 1000 as "PENDING": "Asked for, with nothing more said.",
    PendingNew = 1001 as "PENDING_NEW": "A new order asked for and not yet acknowledged: FIX `PendingNew`, and what a `NewOrderSingle` asks for.",
    Queued = 1002 as "QUEUED": "Waiting its turn.",
    Received = 1003 as "RECEIVED": "Received, not yet processed: an execution, allocation, confirmation or affirmation the counterparty has and has not yet answered.",
    PendingVerification = 1004 as "PENDING_VERIFICATION": "A trade report waiting on its verification.",
    PendingAllocation = 1005 as "PENDING_ALLOCATION": "An allocation asked for and not yet made.",
    PendingApproval = 1006 as "PENDING_APPROVAL": "A give-up or take-up waiting on its approval.",
    Accepted = 2000 as "ACCEPTED": "Accepted, not yet working: FIX `AcceptedForBidding`, an accepted trade report, quote or mass action.",
    New = 2001 as "NEW": "Acknowledged by the venue as a new order.",
    Starting = 2002 as "STARTING": "About to start.",
    Submitted = 2003 as "SUBMITTED": "Handed over to whoever does it.",
    Acknowledged = 2004 as "ACKNOWLEDGED": "Acknowledged by the counterparty: an execution it accepted.",
    Running = 3000 as "RUNNING": "Working.",
    Status = 3001 as "STATUS": "A status report: working, with nothing new to say.",
    Triggered = 3002 as "TRIGGERED": "Triggered or activated by the system.",
    Active = 3003 as "ACTIVE": "A quote standing in the market.",
    InProgress = 4000 as "IN_PROGRESS": "Working, with some of it done.",
    PartiallyFilled = 4001 as "PARTIALLY_FILLED": "Some of the order filled.",
    Trade = 4002 as "TRADE": "A report of one trade.",
    TradeCorrect = 4003 as "TRADE_CORRECT": "A correction of a trade reported before.",
    TradeCancel = 4004 as "TRADE_CANCEL": "A cancellation of a trade reported before.",
    TradeInClearingHold = 4005 as "TRADE_IN_CLEARING_HOLD": "A trade held before clearing.",
    Paused = 5000 as "PAUSED": "Halted by its owner.",
    Stopped = 5001 as "STOPPED": "Stopped, and able to resume.",
    Suspended = 5002 as "SUSPENDED": "Suspended by the venue.",
    Locked = 5003 as "LOCKED": "Locked by the venue: FIX `ExecType` `Locked`.",
    Disputed = 5004 as "DISPUTED": "A trade report its counterparty disputes.",
    Incomplete = 5005 as "INCOMPLETE": "An allocation missing some of what it needs.",
    PendingCancel = 6000 as "PENDING_CANCEL": "A cancel asked for and not yet answered.",
    PendingReplace = 6001 as "PENDING_REPLACE": "A replace asked for and not yet answered.",
    PendingReversal = 6002 as "PENDING_REVERSAL": "A reversal asked for and not yet made.",
    Replaced = 7000 as "REPLACED": "Replaced, and the replacement carries on.",
    Restated = 7001 as "RESTATED": "Restated by the venue, and carrying on.",
    Amended = 7002 as "AMENDED": "Amended, and carrying on.",
    Released = 7003 as "RELEASED": "Released from a lock: FIX `ExecType` `Released`.",
    Calculated = 8000 as "CALCULATED": "Its value calculated.",
    Complete = 8001 as "COMPLETE": "Complete.",
    DoneForDay = 8002 as "DONE_FOR_DAY": "Done for the day.",
    Filled = 8003 as "FILLED": "All of the order filled.",
    Succeeded = 8004 as "SUCCEEDED": "Succeeded.",
    TradeReleasedToClearing = 8005 as "TRADE_RELEASED_TO_CLEARING": "A trade released to clearing.",
    Allocated = 8006 as "ALLOCATED": "An allocation accepted and made.",
    Confirmed = 8007 as "CONFIRMED": "A trade confirmed.",
    Affirmed = 8008 as "AFFIRMED": "A confirmation affirmed.",
    Verified = 8009 as "VERIFIED": "A trade report verified, or deemed verified.",
    Cleared = 8010 as "CLEARED": "Cleared.",
    Settled = 8011 as "SETTLED": "Settled.",
    Claimed = 8012 as "CLAIMED": "An allocation claimed.",
    Canceled = 9000 as "CANCELED": "Cancelled by someone.",
    Reversed = 9001 as "REVERSED": "An allocation reversed.",
    Removed = 9002 as "REMOVED": "A quote removed from the market.",
    Terminated = 9003 as "TERMINATED": "A trade report terminated, or a contract terminated under a quote.",
    Expired = 9500 as "EXPIRED": "Its time ran out.",
    Failed = 9501 as "FAILED": "Failed.",
    Rejected = 9502 as "REJECTED": "Refused.",
    TimedOut = 9503 as "TIMED_OUT": "Took too long.",
    DontKnow = 9504 as "DONT_KNOW": "An execution the counterparty does not know: FIX's DK.",
    Mismatched = 9505 as "MISMATCHED": "A confirmation whose account or settlement instructions do not match.",
    NotFound = 9506 as "NOT_FOUND": "A quote the venue does not know.",
}

impl State {
    /// The state stated as none: [`Self::Unknown`], rank zero, which every
    /// other state is further along than.
    #[must_use]
    pub const fn unknown() -> Self {
        Self::Unknown
    }

    /// The `int32` a column stores for this state.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }

    /// The rank this state stands at, first to terminal: its code's hundreds,
    /// `0` to `99`.
    #[must_use]
    pub const fn rank(self) -> u8 {
        (self.code() / 100) as u8
    }

    /// The better of two states: this one, unless it reached none or the
    /// other reached a further rank.
    ///
    /// What a lifecycle folds along a chain: the furthest state its chain
    /// knows.
    #[must_use]
    pub const fn merge_with(self, other: Self) -> Self {
        if matches!(self, Self::Unknown) || other.rank() > self.rank() {
            other
        } else {
            self
        }
    }

    /// Whether this state was asked for and not yet acknowledged: rank `10`.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        matches!(self.rank(), 10..=19)
    }

    /// Whether this state can still change: every rank below `80`, the first
    /// terminal band. A reader asking "is this still going" asks this rather
    /// than listing names.
    #[must_use]
    pub const fn is_live(self) -> bool {
        self.rank() < 80
    }

    /// Whether this state ended having done what was asked: rank `80`-`89`.
    #[must_use]
    pub const fn is_done(self) -> bool {
        matches!(self.rank(), 80..=89)
    }

    /// Whether this state ended because someone stopped it: rank `90`-`94`.
    #[must_use]
    pub const fn is_cancelled(self) -> bool {
        matches!(self.rank(), 90..=94)
    }

    /// Whether this state ended because it could not be done: rank `95`-`99`.
    #[must_use]
    pub const fn is_failed(self) -> bool {
        matches!(self.rank(), 95..=99)
    }

    /// Whether this state itself reports an execution.
    ///
    /// Exact states rather than a rank band: rank `40` also holds generic
    /// in-progress work, and rank `80` other successful endings. A trade
    /// correction, cancellation or clearing transition refers to an earlier
    /// execution and must carry that execution's instant rather than invent
    /// one from the later report.
    #[must_use]
    pub const fn is_execution(self) -> bool {
        matches!(self, Self::PartiallyFilled | Self::Trade | Self::Filled)
    }

    /// The state one spelling names, refused where none does.
    ///
    /// [`Self::from_spelling`] as the value contract reads it: a column typed
    /// `state` holds members only, so text that names no state leaves the
    /// column null rather than storing a value nothing can rank.
    ///
    /// # Errors
    ///
    /// Returns an error naming the spelling.
    pub fn read(spelling: &str) -> Result<Self> {
        Self::from_spelling(spelling).ok_or_else(|| crate::Error::InvalidDataType {
            kind: "state",
            reason: format_smolstr!("expected a state code, name or stored name, got {spelling:?}"),
        })
    }

    /// The member one stored code names, refused where none does.
    ///
    /// # Errors
    ///
    /// Returns an error naming the code.
    pub fn read_code(code: i64) -> Result<Self> {
        i32::try_from(code)
            .ok()
            .and_then(Self::from_code)
            .ok_or_else(|| crate::Error::InvalidDataType {
                kind: "state",
                reason: format_smolstr!("expected the code of a state, got {code}"),
            })
    }

    /// The state one spelling names, or `None` where none does.
    ///
    /// Five vocabularies reach one value, because they name one thing:
    ///
    /// - the stored name - `PARTIALLY_FILLED`, `DONE_FOR_DAY`;
    /// - a FIX `OrdStatus(39)` or `ExecType(150)` wire code - `0`, `1`, `F`;
    /// - the specification's own name for it - `PartiallyFilled`, `DoneForDay`;
    /// - the word a scheduler uses - `running`, `succeeded`, `timed out`;
    /// - the short name a FIX bridge logs - `PartFill`, `PendNew`, `DoneDay`.
    ///
    /// Names fold the way every other name in this crate folds: ASCII case
    /// insensitive, with `_`, `-` and spaces ignored, so `DoneForDay`,
    /// `done_for_day` and `DONE FOR DAY` are one spelling. A wire code does
    /// **not** fold, because `A` and `a` are different codes in FIX and a
    /// folded lookup would answer the wrong state for one of them. A stored
    /// code is an integer, never text: `State::from_code` reads it.
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some(held) = Self::from_name(spelling) {
            return Some(held);
        }
        if let Some(held) = STATE_CODES
            .iter()
            .find(|(code, _)| *code == spelling)
            .map(|(_, state)| *state)
        {
            return Some(held);
        }
        let folded = folded_spelling(spelling);
        FOLDED_NAMES
            .iter()
            .map(|(name, state)| (name.as_str(), *state))
            .chain(STATE_NAMES.iter().copied())
            .find(|(name, _)| *name == folded.as_str())
            .map(|(_, state)| state)
    }

    /// The state one FIX status field's code names, or `None` where the tag
    /// is no status this vocabulary reads or its code says nothing about one.
    ///
    /// Every status field a FIX message answers a request by, each under its
    /// own code set:
    ///
    /// | tag | field |
    /// | --- | --- |
    /// | 39 | `OrdStatus` |
    /// | 150 | `ExecType` |
    /// | 1036 | `ExecAckStatus` |
    /// | 939 | `TrdRptStatus` |
    /// | 297 | `QuoteStatus` |
    /// | 87 | `AllocStatus` |
    /// | 665 | `ConfirmStatus` |
    /// | 940 | `AffirmStatus` |
    /// | 1375 | `MassActionResponse` |
    /// | 531 | `MassCancelResponse` |
    ///
    /// A warning a quote status gives - a locked or crossed market - states
    /// no state, and neither does a code a set does not define.
    ///
    /// ```
    /// use yggdryl::State;
    ///
    /// assert_eq!(State::from_fix_status(1036, "1"), Some(State::Acknowledged));
    /// assert_eq!(State::from_fix_status(1036, "2"), Some(State::DontKnow));
    /// assert_eq!(State::from_fix_status(87, "0"), Some(State::Allocated));
    /// assert_eq!(State::from_fix_status(297, "12"), None);
    /// ```
    #[must_use]
    pub fn from_fix_status(tag: i32, code: &str) -> Option<Self> {
        let code = code.trim();
        let table: &[(&str, Self)] = match tag {
            39 | 150 => STATE_CODES,
            1036 => EXEC_ACK_STATUS,
            939 => TRD_RPT_STATUS,
            297 => QUOTE_STATUS,
            87 => ALLOC_STATUS,
            665 => CONFIRM_STATUS,
            940 => AFFIRM_STATUS,
            1375 => MASS_ACTION_RESPONSE,
            531 => {
                // Every response but a rejection says which orders it
                // cancelled.
                return match code {
                    "0" => Some(Self::Rejected),
                    "" => None,
                    _ => Some(Self::Canceled),
                };
            }
            _ => return None,
        };
        table
            .iter()
            .find(|(held, _)| *held == code)
            .map(|(_, state)| *state)
    }

    /// The tags [`Self::from_fix_status`] reads, in the order a message's
    /// state is read off them: the first one stated wins.
    pub const FIX_STATUS_TAGS: [i32; 10] = [39, 150, 1036, 939, 297, 87, 665, 940, 1375, 531];

    /// The state a FIX message asks for by being the message it is, where no
    /// status field states one: a new order asks for a new order, a cancel
    /// request for a cancel, a reject refuses.
    ///
    /// ```
    /// use yggdryl::State;
    ///
    /// assert_eq!(State::from_fix_msgtype("D"), Some(State::PendingNew));
    /// assert_eq!(State::from_fix_msgtype("F"), Some(State::PendingCancel));
    /// assert_eq!(State::from_fix_msgtype("j"), Some(State::Rejected));
    /// assert_eq!(State::from_fix_msgtype("8"), None);
    /// ```
    #[must_use]
    pub fn from_fix_msgtype(msgtype: &str) -> Option<Self> {
        Some(match msgtype {
            // New orders: single, list, cross, multileg.
            "D" | "E" | "s" | "AB" => Self::PendingNew,
            // A cancel and a cancel-replace request, single and cross.
            "F" | "u" => Self::PendingCancel,
            "G" | "t" | "AC" => Self::PendingReplace,
            // A request for a quote, and a quote answering one.
            "R" => Self::Pending,
            "S" => Self::Active,
            // A session or business level reject.
            "3" | "j" => Self::Rejected,
            _ => return None,
        })
    }
}

/// FIX's `OrdStatus(39)` and `ExecType(150)` wire codes, unfolded.
///
/// The two code sets agree on every value they share, which is why one table
/// answers both: `0` is New in each, `1` PartiallyFilled, `2` Filled. Where
/// only `ExecType` defines a value - `F` Trade, `L` Triggered, `M` Locked -
/// the state is what that report says the order is doing.
static STATE_CODES: &[(&str, State)] = &[
    ("0", State::New),
    ("1", State::PartiallyFilled),
    ("2", State::Filled),
    ("3", State::DoneForDay),
    ("4", State::Canceled),
    ("5", State::Replaced),
    ("6", State::PendingCancel),
    ("7", State::Stopped),
    ("8", State::Rejected),
    ("9", State::Suspended),
    ("A", State::PendingNew),
    ("B", State::Calculated),
    ("C", State::Expired),
    ("D", State::Accepted),
    ("E", State::PendingReplace),
    ("F", State::Trade),
    ("G", State::TradeCorrect),
    ("H", State::TradeCancel),
    ("I", State::Status),
    ("J", State::TradeInClearingHold),
    ("K", State::TradeReleasedToClearing),
    ("L", State::Triggered),
    ("M", State::Locked),
    ("N", State::Released),
];

/// FIX's `ExecAckStatus(1036)`: an execution received, accepted, or not known.
static EXEC_ACK_STATUS: &[(&str, State)] = &[
    ("0", State::Received),
    ("1", State::Acknowledged),
    ("2", State::DontKnow),
];

/// FIX's `TrdRptStatus(939)`: where a trade report stands.
static TRD_RPT_STATUS: &[(&str, State)] = &[
    ("0", State::Accepted),
    ("1", State::Rejected),
    ("2", State::Canceled),
    ("3", State::Accepted),
    ("4", State::PendingNew),
    ("5", State::PendingCancel),
    ("6", State::PendingReplace),
    ("7", State::Terminated),
    ("8", State::PendingVerification),
    ("9", State::Verified),
    ("10", State::Verified),
    ("11", State::Disputed),
];

/// FIX's `QuoteStatus(297)`: where a quote stands. The two market warnings,
/// `12` and `13`, and the end-trade codes `19` and `20` state no state.
static QUOTE_STATUS: &[(&str, State)] = &[
    ("0", State::Accepted),
    ("1", State::Canceled),
    ("2", State::Canceled),
    ("3", State::Canceled),
    ("4", State::Canceled),
    ("5", State::Rejected),
    ("6", State::Removed),
    ("7", State::Expired),
    ("8", State::Status),
    ("9", State::NotFound),
    ("10", State::Pending),
    ("11", State::Rejected),
    ("14", State::Canceled),
    ("15", State::Canceled),
    ("16", State::Active),
    ("17", State::Canceled),
    ("18", State::Active),
    ("21", State::Trade),
    ("22", State::Filled),
    ("23", State::Terminated),
];

/// FIX's `AllocStatus(87)`: where an allocation stands.
static ALLOC_STATUS: &[(&str, State)] = &[
    ("0", State::Allocated),
    ("1", State::Rejected),
    ("2", State::Rejected),
    ("3", State::Received),
    ("4", State::Incomplete),
    ("5", State::Rejected),
    ("6", State::PendingAllocation),
    ("7", State::Reversed),
    ("8", State::Canceled),
    ("9", State::Claimed),
    ("10", State::Rejected),
    ("11", State::PendingApproval),
    ("12", State::Canceled),
    ("13", State::PendingApproval),
    ("14", State::PendingReversal),
];

/// FIX's `ConfirmStatus(665)`: where a confirmation stands.
static CONFIRM_STATUS: &[(&str, State)] = &[
    ("1", State::Received),
    ("2", State::Mismatched),
    ("3", State::Mismatched),
    ("4", State::Confirmed),
    ("5", State::Rejected),
];

/// FIX's `AffirmStatus(940)`: where an affirmation stands.
static AFFIRM_STATUS: &[(&str, State)] = &[
    ("1", State::Received),
    ("2", State::Rejected),
    ("3", State::Affirmed),
];

/// FIX's `MassActionResponse(1375)`.
static MASS_ACTION_RESPONSE: &[(&str, State)] = &[
    ("0", State::Rejected),
    ("1", State::Accepted),
    ("2", State::Complete),
];

/// Every stored name, folded: `PARTIALLY_FILLED` as `partiallyfilled`,
/// which is also the specification's name for most of them.
static FOLDED_NAMES: LazyLock<Vec<(SmolStr, State)>> = LazyLock::new(|| {
    State::ALL
        .iter()
        .map(|state| (folded_spelling(state.as_str()), *state))
        .collect()
});

/// Every name that reaches a state beside its stored one, folded: FIX's, a
/// scheduler's, and the short names a FIX bridge logs - `PartFill`,
/// `PendNew`, `DoneDay`, `Cancel`, `Reject`.
static STATE_NAMES: &[(&str, State)] = &[
    ("ack", State::Acknowledged),
    ("acked", State::Acknowledged),
    ("acceptedforbidding", State::Accepted),
    ("acceptedwitherrors", State::Accepted),
    ("allocationpending", State::PendingAllocation),
    ("amend", State::Amended),
    ("cancel", State::Canceled),
    ("cancelled", State::Canceled),
    ("completed", State::Complete),
    ("deemedverified", State::Verified),
    ("dk", State::DontKnow),
    ("doneday", State::DoneForDay),
    ("failure", State::Failed),
    ("orderstatus", State::Status),
    ("partfill", State::PartiallyFilled),
    ("partfilled", State::PartiallyFilled),
    ("pendcancel", State::PendingCancel),
    ("pendinggiveupapproval", State::PendingApproval),
    ("pendingtakeupapproval", State::PendingApproval),
    ("pendnew", State::PendingNew),
    ("pendreplace", State::PendingReplace),
    ("quotenotfound", State::NotFound),
    ("reject", State::Rejected),
    ("removedfrommarket", State::Removed),
    ("reversalpending", State::PendingReversal),
    ("success", State::Succeeded),
    ("timeout", State::TimedOut),
    (
        "tradehasbeenreleasedtoclearing",
        State::TradeReleasedToClearing,
    ),
    ("tradeinaclearinghold", State::TradeInClearingHold),
    ("triggeredoractivatedbysystem", State::Triggered),
];

impl fmt::Display for State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for State {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for State {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Held {
            Code(i64),
            Spelling(String),
        }
        match Held::deserialize(deserializer)? {
            Held::Code(code) => Self::read_code(code),
            Held::Spelling(spelling) => Self::read(&spelling),
        }
        .map_err(D::Error::custom)
    }
}

impl Value for State {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::State)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::State(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::State(value) => Some(value),
            _ => None,
        }
    }
}

impl From<State> for Scalar {
    fn from(value: State) -> Self {
        Self::State(value)
    }
}

impl From<State> for i32 {
    fn from(value: State) -> Self {
        value.code()
    }
}

impl TryFrom<i32> for State {
    type Error = crate::Error;

    fn try_from(code: i32) -> Result<Self> {
        Self::read_code(i64::from(code))
    }
}

/// Arrow casts owned by this datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, NullBuffer};
    use arrow_schema::DataType as ArrowDataType;

    use super::State;
    use crate::arrow::Result;
    use crate::budget::MaterializationBudget;
    use crate::cast::columns::is_exposed;
    use crate::cast::{arrow_cast_exposed, downcast, named_cell};
    use crate::{DataType, Field};

    /// Validates every exposed, non-null value entering a state and stores
    /// the code of its member.
    ///
    /// An integer source is read as the codes it holds; anything else renders
    /// as Utf8 through Arrow's kernel and is read as the spelling it is. A
    /// failing cell is null under `safe` and an error naming the row
    /// otherwise; the target's own null policy runs after the reading.
    pub(crate) fn ingest_state_array(
        array: &ArrayRef,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        if array.data_type().is_integer() {
            let codes = arrow_cast_exposed(
                array,
                &ArrowDataType::Int64,
                safe,
                exposure,
                &Field::new(field.name(), DataType::Int64, true),
                budget,
            )?;
            let codes = downcast::<Int64Array>(codes.as_ref())?;
            return state_storage(field, codes.len(), safe, exposure, budget, |index| {
                codes
                    .is_valid(index)
                    .then(|| State::read_code(codes.value(index)))
            });
        }
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                safe,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let cells = downcast::<StringArray>(text.as_ref())?;
        state_storage(field, cells.len(), safe, exposure, budget, |index| {
            cells
                .is_valid(index)
                .then(|| State::read(cells.value(index)))
        })
    }

    /// Builds the `int32` codes of a state column from one reading per row.
    fn state_storage(
        field: &Field,
        rows: usize,
        safe: bool,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        cell: impl Fn(usize) -> Option<crate::Result<State>>,
    ) -> Result<ArrayRef> {
        budget.add_array(field.dtype(), rows)?;
        let mut codes = vec![0_i32; rows];
        let mut validity = BooleanBufferBuilder::new(rows);
        for (index, code) in codes.iter_mut().enumerate() {
            let state = match is_exposed(exposure, index).then(|| cell(index)).flatten() {
                Some(Ok(state)) => Some(state),
                Some(Err(_)) if safe => None,
                Some(read @ Err(_)) => Some(named_cell(field, index, read)?),
                None => None,
            };
            if let Some(state) = state {
                *code = state.code();
            }
            validity.append(state.is_some());
        }
        Ok(Arc::new(Int32Array::new(
            codes.into(),
            Some(NullBuffer::new(validity.finish())),
        )))
    }
}

/// The Arrow extension name of a thing's state, over `int32` storage.
pub(crate) const STATE_EXTENSION_NAME: &str = "yggdryl.state";

// /// A field declared as a thing's state.
define_field_types!(StateType, State);

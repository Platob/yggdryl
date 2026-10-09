//! The state a FIX message states: each status field a message answers a
//! request by, read under its own code set, and the state a message asks for
//! by being the message it is - the FIX readings of [`State`] a message's
//! state is read through.

use crate::State;

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
/// `OrdStatus` and `ExecType` agree on every value they share, so one table
/// answers both: the one [`State::from_spelling`] reads a wire code by. A
/// warning a quote status gives - a locked or crossed market - states no
/// state, and neither does a code a set does not define.
///
/// ```
/// use yggdryl::State;
/// use yggdryl::fix::state;
///
/// assert_eq!(state::from_status(39, "1"), Some(State::PartiallyFilled));
/// assert_eq!(state::from_status(1036, "1"), Some(State::Acknowledged));
/// assert_eq!(state::from_status(1036, "2"), Some(State::DontKnow));
/// assert_eq!(state::from_status(87, "0"), Some(State::Allocated));
/// assert_eq!(state::from_status(297, "12"), None);
/// ```
#[must_use]
pub fn from_status(tag: i32, code: &str) -> Option<State> {
    let code = code.trim();
    let table: &[(&str, State)] = match tag {
        39 | 150 => return crate::implementer::state_from_wire_code(code),
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
                "0" => Some(State::Rejected),
                "" => None,
                _ => Some(State::Canceled),
            };
        }
        _ => return None,
    };
    table
        .iter()
        .find(|(held, _)| *held == code)
        .map(|(_, state)| *state)
}

/// The tags [`from_status`] reads, in the order a message's state is read
/// off them: the first one stated wins.
pub const STATUS_TAGS: [i32; 10] = [39, 150, 1036, 939, 297, 87, 665, 940, 1375, 531];

/// The state a FIX message asks for by being the message it is, where no
/// status field states one: a new order asks for a new order, a cancel
/// request for a cancel, a reject refuses.
///
/// ```
/// use yggdryl::State;
/// use yggdryl::fix::state;
///
/// assert_eq!(state::from_msgtype("D"), Some(State::PendingNew));
/// assert_eq!(state::from_msgtype("F"), Some(State::PendingCancel));
/// assert_eq!(state::from_msgtype("Z"), Some(State::PendingCancel));
/// assert_eq!(state::from_msgtype("j"), Some(State::Rejected));
/// assert_eq!(state::from_msgtype("8"), None);
/// ```
#[must_use]
pub fn from_msgtype(msgtype: &str) -> Option<State> {
    Some(match msgtype {
        // New orders: single, list, cross, multileg.
        "D" | "E" | "s" | "AB" => State::PendingNew,
        // A cancel request, single and cross, and a quote cancel.
        "F" | "u" | "Z" => State::PendingCancel,
        // A cancel-replace request, single, cross and multileg.
        "G" | "t" | "AC" => State::PendingReplace,
        // A request for a quote, and a quote answering one.
        "R" => State::Pending,
        "S" => State::Active,
        // A session or business level reject.
        "3" | "j" => State::Rejected,
        _ => return None,
    })
}

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

//! `rust/fix/src/state.rs`: the FIX readings of [`State`] - each status field
//! a message answers a request by, read under its own code set, and the state
//! a message asks for by being the message it is.
//!
//! The wire codes `OrdStatus(39)` and `ExecType(150)` share are the ones
//! [`State::from_spelling`] reads a spelling by, so that table stays with the
//! state (`rust/tests/root/state.rs` pins it) and the other eight code sets
//! live here, beside the free functions that read them.

use yggdryl::State;
use yggdryl_fix::state::{STATUS_TAGS, from_msgtype, from_status};

#[test]
fn every_fix_status_field_answers_by_its_own_code_set() {
    crate::install::installed();
    for (tag, code, expected) in [
        (39, "0", Some(State::New)),
        (39, "A", Some(State::PendingNew)),
        (150, "F", Some(State::Trade)),
        (150, "M", Some(State::Locked)),
        (1036, "0", Some(State::Received)),
        (1036, "1", Some(State::Acknowledged)),
        (1036, "2", Some(State::DontKnow)),
        (939, "0", Some(State::Accepted)),
        (939, "8", Some(State::PendingVerification)),
        (939, "10", Some(State::Verified)),
        (939, "11", Some(State::Disputed)),
        (297, "16", Some(State::Active)),
        (297, "6", Some(State::Removed)),
        (297, "9", Some(State::NotFound)),
        // A market warning states no state.
        (297, "12", None),
        (297, "13", None),
        (87, "0", Some(State::Allocated)),
        (87, "6", Some(State::PendingAllocation)),
        (87, "7", Some(State::Reversed)),
        (87, "11", Some(State::PendingApproval)),
        (87, "13", Some(State::PendingApproval)),
        (87, "14", Some(State::PendingReversal)),
        (665, "2", Some(State::Mismatched)),
        (665, "4", Some(State::Confirmed)),
        (940, "3", Some(State::Affirmed)),
        (1375, "2", Some(State::Complete)),
        (531, "0", Some(State::Rejected)),
        (531, "7", Some(State::Canceled)),
        (531, "C", Some(State::Canceled)),
        // A code a set does not define, and a tag no status is read off.
        (1036, "9", None),
        (54, "1", None),
    ] {
        assert_eq!(from_status(tag, code), expected, "{tag}={code}");
    }
    assert_eq!(STATUS_TAGS[0], 39, "OrdStatus is read first");
}

/// The list names the tags `from_status` reads and no others: a tag in it
/// answers some code of its set, and a tag outside it answers none.
#[test]
fn the_status_tags_are_exactly_the_tags_a_status_is_read_off() {
    crate::install::installed();
    let candidates: Vec<String> = (0..=30)
        .map(|number| number.to_string())
        .chain(('A'..='Z').map(String::from))
        .collect();
    for tag in 0..=1500 {
        let reads = candidates
            .iter()
            .any(|code| from_status(tag, code).is_some());
        assert_eq!(
            reads,
            STATUS_TAGS.contains(&tag),
            "{tag} is read exactly when the list names it"
        );
    }
    let mut distinct = STATUS_TAGS.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), STATUS_TAGS.len(), "each tag once");
}

/// `OrdStatus` and `ExecType` agree on every value they share, so the one
/// table answers both and answers the wire code a state's spelling reads; a
/// code is trimmed before it is looked up, as every set's is.
#[test]
fn ordstatus_and_exectype_share_the_wire_codes_a_state_reads_by() {
    crate::install::installed();
    for code in (0..=9)
        .map(|number: u8| number.to_string())
        .chain(('A'..='N').map(String::from))
    {
        let ord = from_status(39, &code);
        assert_eq!(ord, from_status(150, &code), "39 and 150 on {code}");
        if let Some(state) = ord {
            assert_eq!(State::from_spelling(&code), Some(state), "{code}");
        }
    }
    assert_eq!(from_status(39, " 1 "), Some(State::PartiallyFilled));
    assert_eq!(from_status(150, "\tF"), Some(State::Trade));
    assert_eq!(from_status(1036, " 2"), Some(State::DontKnow));
    // The mass cancel response says which orders it cancelled unless it
    // rejects: a blank code states nothing.
    assert_eq!(from_status(531, ""), None);
    assert_eq!(from_status(531, "  "), None);
    assert_eq!(from_status(531, " 0 "), Some(State::Rejected));
}

/// A trade report awaiting its verification, an allocation awaiting its
/// making and a give-up awaiting its approval were each acknowledged first
/// (`rust/tests/root/state.rs` pins their ranks); FIX states an approved
/// give-up as `AllocStatus` accepted, so no status code answers it.
#[test]
fn no_status_code_states_an_approved_give_up() {
    crate::install::installed();
    for code in 0..=20 {
        let code = code.to_string();
        assert_ne!(from_status(87, &code), Some(State::Approved), "87={code}");
    }
}

#[test]
fn a_message_asks_for_the_state_its_type_names() {
    crate::install::installed();
    for (msgtype, expected) in [
        ("D", Some(State::PendingNew)),
        ("E", Some(State::PendingNew)),
        ("s", Some(State::PendingNew)),
        ("AB", Some(State::PendingNew)),
        ("F", Some(State::PendingCancel)),
        ("u", Some(State::PendingCancel)),
        ("G", Some(State::PendingReplace)),
        ("t", Some(State::PendingReplace)),
        ("AC", Some(State::PendingReplace)),
        ("R", Some(State::Pending)),
        ("S", Some(State::Active)),
        ("3", Some(State::Rejected)),
        ("j", Some(State::Rejected)),
        // An execution report states its state in a status field instead.
        ("8", None),
        ("0", None),
        ("", None),
    ] {
        assert_eq!(from_msgtype(msgtype), expected, "35={msgtype}");
    }
}

/// A `QuoteCancel(Z)` asks for its quote's cancel as an order cancel request
/// does: it moves the quote's chain to a pending cancel, which its
/// acknowledgement then ends.
#[test]
fn a_quote_cancel_asks_for_a_cancel() {
    crate::install::installed();
    assert_eq!(from_msgtype("Z"), Some(State::PendingCancel));
    assert_eq!(
        from_msgtype("Z"),
        from_msgtype("F"),
        "a quote cancel asks for what an order cancel request asks for"
    );
}

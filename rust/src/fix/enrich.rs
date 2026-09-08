//! The fields a message implies but did not carry.
//!
//! A venue sends what its counterparty needs and nothing more, so a row is
//! routinely missing values the message itself already determines: an order
//! stating `OrderQty` and `CumQty` has said what `LeavesQty` is, and a fill
//! stating `LastQty` and `LastPx` has said what it was worth. Every consumer
//! then derives those independently, which is how two systems come to
//! disagree about one message.
//!
//! The specification tabulates these rather than stating them as arithmetic:
//! FIX 4.4's Appendix D walks an order's whole life and shows what each
//! report carries at every step, and FIX 4.2's Appendix O does the same for
//! the fields a foreign exchange trade settles on. This module is those
//! tables read as the implications they are.
//!
//! # It is a table, not code per field
//!
//! A rule is a tag, the message types it speaks for, the conditions that must
//! hold, and how the value is derived. Adding one is a row, exactly as adding
//! a [lift](super::lift) is. There is no expression layer and no venue
//! condition: a rule the specification does not state is not a rule.
//!
//! # Enrichment never touches the entries
//!
//! The row is the interpretation and the entries are what arrived, so a
//! derived value goes to the row alone. Re-emitting an enriched message
//! therefore reproduces the received line byte for byte, which is the whole
//! reason the two facts are held apart. It also means enrichment is
//! idempotent: a stated value is never overwritten, so a value derived once
//! is a stated value the second time and derives to itself.
//!
//! # A rule answers only when the answer is certain
//!
//! Every input must be present and typed. A missing input, an untyped one, or
//! a condition that does not hold answers nothing rather than a guess, and a
//! rule that would contradict a stated value never runs at all. The cost of
//! silence is a null column; the cost of a guess is a wrong number nobody can
//! tell from a sent one.

use std::sync::Arc;

use crate::types::State;
use crate::types::ascii::AsciiFamily;
use crate::{DataType, Field, Result, Scalar};

use super::msg::FixMsg;
use super::registry::FixRegistry;

/// A condition one rule requires, read from the row.
struct FixWhen {
    /// The tag consulted.
    tag: i32,
    /// The values it must carry, as the row spells them. Empty means the tag
    /// need only be present.
    values: &'static [&'static str],
}

impl FixWhen {
    /// Whether the row satisfies this condition.
    fn holds(&self, msg: &FixMsg) -> bool {
        let Some(held) = msg.get_by_tag(self.tag) else {
            return false;
        };
        if held == &Scalar::Null {
            return false;
        }
        if self.values.is_empty() {
            return true;
        }
        // A condition is written in the codes the specification's own
        // matrices are written in. A state column holds the ranked value
        // rather than the code, so the code is read the way the column read
        // it before the two are compared; every other column holds the code.
        if let Scalar::Ascii(AsciiFamily::State(state)) = held {
            return self
                .values
                .iter()
                .any(|value| State::from_spelling(value).as_ref() == Some(state));
        }
        let Some(rendered) = held.as_str() else {
            return false;
        };
        self.values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(rendered))
    }
}

/// How one rule answers.
enum FixDerivation {
    /// `left - right`, refused below zero: a quantity cannot be negative, and
    /// a negative result means the two inputs were never about one order.
    Difference(i32, i32),
    /// `left + right`.
    Sum(i32, i32),
    /// `left * right`.
    Product(i32, i32),
    /// Whatever another tag carries, unchanged and re-typed for the column it
    /// lands in.
    Same(i32),
    /// The constant zero.
    Zero,
}

/// One field a message implies, and what implies it.
struct FixRule {
    /// The tag filled.
    tag: i32,
    /// The message types this rule speaks for; empty means all of them.
    msgtypes: &'static [&'static str],
    /// Every condition that must hold before the rule runs.
    when: &'static [FixWhen],
    /// How the value is derived.
    from: FixDerivation,
}

/// The message types that report an order's state.
const REPORTS: &[&str] = &["8", "9"];

/// The order statuses that leave quantity still working.
///
/// Appendix D's matrices: a `New`, `PartiallyFilled`, `PendingCancel`,
/// `PendingReplace`, `Replaced` or `Stopped` order still has quantity the
/// market can fill, so what is left is what was asked for minus what was
/// done. `Suspended` is here too - the order exists and is not working, but
/// its remainder is unchanged.
const WORKING: &[&str] = &["0", "1", "6", "E", "5", "7", "9"];

/// The order statuses that leave nothing working.
///
/// A `Filled`, `DoneForDay`, `Canceled`, `Rejected` or `Expired` order has no
/// remainder, whatever the arithmetic of the other two would say. Appendix D
/// shows `LeavesQty` at zero on every one of these.
const CLOSED: &[&str] = &["2", "3", "4", "8", "C"];

/// Every rule, in the order they are applied.
///
/// Order matters only where one rule's output is another's input, and the two
/// cases here are deliberate: `GrossTradeAmt` is derived before
/// `SettlCurrAmt`, which is derived from it, so a fill stating only a
/// quantity, a price and a rate answers both.
static RULES: &[FixRule] = &[
    // Appendix D, every matrix: what is left is what was ordered minus what
    // was done, and nothing is left once the order is closed.
    FixRule {
        tag: 151,
        msgtypes: REPORTS,
        when: &[FixWhen {
            tag: 39,
            values: CLOSED,
        }],
        from: FixDerivation::Zero,
    },
    FixRule {
        tag: 151,
        msgtypes: REPORTS,
        when: &[FixWhen {
            tag: 39,
            values: WORKING,
        }],
        from: FixDerivation::Difference(38, 14),
    },
    // The same identity read the other two ways. A report stating what is
    // left and what was done has stated what was ordered.
    FixRule {
        tag: 38,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Sum(14, 151),
    },
    FixRule {
        tag: 14,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Difference(38, 151),
    },
    // A fill's worth, which Appendix O settles on and Appendix D's execution
    // reports carry.
    FixRule {
        tag: 381,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Product(32, 31),
    },
    // Appendix O: the settled amount is the traded amount at the stated rate.
    FixRule {
        tag: 119,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Product(381, 155),
    },
    // Appendix O: a trade settling in the currency it was dealt in states the
    // dealt currency once. `SettlCurrency` absent means "the same one", which
    // is a default rather than an absence, and a reader joining two captures
    // on the settlement currency needs it stated.
    FixRule {
        tag: 120,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Same(15),
    },
    // The average of one fill is that fill's price. Stated only where the
    // report says the whole done quantity is this fill, because an average
    // over two fills is not derivable from one of them.
    FixRule {
        tag: 6,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Same(31),
    },
];

/// The value one derivation answers, or nothing when it cannot be certain.
fn derive(msg: &FixMsg, from: &FixDerivation) -> Option<Scalar> {
    let number = |tag: i32| -> Option<f64> {
        let held = msg.get_by_tag(tag)?;
        match held {
            Scalar::Null => None,
            other => other.as_f64(),
        }
    };
    match from {
        FixDerivation::Zero => Some(Scalar::from(0.0_f64)),
        FixDerivation::Same(tag) => match msg.get_by_tag(*tag)? {
            Scalar::Null => None,
            held => Some(held.clone()),
        },
        FixDerivation::Sum(left, right) => Some(Scalar::from(number(*left)? + number(*right)?)),
        FixDerivation::Product(left, right) => Some(Scalar::from(number(*left)? * number(*right)?)),
        FixDerivation::Difference(left, right) => {
            let held = number(*left)? - number(*right)?;
            // A negative remainder means the two inputs were never about one
            // order, and answering it would state a quantity that cannot
            // exist.
            (held >= 0.0).then(|| Scalar::from(held))
        }
    }
}

/// Whether the average price this report states is one fill's price.
///
/// `AvgPx` over two fills is not derivable from one of them, so the rule runs
/// only where the report says the whole done quantity arrived in this fill.
fn single_fill(msg: &FixMsg) -> bool {
    let (Some(done), Some(last)) = (msg.get_by_tag(14), msg.get_by_tag(32)) else {
        return false;
    };
    match (done.as_f64(), last.as_f64()) {
        (Some(done), Some(last)) => done == last && last > 0.0,
        _ => false,
    }
}

/// Fills what `msg` implies, leaving what it stated and what arrived alone.
///
/// # Errors
///
/// Returns the value contract's refusal when a derived value does not fit the
/// column the dictionary declares for it, which a rule's own arithmetic
/// cannot provoke.
pub(super) fn enrich(registry: &Arc<FixRegistry>, msg: FixMsg) -> Result<FixMsg> {
    let msgtype = msg
        .get_by_tag(35)
        .and_then(Scalar::as_str)
        .unwrap_or_default()
        .to_owned();

    let mut held = msg;
    for rule in RULES {
        if !rule.msgtypes.is_empty() && !rule.msgtypes.iter().any(|known| *known == msgtype) {
            continue;
        }
        // A stated value is never overwritten, which is what makes this
        // idempotent: the second pass finds the first pass's answer stated.
        if held
            .get_by_tag(rule.tag)
            .is_some_and(|value| value != &Scalar::Null)
        {
            continue;
        }
        if !rule.when.iter().all(|when| when.holds(&held)) {
            continue;
        }
        if rule.tag == 6 && !single_fill(&held) {
            continue;
        }
        let Some(value) = derive(&held, &rule.from) else {
            continue;
        };
        // The dictionary's own field types the value, so a derived column is
        // indistinguishable from a stated one and carries the same display,
        // description and `fix:tag` a reader resolves it by.
        let Some(declared) = registry.get_field_by_tag(rule.tag) else {
            continue;
        };
        let Ok(typed) = declared.scalar(value) else {
            continue;
        };
        // A rule whose output another rule reads has to be visible to it, so
        // the message is rebuilt as each one answers rather than once at the
        // end.
        held = append(registry, held, declared.clone(), typed)?;
    }
    Ok(held)
}

/// One message with `field` appended, carrying `value`.
///
/// Appended rather than inserted in tag order: the row's existing positions
/// are what every reader that already holds it addresses by, and a derived
/// field is found by tag rather than by position. The entries are carried
/// through untouched.
fn append(registry: &Arc<FixRegistry>, msg: FixMsg, field: Field, value: Scalar) -> Result<FixMsg> {
    let root = msg.as_field().clone();
    let mut members: Vec<Field> = root
        .dtype()
        .as_fields()
        .map(<[Field]>::to_vec)
        .unwrap_or_default();
    let mut values: Vec<Scalar> = msg
        .as_value()
        .as_sequence()
        .map(<[Scalar]>::to_vec)
        .unwrap_or_default();
    members.push(field);
    values.push(value);
    let rebuilt = DataType::from_fields(members)?.required_field(root.name());
    let entries = msg.into_entries();
    FixMsg::from_parts(
        Arc::clone(registry),
        rebuilt,
        Scalar::from_sequence(values),
        entries,
    )
}

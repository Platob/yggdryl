//! A trade as one market event composed of its executions.

use std::collections::{BTreeMap, HashSet};
use std::hash::Hasher;

use smol_str::{SmolStr, format_smolstr};

use super::{Element, Event, Execution, MarketElement, MarketEvent, MarketEventData};
use crate::{Error, Result, Uuid};

/// One trade and the nonempty, canonically ordered executions it comprises.
///
/// The root carries the trade-level market event. Its executions are unique
/// by cross code and ordered by side, cross code and identity, so construction
/// order never changes the trade's content identity.
#[derive(Clone, Debug, PartialEq)]
pub struct Trade {
    event: MarketEventData,
    executions: Vec<Execution>,
}

impl Trade {
    /// Builds a trade from its root event and executions.
    ///
    /// Every execution must occur at the root instant, state a bid or ask
    /// side, carry a symbol compatible with the root and have a cross code no
    /// other execution in the trade carries. Executions are finalized and
    /// placed in canonical order before the root bounds and identity are
    /// derived.
    pub fn from_parts(event: MarketEventData, mut executions: Vec<Execution>) -> Result<Self> {
        for execution in &mut executions {
            execution.finalize();
        }
        executions.sort_by(compare_executions);
        let mut trade = Self { event, executions };
        trade.validate_parts()?;
        trade.refresh();
        Ok(trade)
    }

    /// The executions this trade comprises, in canonical order.
    #[must_use]
    pub fn executions(&self) -> &[Execution] {
        &self.executions
    }

    /// Consumes this trade into the executions a book records.
    pub(crate) fn into_executions(self) -> Vec<Execution> {
        self.executions
    }

    /// Redates the composite observation while retaining each child's precise
    /// execution clock. Used when a prior side is carried into a later trade
    /// view and when a supplied snapshot gives the trade an effective instant.
    pub(super) fn rebase_currunix(&mut self, unix: i64) {
        self.event.set_currunix(unix);
        rebase_executions(&mut self.executions, unix);
        self.refresh();
    }

    /// Validates the composite invariants without changing the trade.
    pub(super) fn validate_parts(&self) -> Result<()> {
        if self.executions.is_empty() {
            return Err(invalid(
                "$.executions",
                "expected a trade to contain at least one execution",
            ));
        }

        let root_unix = self.event.get_currunix();
        let root_symbol = self.event.get_symbolticker();
        let mut crosscodes = HashSet::with_capacity(self.executions.len());
        for (index, execution) in self.executions.iter().enumerate() {
            let path = |name: &str| format_smolstr!("$.executions[{index}].{name}");
            if !execution.get_side().is_bid() && !execution.get_side().is_ask() {
                return Err(invalid(
                    path("side"),
                    format_smolstr!(
                        "expected a bid or ask side, got {:?}",
                        execution.get_side().as_str()
                    ),
                ));
            }
            if execution.get_currunix() != root_unix {
                return Err(invalid(
                    path("currunix"),
                    format_smolstr!(
                        "expected the trade timestamp {root_unix}, got {}",
                        execution.get_currunix()
                    ),
                ));
            }
            let symbol = execution.get_symbolticker();
            if symbol.is_some() && symbol != root_symbol {
                return Err(invalid(
                    path("symbolticker"),
                    format_smolstr!("expected symbol {root_symbol:?}, got {symbol:?}"),
                ));
            }
            if !crosscodes.insert(execution.get_crosscode()) {
                return Err(invalid(
                    path("crosscode"),
                    format_smolstr!(
                        "expected a unique execution cross code, got {:?}",
                        execution.get_crosscode()
                    ),
                ));
            }
        }

        if self
            .executions
            .windows(2)
            .any(|pair| compare_executions(&pair[0], &pair[1]).is_gt())
        {
            return Err(invalid(
                "$.executions",
                "expected executions ordered by side, cross code and identity",
            ));
        }
        Ok(())
    }

    /// The canonical root event, including child-derived bounds and digest.
    #[must_use]
    pub(super) fn canonical_event(&self) -> MarketEventData {
        let mut event = self.event.clone();
        for execution in &self.executions {
            event.set_seqnum(event.get_seqnum().max(execution.get_seqnum()));
            event.set_creaunix(earliest(event.get_creaunix(), execution.get_creaunix()));
            event.set_recdunix(earliest(event.get_recdunix(), execution.get_recdunix()));
            event.set_execunix(latest(event.get_execunix(), execution.get_execunix()));
        }
        event.fill_market();
        event.sync_cross();
        let mut digest = event.digest_market_event();
        digest.write(&(self.executions.len() as u64).to_be_bytes());
        for execution in &self.executions {
            digest.write(&execution.get_curruuid().get().to_be_bytes());
        }
        event.finalized(digest.finish());
        event
    }

    fn refresh(&mut self) {
        rebase_executions(&mut self.executions, self.event.get_currunix());
        self.event = self.canonical_event();
    }
}

impl AsRef<MarketEventData> for Trade {
    fn as_ref(&self) -> &MarketEventData {
        &self.event
    }
}

impl AsMut<MarketEventData> for Trade {
    fn as_mut(&mut self) -> &mut MarketEventData {
        &mut self.event
    }
}

impl Element for Trade {
    fn get_curruuid(&self) -> Uuid {
        self.event.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.event.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.event.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.event.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.event.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.event.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.event.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.event.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.event.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.event.set_crosshashcode(crosshashcode);
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        self.event.get_identifiers()
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.event.set_identifiers(identifiers);
    }

    fn get_parentuuids(&self) -> &[Uuid] {
        self.event.get_parentuuids()
    }

    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.event.set_parentuuids(parents);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.event.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.event.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.get_currunix() > other.get_currunix()
            || self.get_currunix() == other.get_currunix()
                && self.get_crosscode() > other.get_crosscode()
    }

    fn finalize(&mut self) {
        self.refresh();
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        if self.get_crosscode() != previous.get_crosscode()
            || self.get_currunix() < previous.get_currunix()
        {
            return None;
        }
        let event = self.event.clone().following_market(&previous.event)?;
        let mut executions = combine_executions(&self.executions, &previous.executions);
        rebase_executions(&mut executions, event.get_currunix());
        Self::from_parts(event, executions).ok()
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        if self.get_crosscode() != other.get_crosscode() {
            return None;
        }
        let right = reference_key(other) > reference_key(&self);
        let (reference, supplement) = if right {
            (other, &self)
        } else {
            (&self, other)
        };
        let mut event = reference.event.clone();
        super::element::merge_market_event_into_reference(&mut event, &supplement.event);
        let mut executions = combine_executions(&reference.executions, &supplement.executions);
        rebase_executions(&mut executions, event.get_currunix());
        let merged = Self::from_parts(event, executions).ok()?;
        (merged != self).then_some(merged)
    }
}

delegate_market_event!(Trade, event, market_only);
delegate_market_event!(
    Trade,
    event,
    event_only,
    true,
    |this: &mut Trade, unix: i64| this.rebase_currunix(unix)
);

fn compare_executions(left: &Execution, right: &Execution) -> std::cmp::Ordering {
    (
        left.get_side().as_str(),
        left.get_crosscode(),
        left.get_curruuid(),
    )
        .cmp(&(
            right.get_side().as_str(),
            right.get_crosscode(),
            right.get_curruuid(),
        ))
}

fn combine_executions(left: &[Execution], right: &[Execution]) -> Vec<Execution> {
    let mut combined = BTreeMap::<String, Execution>::new();
    for execution in left.iter().chain(right) {
        let crosscode = execution.get_crosscode().to_owned();
        combined
            .entry(crosscode)
            .and_modify(|held| *held = merge_execution(held, execution))
            .or_insert_with(|| execution.clone());
    }
    combined.into_values().collect()
}

fn rebase_executions(executions: &mut [Execution], unix: i64) {
    for execution in executions {
        if execution.get_currunix() != unix {
            execution.set_currunix(unix);
            execution.finalize();
        }
    }
}

fn merge_execution(left: &Execution, right: &Execution) -> Execution {
    let right_leads = reference_key(right) > reference_key(left);
    let (reference, supplement) = if right_leads {
        (right, left)
    } else {
        (left, right)
    };
    let mut event = reference.as_ref().clone();
    super::element::merge_market_event_into_reference(&mut event, supplement.as_ref());
    Execution::from(event)
}

fn reference_key<E: Event + ?Sized>(event: &E) -> (Option<i64>, i64, Uuid) {
    (
        super::element::reference_recdunix(event),
        event.get_currunix(),
        event.get_curruuid(),
    )
}

fn earliest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

fn latest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Side;

    #[test]
    fn composite_digest_consumes_a_child_uuid_once() {
        let mut root = MarketEventData::at(1);
        root.set_crosscode("trade".to_owned());
        root.finalize();

        let mut child = MarketEventData::at(1);
        child.set_crosscode("execution".to_owned());
        child.set_side(Side::read("Buy").expect("the shipped Buy side"));
        child.finalize();
        let mut left_child = Execution::from(child.clone());
        let mut right_child = Execution::from(child);
        let child_uuid = left_child.get_curruuid();
        left_child.set_currhashcode(11);
        right_child.set_currhashcode(29);
        left_child.set_curruuid(child_uuid);
        right_child.set_curruuid(child_uuid);
        assert_eq!(left_child.get_curruuid(), right_child.get_curruuid());

        let left = Trade {
            event: root.clone(),
            executions: vec![left_child],
        };
        let right = Trade {
            event: root,
            executions: vec![right_child],
        };
        assert_eq!(
            left.canonical_event().get_currhashcode(),
            right.canonical_event().get_currhashcode()
        );
        assert_eq!(
            left.canonical_event().get_curruuid(),
            right.canonical_event().get_curruuid()
        );
    }
}

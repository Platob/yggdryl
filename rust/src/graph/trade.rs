//! A trade as one market operation composed of its executions.

use std::collections::{BTreeMap, HashSet};
use std::hash::Hasher;

use smol_str::{SmolStr, format_smolstr};

use super::facts::OperationEventFacts;
use super::operation::ExecutionEvent;
use super::{Element, Event, Market, Operation};
use crate::{Error, Result, Uuid};

/// A composite trade: one market operation event whose executions are the
/// sided fills it is made of.
///
/// The root's identity is derived from its own facts and its executions'
/// identities, so two trades stating the same fills are one trade. The
/// executions are canonical: finalized, ordered by side, cross code and
/// identity, unique by cross code, and dated at the trade's instant.
#[derive(Clone, Debug, PartialEq)]
pub struct TradeEvent {
    data: OperationEventFacts,
    executions: Vec<ExecutionEvent>,
}

impl TradeEvent {
    /// A trade over the facts `root` states - its instant, its market and
    /// its operation facts - and its executions, finalized.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when there is no execution, one
    /// takes no bid or ask side, one is dated at another instant, one names
    /// another symbol, or two share a cross code.
    pub fn from_parts<E: Event + Operation + ?Sized>(
        root: &E,
        executions: Vec<ExecutionEvent>,
    ) -> Result<Self> {
        Self::from_facts(OperationEventFacts::from(root), executions)
    }

    /// [`Self::from_parts`] over root facts already held: a move.
    pub(crate) fn from_facts(
        data: OperationEventFacts,
        mut executions: Vec<ExecutionEvent>,
    ) -> Result<Self> {
        for execution in &mut executions {
            execution.finalize();
        }
        executions.sort_by(compare_executions);
        let mut trade = Self { data, executions };
        trade.validate_parts()?;
        trade.refresh();
        Ok(trade)
    }

    /// The executions the trade is made of, in canonical order.
    #[must_use]
    pub fn executions(&self) -> &[ExecutionEvent] {
        &self.executions
    }

    /// The root facts.
    pub(crate) fn data(&self) -> &OperationEventFacts {
        &self.data
    }

    pub(crate) fn into_executions(self) -> Vec<ExecutionEvent> {
        self.executions
    }

    pub(super) fn rebase_currunix(&mut self, unix: i64) {
        self.data.set_currunix(unix);
        rebase_executions(&mut self.executions, unix);
        self.refresh();
    }

    pub(super) fn validate_parts(&self) -> Result<()> {
        if self.executions.is_empty() {
            return Err(invalid(
                "$.executions",
                "expected a trade to contain at least one execution",
            ));
        }
        let root_unix = self.data.get_currunix();
        let root_symbol = self.data.get_ticker();
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
            let symbol = execution.get_ticker();
            if symbol.is_some() && symbol != root_symbol {
                return Err(invalid(
                    path("ticker"),
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

    /// The root as the canonical composite: its clocks folded with its
    /// executions', its market filled, and its identity digested over its
    /// own facts and each execution's identity.
    #[must_use]
    pub(super) fn canonical_data(&self) -> OperationEventFacts {
        let mut data = self.data.clone();
        for execution in &self.executions {
            data.set_seqnum(data.get_seqnum().max(execution.get_seqnum()));
            data.set_creaunix(earliest(data.get_creaunix(), execution.get_creaunix()));
            data.set_recdunix(earliest(data.get_recdunix(), execution.get_recdunix()));
            data.set_execunix(latest(data.get_execunix(), execution.get_execunix()));
        }
        data.fill_market();
        data.fill_operation();
        data.sync_cross();
        let mut digest = data.digest_operation_event();
        digest.write(&(self.executions.len() as u64).to_be_bytes());
        for execution in &self.executions {
            digest.write(&execution.get_curruuid().get().to_be_bytes());
        }
        data.finalized(digest.finish());
        data
    }

    fn refresh(&mut self) {
        rebase_executions(&mut self.executions, self.data.get_currunix());
        self.data = self.canonical_data();
    }
}

impl Element for TradeEvent {
    fn get_curruuid(&self) -> Uuid {
        self.data.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.data.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.data.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.data.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.data.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.data.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.data.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.data.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.data.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.data.set_crosshashcode(crosshashcode);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.data.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.data.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.data.is_after(&other.data)
    }

    fn finalize(&mut self) {
        self.refresh();
    }

    /// A trade is one root over its executions, so it follows the earlier
    /// statement of the same crosscode and keeps every execution either
    /// stated; its composite identity is not the gate, since the executions
    /// are part of it.
    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if self.get_crosscode() != previous.get_crosscode()
            || self.get_currunix() < previous.get_currunix()
        {
            return None;
        }
        let data = std::mem::take(&mut self.data).following_operation(&previous.data)?;
        let mut executions = combine_executions(&self.executions, &previous.executions);
        rebase_executions(&mut executions, data.get_currunix());
        Self::from_facts(data, executions).ok()
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        if self.get_crosscode() != other.get_crosscode() {
            return None;
        }
        let right_leads = reference_key(&other.data) > reference_key(&self.data);
        let (reference, supplement) = if right_leads {
            (other, &self)
        } else {
            (&self, other)
        };
        let mut data = reference.data.clone();
        super::market::merge_operation_event_into_reference(&mut data, &supplement.data);
        let mut executions = combine_executions(&reference.executions, &supplement.executions);
        rebase_executions(&mut executions, data.get_currunix());
        let merged = Self::from_facts(data, executions).ok()?;
        (merged != self).then_some(merged)
    }
}

impl Event for TradeEvent {
    fn get_currunix(&self) -> i64 {
        self.data.get_currunix()
    }
    fn set_currunix(&mut self, unix: i64) {
        self.rebase_currunix(unix);
    }
    fn get_state(&self) -> &crate::State {
        self.data.get_state()
    }
    fn set_state(&mut self, state: crate::State) {
        self.data.set_state(state);
    }
    fn is_execution(&self) -> bool {
        true
    }
    fn get_seqnum(&self) -> u64 {
        self.data.get_seqnum()
    }
    fn set_seqnum(&mut self, seqnum: u64) {
        self.data.set_seqnum(seqnum);
    }
    fn get_creaunix(&self) -> Option<i64> {
        self.data.get_creaunix()
    }
    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.data.set_creaunix(unix);
    }
    fn get_execunix(&self) -> Option<i64> {
        self.data.get_execunix()
    }
    fn set_execunix(&mut self, unix: Option<i64>) {
        self.data.set_execunix(unix);
    }
    fn get_recdunix(&self) -> Option<i64> {
        self.data.get_recdunix()
    }
    fn set_recdunix(&mut self, unix: Option<i64>) {
        self.data.set_recdunix(unix);
    }
    fn get_exprtime(&self) -> Option<i64> {
        self.data.get_exprtime()
    }
    fn set_exprtime(&mut self, unix: Option<i64>) {
        self.data.set_exprtime(unix);
    }
    fn get_prevunix(&self) -> Option<i64> {
        self.data.get_prevunix()
    }
    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.data.set_prevunix(unix);
    }
    fn get_prevuuid(&self) -> Option<Uuid> {
        self.data.get_prevuuid()
    }
    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.data.set_prevuuid(uuid);
    }
    fn get_snapunix(&self) -> Option<i64> {
        self.data.get_snapunix()
    }
    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.data.set_snapunix(unix);
    }
    fn restating(mut self, live: &Self) -> Self {
        let data = std::mem::take(&mut self.data).restating(&live.data);
        self.data = data;
        self.refresh();
        self
    }
    fn finalized(&mut self, hashcode: u64) {
        self.data.finalized(hashcode);
    }
}

delegate_market!(TradeEvent, data);
delegate_operation!(TradeEvent, data);

fn compare_executions(left: &ExecutionEvent, right: &ExecutionEvent) -> std::cmp::Ordering {
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

fn combine_executions(left: &[ExecutionEvent], right: &[ExecutionEvent]) -> Vec<ExecutionEvent> {
    let mut combined = BTreeMap::<String, ExecutionEvent>::new();
    for execution in left.iter().chain(right) {
        let crosscode = execution.get_crosscode().to_owned();
        combined
            .entry(crosscode)
            .and_modify(|held| *held = merge_execution(held, execution))
            .or_insert_with(|| execution.clone());
    }
    combined.into_values().collect()
}

fn rebase_executions(executions: &mut [ExecutionEvent], unix: i64) {
    for execution in executions {
        if execution.get_currunix() != unix {
            execution.set_currunix(unix);
            execution.finalize();
        }
    }
}

fn merge_execution(left: &ExecutionEvent, right: &ExecutionEvent) -> ExecutionEvent {
    let right_leads = reference_key(right) > reference_key(left);
    let (reference, supplement) = if right_leads {
        (right, left)
    } else {
        (left, right)
    };
    let mut merged = reference.clone();
    super::market::merge_operation_event_into_reference(&mut merged, supplement);
    merged.finalize();
    merged
}

fn reference_key<E: Event + ?Sized>(event: &E) -> (Option<i64>, i64, Uuid) {
    (
        event.get_recdunix(),
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

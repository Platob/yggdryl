//! `rust/src/graph/execution.rs`: an execution is a transparent market event
//! whose operation kind is execution independently of generic lifecycle state.

use std::mem::size_of;

use yggdryl::graph::{
    Event, Execution, ExecutionEntry, MarketElement, MarketElementData, MarketEventData,
};
use yggdryl::{Decimal18, State};

#[test]
fn execution_wrappers_are_transparent_and_always_classify_as_executions() {
    assert_eq!(size_of::<Execution>(), size_of::<MarketEventData>());
    assert_eq!(size_of::<ExecutionEntry>(), size_of::<MarketElementData>());

    let event = MarketEventData::at(23);
    assert!(
        !event.is_execution(),
        "an unknown generic state is not a fill"
    );
    let execution = Execution::from(event);
    assert!(execution.is_execution(), "the concrete operation kind wins");
    assert_eq!(execution.get_state(), &State::unknown());
}

#[test]
fn execution_conversions_preserve_event_and_market_facts() {
    let mut event = MarketEventData::at(29);
    event.set_lastpx(Some(Decimal18::from_int(101)));
    event.set_lastqty(Some(Decimal18::from_int(3)));
    event.set_symbolticker(Some("DEF".to_owned()));
    let execution = Execution::from(&event);
    assert_eq!(MarketEventData::from(execution.clone()), event);

    let entry = ExecutionEntry::from(execution);
    assert_eq!(entry.get_lastpx(), Some(Decimal18::from_int(101)));
    assert_eq!(entry.get_lastqty(), Some(Decimal18::from_int(3)));
    assert_eq!(entry.get_symbolticker(), Some("DEF"));

    let execution = Execution::from(entry);
    assert_eq!(execution.get_currunix(), 0);
    assert!(execution.is_execution());
}

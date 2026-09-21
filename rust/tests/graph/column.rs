//! `rust/src/graph/column.rs`: the nineteen columns every event schema opens
//! with, each stating back exactly the fact it read.

use yggdryl::graph::{Element, Event, EventColumn, MarketEventData};
use yggdryl::{Scalar, State, Uuid};

#[test]
fn every_column_states_back_what_it_read() {
    let mut event = MarketEventData::at(1_700_000_000_000_000_000);
    event.set_creaunix(Some(1_600_000_000_000_000_000));
    event.set_execunix(Some(1_650_000_000_000_000_000));
    event.set_recdunix(Some(1_675_000_000_000_000_000));
    event.set_refrecdunix(Some(1_676_000_000_000_000_000));
    event.set_exprtime(Some(1_800_000_000_000_000_000));
    event.set_prevunix(Some(1_650_000_000_000_000_000));
    event.set_snapunix(Some(1_700_000_000_000_000_001));
    event.set_curruuid(Uuid::from_v8(1));
    event.set_crossuuid(Uuid::from_v8(2));
    event.set_crosscode("O-1".to_owned());
    event.set_currhashcode(3);
    event.set_crosshashcode(4);
    event.set_prevuuid(Some(Uuid::from_v8(5)));
    event.set_seqnum(6);
    event.set_parentuuids(vec![Uuid::from_v8(7), Uuid::from_v8(8)]);
    event.set_srcuuids(vec![Uuid::from_v8(9)]);
    event.set_identifiers([("OrderID".to_owned(), "O-1".to_owned())].into());
    event.set_state(State::read("Filled").expect("a state"));
    let mut again = MarketEventData::default();
    for column in EventColumn::ALL {
        let fact = column.fact(&event).expect("every fact is stated");
        let dtype = column.datatype().expect("a datatype");
        dtype
            .required_field(column.name())
            .scalar(fact.clone())
            .expect("the fact fits the column");
        column.record(&mut again, &fact);
    }
    assert_eq!(again.get_currunix(), event.get_currunix());
    assert_eq!(again.get_creaunix(), event.get_creaunix());
    assert_eq!(again.get_execunix(), event.get_execunix());
    assert_eq!(again.get_recdunix(), event.get_recdunix());
    assert_eq!(again.get_refrecdunix(), event.get_refrecdunix());
    assert_eq!(again.get_exprtime(), event.get_exprtime());
    assert_eq!(again.get_prevunix(), event.get_prevunix());
    assert_eq!(again.get_snapunix(), event.get_snapunix());
    assert_eq!(again.get_curruuid(), event.get_curruuid());
    assert_eq!(again.get_crossuuid(), event.get_crossuuid());
    assert_eq!(again.get_crosscode(), "O-1");
    assert_eq!(again.get_currhashcode(), 3);
    assert_eq!(again.get_crosshashcode(), 4);
    assert_eq!(again.get_prevuuid(), Some(Uuid::from_v8(5)));
    assert_eq!(again.get_seqnum(), 6);
    assert_eq!(again.get_parentuuids(), event.get_parentuuids());
    assert_eq!(again.get_srcuuids(), event.get_srcuuids());
    assert_eq!(again.get_identifiers(), event.get_identifiers());
    assert_eq!(again.get_state(), event.get_state());
}

#[test]
fn a_null_clears_and_nothing_stated_is_none() {
    let mut event = MarketEventData::at(7);
    event.set_seqnum(3);
    event.set_crosscode("X".to_owned());
    event.set_execunix(Some(4));
    event.set_recdunix(Some(5));
    event.set_refrecdunix(Some(6));
    EventColumn::SeqNum.record(&mut event, &Scalar::Null);
    EventColumn::CrossCode.record(&mut event, &Scalar::Null);
    EventColumn::State.record(&mut event, &Scalar::Null);
    EventColumn::ExecUnix.record(&mut event, &Scalar::Null);
    EventColumn::RecdUnix.record(&mut event, &Scalar::Null);
    EventColumn::RefRecdUnix.record(&mut event, &Scalar::Null);
    assert_eq!(event.get_seqnum(), 0);
    assert_eq!(event.get_crosscode(), "");
    assert_eq!(event.get_state(), &State::unknown());
    assert_eq!(event.get_execunix(), None);
    assert_eq!(event.get_recdunix(), None);
    assert_eq!(event.get_refrecdunix(), None);
    assert_eq!(EventColumn::SeqNum.fact(&event), None);
    assert_eq!(EventColumn::CrossCode.fact(&event), None);
    assert_eq!(
        EventColumn::State.fact(&event),
        Some(Scalar::State(State::unknown())),
        "the state is never absent"
    );
    let names: Vec<&str> = EventColumn::ALL
        .iter()
        .map(|column| column.name())
        .collect();
    assert_eq!(
        names,
        [
            "currunix",
            "creaunix",
            "execunix",
            "recdunix",
            "refrecdunix",
            "exprtime",
            "prevunix",
            "snapunix",
            "curruuid",
            "crossuuid",
            "crosscode",
            "currhashcode",
            "crosshashcode",
            "prevuuid",
            "seqnum",
            "parentuuids",
            "srcuuids",
            "identifiers",
            "state",
        ]
    );
    assert_eq!(EventColumn::of_name("no such"), None);
}

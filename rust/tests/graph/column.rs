//! `rust/src/graph/column.rs`: the nine columns an event adds to its
//! element's, each stating back exactly the fact it read.

use yggdryl::graph::{Event, EventColumn, OrderEvent};
use yggdryl::{Scalar, State, Uuid};

#[test]
fn every_column_states_back_what_it_read() {
    let mut event = OrderEvent::at(1_700_000_000_000_000_000);
    event.set_creaunix(Some(1_600_000_000_000_000_000));
    event.set_recdunix(Some(1_675_000_000_000_000_000));
    event.set_exprunix(Some(1_800_000_000_000_000_000));
    event.set_prevunix(Some(1_650_000_000_000_000_000));
    event.set_snapunix(Some(1_700_000_000_000_000_001));
    event.set_prevuuid(Some(Uuid::from_v8(5)));
    event.set_seqnum(6);
    event.set_state(State::read("Filled").expect("a state"));
    let mut again = OrderEvent::default();
    for column in EventColumn::ALL {
        let fact = column.fact(&event).expect("every fact is stated");
        column
            .datatype()
            .required_field(column.name())
            .scalar(fact.clone())
            .expect("the fact fits the column");
        column.record(&mut again, &fact);
    }
    assert_eq!(again.get_currunix(), event.get_currunix());
    assert_eq!(again.get_creaunix(), event.get_creaunix());
    assert_eq!(again.get_recdunix(), event.get_recdunix());
    assert_eq!(again.get_exprunix(), event.get_exprunix());
    assert_eq!(again.get_prevunix(), event.get_prevunix());
    assert_eq!(again.get_snapunix(), event.get_snapunix());
    assert_eq!(again.get_prevuuid(), Some(Uuid::from_v8(5)));
    assert_eq!(again.get_seqnum(), 6);
    assert_eq!(again.get_state(), event.get_state());
}

#[test]
fn a_null_clears_and_nothing_stated_is_none() {
    let mut event = OrderEvent::at(7);
    event.set_seqnum(3);
    event.set_recdunix(Some(5));
    EventColumn::SeqNum.record(&mut event, &Scalar::Null);
    EventColumn::State.record(&mut event, &Scalar::Null);
    EventColumn::RecdUnix.record(&mut event, &Scalar::Null);
    assert_eq!(event.get_seqnum(), 0);
    assert_eq!(event.get_state(), &State::unknown());
    assert_eq!(event.get_recdunix(), None);
    assert_eq!(
        EventColumn::SeqNum.fact(&event),
        Some(Scalar::from(0_u64)),
        "the place is never absent: a null reads as the first"
    );
    assert_eq!(
        EventColumn::State.fact(&event),
        Some(Scalar::State(State::unknown())),
        "the state is never absent"
    );
    // A null instant is silence: the instant is never absent.
    EventColumn::CurrUnix.record(&mut event, &Scalar::Null);
    assert_eq!(event.get_currunix(), 7);
}

#[test]
fn the_columns_are_the_event_trait_s_in_one_order() {
    let names: Vec<&str> = EventColumn::ALL
        .iter()
        .map(|column| column.name())
        .collect();
    assert_eq!(
        names,
        [
            "currunix", "creaunix", "recdunix", "exprunix", "prevunix", "snapunix", "prevuuid",
            "seqnum", "state",
        ]
    );
    let nullable: Vec<&str> = EventColumn::ALL
        .into_iter()
        .filter(|column| !column.nullable())
        .map(EventColumn::name)
        .collect();
    assert_eq!(
        nullable,
        ["currunix", "seqnum"],
        "only the instant and the place at it are never absent"
    );
    assert_eq!(EventColumn::of_name("no such"), None);
    // An identity, a code and a source are the element's facts, a column of
    // its own.
    for element in [
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "srcuuids",
    ] {
        assert_eq!(EventColumn::of_name(element), None, "{element}");
    }
    // The merge reference is chosen from `recdunix` alone, so no column
    // persists a separate reference recording clock any more.
    assert_eq!(EventColumn::of_name("refrecdunix"), None);
    // The names an element went by left the event: a book control is typed
    // on the operation and the names it goes by are its
    // identifiers, an operation column.
    assert_eq!(EventColumn::of_name("identifiers"), None);
    // When an element last executed is a market fact, a market column: a
    // text line or any other event that is no market element states none.
    assert_eq!(EventColumn::of_name("execunix"), None);
}

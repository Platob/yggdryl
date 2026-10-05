//! `rust/src/zip/leaf.rs`: one member as a byte leaf - here, the create the
//! archive's index decides under its one lock.

use std::sync::{Arc, Barrier};

use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::{ZipArchive, ZipLeaf};
use yggdryl::{Error, IOBase};

/// A fresh in-memory archive, shared as every leaf of it shares it.
fn archive() -> Arc<ZipArchive> {
    Arc::clone(
        ZipArchive::new(Holder::buffer(Buffer::new()))
            .mount()
            .archive(),
    )
}

#[test]
fn a_create_writes_a_member_where_the_index_names_none() {
    let archive = archive();
    let mut leaf = ZipLeaf::new(Arc::clone(&archive), "trades/eu.csv".into());
    leaf.create_bytes(b"symbol,price\nAAPL,187.23\n").unwrap();
    // A complete operation: the directory is published on return.
    assert!(!archive.is_pending());
    assert_eq!(
        archive.read_member("trades/eu.csv").unwrap(),
        b"symbol,price\nAAPL,187.23\n"
    );
    assert_eq!(
        leaf.read_all_bytes().unwrap(),
        b"symbol,price\nAAPL,187.23\n"
    );
}

#[test]
fn a_create_over_a_member_is_the_conflict_and_writes_nothing() {
    let archive = archive();
    archive.write_member("trades/eu.csv", b"AAPL").unwrap();
    archive.flush().unwrap();
    let writes = archive.handle_writes();

    let mut leaf = ZipLeaf::new(Arc::clone(&archive), "trades/eu.csv".into());
    let error = leaf.create_bytes(b"MSFT").unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }), "{error}");
    assert!(error.to_string().contains("trades/eu.csv"), "{error}");
    assert_eq!(
        archive.handle_writes(),
        writes,
        "a refused create writes nothing"
    );
    assert_eq!(archive.read_member("trades/eu.csv").unwrap(), b"AAPL");
}

#[test]
fn of_eight_racing_creators_of_one_member_exactly_one_succeeds() {
    const CREATORS: usize = 8;
    let archive = archive();
    let barrier = Arc::new(Barrier::new(CREATORS));
    let outcomes: Vec<(Vec<u8>, yggdryl::Result<()>)> = std::thread::scope(|scope| {
        let creators: Vec<_> = (0..CREATORS)
            .map(|creator| {
                let archive = Arc::clone(&archive);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    let payload = format!("creator-{creator}").into_bytes();
                    let mut leaf = ZipLeaf::new(archive, "claim".into());
                    barrier.wait();
                    let outcome = leaf.create_bytes(&payload);
                    (payload, outcome)
                })
            })
            .collect();
        creators
            .into_iter()
            .map(|creator| creator.join().unwrap())
            .collect()
    });
    let winners: Vec<&Vec<u8>> = outcomes
        .iter()
        .filter(|(_, outcome)| outcome.is_ok())
        .map(|(payload, _)| payload)
        .collect();
    assert_eq!(winners.len(), 1, "exactly one creator wins");
    assert!(
        outcomes
            .iter()
            .filter_map(|(_, outcome)| outcome.as_ref().err())
            .all(Error::is_conflict)
    );
    assert_eq!(&archive.read_member("claim").unwrap(), winners[0]);
}

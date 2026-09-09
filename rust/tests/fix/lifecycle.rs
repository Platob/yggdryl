//! One order's life across the messages that told it: the three identities
//! the lifecycle pass stamps, the chain they join, and when it ends.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::local::Folder;
use yggdryl::{
    FixCodec, FixLifecycle, FixMsg, FixRegistry, ID_TAG, INSTID_TAG, PERSISTENTID_TAG, Scalar,
};

fn registry() -> Arc<FixRegistry> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    Arc::new(FixRegistry::from_handle(&folder).expect("the committed dictionary loads"))
}

/// The bytes one identity column holds.
fn bytes(message: &FixMsg, tag: i32) -> Option<Vec<u8>> {
    message
        .get_by_tag(tag)
        .filter(|held| !held.is_null())
        .and_then(Scalar::as_bytes)
        .map(<[u8]>::to_vec)
}

/// The messages of one order's life, as a venue and its client tell it.
const LIFE: [&[u8]; 6] = [
    // The order, sent under the client's own identifier.
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|60=20260102-10:15:30.000|10=0|",
    // Acknowledged under the venue's, which now names the same chain.
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|60=20260102-10:15:30.250|10=0|",
    // Half of it done.
    b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|60=20260102-10:15:31.000|10=0|",
    // Replaced: the new client identifier names the old one, and joins.
    b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|207=XNAS|15=USD|54=1|38=120|44=12.6|60=20260102-10:15:32.000|10=0|",
    b"8=FIX.4.4|35=8|41=A1|11=A2|37=O1|17=E3|150=5|39=5|55=AAPL|207=XNAS|15=USD|38=120|14=50|151=70|60=20260102-10:15:32.100|10=0|",
    // Filled under the new identifier alone: the chain ends here.
    b"8=FIX.4.4|35=8|11=A2|17=E4|150=F|39=2|55=AAPL|207=XNAS|15=USD|38=120|14=120|151=0|32=70|31=12.6|60=20260102-10:15:33.000|10=0|",
];

#[test]
fn every_message_of_one_order_carries_the_chains_identity_until_it_ends() {
    let registry = registry();
    let reader = FixCodec::new(Arc::clone(&registry));
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut stamped = Vec::with_capacity(LIFE.len());
    for line in LIFE {
        let message = reader.transform_line(line, false).expect("the line reads");
        stamped.push(life.fill(message).expect("the stamp lands"));
        // Alive from the first message to the fill that ends it.
        assert_eq!(life.alive(), usize::from(stamped.len() < LIFE.len()));
    }

    // One instrument, one chain, six messages.
    let instruments: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, INSTID_TAG).expect("an instrument"))
        .collect();
    assert!(instruments.iter().all(|held| *held == instruments[0]));
    assert_eq!(instruments[0].len(), 16);
    let chains: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, PERSISTENTID_TAG).expect("a chain"))
        .collect();
    assert!(
        chains.iter().all(|held| *held == chains[0]),
        "the replace's new identifier joined the chain the old one opened"
    );
    let ids: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, ID_TAG).expect("an id"))
        .collect();
    for pair in ids.windows(2) {
        assert!(pair[0] < pair[1], "ids sort by the impact clock");
    }
    // The chain is dated by the order's own transaction time, in
    // microseconds, and every id after it opens with a later instant.
    let created = i64::from_be_bytes(chains[0][..8].try_into().unwrap());
    assert_eq!(created, 1_767_348_930_000_000);
    assert_eq!(&ids[0][..8], &chains[0][..8]);

    // Nothing here is an entry: the wire re-emits byte for byte.
    for (line, message) in LIFE.iter().zip(&stamped) {
        assert_eq!(message.into_bytes(b'|'), *line);
    }

    // The identifier a venue reuses tomorrow opens a new chain rather than
    // joining yesterday's, which ended: dated by its own clock, it is
    // another identity.
    let tomorrow = String::from_utf8(LIFE[0].to_vec())
        .unwrap()
        .replace("20260102", "20260103");
    let again = life
        .fill(reader.transform_line(tomorrow.as_bytes(), false).unwrap())
        .unwrap();
    assert_ne!(bytes(&again, PERSISTENTID_TAG).unwrap(), chains[0]);
    assert_eq!(life.alive(), 1);
    life.clear();
    assert_eq!(life.alive(), 0);
    // The same line at the same instant is the same chain identity, which is
    // what makes two reads of one capture agree.
    let replayed = life
        .fill(reader.transform_line(LIFE[0], false).unwrap())
        .unwrap();
    assert_eq!(bytes(&replayed, PERSISTENTID_TAG).unwrap(), chains[0]);
    assert_eq!(bytes(&replayed, ID_TAG), Some(ids[0].clone()));
}

#[test]
fn a_message_naming_no_order_has_an_id_and_no_chain() {
    let registry = registry();
    let reader = FixCodec::new(Arc::clone(&registry));
    let heartbeat = reader
        .transform_line(b"8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|", false)
        .unwrap();
    let mut stamped = reader.lifecycle([heartbeat]);
    let held = stamped.next().unwrap().unwrap();
    assert!(stamped.next().is_none());
    assert!(bytes(&held, ID_TAG).is_some(), "every message has an id");
    assert!(
        bytes(&held, PERSISTENTID_TAG).is_none(),
        "no identifier, no chain"
    );
    assert!(
        bytes(&held, INSTID_TAG).is_none(),
        "no instrument, no identity"
    );
    // The impact clock is the sending time where no transaction time is
    // stated, and the epoch where the message states no clock at all.
    let sent = i64::from_be_bytes(bytes(&held, ID_TAG).unwrap()[..8].try_into().unwrap());
    assert_eq!(sent, 1_767_348_930_000_000);
    let undated = reader
        .lifecycle([reader
            .transform_line(b"8=FIX.4.4|35=0|10=0|", false)
            .unwrap()])
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(&bytes(&undated, ID_TAG).unwrap()[..8], &[0; 8]);
}

#[test]
fn the_instrument_identity_is_the_same_across_spellings_and_venues() {
    let registry = registry();
    let reader = FixCodec::new(Arc::clone(&registry));
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut identity = |line: &[u8]| {
        bytes(
            &life
                .fill(reader.transform_line(line, false).unwrap())
                .unwrap(),
            INSTID_TAG,
        )
    };
    // An ISIN outranks a symbol, so the same security under two symbols is
    // one instrument, and case is not a difference.
    let by_isin =
        identity(b"8=FIX.4.4|35=D|11=B1|48=US0378331005|22=4|55=AAPL|207=XNAS|15=USD|10=0|");
    let by_isin_again =
        identity(b"8=FIX.4.4|35=D|11=B2|48=us0378331005|22=4|55=APPLE|207=xnas|15=usd|10=0|");
    assert_eq!(by_isin, by_isin_again);
    // Another market is another instrument identity.
    let elsewhere =
        identity(b"8=FIX.4.4|35=D|11=B3|48=US0378331005|22=4|55=AAPL|207=XLON|15=USD|10=0|");
    assert_ne!(by_isin, elsewhere);
    // Without an ISIN the symbol stands in, and a stated one wins over a
    // symbol that would say otherwise.
    let by_symbol = identity(b"8=FIX.4.4|35=D|11=B4|55=AAPL|207=XNAS|15=USD|10=0|");
    assert!(by_symbol.is_some());
    assert_ne!(by_symbol, by_isin);
    // A bridge row names the same facts under its own keys.
    let bridged = identity(b"#ISINCODE=US0378331005|#LASTMKT=XNAS|#CURRENCY=USD|CLORDID=B5|");
    assert_eq!(bridged, by_isin);
}

#[test]
fn a_stamped_stream_read_again_keeps_what_it_carries() {
    let registry = registry();
    let reader = FixCodec::new(Arc::clone(&registry));
    let once: Vec<FixMsg> = reader
        .lifecycle(
            LIFE.iter()
                .map(|line| reader.transform_line(line, false).unwrap()),
        )
        .map(|held| held.unwrap())
        .collect();
    let twice: Vec<FixMsg> = reader
        .lifecycle(once.clone())
        .map(|held| held.unwrap())
        .collect();
    for (first, second) in once.iter().zip(&twice) {
        for tag in [INSTID_TAG, ID_TAG, PERSISTENTID_TAG] {
            assert_eq!(bytes(first, tag), bytes(second, tag), "tag {tag}");
        }
        assert_eq!(first.entries().len(), second.entries().len());
    }
}

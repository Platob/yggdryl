//! One order's life across the messages that told it: the three identities
//! the lifecycle pass stamps, the chain they join, and when it ends.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::types::Uuid;
use yggdryl::{
    DataType, Error, FixCodec, FixLifecycle, FixMsg, FixRegistry, INSTUUID_TAG_NAME,
    PUUID_TAG_NAME, Scalar, TIMESTAMP_TAG_NAME, TimeUnit, Timezone, UUID_TAG_NAME,
};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

/// The bytes one identity column holds.
fn bytes(message: &FixMsg, tag: i32) -> Option<[u8; 16]> {
    let held = message.get_by_tag(tag).filter(|held| !held.is_null())?;
    let Scalar::Uuid(uuid) = held else {
        panic!("tag {tag} must hold a native UUID, got {held:?}");
    };
    let bytes = uuid.into_bytes();
    assert_eq!(
        bytes[6] >> 4,
        if tag == INSTUUID_TAG_NAME.0 { 8 } else { 7 }
    );
    assert_eq!(bytes[8] >> 6, 2);
    Some(bytes)
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
        let message = reader.sole_line(line, false).expect("the line reads");
        stamped.push(life.fill(message).expect("the stamp lands"));
        // Alive from the first message to the fill that ends it.
        assert_eq!(life.alive(), usize::from(stamped.len() < LIFE.len()));
    }

    // One instrument, one chain, six messages.
    let instruments: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, INSTUUID_TAG_NAME.0).expect("an instrument"))
        .collect();
    assert!(instruments.iter().all(|held| *held == instruments[0]));
    assert_eq!(instruments[0].len(), 16);
    let chains: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, PUUID_TAG_NAME.0).expect("a chain"))
        .collect();
    assert!(
        chains.iter().all(|held| *held == chains[0]),
        "the replace's new identifier joined the chain the old one opened"
    );
    let ids: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, UUID_TAG_NAME.0).expect("an id"))
        .collect();
    for pair in ids.windows(2) {
        assert!(pair[0] < pair[1], "ids sort by the impact clock");
    }
    // The first six bytes order by milliseconds; the exact time is still
    // read from its own column, never decoded from a UUID's first eight.
    let millis = 1_767_348_930_000_u64.to_be_bytes();
    assert_eq!(&chains[0][..6], &millis[2..]);
    assert_eq!(&ids[0][..8], &chains[0][..8]);
    assert_eq!(
        yggdryl::txhash::unix_from_scalar(
            stamped[0].by_tag(60).unwrap(),
            yggdryl::TimeUnit::Microsecond,
        )
        .unwrap(),
        1_767_348_930_000_000,
    );

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
        .fill(reader.sole_line(tomorrow.as_bytes(), false).unwrap())
        .unwrap();
    assert_ne!(bytes(&again, PUUID_TAG_NAME.0).unwrap(), chains[0]);
    assert_eq!(life.alive(), 1);
    life.clear();
    assert_eq!(life.alive(), 0);
    // The same line at the same instant is the same chain identity, which is
    // what makes two reads of one capture agree.
    let replayed = life
        .fill(reader.sole_line(LIFE[0], false).unwrap())
        .unwrap();
    assert_eq!(bytes(&replayed, PUUID_TAG_NAME.0).unwrap(), chains[0]);
    assert_eq!(bytes(&replayed, UUID_TAG_NAME.0), Some(ids[0]));
}

#[test]
fn a_message_naming_no_order_has_an_id_and_no_chain() {
    let registry = registry();
    let reader = FixCodec::new(Arc::clone(&registry));
    let heartbeat = reader
        .sole_line(b"8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|", false)
        .unwrap();
    let mut stamped = reader.lifecycle([heartbeat]);
    let held = stamped.next().unwrap().unwrap();
    assert!(stamped.next().is_none());
    assert!(
        bytes(&held, UUID_TAG_NAME.0).is_some(),
        "every message has an id"
    );
    assert!(
        bytes(&held, PUUID_TAG_NAME.0).is_none(),
        "no identifier, no chain"
    );
    assert!(
        bytes(&held, INSTUUID_TAG_NAME.0).is_none(),
        "no instrument, no identity"
    );
    // The impact clock is the sending time where no transaction time is
    // stated, and the epoch where the message states no clock at all.
    let millis = 1_767_348_930_000_u64.to_be_bytes();
    assert_eq!(&bytes(&held, UUID_TAG_NAME.0).unwrap()[..6], &millis[2..]);
    let undated = reader
        .lifecycle([reader.sole_line(b"8=FIX.4.4|35=0|10=0|", false).unwrap()])
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        &bytes(&undated, UUID_TAG_NAME.0).unwrap()[..8],
        &[0, 0, 0, 0, 0, 0, 0x70, 0],
    );
}

#[test]
fn the_instrument_identity_is_the_same_across_spellings_and_venues() {
    let registry = registry();
    let reader = FixCodec::new(Arc::clone(&registry));
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut identity = |line: &[u8]| {
        bytes(
            &life.fill(reader.sole_line(line, false).unwrap()).unwrap(),
            INSTUUID_TAG_NAME.0,
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
                .map(|line| reader.sole_line(line, false).unwrap()),
        )
        .map(|held| held.unwrap())
        .collect();
    let twice: Vec<FixMsg> = reader
        .lifecycle(once.clone())
        .map(|held| held.unwrap())
        .collect();
    for (first, second) in once.iter().zip(&twice) {
        for tag in [INSTUUID_TAG_NAME.0, UUID_TAG_NAME.0, PUUID_TAG_NAME.0] {
            assert_eq!(bytes(first, tag), bytes(second, tag), "tag {tag}");
        }
        assert_eq!(first.entries().len(), second.entries().len());
        assert_eq!(first, second);
    }
}

/// Keep each clock's declared resolution, including instants outside the
/// nanosecond timestamp range a FIX wire field normally has.
fn row_message(
    registry: Arc<FixRegistry>,
    cells: impl IntoIterator<Item = (i32, Scalar)>,
) -> FixMsg {
    let mut fields = Vec::new();
    let mut values = Vec::new();
    for (tag, value) in std::iter::once((35, Scalar::from("D"))).chain(cells) {
        let name = registry.get_field_by_tag(tag).unwrap().name();
        let mut field = value.dtype().unwrap().nullable_field(name);
        field.as_fix_mut().set_tag(tag).unwrap();
        fields.push(field);
        values.push(value);
    }
    let field = DataType::from_fields(fields).unwrap().required_field("D");
    FixMsg::with_registry(registry, field, Scalar::from_sequence(values)).unwrap()
}

fn micros(instant: i64) -> Scalar {
    Scalar::datetime64(instant, TimeUnit::Microsecond, Timezone::UTC).unwrap()
}

#[test]
fn uuid_payloads_keep_the_original_inputs_and_effective_instrument_scope() {
    let registry = registry();
    let codec = FixCodec::new(Arc::clone(&registry));
    let original = codec.sole_line(LIFE[0], false).unwrap();
    let arrival_digest = original.digest();
    let entries = original.entries().to_vec();
    let message = FixLifecycle::new(registry).fill(original).unwrap();
    let raw_instrument = yggdryl::xxhash::xxh128(b"XNAS\x1f\x1fAAPL\x1fUSD\x1f");
    let instuuid = Uuid::from_v8(raw_instrument);
    assert_ne!(
        raw_instrument,
        instuuid.get(),
        "this fixture exercises the replaced bits"
    );
    assert_eq!(
        message.by_tag(INSTUUID_TAG_NAME.0).unwrap(),
        &Scalar::Uuid(instuuid)
    );
    let chain_input = [
        [0x01].as_slice(),
        instuuid.into_bytes().as_slice(),
        b"\x1fA1",
    ]
    .concat();
    let chain_payload = yggdryl::xxhash::xxh3(&chain_input);
    let impact = 1_767_348_930_000_000;
    assert_eq!(
        bytes(&message, PUUID_TAG_NAME.0),
        Some(Uuid::from_v7(impact, chain_payload).unwrap().into_bytes()),
    );
    assert_eq!(
        bytes(&message, UUID_TAG_NAME.0),
        Some(
            Uuid::from_v7(impact, yggdryl::xxhash::xxh3(&arrival_digest.to_be_bytes()))
                .unwrap()
                .into_bytes()
        ),
    );
    assert_eq!(message.entries(), entries);
    assert_eq!(message.digest(), arrival_digest);
    assert_eq!(message.into_bytes(b'|'), LIFE[0]);
}

#[test]
fn uuid_clock_precedence_is_transaction_then_sending_then_market_then_epoch() {
    let registry = registry();
    for (clocks, expected) in [
        (
            vec![(60, 1_001), (52, 2_002), (TIMESTAMP_TAG_NAME.0, 3_003)],
            1_001,
        ),
        (vec![(52, 2_002), (TIMESTAMP_TAG_NAME.0, 3_003)], 2_002),
        (vec![(TIMESTAMP_TAG_NAME.0, 3_003)], 3_003),
        (Vec::new(), 0),
    ] {
        let row = row_message(
            Arc::clone(&registry),
            clocks.into_iter().map(|(tag, time)| (tag, micros(time))),
        );
        let expected =
            Uuid::from_v7(expected, yggdryl::xxhash::xxh3(&row.digest().to_be_bytes())).unwrap();
        let message = FixLifecycle::new(Arc::clone(&registry)).fill(row).unwrap();
        assert_eq!(
            bytes(&message, UUID_TAG_NAME.0),
            Some(expected.into_bytes())
        );
    }
}

#[test]
fn lifecycle_uuid_order_keeps_each_microsecond_across_a_millisecond_boundary() {
    let registry = registry();
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut previous = None;
    for time in 0..=1_001 {
        let row = row_message(Arc::clone(&registry), [(60, micros(time))]);
        let message = life.fill(row).unwrap();
        let current = bytes(&message, UUID_TAG_NAME.0).unwrap();
        if let Some(previous) = previous {
            assert!(previous < current, "microsecond {time}");
        }
        previous = Some(current);
    }
    assert_eq!(life.alive(), 0);
}

#[test]
fn invalid_uuid_instants_neither_open_join_nor_close_a_chain() {
    let registry = registry();
    for invalid in [i64::MIN, -1, 281_474_976_710_656_000, i64::MAX] {
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        let make = |key: &str, time| {
            row_message(
                Arc::clone(&registry),
                [(11, Scalar::from(key)), (60, micros(time))],
            )
        };
        let error = life.fill(make("NEW", invalid)).unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { ref path, .. } if path == "$.uuid"));
        assert!(error.to_string().contains(&invalid.to_string()));
        assert_eq!(life.alive(), 0);
        let first = life.fill(make("LIVE", 0)).unwrap();
        let joining = row_message(
            Arc::clone(&registry),
            [
                (11, Scalar::from("LIVE")),
                (526, Scalar::from("NEW")),
                (39, Scalar::from("2")),
                (60, micros(invalid)),
            ],
        );
        assert!(life.fill(joining).is_err());
        assert_eq!(
            life.alive(),
            1,
            "a refused terminal message cannot close the live chain"
        );
        let second = life.fill(make("NEW", 1)).unwrap();
        assert_eq!(
            life.alive(),
            2,
            "a refused join cannot attach its new alias"
        );
        assert_ne!(
            bytes(&first, PUUID_TAG_NAME.0),
            bytes(&second, PUUID_TAG_NAME.0)
        );
    }
}

#[test]
fn invalid_new_puuid_is_located_without_rechecking_stated_uuids() {
    let registry = registry();
    let uuid = Uuid::from_v7(0, 7).unwrap();
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let message = row_message(
        Arc::clone(&registry),
        [
            (11, Scalar::from("NEW")),
            (60, micros(-1)),
            (UUID_TAG_NAME.0, Scalar::Uuid(uuid)),
        ],
    );
    assert!(
        matches!(life.fill(message), Err(Error::InvalidRecord { path, .. }) if path == "$.puuid")
    );
    assert_eq!(life.alive(), 0);
    let stated = row_message(
        registry,
        [
            (60, micros(-1)),
            (UUID_TAG_NAME.0, Scalar::Uuid(uuid)),
            (PUUID_TAG_NAME.0, Scalar::Uuid(uuid)),
            (INSTUUID_TAG_NAME.0, Scalar::Uuid(Uuid::from_v8(123))),
        ],
    );
    assert_eq!(life.fill(stated.clone()).unwrap(), stated);
    assert_eq!(life.alive(), 1);
}

#[test]
fn a_refused_message_column_does_not_publish_the_planned_chain() {
    let registry = registry();
    let mut message_registry = registry.as_ref().clone();
    assert!(message_registry.remove(UUID_TAG_NAME.0).is_some());
    let mut field = DataType::Int32.nullable_field(UUID_TAG_NAME.1);
    field.as_fix_mut().set_tag(UUID_TAG_NAME.0).unwrap();
    let key = registry.get_field_by_tag(11).unwrap().clone();
    let msgtype = registry.get_field_by_tag(35).unwrap().clone();
    let row = DataType::from_fields([field, key, msgtype])
        .unwrap()
        .required_field("D");
    let message = FixMsg::with_registry(
        Arc::new(message_registry),
        row,
        Scalar::from_sequence([Scalar::Null, Scalar::from("NEW"), Scalar::from("D")]),
    )
    .unwrap();
    let mut life = FixLifecycle::new(registry);
    assert!(matches!(
        life.fill(message),
        Err(Error::InvalidRecord { .. })
    ));
    assert_eq!(life.alive(), 0);
}

#[test]
fn the_committed_capture_keeps_its_direct_count_and_each_doors_full_replay() {
    let codec = super::dataset::codec();
    let lines = super::dataset::text_lines();
    assert_eq!(lines.len(), 129);
    let mut direct_life = FixLifecycle::new(Arc::clone(codec.registry()));
    let mut enriched_life = FixLifecycle::new(Arc::clone(codec.registry()));
    let mut direct_trace = Vec::new();
    let mut enriched_trace = Vec::new();
    let mut messages = 0;
    for (index, line) in lines.iter().enumerate() {
        for message in codec.parse_text_line(line).expect("a capture line reads") {
            let message = message.expect("a capture message reads");
            let enriched = codec
                .enrich_message(message.clone())
                .expect("the capture message enriches");
            for (life, message, trace) in [
                (&mut direct_life, message, &mut direct_trace),
                (&mut enriched_life, enriched, &mut enriched_trace),
            ] {
                let entries = message.entries().to_vec();
                let digest = message.digest();
                let wire = message.into_bytes(b'|');
                let stamped = life.fill(message).expect("the capture message stamps");
                if [101, 102].contains(&(index + 1)) {
                    assert!(stamped.get_by_tag(35).is_none());
                    assert!(stamped.get_by_tag(PUUID_TAG_NAME.0).is_none());
                }
                assert_eq!(stamped.entries(), entries);
                assert_eq!(stamped.digest(), digest);
                assert_eq!(stamped.into_bytes(b'|'), wire);
                trace.push((stamped, life.alive()));
            }
            messages += 1;
        }
    }
    assert_eq!(messages, 83);
    assert_eq!(direct_life.alive(), 4);
    // Line 73's derived terminal state closes ABBN.S. The untyped FIXML
    // at line 101 no longer reopens it through a hard-tag fallback.
    assert_eq!(enriched_life.alive(), 3);
    // Each door replays its own projection: enrichment may end a chain at a
    // different message and the untyped FIXML supplies no identifiers.
    for (trace, expected) in [(direct_trace, 4), (enriched_trace, 3)] {
        assert_eq!(trace.len(), 83);
        let mut replay = FixLifecycle::new(Arc::clone(codec.registry()));
        for (message, alive) in trace {
            let stamped = replay
                .fill(message.clone())
                .expect("the stamped capture message replays");
            assert_eq!(stamped, message);
            assert_eq!(stamped.entries(), message.entries());
            assert_eq!(stamped.digest(), message.digest());
            assert_eq!(stamped.into_bytes(b'|'), message.into_bytes(b'|'));
            assert_eq!(replay.alive(), alive);
        }
        assert_eq!(replay.alive(), expected);
    }
}

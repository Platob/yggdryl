use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::{UInt32Type, UInt64Type};
use arrow_array::{
    Array, ArrayRef, Date32Array, Date64Array, FixedSizeBinaryArray, Int32Array, Int64Array,
    RecordBatch, StringArray, StructArray, TimestampMicrosecondArray, TimestampMillisecondArray,
    TimestampNanosecondArray, TimestampSecondArray, UInt64Array,
};
use arrow_schema::{DataType as ArrowDataType, Schema, TimeUnit as ArrowTimeUnit};

use super::{accepts_time, column_txhashes, compose, decompose, row_txhashes, unix_array};
use crate::txhash::{TxHash, TxHasher};
use crate::xxhash::Xxh3;
use crate::xxhash::arrow::{column_digests, row_digests};
use crate::{ArrowCastOptions, DataType, DigestAlgorithm, Field, Scalar, TimeUnit, Timezone};

const INSTANTS: [i64; 3] = [
    1_700_000_000_000_000,
    1_700_000_000_000_001,
    1_700_000_000_000_000,
];
const UNIT: TimeUnit = TimeUnit::Microsecond;

fn struct_root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields).unwrap().required_field("row")
}

fn batch(fields: &[Field], columns: Vec<ArrayRef>) -> RecordBatch {
    let fields = fields
        .iter()
        .cloned()
        .map(Field::into_arrow)
        .collect::<crate::Result<Vec<_>>>()
        .unwrap();
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap()
}

fn event_field() -> Field {
    Field::new(
        "event",
        DataType::DateTime64 {
            unit: UNIT,
            timezone: Timezone::UTC,
        },
        false,
    )
}

fn events() -> ArrayRef {
    Arc::new(TimestampMicrosecondArray::from(INSTANTS.to_vec()).with_timezone("UTC"))
}

fn symbols() -> ArrayRef {
    Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"]))
}

fn quantities() -> ArrayRef {
    Arc::new(Int64Array::from(vec![100, 250, 100]))
}

/// A coupled holder of the given width, naming its instant.
fn coupled(name: &str, width: i32, time: &str) -> Field {
    let mut field = Field::new(name, DataType::FixedSizeBinary(width), false);
    field.as_digest_mut().set_holder().unwrap();
    field.as_digest_mut().set_time(time).unwrap();
    field
}

/// Read a coupled column back as the values it holds.
fn values(array: &dyn Array, unit: TimeUnit, algorithm: DigestAlgorithm) -> Vec<Option<TxHash>> {
    let array = array
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .expect("a coupled column is fixed-size binary");
    (0..array.len())
        .map(|row| {
            array
                .is_valid(row)
                .then(|| TxHash::from_bytes(unit, algorithm, array.value(row)).unwrap())
        })
        .collect()
}

#[test]
fn unix_array_reads_timestamps_dates_and_integers() {
    let micros = |array: &dyn Array| unix_array(array, UNIT).unwrap();
    assert_eq!(
        micros(&TimestampSecondArray::from(vec![Some(1), None])).values(),
        &[1_000_000, 0]
    );
    assert!(micros(&TimestampSecondArray::from(vec![Some(1), None])).is_null(1));
    assert_eq!(
        micros(&TimestampMillisecondArray::from(vec![1_500]).with_timezone("Asia/Kolkata"))
            .values(),
        &[1_500_000],
        "a zone moves nothing: the count already names the instant"
    );
    assert_eq!(
        micros(&TimestampMicrosecondArray::from(vec![7])).values(),
        &[7]
    );
    assert_eq!(
        micros(&TimestampNanosecondArray::from(vec![1_999, -1, -1_001])).values(),
        &[1, -1, -2],
        "a finer count floors"
    );
    assert_eq!(
        micros(&Date32Array::from(vec![1])).values(),
        &[86_400_000_000]
    );
    assert_eq!(
        micros(&Date64Array::from(vec![1_500])).values(),
        &[1_500_000]
    );
    assert_eq!(micros(&Int64Array::from(vec![42])).values(), &[42]);
    assert_eq!(micros(&Int32Array::from(vec![-42])).values(), &[-42]);
    assert_eq!(micros(&UInt64Array::from(vec![42])).values(), &[42]);
    assert_eq!(
        unix_array(
            &TimestampMicrosecondArray::from(vec![1_999_999]),
            TimeUnit::Second
        )
        .unwrap()
        .values(),
        &[1]
    );

    assert!(unix_array(&UInt64Array::from(vec![u64::MAX]), UNIT).is_err());
    assert!(unix_array(&StringArray::from(vec!["2024-01-01"]), UNIT).is_err());
    assert!(unix_array(&Int64Array::from(vec![1]), TimeUnit::Day).is_err());
    assert!(
        unix_array(
            &TimestampSecondArray::from(vec![i64::MAX]),
            TimeUnit::Nanosecond
        )
        .is_err()
    );
}

#[test]
fn row_txhashes_couple_row_digests_with_instants() {
    let fields = [event_field(), Field::new("symbol", DataType::Utf8, false)];
    let rows = batch(&fields, vec![events(), symbols()]);
    for algorithm in DigestAlgorithm::ALL {
        let coupled = row_txhashes(&rows, events().as_ref(), UNIT, algorithm).unwrap();
        assert_eq!(
            coupled.data_type(),
            &ArrowDataType::FixedSizeBinary(
                i32::try_from(crate::txhash::width(algorithm)).unwrap()
            )
        );
        assert_eq!(coupled.null_count(), 0);
        let digests = row_digests(&rows, algorithm).unwrap();
        let (instants, split) = decompose(coupled.as_ref(), UNIT, algorithm).unwrap();
        assert_eq!(
            &split, &digests,
            "{algorithm}: the digest half is the row digest"
        );
        assert_eq!(
            instants.as_ref(),
            events().as_ref(),
            "{algorithm}: the instant half is the column"
        );
        // The instant column feeds the digest like any other column, so the
        // rows at one instant with one symbol agree and the rest differ.
        let read = values(coupled.as_ref(), UNIT, algorithm);
        assert_eq!(read[0], read[2], "{algorithm}");
        assert_ne!(read[0], read[1], "{algorithm}");
        let expected = Scalar::from_sequence([
            Scalar::from_datetime(INSTANTS[0], UNIT, Timezone::UTC).unwrap(),
            Scalar::from("AAPL"),
        ])
        .txhash(INSTANTS[0], algorithm);
        assert_eq!(read[0], Some(expected), "{algorithm}");
    }
}

#[test]
fn the_instant_column_need_not_be_a_column_of_the_batch() {
    let fields = [Field::new("symbol", DataType::Utf8, false)];
    let rows = batch(&fields, vec![symbols()]);
    let coupled = row_txhashes(&rows, events().as_ref(), UNIT, DigestAlgorithm::Xxh3).unwrap();
    let read = values(coupled.as_ref(), UNIT, DigestAlgorithm::Xxh3);
    assert_eq!(
        read[1],
        Some(
            Scalar::from_sequence([Scalar::from("MSFT")])
                .txhash(INSTANTS[1], DigestAlgorithm::Xxh3)
        )
    );
    // Rows at different instants with one content differ only in front.
    let first = read[0].unwrap().into_bytes();
    let third = read[2].unwrap().into_bytes();
    assert_eq!(&first[8..], &third[8..]);
    assert_eq!(read[0].unwrap().digest(), read[2].unwrap().digest());

    let short = TimestampMicrosecondArray::from(vec![1]);
    assert!(row_txhashes(&rows, &short, UNIT, DigestAlgorithm::Xxh3).is_err());
}

#[test]
fn a_null_instant_is_a_null_cell() {
    let fields = [Field::new("symbol", DataType::Utf8, false)];
    let rows = batch(&fields, vec![symbols()]);
    let times = TimestampMicrosecondArray::from(vec![Some(1), None, Some(3)]);
    let coupled = row_txhashes(&rows, &times, UNIT, DigestAlgorithm::Xxh3).unwrap();
    assert_eq!(coupled.null_count(), 1);
    assert!(coupled.is_null(1));
    assert!(coupled.is_valid(0) && coupled.is_valid(2));
    let column = column_txhashes(
        &times,
        symbols(),
        &Field::new("symbol", DataType::Utf8, false),
        UNIT,
        DigestAlgorithm::Xxh3,
    )
    .unwrap();
    assert!(column.is_null(1));
}

#[test]
fn column_txhashes_couple_cell_digests() {
    let field = Field::new("symbol", DataType::Utf8, false);
    let coupled = column_txhashes(
        events().as_ref(),
        symbols(),
        &field,
        UNIT,
        DigestAlgorithm::Xxh3,
    )
    .unwrap();
    let digests = column_digests(symbols(), &field, DigestAlgorithm::Xxh3).unwrap();
    let (instants, split) = decompose(coupled.as_ref(), UNIT, DigestAlgorithm::Xxh3).unwrap();
    assert_eq!(&split, &digests);
    assert_eq!(instants.as_ref(), events().as_ref());
    assert_eq!(
        values(coupled.as_ref(), UNIT, DigestAlgorithm::Xxh3)[0],
        Some(Scalar::from("AAPL").txhash(INSTANTS[0], DigestAlgorithm::Xxh3))
    );
    // Reconciliation is the column digest's: an int32 column read under an
    // int64 declaration is the same numbers.
    let declared = Field::new("quantity", DataType::Int64, false);
    let narrow: ArrayRef = Arc::new(Int32Array::from(vec![100, 250, 100]));
    assert_eq!(
        &column_txhashes(
            events().as_ref(),
            narrow,
            &declared,
            UNIT,
            DigestAlgorithm::Xxh3
        )
        .unwrap(),
        &column_txhashes(
            events().as_ref(),
            quantities(),
            &declared,
            UNIT,
            DigestAlgorithm::Xxh3
        )
        .unwrap()
    );
    assert!(
        column_txhashes(
            &TimestampMicrosecondArray::from(vec![1]),
            symbols(),
            &field,
            UNIT,
            DigestAlgorithm::Xxh3
        )
        .is_err()
    );
}

#[test]
fn compose_and_decompose_are_inverses_at_every_width() {
    let fields = [Field::new("symbol", DataType::Utf8, false)];
    let rows = batch(&fields, vec![symbols()]);
    let times = TimestampMillisecondArray::from(vec![Some(1_000), None, Some(3_000)]);
    for algorithm in DigestAlgorithm::ALL {
        let digests = row_digests(&rows, algorithm).unwrap();
        let coupled = compose(&times, digests.as_ref(), TimeUnit::Second, algorithm).unwrap();
        assert!(coupled.is_null(1), "{algorithm}");
        let (instants, split) = decompose(coupled.as_ref(), TimeUnit::Second, algorithm).unwrap();
        assert_eq!(
            instants.data_type(),
            &ArrowDataType::Timestamp(ArrowTimeUnit::Second, Some("UTC".into())),
            "{algorithm}"
        );
        assert!(
            instants.is_null(1) && split.is_null(1),
            "{algorithm}: nulls carry to both halves"
        );
        let instants = instants.as_primitive::<arrow_array::types::TimestampSecondType>();
        assert_eq!(instants.value(0), 1);
        assert_eq!(instants.value(2), 3);
        match algorithm {
            DigestAlgorithm::Xxh32 => assert_eq!(
                split.as_primitive::<UInt32Type>().value(0),
                digests.as_primitive::<UInt32Type>().value(0)
            ),
            DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => assert_eq!(
                split.as_primitive::<UInt64Type>().value(0),
                digests.as_primitive::<UInt64Type>().value(0)
            ),
            DigestAlgorithm::Xxh128 => assert_eq!(
                split.as_fixed_size_binary().value(0),
                digests.as_fixed_size_binary().value(0)
            ),
        }
        assert_eq!(
            &compose(instants, split.as_ref(), TimeUnit::Second, algorithm).unwrap(),
            &coupled,
            "{algorithm}: composing the halves is the column again"
        );
    }
}

#[test]
fn compose_reads_signed_digest_storage_as_the_same_bits() {
    let times = TimestampMicrosecondArray::from(vec![1]);
    let unsigned: ArrayRef = Arc::new(UInt64Array::from(vec![u64::MAX]));
    let signed: ArrayRef = Arc::new(Int64Array::from(vec![-1]));
    assert_eq!(
        &compose(&times, unsigned.as_ref(), UNIT, DigestAlgorithm::Xxh3).unwrap(),
        &compose(&times, signed.as_ref(), UNIT, DigestAlgorithm::Xxh3).unwrap()
    );
    let narrow: ArrayRef = Arc::new(Int32Array::from(vec![-1]));
    assert_eq!(
        values(
            compose(&times, narrow.as_ref(), UNIT, DigestAlgorithm::Xxh32)
                .unwrap()
                .as_ref(),
            UNIT,
            DigestAlgorithm::Xxh32
        )[0]
        .unwrap()
        .digest()
        .as_u32(),
        Some(u32::MAX)
    );
    // The wrong width for the algorithm is named, never reinterpreted.
    assert!(compose(&times, unsigned.as_ref(), UNIT, DigestAlgorithm::Xxh32).is_err());
    assert!(compose(&times, unsigned.as_ref(), UNIT, DigestAlgorithm::Xxh128).is_err());
    let wide: ArrayRef = Arc::new(FixedSizeBinaryArray::new(16, vec![0_u8; 16].into(), None));
    assert!(compose(&times, wide.as_ref(), UNIT, DigestAlgorithm::Xxh3).is_err());
    assert!(decompose(wide.as_ref(), UNIT, DigestAlgorithm::Xxh128).is_err());
    assert!(decompose(unsigned.as_ref(), UNIT, DigestAlgorithm::Xxh3).is_err());
    assert!(decompose(wide.as_ref(), TimeUnit::Day, DigestAlgorithm::Xxh3).is_err());
}

#[test]
fn a_seeded_hasher_answers_seeded_digests_over_columns() {
    let fields = [Field::new("symbol", DataType::Utf8, false)];
    let rows = batch(&fields, vec![symbols()]);
    let hasher = TxHasher::new(DigestAlgorithm::Xxh3).with_seed(7);
    let coupled = hasher.row_txhashes(&rows, events().as_ref()).unwrap();
    let mut expected = Xxh3::with_seed(7);
    expected.write_scalar(&Scalar::from_sequence([Scalar::from("MSFT")]));
    let read = values(coupled.as_ref(), UNIT, DigestAlgorithm::Xxh3);
    assert_eq!(read[1].unwrap().digest(), expected.as_digest());
    assert_eq!(read[1].unwrap().unix(), INSTANTS[1]);
    assert_ne!(
        &coupled,
        &row_txhashes(&rows, events().as_ref(), UNIT, DigestAlgorithm::Xxh3).unwrap()
    );

    let seconds = TxHasher::new_in(TimeUnit::Second, DigestAlgorithm::Xxh32).unwrap();
    let column = seconds
        .column_txhashes(events().as_ref(), symbols(), &fields[0])
        .unwrap();
    let read = values(column.as_ref(), TimeUnit::Second, DigestAlgorithm::Xxh32);
    assert_eq!(
        read[0].unwrap().unix(),
        1_700_000_000,
        "restated at the hasher's unit"
    );
    assert_eq!(
        read[0].unwrap().digest(),
        Scalar::from("AAPL").digest(DigestAlgorithm::Xxh32)
    );
}

#[test]
fn a_coupled_holder_is_filled_with_the_instant_in_front() {
    let symbol = Field::new("symbol", DataType::Utf8, false);
    let root = struct_root([event_field(), symbol.clone(), coupled("key", 16, "event")]);
    let source = batch(&[event_field(), symbol], vec![events(), symbols()]);

    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    assert_eq!(filled.num_columns(), 3);
    assert_eq!(
        filled.column(2).data_type(),
        &ArrowDataType::FixedSizeBinary(16)
    );
    let read = values(filled.column(2).as_ref(), UNIT, DigestAlgorithm::Xxh3);
    // Every field but the holder feeds the digest, the instant included, and
    // the instant leads the stored bytes.
    let expected = Scalar::from_sequence([
        Scalar::from_datetime(INSTANTS[1], UNIT, Timezone::UTC).unwrap(),
        Scalar::from("MSFT"),
    ])
    .txhash(INSTANTS[1], DigestAlgorithm::Xxh3);
    assert_eq!(read[1], Some(expected));
    assert_eq!(read[0].unwrap().unix(), INSTANTS[0]);

    // The same fill through the schema pipeline, and it is idempotent.
    let applied = root
        .apply_arrow_batch(&source, true, true, true, ArrowCastOptions::new())
        .unwrap();
    assert_eq!(applied, filled);
    assert_eq!(
        root.apply_arrow_batch(&applied, true, true, true, ArrowCastOptions::new())
            .unwrap(),
        applied
    );
    // The seeded fill couples the seeded digest.
    let seeded = TxHasher::new(DigestAlgorithm::Xxh3)
        .with_seed(7)
        .apply_arrow_batch(&root, source, false)
        .unwrap();
    let mut expected = Xxh3::with_seed(7);
    expected.write_scalar(&Scalar::from_sequence([
        Scalar::from_datetime(INSTANTS[1], UNIT, Timezone::UTC).unwrap(),
        Scalar::from("MSFT"),
    ]));
    let read = values(seeded.column(2).as_ref(), UNIT, DigestAlgorithm::Xxh3);
    assert_eq!(read[1].unwrap().digest(), expected.as_digest());
}

#[test]
fn a_coupled_holder_names_its_sources_unit_and_algorithm() {
    let symbol = Field::new("symbol", DataType::Utf8, false);
    let mut key = coupled("key", 24, "event");
    key.as_digest_mut().set_sources(["symbol"]).unwrap();
    key.as_digest_mut().set_unit(TimeUnit::Second).unwrap();
    key.as_digest_mut()
        .set_algorithm(DigestAlgorithm::Xxh128)
        .unwrap();
    let root = struct_root([event_field(), symbol.clone(), key]);
    let source = batch(&[event_field(), symbol], vec![events(), symbols()]);

    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    let read = values(
        filled.column(2).as_ref(),
        TimeUnit::Second,
        DigestAlgorithm::Xxh128,
    );
    assert_eq!(
        read[0].unwrap().unix(),
        1_700_000_000,
        "the instant floors to the declared unit"
    );
    assert_eq!(
        read[0].unwrap().digest(),
        Scalar::from_sequence([Scalar::from("AAPL")]).digest(DigestAlgorithm::Xxh128),
        "the sources narrow the digest and leave the instant out of it"
    );

    // Width alone resolves the algorithm: twelve bytes are XXH32.
    let narrow = struct_root([event_field(), symbol_field(), coupled("key", 12, "event")]);
    let filled = narrow.as_digest().apply_arrow_batch(&source).unwrap();
    let read = values(filled.column(2).as_ref(), UNIT, DigestAlgorithm::Xxh32);
    assert_eq!(
        read[2].unwrap().digest(),
        Scalar::from_sequence([
            Scalar::from_datetime(INSTANTS[2], UNIT, Timezone::UTC).unwrap(),
            Scalar::from("AAPL"),
        ])
        .digest(DigestAlgorithm::Xxh32)
    );

    // A declared algorithm wins over the one the width implies: sixteen
    // bytes hold XXH64 as well as XXH3-64.
    let mut wide = coupled("key", 16, "event");
    wide.as_digest_mut()
        .set_algorithm(DigestAlgorithm::Xxh64)
        .unwrap();
    let root = struct_root([event_field(), symbol_field(), wide]);
    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    let read = values(filled.column(2).as_ref(), UNIT, DigestAlgorithm::Xxh64);
    assert_eq!(
        read[2].unwrap().digest(),
        Scalar::from_sequence([
            Scalar::from_datetime(INSTANTS[2], UNIT, Timezone::UTC).unwrap(),
            Scalar::from("AAPL"),
        ])
        .digest(DigestAlgorithm::Xxh64)
    );
}

#[test]
fn force_recomputes_a_coupled_holder_and_the_holder_unit_wins_over_the_hasher() {
    let mut key = coupled("key", 16, "event");
    key.as_digest_mut().set_unit(TimeUnit::Nanosecond).unwrap();
    let root = struct_root([event_field(), symbol_field(), key]);
    let source = batch(&[event_field(), symbol_field()], vec![events(), symbols()]);
    // The holder's declared unit decides the stored resolution, not the
    // hasher's: the schema owns what a reader will find in the bytes.
    let hasher = TxHasher::new_in(TimeUnit::Second, DigestAlgorithm::Xxh3).unwrap();
    let filled = hasher.apply_arrow_batch(&root, source, false).unwrap();
    let nanos = |array: &dyn Array| values(array, TimeUnit::Nanosecond, DigestAlgorithm::Xxh3);
    let first = nanos(filled.column(2).as_ref());
    assert_eq!(first[1].unwrap().unix(), INSTANTS[1] * 1_000);

    // A filled cell is preserved when its sources change, unless the fill
    // is forced, which recomputes every visible row.
    let changed = RecordBatch::try_new(
        filled.schema(),
        vec![
            events(),
            Arc::new(StringArray::from(vec!["X", "Y", "Z"])),
            Arc::clone(filled.column(2)),
        ],
    )
    .unwrap();
    let kept = hasher
        .apply_arrow_batch(&root, changed.clone(), false)
        .unwrap();
    assert_eq!(nanos(kept.column(2).as_ref()), first);
    let forced = hasher.apply_arrow_batch(&root, changed, true).unwrap();
    let recomputed = nanos(forced.column(2).as_ref());
    assert_ne!(recomputed, first);
    assert_eq!(recomputed[1].unwrap().unix(), INSTANTS[1] * 1_000);
    assert_eq!(
        recomputed[1].unwrap().digest(),
        Scalar::from_sequence([
            Scalar::from_datetime(INSTANTS[1], UNIT, Timezone::UTC).unwrap(),
            Scalar::from("Y"),
        ])
        .digest(DigestAlgorithm::Xxh3)
    );
}

fn quantity_field() -> Field {
    Field::new("quantity", DataType::Int64, false)
}

#[test]
fn a_dotted_time_path_reads_an_instant_under_a_nested_struct() {
    let inner = DataType::from_fields([event_field(), symbol_field()]).unwrap();
    let meta = Field::new("meta", inner, true);
    let struct_fields = match meta.clone().into_arrow().unwrap().data_type() {
        ArrowDataType::Struct(fields) => fields.clone(),
        _ => unreachable!(),
    };
    let column: ArrayRef = Arc::new(StructArray::new(
        struct_fields,
        vec![events(), symbols()],
        Some(vec![true, false, true].into()),
    ));
    let source = batch(
        &[meta.clone(), quantity_field()],
        vec![column, quantities()],
    );

    let mut key = coupled("key", 16, "meta.event");
    key.set_nullable(true);
    let root = struct_root([meta.clone(), quantity_field(), key]);
    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    let read = values(filled.column(2).as_ref(), UNIT, DigestAlgorithm::Xxh3);
    assert_eq!(read[0].unwrap().unix(), INSTANTS[0]);
    assert_eq!(read[1], None, "a null Struct hides the instant under it");
    assert_eq!(read[2].unwrap().unix(), INSTANTS[2]);

    // The digest is what a plain holder over the same fields computes: the
    // whole nested Struct and the quantity, the instant in front.
    let mut plain = Field::new("key", DataType::UInt64, true);
    plain.as_digest_mut().set_holder().unwrap();
    let plain_root = struct_root([meta, quantity_field(), plain]);
    let plain_filled = plain_root.as_digest().apply_arrow_batch(&source).unwrap();
    let plain_digests = plain_filled.column(2).as_primitive::<UInt64Type>();
    for row in [0, 2] {
        assert_eq!(
            read[row].unwrap().digest().as_u64(),
            Some(plain_digests.value(row)),
            "row {row}"
        );
    }
}

#[test]
fn an_instant_that_does_not_fit_the_holder_unit_is_refused_by_cell() {
    let seconds = Field::new(
        "event",
        DataType::DateTime64 {
            unit: TimeUnit::Second,
            timezone: Timezone::UTC,
        },
        false,
    );
    let mut key = coupled("key", 16, "event");
    key.as_digest_mut().set_unit(TimeUnit::Nanosecond).unwrap();
    let root = struct_root([seconds.clone(), symbol_field(), key]);
    let column: ArrayRef =
        Arc::new(TimestampSecondArray::from(vec![1, i64::MAX / 1_000, 3]).with_timezone("UTC"));
    let source = batch(&[seconds, symbol_field()], vec![column, symbols()]);
    let error = root
        .as_digest()
        .apply_arrow_batch(&source)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("key") && error.contains("row 1") && error.contains("ns"),
        "{error}"
    );
}

#[test]
fn accepts_time_agrees_with_the_column_reader() {
    let zoned = |unit| DataType::DateTime64 {
        unit,
        timezone: Timezone::UTC,
    };
    for dtype in [
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
        DataType::Date32,
        DataType::Date64,
        zoned(TimeUnit::Second),
        zoned(TimeUnit::Nanosecond),
        DataType::DateTime64 {
            unit: TimeUnit::Millisecond,
            timezone: Timezone::NAIVE,
        },
        DataType::Utf8,
        DataType::Float64,
        DataType::Boolean,
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Duration64(TimeUnit::Second),
    ] {
        let arrow = Field::new("x", dtype.clone(), true).into_arrow().unwrap();
        let empty = arrow_array::new_empty_array(arrow.data_type());
        assert_eq!(
            accepts_time(&dtype),
            unix_array(empty.as_ref(), UNIT).is_ok(),
            "{dtype}"
        );
    }
}

fn symbol_field() -> Field {
    Field::new("symbol", DataType::Utf8, false)
}

#[test]
fn the_instant_may_be_an_integer_or_date_column() {
    let stamp = Field::new("stamp", DataType::Int64, false);
    let root = struct_root([stamp.clone(), symbol_field(), coupled("key", 16, "stamp")]);
    let source = batch(&[stamp, symbol_field()], vec![quantities(), symbols()]);
    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    let read = values(filled.column(2).as_ref(), UNIT, DigestAlgorithm::Xxh3);
    assert_eq!(
        read[1].unwrap().unix(),
        250,
        "an integer is the count already"
    );

    let day = Field::new("day", DataType::Date32, false);
    let root = struct_root([day.clone(), symbol_field(), coupled("key", 16, "day")]);
    let source = batch(
        &[day, symbol_field()],
        vec![Arc::new(Date32Array::from(vec![1, 2, 3])), symbols()],
    );
    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    let read = values(filled.column(2).as_ref(), UNIT, DigestAlgorithm::Xxh3);
    assert_eq!(
        read[0].unwrap().unix(),
        86_400_000_000,
        "a date is that day's midnight"
    );
}

#[test]
fn a_null_instant_nulls_a_nullable_holder_and_refuses_a_required_one() {
    let event = Field::new(
        "event",
        DataType::DateTime64 {
            unit: UNIT,
            timezone: Timezone::UTC,
        },
        true,
    );
    let sparse: ArrayRef = Arc::new(
        TimestampMicrosecondArray::from(vec![Some(1), None, Some(3)]).with_timezone("UTC"),
    );
    let source = batch(&[event.clone(), symbol_field()], vec![sparse, symbols()]);

    let mut nullable = coupled("key", 16, "event");
    nullable.set_nullable(true);
    let root = struct_root([event.clone(), symbol_field(), nullable]);
    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    assert!(filled.column(2).is_null(1));
    assert!(filled.column(2).is_valid(0));
    // Filling again leaves the null unfilled and the values untouched.
    assert_eq!(root.as_digest().apply_arrow_batch(&filled).unwrap(), filled);

    let required = struct_root([event, symbol_field(), coupled("key", 16, "event")]);
    let refused = required.as_digest().apply_arrow_batch(&source).unwrap_err();
    assert!(refused.to_string().contains("row 1"), "{refused}");
    assert!(refused.to_string().contains("digest:time"), "{refused}");
}

#[test]
fn a_coupled_holder_under_a_null_struct_stays_untouched() {
    let inner = DataType::from_fields([event_field(), symbol_field(), coupled("key", 16, "event")])
        .unwrap();
    let nested = Field::new("nested", inner.clone(), true);
    let root = struct_root([nested.clone()]);
    let children: Vec<ArrayRef> = vec![
        events(),
        symbols(),
        Arc::new(FixedSizeBinaryArray::new(16, vec![0_u8; 48].into(), None)),
    ];
    let struct_fields = match nested.clone().into_arrow().unwrap().data_type() {
        ArrowDataType::Struct(fields) => fields.clone(),
        _ => unreachable!(),
    };
    let column: ArrayRef = Arc::new(StructArray::new(
        struct_fields,
        children,
        Some(vec![true, false, true].into()),
    ));
    let source = batch(&[nested], vec![column]);
    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    let nested = filled.column(0).as_struct();
    assert!(nested.is_null(1));
    let keys = nested.column(2);
    let read = values(keys.as_ref(), UNIT, DigestAlgorithm::Xxh3);
    assert_eq!(read[0].unwrap().unix(), INSTANTS[0]);
    assert_eq!(read[2].unwrap().unix(), INSTANTS[2]);
    // The hidden row keeps the bytes it arrived with.
    assert_eq!(keys.as_fixed_size_binary().value(1), &[0_u8; 16]);
}

#[test]
fn a_containing_holder_reads_a_nested_coupled_holder_as_its_bytes() {
    let inner = DataType::from_fields([event_field(), symbol_field(), coupled("key", 16, "event")])
        .unwrap();
    let nested = Field::new("nested", inner, false);
    let mut outer = Field::new("digest", DataType::UInt64, false);
    outer.as_digest_mut().set_holder().unwrap();
    outer.as_digest_mut().set_sources(["nested"]).unwrap();
    let root = struct_root([nested.clone(), outer]);
    let struct_fields = match nested.clone().into_arrow().unwrap().data_type() {
        ArrowDataType::Struct(fields) => fields.clone(),
        _ => unreachable!(),
    };
    let column: ArrayRef = Arc::new(StructArray::new(
        struct_fields,
        vec![
            events(),
            symbols(),
            Arc::new(FixedSizeBinaryArray::new(16, vec![0_u8; 48].into(), None)),
        ],
        None,
    ));
    let source = batch(&[nested], vec![column]);
    let filled = root.as_digest().apply_arrow_batch(&source).unwrap();
    let keys = filled.column(0).as_struct().column(2).clone();
    let inner_value = values(keys.as_ref(), UNIT, DigestAlgorithm::Xxh3)[0].unwrap();
    // The selected Struct carries one direct holder, so the outer digest
    // reads that holder's bytes rather than hashing the Struct again.
    let expected = Scalar::from_sequence([inner_value.into_scalar()]).digest(DigestAlgorithm::Xxh3);
    assert_eq!(
        filled.column(1).as_primitive::<UInt64Type>().value(0),
        expected.as_u64().unwrap()
    );
}

#[test]
fn coupling_declarations_that_cannot_be_filled_are_refused() {
    let source = batch(&[event_field(), symbol_field()], vec![events(), symbols()]);
    let refused = |root: Field| {
        root.as_digest()
            .apply_arrow_batch(&source)
            .unwrap_err()
            .to_string()
    };

    // A path naming no field.
    let missing = struct_root([event_field(), symbol_field(), coupled("key", 16, "arrival")]);
    let error = refused(missing);
    assert!(
        error.contains("digest:time") && error.contains("arrival"),
        "{error}"
    );

    // A path naming text.
    let text = struct_root([event_field(), symbol_field(), coupled("key", 16, "symbol")]);
    let error = refused(text);
    assert!(error.contains("datetime, date, or integer"), "{error}");

    // A path naming another holder.
    let mut other = Field::new("other", DataType::UInt64, false);
    other.as_digest_mut().set_holder().unwrap();
    let error = refused(struct_root([
        event_field(),
        symbol_field(),
        other,
        coupled("key", 16, "other"),
    ]));
    assert!(error.contains("holders are outputs"), "{error}");

    // A unit without an instant, and coupling metadata off a holder, written
    // raw where the typed setters would have refused.
    let mut unit_only = Field::new("key", DataType::FixedSizeBinary(16), false);
    unit_only.as_digest_mut().set_holder().unwrap();
    unit_only.as_digest_mut().insert("unit", "s").unwrap();
    let error = refused(struct_root([event_field(), symbol_field(), unit_only]));
    assert!(error.contains("digest:unit"), "{error}");
    let mut stray = symbol_field();
    stray.as_digest_mut().insert("time", "event").unwrap();
    let error = refused(struct_root([
        event_field(),
        stray,
        coupled("key", 16, "event"),
    ]));
    assert!(error.contains("belongs only to a digest holder"), "{error}");
    let mut stray_unit = symbol_field();
    stray_unit.as_digest_mut().insert("unit", "s").unwrap();
    let error = refused(struct_root([
        event_field(),
        stray_unit,
        coupled("key", 16, "event"),
    ]));
    assert!(
        error.contains("digest:unit belongs only to a digest holder"),
        "{error}"
    );

    // Coupling metadata under a collection is refused by path.
    let mut listed = coupled("key", 16, "event");
    listed.as_digest_mut().remove_time().unwrap();
    listed.as_digest_mut().remove_role().unwrap();
    listed.as_digest_mut().insert("time", "event").unwrap();
    let list = Field::new("list", DataType::list(listed), false);
    let error = struct_root([event_field(), symbol_field(), list])
        .as_digest()
        .apply_arrow_batch(&batch(
            &[event_field(), symbol_field()],
            vec![events(), symbols()],
        ))
        .unwrap_err()
        .to_string();
    assert!(error.contains("digest:time"), "{error}");
}

#[test]
fn a_coupled_holder_of_the_wrong_width_is_refused_by_name() {
    let mut odd = Field::new("key", DataType::FixedSizeBinary(20), false);
    odd.as_digest_mut().set_holder().unwrap();
    odd.as_digest_mut().insert("time", "event").unwrap();
    let root = struct_root([event_field(), symbol_field(), odd]);
    let source = batch(&[event_field(), symbol_field()], vec![events(), symbols()]);
    let error = root
        .as_digest()
        .apply_arrow_batch(&source)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("fixed_size_binary[12], [16], or [24]"),
        "{error}"
    );
}

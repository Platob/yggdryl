//! `rust/src/join.rs`: the join vocabulary - the kinds, the sides, the
//! options, the sources - and the semantics the engine keeps for every kind,
//! pinned through `Serie::join_with` on small inputs with nulls and
//! duplicates on both sides; then the two paths that answer the same rows
//! another way - a probe batch pruned by the build keys' range, a build side
//! past the spill bound joined in partitions.

use std::collections::HashMap;

use arrow_array::cast::AsArray;
use yggdryl::expression::{IntoJoinKeys, JoinKey, JoinKeys, Term};
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, JoinKind, JoinOptions, JoinSide, JoinSource,
    Scalar, Serie, SerieReader, SpillOptions, StructType,
};

fn record(name: &str, fields: Vec<Field>) -> Field {
    DataType::from(StructType::from_fields(fields).expect("distinct names")).required_field(name)
}

fn row(cells: impl IntoIterator<Item = Scalar>) -> Scalar {
    Scalar::from_sequence(cells)
}

fn int(value: i64) -> Scalar {
    Scalar::from(value)
}

fn text(value: &str) -> Scalar {
    Scalar::from(value)
}

/// The left side: `(id, name)`, a key repeated twice, a null key, a key the
/// right side lacks.
fn trades() -> Serie {
    let root = record(
        "trade",
        vec![
            DataType::Int64.nullable_field("id"),
            DataType::utf8().required_field("name"),
        ],
    );
    Serie::from_scalars(
        root,
        [
            row([int(1), text("a")]),
            row([int(2), text("b")]),
            row([int(2), text("b2")]),
            row([Scalar::Null, text("n")]),
            row([int(4), text("d")]),
        ],
    )
    .expect("the left side")
}

/// The right side: `(id, value)`, a key repeated twice, a null key, a key
/// the left side lacks, the matched key last.
fn values() -> Serie {
    let root = record(
        "value",
        vec![
            DataType::Int64.nullable_field("id"),
            DataType::Int64.required_field("value"),
        ],
    );
    Serie::from_scalars(
        root,
        [
            row([int(2), int(20)]),
            row([int(2), int(21)]),
            row([int(3), int(30)]),
            row([Scalar::Null, int(99)]),
            row([int(1), int(10)]),
        ],
    )
    .expect("the right side")
}

fn names(serie: &Serie) -> Vec<String> {
    serie
        .require_field()
        .expect("a column")
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .collect()
}

fn nullables(serie: &Serie) -> Vec<bool> {
    serie
        .require_field()
        .expect("a column")
        .fields()
        .iter()
        .map(Field::is_nullable)
        .collect()
}

/// The options every order-sensitive case runs under: the right side built,
/// so the rows come in left order whatever the two sides weigh.
fn right_built() -> JoinOptions {
    JoinOptions::new().with_build(Some(JoinSide::Right))
}

fn joined(how: JoinKind, options: &JoinOptions) -> Serie {
    trades()
        .join_with(&values(), "id", how, options)
        .expect("a join over one key")
}

// ---------------------------------------------------------------------------
// Vocabulary
// ---------------------------------------------------------------------------

#[test]
fn every_kind_reads_its_words_and_prints_one_way() {
    for (spelling, kind) in [
        ("inner", JoinKind::Inner),
        ("INNER JOIN", JoinKind::Inner),
        ("left", JoinKind::Left),
        ("left outer", JoinKind::Left),
        ("Left Outer Join", JoinKind::Left),
        ("right", JoinKind::Right),
        ("full", JoinKind::Full),
        ("outer", JoinKind::Full),
        ("full outer join", JoinKind::Full),
        ("semi", JoinKind::Semi),
        ("anti", JoinKind::Anti),
    ] {
        assert_eq!(spelling.parse::<JoinKind>().expect(spelling), kind);
        assert_eq!(
            kind.to_string().parse::<JoinKind>().expect("round trip"),
            kind
        );
    }
    assert_eq!(JoinKind::ALL.len(), 6);
    let refused = "cross".parse::<JoinKind>().expect_err("no cross join");
    assert!(refused.to_string().contains("cross"), "{refused}");
    assert_eq!("left".parse::<JoinSide>().expect("a side"), JoinSide::Left);
    assert_eq!(JoinSide::Right.other(), JoinSide::Left);
    assert!("middle".parse::<JoinSide>().is_err());
}

#[test]
fn the_kinds_state_which_unmatched_rows_they_keep() {
    assert!(!JoinKind::Inner.keeps_unmatched_left() && !JoinKind::Inner.keeps_unmatched_right());
    assert!(JoinKind::Left.keeps_unmatched_left() && !JoinKind::Left.keeps_unmatched_right());
    assert!(!JoinKind::Right.keeps_unmatched_left() && JoinKind::Right.keeps_unmatched_right());
    assert!(JoinKind::Full.keeps_unmatched_left() && JoinKind::Full.keeps_unmatched_right());
    assert!(JoinKind::Semi.is_filtering() && JoinKind::Anti.is_filtering());
    assert!(!JoinKind::Semi.emits_right() && JoinKind::Left.emits_right());
}

#[test]
fn the_options_default_as_the_docs_say_and_each_setter_moves_one_fact() {
    let options = JoinOptions::new();
    assert!(options.coalesce());
    assert_eq!(options.suffix(), yggdryl::DEFAULT_JOIN_SUFFIX);
    assert_eq!(options.build(), None);
    assert!(options.prune());
    assert!(options.spill().is_none());
    assert_eq!(options.pushdown_keys(), yggdryl::DEFAULT_PUSHDOWN_KEYS);
    assert_eq!(options, JoinOptions::default());
    let moved = options
        .clone()
        .with_coalesce(false)
        .with_suffix("_r")
        .with_build(Some(JoinSide::Left))
        .with_prune(false)
        .with_spill(SpillOptions::new().with_byte_size(SpillOptions::NEVER))
        .with_pushdown_keys(7);
    assert!(!moved.coalesce());
    assert_eq!(moved.suffix(), "_r");
    assert_eq!(moved.build(), Some(JoinSide::Left));
    assert!(!moved.prune());
    assert!(moved.spill().expect("a bound").is_never());
    assert_eq!(moved.pushdown_keys(), 7);
    assert_ne!(moved, options);
}

#[test]
fn a_source_names_its_root_and_whether_it_is_held() {
    let held = JoinSource::from(trades());
    assert!(held.is_held());
    assert_eq!(held.root().expect("a record root").name(), "trade");
    assert!(held.memory_size().is_some());

    // A plain column keys as the one child of a `row` record.
    let plain = JoinSource::from(
        Serie::from_scalars(DataType::Int64.required_field("id"), [int(1)]).expect("a column"),
    );
    let root = plain.root().expect("a record root");
    assert_eq!(root.name(), "row");
    assert_eq!(
        names(&Serie::empty(root.clone()).expect("an empty record")),
        ["id"]
    );

    let chunked = JoinSource::from(yggdryl::ChunkedSerie::from_serie(trades()).expect("one chunk"));
    assert!(chunked.is_held());
    let stream = JoinSource::from(yggdryl::SerieReader::from_serie(trades()).expect("a stream"));
    assert!(!stream.is_held());
    assert_eq!(stream.memory_size(), None);
    assert_eq!(stream.root().expect("the stream's root").name(), "trade");

    let run = JoinSource::from(Serie::new(vec![int(1)]));
    assert!(run.root().is_err(), "a run names no column to key by");
}

#[test]
fn the_keys_read_every_spelling_into_one_list() {
    let keys: JoinKeys = "id, venue = market, lower(a) = b"
        .parse()
        .expect("three keys");
    assert_eq!(keys.len(), 3);
    assert!(keys.keys()[0].is_using());
    assert_eq!(keys.keys()[0].using_column(), Some("id"));
    assert!(!keys.keys()[1].is_using());
    assert_eq!(keys.keys()[1].left().to_string(), "venue");
    assert_eq!(keys.keys()[1].right().to_string(), "market");
    assert_eq!(keys.to_string(), "id, venue = market, lower(a) = b");
    assert_eq!(
        keys.to_string().parse::<JoinKeys>().expect("round trip"),
        keys
    );

    assert_eq!(
        ["id", "venue = market", "lower(a) = b"]
            .into_join_keys()
            .expect("texts"),
        keys
    );
    assert_eq!(
        vec![
            (Term::column("id"), Term::column("id")),
            (Term::column("venue"), Term::column("market")),
            (
                "lower(a)".parse::<Term>().expect("a term"),
                Term::column("b")
            ),
        ]
        .into_join_keys()
        .expect("pairs"),
        keys
    );
    assert_eq!(
        Scalar::from("id, venue = market, lower(a) = b")
            .into_join_keys()
            .expect("a text scalar"),
        keys
    );
    let listed = Scalar::from_sequence([
        text("id"),
        Scalar::from_sequence([text("venue"), text("market")]),
        text("lower(a) = b"),
    ]);
    assert_eq!(listed.into_join_keys().expect("a list scalar"), keys);
    assert_eq!(
        JoinKey::using(Term::column("id"))
            .into_join_keys()
            .expect("one key")
            .len(),
        1
    );
    let (left, right) = (
        yggdryl::Selector::from_columns(["id", "venue"]),
        yggdryl::Selector::from_columns(["id", "market"]),
    );
    assert_eq!(
        (left, right)
            .into_join_keys()
            .expect("two selectors")
            .to_string(),
        "id, venue = market"
    );

    // Refusals name what failed.
    assert!(
        "id <> other".parse::<JoinKeys>().is_err(),
        "only an equality pairs two terms"
    );
    let uneven = (
        yggdryl::Selector::from_columns(["id"]),
        yggdryl::Selector::from_columns(["id", "venue"]),
    )
        .into_join_keys()
        .expect_err("uneven selectors");
    assert!(uneven.to_string().contains("as many"), "{uneven}");
    assert!(Scalar::from(1_i64).into_join_keys().is_err());
}

// ---------------------------------------------------------------------------
// Semantics, every kind against a hand-computed table
// ---------------------------------------------------------------------------

#[test]
fn an_inner_join_multiplies_duplicates_in_probe_then_build_order_and_skips_null_keys() {
    let joined = joined(JoinKind::Inner, &right_built());
    assert_eq!(names(&joined), ["id", "name", "value"]);
    assert_eq!(joined.require_field().expect("a record").name(), "trade");
    assert_eq!(
        joined.rows().to_vec(),
        vec![
            row([int(1), text("a"), int(10)]),
            row([int(2), text("b"), int(20)]),
            row([int(2), text("b"), int(21)]),
            row([int(2), text("b2"), int(20)]),
            row([int(2), text("b2"), int(21)]),
        ]
    );
}

#[test]
fn a_left_join_keeps_every_left_row_with_the_right_columns_null() {
    let joined = joined(JoinKind::Left, &right_built());
    assert_eq!(names(&joined), ["id", "name", "value"]);
    // The right side is optional, so its column is nullable; the left keeps
    // its own nullability.
    assert_eq!(nullables(&joined), [true, false, true]);
    assert_eq!(
        joined.rows().to_vec(),
        vec![
            row([int(1), text("a"), int(10)]),
            row([int(2), text("b"), int(20)]),
            row([int(2), text("b"), int(21)]),
            row([int(2), text("b2"), int(20)]),
            row([int(2), text("b2"), int(21)]),
            row([Scalar::Null, text("n"), Scalar::Null]),
            row([int(4), text("d"), Scalar::Null]),
        ]
    );
}

#[test]
fn a_right_join_appends_the_unmatched_build_rows_in_build_order_with_the_key_coalesced() {
    let joined = joined(JoinKind::Right, &right_built());
    assert_eq!(names(&joined), ["id", "name", "value"]);
    assert_eq!(nullables(&joined), [true, true, false]);
    assert_eq!(
        joined.rows().to_vec(),
        vec![
            row([int(1), text("a"), int(10)]),
            row([int(2), text("b"), int(20)]),
            row([int(2), text("b"), int(21)]),
            row([int(2), text("b2"), int(20)]),
            row([int(2), text("b2"), int(21)]),
            // The right rows nothing matched, in build order: the key is the
            // right side's, since the left is absent.
            row([int(3), Scalar::Null, int(30)]),
            row([Scalar::Null, Scalar::Null, int(99)]),
        ]
    );
}

#[test]
fn a_full_join_keeps_both_sides_unmatched_rows() {
    let joined = joined(JoinKind::Full, &right_built());
    assert_eq!(nullables(&joined), [true, true, true]);
    assert_eq!(
        joined.rows().to_vec(),
        vec![
            row([int(1), text("a"), int(10)]),
            row([int(2), text("b"), int(20)]),
            row([int(2), text("b"), int(21)]),
            row([int(2), text("b2"), int(20)]),
            row([int(2), text("b2"), int(21)]),
            row([Scalar::Null, text("n"), Scalar::Null]),
            row([int(4), text("d"), Scalar::Null]),
            row([int(3), Scalar::Null, int(30)]),
            row([Scalar::Null, Scalar::Null, int(99)]),
        ]
    );
}

#[test]
fn semi_and_anti_emit_left_rows_once_with_left_columns_only() {
    let semi = joined(JoinKind::Semi, &right_built());
    assert_eq!(names(&semi), ["id", "name"]);
    assert_eq!(
        semi.require_field().expect("a record"),
        trades().require_field().expect("a record")
    );
    assert_eq!(
        semi.rows().to_vec(),
        vec![
            row([int(1), text("a")]),
            row([int(2), text("b")]),
            row([int(2), text("b2")]),
        ]
    );
    let anti = joined(JoinKind::Anti, &right_built());
    assert_eq!(names(&anti), ["id", "name"]);
    assert_eq!(
        anti.rows().to_vec(),
        vec![row([Scalar::Null, text("n")]), row([int(4), text("d")])]
    );
}

#[test]
fn the_default_build_side_is_the_smaller_one_so_the_rows_come_in_the_other_side_s_order() {
    // The trades weigh less than the values, so they are built and the
    // values probe: the matched rows come in the values' order.
    assert!(trades().memory_size() < values().memory_size());
    let joined = joined(JoinKind::Inner, &JoinOptions::new());
    assert_eq!(
        joined
            .rows()
            .iter()
            .map(|row| row.as_sequence().expect("a row")[2].clone())
            .collect::<Vec<_>>(),
        vec![int(20), int(20), int(21), int(21), int(10)]
    );
}

#[test]
fn either_build_side_answers_the_same_rows_for_every_kind() {
    for how in JoinKind::ALL {
        let right_built = joined(how, &JoinOptions::new().with_build(Some(JoinSide::Right)));
        let left_built = joined(how, &JoinOptions::new().with_build(Some(JoinSide::Left)));
        assert_eq!(names(&right_built), names(&left_built), "{how}");
        let mut right_rows = right_built.rows().to_vec();
        let mut left_rows = left_built.rows().to_vec();
        right_rows.sort();
        left_rows.sort();
        assert_eq!(right_rows, left_rows, "{how}");
        if how.is_filtering() {
            // A filtering join never multiplies: at most one row per left row.
            assert!(left_rows.len() <= trades().len(), "{how}");
        }
    }
}

#[test]
fn coalesce_off_keeps_both_keys_and_a_collision_takes_the_suffix() {
    let kept = trades()
        .join_with(
            &values(),
            "id",
            JoinKind::Inner,
            &right_built().with_coalesce(false),
        )
        .expect("a join");
    assert_eq!(names(&kept), ["id", "name", "id_right", "value"]);
    assert_eq!(kept.rows()[0], row([int(1), text("a"), int(1), int(10)]));

    let suffixed = trades()
        .join_with(
            &values(),
            "id",
            JoinKind::Inner,
            &right_built().with_coalesce(false).with_suffix("_r"),
        )
        .expect("a join");
    assert_eq!(names(&suffixed), ["id", "name", "id_r", "value"]);

    // `id = id` is the same bare column on both sides, so it coalesces too;
    // a key over two different terms never does.
    let paired = trades()
        .join_with(&values(), "id = id", JoinKind::Inner, &right_built())
        .expect("a join");
    assert_eq!(names(&paired), ["id", "name", "value"]);
    let computed = trades()
        .join_with(&values(), "id = id + 0", JoinKind::Inner, &right_built())
        .expect("a join");
    assert_eq!(names(&computed), ["id", "name", "id_right", "value"]);
}

#[test]
fn a_coalesced_key_over_two_required_columns_stays_required() {
    let left = Serie::from_scalars(
        record("l", vec![DataType::Int64.required_field("id")]),
        [row([int(1)]), row([int(2)])],
    )
    .expect("left");
    let right = Serie::from_scalars(
        record(
            "r",
            vec![
                DataType::Int64.required_field("id"),
                DataType::utf8().required_field("v"),
            ],
        ),
        [row([int(2), text("two")])],
    )
    .expect("right");
    let inner = left
        .join_with(&right, "id", JoinKind::Inner, &right_built())
        .expect("a join");
    assert_eq!(nullables(&inner), [false, false]);
    assert_eq!(inner.rows().to_vec(), vec![row([int(2), text("two")])]);
    // Under a left join the right column turns optional, the key does not.
    let left_join = left
        .join_with(&right, "id", JoinKind::Left, &right_built())
        .expect("a join");
    assert_eq!(nullables(&left_join), [false, true]);
    assert_eq!(
        left_join.rows().to_vec(),
        vec![row([int(1), Scalar::Null]), row([int(2), text("two")])]
    );
}

#[test]
fn keys_of_two_datatypes_meet_at_their_common_one_and_computed_keys_bind_on_each_side() {
    let left = Serie::from_scalars(
        record(
            "l",
            vec![
                DataType::Int32.required_field("id"),
                DataType::utf8().required_field("venue"),
            ],
        ),
        [
            row([Scalar::from(1_i32), text("xnas")]),
            row([Scalar::from(2_i32), text("XNYS")]),
        ],
    )
    .expect("left");
    let right = Serie::from_scalars(
        record(
            "r",
            vec![
                DataType::Int64.required_field("id"),
                DataType::utf8().required_field("mic"),
            ],
        ),
        [row([int(2), text("xnys")]), row([int(1), text("xnas")])],
    )
    .expect("right");
    let joined = left
        .join_with(
            &right,
            "id, lower(venue) = mic",
            JoinKind::Inner,
            &right_built(),
        )
        .expect("int32 meets int64, lower(venue) meets mic");
    assert_eq!(names(&joined), ["id", "venue", "mic"]);
    assert_eq!(
        joined.rows().to_vec(),
        vec![
            row([Scalar::from(1_i32), text("xnas"), text("xnas")]),
            row([Scalar::from(2_i32), text("XNYS"), text("xnys")]),
        ]
    );
}

#[test]
fn a_plain_column_joins_as_the_one_child_of_a_row_record() {
    let ids = Serie::from_scalars(
        DataType::Int64.required_field("id"),
        [int(2), int(1), int(9)],
    )
    .expect("ids");
    let joined = ids
        .join_with(&values(), "id", JoinKind::Inner, &right_built())
        .expect("a join");
    assert_eq!(joined.require_field().expect("a record").name(), "row");
    assert_eq!(names(&joined), ["id", "value"]);
    assert_eq!(
        joined.rows().to_vec(),
        vec![
            row([int(2), int(20)]),
            row([int(2), int(21)]),
            row([int(1), int(10)])
        ]
    );
}

#[test]
fn refusals_come_before_any_row_and_name_what_failed() {
    let empty = trades()
        .join_with(
            &values(),
            JoinKeys::default(),
            JoinKind::Inner,
            &JoinOptions::new(),
        )
        .expect_err("an empty key list");
    assert!(empty.to_string().contains("empty key list"), "{empty}");

    let missing = trades()
        .join_with(&values(), "venue", JoinKind::Inner, &JoinOptions::new())
        .expect_err("a term naming no column");
    assert!(missing.to_string().contains("venue"), "{missing}");

    let run = Serie::new(vec![int(1)])
        .join_with(&values(), "id", JoinKind::Inner, &JoinOptions::new())
        .expect_err("a run on the left");
    assert!(run.to_string().contains("run"), "{run}");

    let nested = Serie::from_scalars(
        record(
            "l",
            vec![
                DataType::from(
                    StructType::from_fields([DataType::Int64.required_field("a")]).expect("fields"),
                )
                .required_field("id"),
            ],
        ),
        [row([row([int(1)])])],
    )
    .expect("a struct key");
    let unshared = nested
        .join_with(&values(), "id", JoinKind::Inner, &JoinOptions::new())
        .expect_err("a struct against an int64 shares no datatype");
    let message = unshared.to_string();
    assert!(
        message.contains("int64") && message.contains("share no datatype"),
        "{message}"
    );

    let unnest = trades()
        .join_with(
            &values(),
            "unnest(id)",
            JoinKind::Inner,
            &JoinOptions::new(),
        )
        .expect_err("an unnest in a key");
    assert!(!unnest.to_string().is_empty());
}

#[test]
fn every_kind_over_an_empty_side_answers_what_the_table_says() {
    let empty_right =
        Serie::empty(values().require_field().expect("a record").clone()).expect("empty");
    assert_eq!(joined_with(JoinKind::Inner, &empty_right).len(), 0);
    assert_eq!(
        joined_with(JoinKind::Left, &empty_right).len(),
        trades().len()
    );
    assert_eq!(joined_with(JoinKind::Right, &empty_right).len(), 0);
    assert_eq!(
        joined_with(JoinKind::Full, &empty_right).len(),
        trades().len()
    );
    assert_eq!(joined_with(JoinKind::Semi, &empty_right).len(), 0);
    assert_eq!(
        joined_with(JoinKind::Anti, &empty_right).len(),
        trades().len()
    );
    let left_rows = joined_with(JoinKind::Left, &empty_right);
    assert_eq!(names(&left_rows), ["id", "name", "value"]);
    assert_eq!(left_rows.rows()[0], row([int(1), text("a"), Scalar::Null]));
}

fn joined_with(how: JoinKind, right: &Serie) -> Serie {
    trades()
        .join_with(right, "id", how, &right_built())
        .expect("a join over an empty side")
}

// ---------------------------------------------------------------------------
// Pruning: a probe batch outside the build keys' range is never hashed
// ---------------------------------------------------------------------------

/// A chunked side of `(id, <payload>)` rows, one chunk per key list, every
/// payload cell naming its chunk and row.
fn chunked_side(name: &str, payload: &str, chunks: &[&[Option<i64>]]) -> ChunkedSerie {
    let root = record(
        name,
        vec![
            DataType::Int64.nullable_field("id"),
            DataType::utf8().required_field(payload),
        ],
    );
    let series: Vec<Serie> = chunks
        .iter()
        .enumerate()
        .map(|(chunk, keys)| {
            Serie::from_scalars(
                root.clone(),
                keys.iter().enumerate().map(|(at, key)| {
                    row([
                        key.map_or(Scalar::Null, int),
                        text(&format!("{payload}{chunk}.{at}")),
                    ])
                }),
            )
            .expect("a chunk")
        })
        .collect();
    ChunkedSerie::from_series(None, series, ArrowCastOptions::new()).expect("chunks of one field")
}

/// The left side's chunks against the right side's key range `[1, 18]`:
/// inside it, outside it, across it, and absent.
const LEFT_KEYS: [&[Option<i64>]; 4] = [
    &[Some(10), Some(12), None, Some(15)],
    &[Some(100), Some(101), None],
    &[Some(5), Some(8), Some(12), Some(25)],
    &[None, None],
];

/// The right side's chunks against the left side's key range `[5, 101]`:
/// inside it, outside it, across it, and absent.
const RIGHT_KEYS: [&[Option<i64>]; 4] = [
    &[Some(10), Some(12), Some(12), None, Some(18)],
    &[Some(1), Some(2)],
    &[Some(3), Some(15)],
    &[None],
];

/// Where the values of a record chunk's `name` column lie.
fn name_buffer(chunk: &Serie) -> *const u8 {
    let batch = chunk.into_arrow_batch().expect("a record chunk");
    batch
        .column_by_name("name")
        .expect("a name column")
        .as_string::<i32>()
        .values()
        .as_ptr()
}

#[test]
fn pruning_answers_the_rows_and_batches_hashing_every_probe_row_answers() {
    let left = chunked_side("trade", "name", &LEFT_KEYS);
    let right = chunked_side("value", "value", &RIGHT_KEYS);
    for build in [JoinSide::Left, JoinSide::Right] {
        for how in JoinKind::ALL {
            let options = JoinOptions::new().with_build(Some(build));
            let pruned = left.join_with(&right, "id", how, &options).expect("pruned");
            let hashed = left
                .join_with(&right, "id", how, &options.clone().with_prune(false))
                .expect("every probe row hashed");
            assert_eq!(
                pruned.rows(),
                hashed.rows(),
                "{how}, the {build} side built"
            );
            assert_eq!(
                pruned.num_chunks(),
                hashed.num_chunks(),
                "{how}, the {build} side built"
            );
        }
    }
    // Keys 10, 12 (twice on the right) and 15 match, across the inside and
    // the overlapping chunks.
    let inner = left
        .join_with(&right, "id", JoinKind::Inner, &right_built())
        .expect("an inner join");
    assert_eq!(inner.len(), 6);
}

#[test]
fn a_probe_batch_the_build_keys_cannot_match_stands_alone_on_its_own_buffers() {
    let left = chunked_side("trade", "name", &LEFT_KEYS);
    let right = chunked_side("value", "value", &RIGHT_KEYS);
    for how in [JoinKind::Left, JoinKind::Full, JoinKind::Anti] {
        let pruned = left
            .join_with(&right, "id", how, &right_built())
            .expect("pruned");
        let hashed = left
            .join_with(&right, "id", how, &right_built().with_prune(false))
            .expect("every probe row hashed");
        // One output batch per probe chunk, then - for a full join - one per
        // build chunk holding an unmatched row.
        let drained = if how == JoinKind::Full { 4 } else { 0 };
        assert_eq!(pruned.num_chunks(), 4 + drained, "{how}");
        // The chunk of keys outside `[1, 18]` and the chunk of absent keys:
        // never hashed, so never gathered - the output batch is the probe
        // chunk's own rows, its buffers shared.
        for at in [1, 3] {
            let own = name_buffer(&left.chunks()[at]);
            assert_eq!(
                pruned.chunks()[at].rows().to_vec(),
                hashed.chunks()[at].rows().to_vec(),
                "{how}"
            );
            assert_eq!(name_buffer(&pruned.chunks()[at]), own, "{how}, chunk {at}");
            if how.emits_right() {
                // Hashing the same rows gathers them, a copy.
                assert_ne!(name_buffer(&hashed.chunks()[at]), own, "{how}, chunk {at}");
            } else {
                // An anti join keeping every row of a hashed chunk keeps the
                // chunk itself.
                assert_eq!(name_buffer(&hashed.chunks()[at]), own, "{how}, chunk {at}");
            }
        }
    }
    // A kind that keeps no unmatched probe row answers nothing for them.
    let inner = left
        .join_with(&right, "id", JoinKind::Inner, &right_built())
        .expect("an inner join");
    assert_eq!(
        inner.num_chunks(),
        2,
        "the inside and the overlapping chunks alone"
    );
}

// ---------------------------------------------------------------------------
// Partitioning: a build side past the spill bound
// ---------------------------------------------------------------------------

/// `rows` rows of `(id, <payload>)` in chunks of `chunk`: `id` cycling
/// through `keys` values, every `gap`th absent.
fn corpus(
    name: &str,
    payload: &str,
    rows: usize,
    keys: i64,
    gap: usize,
    chunk: usize,
) -> ChunkedSerie {
    let ids: Vec<Option<i64>> = (0..rows)
        .map(|at| (at % gap != 0).then(|| (at as i64 * 7) % keys))
        .collect();
    let chunks: Vec<&[Option<i64>]> = ids.chunks(chunk).collect();
    chunked_side(name, payload, &chunks)
}

/// How many times each row occurs: a join's rows as the multiset they are,
/// whatever order they came in.
// `Scalar` hashes its canonical content alone, never a cache.
#[allow(clippy::mutable_key_type)]
fn multiset(rows: Vec<Scalar>) -> HashMap<Scalar, usize> {
    let mut counts = HashMap::new();
    for row in rows {
        *counts.entry(row).or_insert(0) += 1;
    }
    counts
}

#[test]
fn a_build_side_past_the_spill_bound_joins_in_partitions_and_answers_the_same_rows() {
    // A few rows per key on either side, some keys on one side alone.
    let left = corpus("trade", "name", 2_000, 701, 13, 1_000);
    let right = corpus("value", "value", 1_500, 557, 11, 1_000);
    // Every byte passes a one-byte bound: both sides are partitioned and
    // every batch the join holds or answers is written to disk.
    let everything = JoinOptions::new().with_spill(SpillOptions::new().with_byte_size(1));
    for build in [JoinSide::Left, JoinSide::Right] {
        for how in JoinKind::ALL {
            let held = left
                .join_with(
                    &right,
                    "id",
                    how,
                    &JoinOptions::new().with_build(Some(build)),
                )
                .expect("held whole");
            let partitioned = left
                .join_with(
                    &right,
                    "id",
                    how,
                    &everything.clone().with_build(Some(build)),
                )
                .expect("partitioned");
            assert!(!held.is_empty(), "{how}, the {build} side built");
            // The rows come partition by partition: the same rows, in
            // another order.
            assert_eq!(
                multiset(partitioned.rows()),
                multiset(held.rows()),
                "{how}, the {build} side built"
            );
            assert!(
                partitioned.chunks().iter().all(Serie::is_spilled),
                "{how}, the {build} side built"
            );
            assert!(
                !held.chunks().iter().any(Serie::is_spilled),
                "{how}, the {build} side built"
            );
        }
    }
}

#[test]
fn keys_hashed_through_their_values_are_never_partitioned() {
    // The same key twice: `id` as windows-1252 text, which orders by the
    // characters its bytes decode to and so is hashed through its values,
    // and `n` as the integer it spells, hashed on the row format.
    let side = |name: &str, payload: &str, rows: usize, keys: usize| {
        let root = record(
            name,
            vec![
                DataType::cp1252().required_field("id"),
                DataType::Int64.required_field("n"),
                DataType::Int64.required_field(payload),
            ],
        );
        Serie::from_scalars(
            root,
            (0..rows).map(|at| {
                let key = (at * 7) % keys;
                row([text(&format!("k{key}")), int(key as i64), int(at as i64)])
            }),
        )
        .expect("a side")
    };
    let (left, right) = (side("l", "left", 300, 37), side("r", "right", 200, 23));
    let bounded = right_built().with_spill(SpillOptions::new().with_byte_size(1));
    let held = left
        .join_with(&right, "id", JoinKind::Full, &right_built())
        .expect("held whole");
    let past = left
        .join_with(&right, "id", JoinKind::Full, &bounded)
        .expect("past the bound");
    // Past the bound, and still the order of a build side hashed whole.
    assert_eq!(past.rows(), held.rows());
    assert!(past.is_spilled());
    // The integer key does pass into partitions, so its rows reorder.
    let held = left
        .join_with(&right, "n", JoinKind::Full, &right_built())
        .expect("held whole");
    let past = left
        .join_with(&right, "n", JoinKind::Full, &bounded)
        .expect("partitioned");
    assert_ne!(past.rows(), held.rows());
    assert_eq!(
        multiset(past.rows().to_vec()),
        multiset(held.rows().to_vec())
    );
}

// ---------------------------------------------------------------------------
// The merge path: two sides whose roots declare the key order ascending,
// every key the same datatype on both, walk each other with one cursor and
// build no table. The rows are the hash join's.
// ---------------------------------------------------------------------------

/// Every row of `serie`, sorted, so two joins compare as multisets.
fn sorted_rows(serie: &Serie) -> Vec<Scalar> {
    let mut rows = serie.rows().into_owned();
    rows.sort();
    rows
}

#[test]
fn sorted_sides_merge_and_answer_what_the_hash_join_answers_for_every_kind() {
    let left = trades().into_sort_by("id").expect("sorted");
    let right = values().into_sort_by("id").expect("sorted");
    assert!(left.declared_order().expect("read").is_some());
    assert!(right.declared_order().expect("read").is_some());
    for how in JoinKind::ALL {
        for build in [JoinSide::Left, JoinSide::Right] {
            let options = JoinOptions::new().with_build(Some(build));
            let merged = left
                .join_with(&right, "id", how, &options)
                .expect("a merge join");
            let hashed = trades()
                .join_with(&values(), "id", how, &options)
                .expect("a hash join");
            assert_eq!(
                sorted_rows(&merged),
                sorted_rows(&hashed),
                "{how} built {build}"
            );
            assert_eq!(names(&merged), names(&hashed), "{how} built {build}");
            // The probe's rows come out in the probe's order either way, so a
            // merge over sorted sides answers sorted rows where the hash
            // join answers the probe's: the merge's left-probed inner rows
            // are in key order.
            if how == JoinKind::Inner && build == JoinSide::Right {
                let ids = merged.child("id").expect("the key").rows().into_owned();
                let mut sorted = ids.clone();
                sorted.sort();
                assert_eq!(ids, sorted);
            }
        }
    }
}

#[test]
fn a_merge_reads_a_key_run_across_the_build_s_chunk_edge_and_steps_over_absent_keys() {
    // The build is two chunks of one declaring field, the run of `2` crossing
    // their edge and an absent key at each end; the probe a sorted serie.
    let right_root = record(
        "value",
        vec![
            DataType::Int64.nullable_field("id"),
            DataType::utf8().required_field("value"),
        ],
    );
    let mut declaring = right_root.clone();
    declaring
        .as_sort_mut()
        .set_by_texts(["id nulls first"])
        .expect("declared");
    let chunk =
        |rows: Vec<Scalar>| Serie::from_scalars(declaring.clone(), rows).expect("a sorted chunk");
    let right = ChunkedSerie::from_series(
        Some(&declaring),
        [
            chunk(vec![
                row([Scalar::Null, text("n")]),
                row([int(1), text("x")]),
                row([int(2), text("y1")]),
            ]),
            chunk(vec![row([int(2), text("y2")]), row([int(4), text("z")])]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two declaring chunks");
    assert!(right.declared_order().expect("read").is_some());
    let left = trades().into_sort_by("id nulls first").expect("sorted");
    let probe = ChunkedSerie::from_serie(left).expect("one chunk");
    for how in JoinKind::ALL {
        let mut merged = probe
            .join_with(&right, "id", how, &right_built())
            .expect("a merge join")
            .rows();
        merged.sort();
        let hashed = trades()
            .join_with(
                &right.into_serie().expect("joined"),
                "id",
                how,
                &right_built(),
            )
            .expect("a hash join");
        assert_eq!(merged, sorted_rows(&hashed), "{how}");
    }
    // Both ids `2` on the left meet both `2`s on the right: four pairs.
    let inner = probe
        .join_with(&right, "id", JoinKind::Inner, &right_built())
        .expect("inner");
    let twos = inner
        .rows()
        .iter()
        .filter(|joined| joined.get(0).is_some_and(|id| *id == int(2)))
        .count();
    assert_eq!(twos, 4);
}

#[test]
fn a_declaring_stream_probes_a_sorted_build_by_merging_and_a_cast_or_descending_key_hashes() {
    let left = trades().into_sort_by("id").expect("sorted");
    let right = values().into_sort_by("id").expect("sorted");
    let root = left.field().expect("a record").clone();
    let stream = SerieReader::from_arrow_reader(
        Some(&root),
        left.into_arrow_reader().expect("a reader"),
        ArrowCastOptions::new(),
    )
    .expect("a declaring stream");
    let merged: Vec<Scalar> = stream
        .join_with(right.clone(), "id", JoinKind::Left, &right_built())
        .expect("a merge over a stream")
        .flat_map(|batch| batch.expect("rows").rows().into_owned())
        .collect();
    let hashed = trades()
        .join_with(&values(), "id", JoinKind::Left, &right_built())
        .expect("a hash join");
    let mut merged_sorted = merged;
    merged_sorted.sort();
    assert_eq!(merged_sorted, sorted_rows(&hashed));

    // A descending declaration is not the order the walk needs; a key cast to
    // a common datatype may order otherwise. Both hash, and answer the same rows.
    let descending = trades().into_sort_by("id desc").expect("sorted");
    let joined = descending
        .join_with(&right, "id", JoinKind::Inner, &right_built())
        .expect("a hash join");
    assert_eq!(
        sorted_rows(&joined),
        sorted_rows(
            &trades()
                .join_with(&values(), "id", JoinKind::Inner, &right_built())
                .expect("inner")
        )
    );
    let narrow_root = record(
        "value",
        vec![
            DataType::Int32.nullable_field("id"),
            DataType::utf8().required_field("value"),
        ],
    );
    let narrow = Serie::from_scalars(
        narrow_root,
        [
            row([Scalar::from(2_i32), text("v")]),
            row([Scalar::from(4_i32), text("w")]),
        ],
    )
    .expect("an int32 side")
    .into_sort_by("id")
    .expect("sorted");
    let cast = trades()
        .into_sort_by("id")
        .expect("sorted")
        .join_with(&narrow, "id", JoinKind::Inner, &right_built())
        .expect("a hash join over a cast key");
    assert_eq!(cast.len(), 3);
}

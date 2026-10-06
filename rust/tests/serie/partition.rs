//! `rust/src/serie/partition.rs`: a stream's rows cut by a key into
//! partitions, held under the process spill bound, closed as they complete -
//! past a bound the lowest keys, clustered as soon as another key arrives,
//! the rest in key order when the stream ends.

use arrow_array::{Array, Int64Array};
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, PartitionOptions, Scalar, Serie, SerieReader,
    StructType,
};

fn root() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row")
}

fn batch(root: &Field, rows: &[(&str, i64)]) -> Serie {
    Serie::from_scalars(
        root.clone(),
        rows.iter().map(|(venue, qty)| {
            Scalar::from_sequence(vec![Scalar::from(*venue), Scalar::from(*qty)])
        }),
    )
    .unwrap()
}

/// One stream of `batches` under `root`, a chunk per batch.
fn stream(root: &Field, batches: Vec<Serie>) -> SerieReader {
    SerieReader::from_chunked(
        ChunkedSerie::from_series(Some(root), batches, ArrowCastOptions::new()).unwrap(),
    )
    .unwrap()
}

fn venue(name: &str) -> Scalar {
    Scalar::from_sequence(vec![Scalar::from(name)])
}

/// Every partition `by` closes, in the order it closes: its key, and the
/// quantities of its rows in the order they are held.
fn closed(stream: SerieReader, by: &str, options: PartitionOptions) -> Vec<(Scalar, Vec<i64>)> {
    stream
        .partition_by(by, options)
        .unwrap()
        .map(|partition| {
            let (key, rows) = partition.unwrap().into_parts();
            let quantities = rows
                .into_arrow_reader()
                .unwrap()
                .flat_map(|batch| {
                    let batch = batch.unwrap();
                    let column = batch.column_by_name("qty").unwrap();
                    column
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .unwrap()
                        .values()
                        .to_vec()
                })
                .collect();
            (key, quantities)
        })
        .collect()
}

#[test]
fn an_unbounded_stream_holds_every_partition_and_closes_them_in_key_order() {
    let root = root();
    let batches = vec![
        batch(&root, &[("XPAR", 1), ("XNAS", 2)]),
        batch(&root, &[("XLON", 3), ("XNAS", 4)]),
    ];
    assert_eq!(
        closed(stream(&root, batches), "venue", PartitionOptions::new()),
        [
            (venue("XLON"), vec![3]),
            (venue("XNAS"), vec![2, 4]),
            (venue("XPAR"), vec![1]),
        ]
    );
}

#[test]
fn past_max_open_the_lowest_keys_close_and_a_returning_key_opens_a_new_piece() {
    let root = root();
    let batches = vec![
        batch(&root, &[("XNAS", 1), ("XLON", 2)]),
        batch(&root, &[("XPAR", 3)]),
        batch(&root, &[("XLON", 4)]),
    ];
    // The third venue closes the lowest open, XLON; XLON returning is the
    // lowest open again and closes at once; the rest close at the end.
    assert_eq!(
        closed(
            stream(&root, batches),
            "venue",
            PartitionOptions::new().with_max_open(2)
        ),
        [
            (venue("XLON"), vec![2]),
            (venue("XLON"), vec![4]),
            (venue("XNAS"), vec![1]),
            (venue("XPAR"), vec![3]),
        ]
    );
    // A bound of none is a bound of one.
    let options = PartitionOptions::new().with_max_open(0);
    assert_eq!(options.max_open(), Some(1));
}

#[test]
fn a_clustered_stream_closes_each_partition_once_another_key_arrives() {
    let root = root();
    let batches = vec![
        batch(&root, &[("XNAS", 1), ("XNAS", 2), ("XLON", 3)]),
        batch(&root, &[("XLON", 4), ("XPAR", 5)]),
        batch(&root, &[("XNAS", 6)]),
    ];
    // In arrival order, a run across a batch edge one partition, and a key
    // the stream returns to a second piece of it: pieces, never rows.
    let options = PartitionOptions::new().with_clustered(true);
    assert!(options.is_clustered());
    assert_eq!(
        closed(stream(&root, batches), "venue", options),
        [
            (venue("XNAS"), vec![1, 2]),
            (venue("XLON"), vec![3, 4]),
            (venue("XPAR"), vec![5]),
            (venue("XNAS"), vec![6]),
        ]
    );
}

#[test]
fn a_root_declaring_an_order_that_leads_with_the_key_is_clustered_untold() {
    let root = root();
    let rows = batch(&root, &[("XLON", 4), ("XNAS", 3), ("XPAR", 1), ("XNAS", 2)]);
    // Venue descending, then quantity: the declaration the sort writes,
    // proven where the rows land.
    let sorted = rows.into_sort_by("venue desc, qty").unwrap();
    let declaring = sorted.field().unwrap().clone();
    let chunks = || vec![sorted.slice(0, 2).unwrap(), sorted.slice(2, 2).unwrap()];
    // Clustered by the declaration: the partitions close in the order they
    // arrive, venue descending, where an undeclared stream closes in key
    // order.
    assert_eq!(
        closed(
            stream(&declaring, chunks()),
            "venue",
            PartitionOptions::new()
        ),
        [
            (venue("XPAR"), vec![1]),
            (venue("XNAS"), vec![2, 3]),
            (venue("XLON"), vec![4]),
        ]
    );
    let undeclared = declaring.clone().with_metadata_removed("SORT:by");
    let plain = || {
        vec![
            sorted
                .slice(0, 2)
                .unwrap()
                .cast(&undeclared, ArrowCastOptions::new())
                .unwrap(),
            sorted
                .slice(2, 2)
                .unwrap()
                .cast(&undeclared, ArrowCastOptions::new())
                .unwrap(),
        ]
    };
    assert_eq!(
        closed(
            stream(&undeclared, plain()),
            "venue",
            PartitionOptions::new()
        ),
        [
            (venue("XLON"), vec![4]),
            (venue("XNAS"), vec![2, 3]),
            (venue("XPAR"), vec![1]),
        ]
    );
    // A key the order does not lead with is not clustered by it.
    let by_qty = closed(stream(&declaring, chunks()), "qty", PartitionOptions::new());
    let keys: Vec<Scalar> = by_qty.into_iter().map(|(key, _)| key).collect();
    let qty = |value: i64| Scalar::from_sequence(vec![Scalar::from(value)]);
    assert_eq!(keys, [qty(1), qty(2), qty(3), qty(4)]);
}

#[test]
fn partitions_cut_on_many_threads_are_the_ones_cut_on_one() {
    let root = root();
    let venues = ["XNAS", "XLON", "XPAR", "XAMS", "XETR"];
    let batches = || {
        (0..40_i64)
            .map(|index| {
                let rows: Vec<(&str, i64)> = (0..25_i64)
                    .map(|row| {
                        let position = index * 25 + row;
                        (venues[(position % 5) as usize], position)
                    })
                    .collect();
                batch(&root, &rows)
            })
            .collect::<Vec<_>>()
    };
    for options in [
        PartitionOptions::new(),
        PartitionOptions::new().with_max_open(2),
        PartitionOptions::new().with_clustered(true),
    ] {
        let one = closed(stream(&root, batches()), "venue", options.with_threads(1));
        let many = closed(stream(&root, batches()), "venue", options.with_threads(8));
        assert_eq!(one, many, "{options:?}");
        let rows: usize = one.iter().map(|(_, quantities)| quantities.len()).sum();
        assert_eq!(rows, 1_000, "{options:?}");
    }
}

#[test]
fn a_key_the_root_cannot_bind_is_refused_before_a_batch_is_pulled() {
    let root = root();
    let refused = stream(&root, vec![batch(&root, &[("XNAS", 1)])])
        .partition_by("missing", PartitionOptions::new())
        .err()
        .expect("an unbound key");
    assert!(refused.to_string().contains("missing"), "{refused}");
    let partitions = stream(&root, vec![batch(&root, &[("XNAS", 1)])])
        .partition_by("venue", PartitionOptions::new())
        .unwrap();
    assert_eq!(partitions.field(), &root);
}

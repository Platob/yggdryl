//! `rust/src/serie_slice.rs`: a window over a serie that reads and writes
//! through the serie's own implementation, every index window-relative.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, StringArray};
use yggdryl::{
    ArrowCastOptions, DataType, Error, Field, Scalar, Serie, SerieSlice, SerieWindows, SortOptions,
};

fn i64s(values: &[i64]) -> Vec<Scalar> {
    values.iter().map(|value| Scalar::from(*value)).collect()
}

/// Six prices as an int64 column holding its buffer alone.
fn column(values: Vec<Option<i64>>) -> Serie {
    let nullable = values.iter().any(Option::is_none);
    let array: ArrayRef = Arc::new(Int64Array::from(values));
    let field = Field::new("price", DataType::Int64, nullable);
    Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::new()).expect("an int64 column")
}

fn run(values: &[i64]) -> Serie {
    Serie::new(i64s(values))
}

fn hashed(value: &impl Hash) -> u64 {
    let mut state = DefaultHasher::new();
    value.hash(&mut state);
    state.finish()
}

#[test]
fn a_window_reads_through_the_serie_window_relative_on_both_leaves() {
    let column = column(vec![Some(1), Some(2), None, Some(4), Some(5), Some(6)]);
    let run = Serie::new(vec![
        Scalar::from(1_i64),
        Scalar::from(2_i64),
        Scalar::Null,
        Scalar::from(4_i64),
        Scalar::from(5_i64),
        Scalar::from(6_i64),
    ]);
    for serie in [&column, &run] {
        let window = serie.window(1, 3).expect("a window");
        assert_eq!(
            (window.len(), window.offset(), window.is_empty()),
            (3, 1, false)
        );
        assert!(std::ptr::eq(window.serie(), serie));
        assert_eq!(window.field(), serie.field());
        assert_eq!(window.scalar(0).expect("in range"), Scalar::from(2_i64));
        assert!(window.is_null(1).expect("in range"));
        assert_eq!(
            window.get(2).map(|row| row.into_owned()),
            Some(Scalar::from(4_i64))
        );
        assert_eq!(window.get(3), None);
        assert_eq!(window.null_count(), 1);
        assert_eq!(
            window.rows().to_vec(),
            vec![Scalar::from(2_i64), Scalar::Null, Scalar::from(4_i64)]
        );
        assert_eq!(window.iter().count(), 3);
        assert_eq!(
            window.iter().next_back().map(|row| row.into_owned()),
            Some(Scalar::from(4_i64))
        );
        assert_eq!((&window).into_iter().len(), 3);
        assert_eq!(
            window.dtype().expect("a datatype"),
            DataType::serie(Field::new("item", DataType::Int64, true))
        );
        assert!(window.memory_size() > 0);
        assert!(window.memory_size() <= serie.memory_size());

        // A narrower window rebases onto the serie.
        let narrower = window.window(2, 1).expect("a window");
        assert_eq!((narrower.offset(), narrower.len()), (3, 1));
        assert_eq!(narrower.scalar(0).expect("in range"), Scalar::from(4_i64));

        // The window as a serie is `slice`.
        assert_eq!(window.into_serie(), serie.slice(1, 3).expect("a slice"));
        assert_eq!(serie.window(6, 0).expect("the empty tail").len(), 0);
    }
    // A column window lends the buffer: the run only answers borrowed rows.
    let window = column.window(1, 3).expect("a window");
    assert!(matches!(window.rows(), std::borrow::Cow::Owned(_)));
    let window = run.window(1, 3).expect("a window");
    assert!(matches!(window.rows(), std::borrow::Cow::Borrowed(_)));
}

#[test]
fn every_edge_is_refused_naming_the_serie_and_both_counts() {
    let column = column(vec![Some(1), Some(2), Some(3)]);
    let refused = column.window(2, 2).unwrap_err().to_string();
    assert!(
        refused.contains("rows 2..4 reach past the 3 rows price holds"),
        "{refused}"
    );
    let refused = run(&[1]).window(0, 2).unwrap_err().to_string();
    assert!(
        refused.contains("reach past the 1 rows $ holds"),
        "{refused}"
    );
    let window = column.window(1, 2).expect("a window");
    let refused = window.scalar(2).unwrap_err().to_string();
    assert!(
        refused.contains("row 2 is past the 2 rows price holds"),
        "{refused}"
    );
    assert!(window.is_null(2).is_err());
    let refused = window.window(1, 2).unwrap_err().to_string();
    assert!(
        refused.contains("rows 1..3 reach past the 2 rows price holds"),
        "{refused}"
    );
    let mut column = column;
    let mut window = column.window_mut(1, 2).expect("a window");
    assert!(window.set(2, Scalar::from(0_i64)).is_err());
    assert!(window.swap(0, 2).is_err());
    let refused = window.splice(0..1, vec![]).unwrap_err().to_string();
    assert!(
        refused.contains("never grows or shrinks what it views: 0 rows cannot replace 1"),
        "{refused}"
    );
    let refused = window
        .splice(0..3, i64s(&[1, 2, 3]))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("rows 0..3 reach past the 2 rows price holds"),
        "{refused}"
    );
    let refused = window
        .as_taken(&Serie::new(vec![Scalar::from(0_u32)]))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("1 indices cannot rearrange 2 rows"),
        "{refused}"
    );
    // Nothing moved.
    assert_eq!(column.rows().to_vec(), i64s(&[1, 2, 3]));
}

#[test]
fn identity_is_the_window_rows_alone_like_a_serie() {
    let column = column(vec![Some(9), Some(1), Some(2), Some(9)]);
    let run = run(&[1, 2]);
    let window = column.window(1, 2).expect("a window");
    let other = run.window(0, 2).expect("a window");
    assert_eq!(window, other);
    assert_eq!(hashed(&window), hashed(&other));
    assert_eq!(hashed(&window), hashed(&run));
    assert!(window == run);
    assert!(run == window);
    assert!(window < column.window(2, 2).expect("a window"));
    assert_eq!(
        window.to_string(),
        "price[Int64(Int64(1)), Int64(Int64(2))]"
    );
    assert_eq!(other.to_string(), "$[Int64(Int64(1)), Int64(Int64(2))]");
    assert!(format!("{window:?}").contains("offset: 1"));
    let mut held = column.clone();
    let mutable = held.window_mut(1, 2).expect("a window");
    assert_eq!(mutable.to_string(), window.to_string());
    assert!(format!("{mutable:?}").starts_with("SerieSliceMut"));
    // The window is `Copy`.
    let copied: SerieSlice<'_> = window;
    assert_eq!(copied, window);
}

#[test]
fn the_reads_of_a_window_answer_what_the_sliced_serie_answers() {
    let column = column(vec![Some(9), Some(3), Some(1), Some(3), None, Some(0)]);
    let window = column.window(1, 4).expect("a window");
    assert!(!window.is_sorted(SortOptions::default()));
    assert!(!window.is_unique());
    assert_eq!(window.unique_count(), 3);
    assert_eq!(
        window
            .sort_indices(SortOptions::default())
            .expect("an order")
            .rows()
            .to_vec(),
        vec![
            Scalar::from(1_u32),
            Scalar::from(0_u32),
            Scalar::from(2_u32),
            Scalar::from(3_u32)
        ]
    );
    assert_eq!(
        window
            .into_sorted(SortOptions::default())
            .expect("sorted")
            .rows()
            .to_vec(),
        vec![
            Scalar::from(1_i64),
            Scalar::from(3_i64),
            Scalar::from(3_i64),
            Scalar::Null
        ]
    );
    assert_eq!(
        window.into_unique().expect("unique").rows().to_vec(),
        vec![Scalar::from(3_i64), Scalar::from(1_i64), Scalar::Null]
    );
    assert_eq!(
        window.into_reversed().rows().to_vec(),
        vec![
            Scalar::Null,
            Scalar::from(3_i64),
            Scalar::from(1_i64),
            Scalar::from(3_i64)
        ]
    );
    assert_eq!(
        window
            .into_taken(&Serie::new(vec![Scalar::from(1_u32)]))
            .expect("taken")
            .rows()
            .to_vec(),
        vec![Scalar::from(1_i64)]
    );
    let mask = Serie::new(vec![
        Scalar::from(true),
        Scalar::from(false),
        Scalar::from(false),
        Scalar::from(true),
    ]);
    assert_eq!(
        window
            .into_filtered(&mask)
            .expect("filtered")
            .rows()
            .to_vec(),
        vec![Scalar::from(3_i64), Scalar::Null]
    );
    let keys = Serie::new(vec![
        Scalar::from("a"),
        Scalar::from("b"),
        Scalar::from("a"),
        Scalar::from("b"),
    ]);
    let groups = window.partition_by(&keys).expect("groups");
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0].1.rows().to_vec(),
        vec![Scalar::from(3_i64), Scalar::from(3_i64)]
    );
    // The same through the mutable window.
    let mut held = column.clone();
    let mutable = held.window_mut(1, 4).expect("a window");
    assert_eq!(mutable.unique_count(), 3);
    assert_eq!(mutable.as_window().len(), 4);
    assert_eq!(mutable.into_serie(), window.into_serie());
    assert_eq!(mutable.partition_by(&keys).expect("groups").len(), 2);
    assert_eq!(mutable.len(), 4);
    assert_eq!(mutable.offset(), 1);
    assert!(!mutable.is_empty());
    assert_eq!(mutable.field(), column.field());
    assert_eq!(mutable.null_count(), 1);
    assert_eq!(mutable.scalar(0).expect("in range"), Scalar::from(3_i64));
    assert_eq!(mutable.get(4), None);
    assert_eq!(mutable.iter().count(), 4);
    assert_eq!(mutable.rows().len(), 4);
    assert!(mutable.dtype().is_ok());
    assert!(mutable.memory_size() > 0);
    assert_eq!(mutable.window(1, 1).expect("a window").offset(), 2);
    assert!(mutable.is_null(3).expect("in range"));
    assert!(!mutable.is_sorted(SortOptions::default()));
    assert!(!mutable.is_unique());
    assert_eq!(
        mutable
            .sort_indices(SortOptions::default())
            .expect("an order")
            .len(),
        4
    );
    assert_eq!(
        mutable
            .into_sorted(SortOptions::default())
            .expect("sorted")
            .len(),
        4
    );
    assert_eq!(mutable.into_unique().expect("unique").len(), 3);
    assert_eq!(mutable.into_reversed().len(), 4);
    assert_eq!(
        mutable
            .into_taken(&Serie::new(vec![Scalar::from(0_u32)]))
            .expect("taken")
            .len(),
        1
    );
    assert_eq!(mutable.into_filtered(&mask).expect("filtered").len(), 2);
    assert!(std::ptr::eq(mutable.serie(), &held));
}

#[test]
fn every_write_goes_through_the_serie_on_the_rebased_range_and_never_past_the_window() {
    let mut column = column(vec![Some(1), Some(2), Some(3), Some(4), Some(5)]);
    let mut window = column.window_mut(1, 3).expect("a window");
    window.set(0, Scalar::from(20_i64)).expect("one slot");
    window.swap(0, 2).expect("two slots");
    assert_eq!(window.rows().to_vec(), i64s(&[4, 3, 20]));
    window.splice(1..3, i64s(&[30, 40])).expect("two rows");
    assert_eq!(column.rows().to_vec(), i64s(&[1, 4, 30, 40, 5]));

    let mut window = column.window_mut(1, 3).expect("a window");
    window.fill(Scalar::from(7_i64)).expect("filled");
    assert_eq!(column.rows().to_vec(), i64s(&[1, 7, 7, 7, 5]));
    // A value the field refuses refuses the fill and leaves the column.
    let mut window = column.window_mut(1, 3).expect("a window");
    assert!(window.fill(Scalar::Null).is_err());
    assert!(window.set(0, Scalar::from("x")).is_err());
    assert_eq!(column.rows().to_vec(), i64s(&[1, 7, 7, 7, 5]));

    let source = run(&[8, 9, 10]);
    let mut window = column.window_mut(1, 3).expect("a window");
    window
        .copy_from(&source.window(0, 3).expect("a window"))
        .expect("copied");
    let refused = window
        .copy_from(&source.window(0, 2).expect("a window"))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("2 rows cannot replace 3"), "{refused}");
    assert_eq!(column.rows().to_vec(), i64s(&[1, 8, 9, 10, 5]));

    // A run is written the same way.
    let mut run = run(&[1, 2, 3, 4, 5]);
    let mut window = run.window_mut(1, 3).expect("a window");
    window
        .set(1, Scalar::from("mixed"))
        .expect("a run accepts any value");
    window.swap(0, 2).expect("two slots");
    assert_eq!(
        run.rows().to_vec(),
        vec![
            Scalar::from(1_i64),
            Scalar::from(4_i64),
            Scalar::from("mixed"),
            Scalar::from(2_i64),
            Scalar::from(5_i64)
        ]
    );
}

#[test]
fn the_window_writes_sort_reverse_and_rearrange_in_place_within_the_window() {
    // A primitive column holding its buffer alone sorts the window of its
    // native slice where it stands, absences gathered within the window.
    let mut column = column(vec![Some(9), None, Some(3), Some(1), Some(2), Some(0)]);
    column
        .window_mut(1, 4)
        .expect("a window")
        .as_sorted(SortOptions::default())
        .expect("sorted")
        .as_reversed()
        .expect("reversed");
    let leaf = column.as_int64().expect("an int64 column");
    let read: Vec<Option<i64>> = (0..6).map(|index| leaf.value(index)).collect();
    assert_eq!(
        read,
        vec![Some(9), None, Some(3), Some(2), Some(1), Some(0)]
    );
    column
        .window_mut(1, 4)
        .expect("a window")
        .as_sorted(SortOptions::ascending().with_nulls_first(true))
        .expect("sorted");
    let leaf = column.as_int64().expect("an int64 column");
    let read: Vec<Option<i64>> = (0..6).map(|index| leaf.value(index)).collect();
    assert_eq!(
        read,
        vec![Some(9), None, Some(1), Some(2), Some(3), Some(0)]
    );
    column
        .window_mut(2, 3)
        .expect("a window")
        .as_taken(&Serie::new(vec![
            Scalar::from(2_u32),
            Scalar::from(0_u32),
            Scalar::from(1_u32),
        ]))
        .expect("rearranged");
    let leaf = column.as_int64().expect("an int64 column");
    let read: Vec<Option<i64>> = (0..6).map(|index| leaf.value(index)).collect();
    assert_eq!(
        read,
        vec![Some(9), None, Some(3), Some(1), Some(2), Some(0)]
    );

    // A string column writes the sorted rows back through splice.
    let venues: ArrayRef = Arc::new(StringArray::from(vec!["z", "c", "a", "b", "y"]));
    let mut venues = Serie::from_arrow_array(
        Some(&Field::new("venue", DataType::utf8(), false)),
        venues,
        ArrowCastOptions::new(),
    )
    .expect("a utf8 column");
    venues
        .window_mut(1, 3)
        .expect("a window")
        .as_sorted(SortOptions::default())
        .expect("sorted")
        .as_reversed()
        .expect("reversed");
    assert_eq!(
        venues.rows().to_vec(),
        vec![
            Scalar::from("z"),
            Scalar::from("c"),
            Scalar::from("b"),
            Scalar::from("a"),
            Scalar::from("y")
        ]
    );

    // A run sorts its values in place.
    let mut run = run(&[9, 3, 1, 2, 0]);
    run.window_mut(1, 3)
        .expect("a window")
        .as_sorted(SortOptions::descending())
        .expect("sorted");
    assert_eq!(run.rows().to_vec(), i64s(&[9, 3, 2, 1, 0]));
    run.window_mut(0, 5)
        .expect("a window")
        .as_reversed()
        .expect("reversed");
    assert_eq!(run.rows().to_vec(), i64s(&[0, 1, 2, 3, 9]));
}

#[test]
fn a_window_over_a_shared_column_copies_the_column_once_and_leaves_the_clone() {
    let column = column(vec![Some(2), Some(1), Some(3)]);
    let mut other = column.clone();
    other
        .window_mut(0, 2)
        .expect("a window")
        .as_sorted(SortOptions::default())
        .expect("sorted");
    assert_eq!(other.rows().to_vec(), i64s(&[1, 2, 3]));
    assert_eq!(column.rows().to_vec(), i64s(&[2, 1, 3]));
}

/// The venues XNAS, XNAS, XNAS, XNYS, XNYS, XNAS, XNAS as a utf8 column.
fn venues() -> Serie {
    let array: ArrayRef = Arc::new(StringArray::from(vec![
        "XNAS", "XNAS", "XNAS", "XNYS", "XNYS", "XNAS", "XNAS",
    ]));
    let field = Field::new("venue", DataType::utf8(), false);
    Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::new()).expect("a utf8 column")
}

/// Each window as its key, its offset and its length.
fn cuts(windows: &SerieWindows<'_>) -> Vec<(Scalar, usize, usize)> {
    windows
        .iter()
        .map(|(key, window)| (key, window.offset(), window.len()))
        .collect()
}

/// Each window as its key and its rows.
fn keyed_rows(windows: &SerieWindows<'_>) -> Vec<(Scalar, Vec<Scalar>)> {
    windows
        .iter()
        .map(|(key, window)| (key, window.rows().into_owned()))
        .collect()
}

#[test]
fn a_window_windows_by_its_own_rows_over_the_serie() {
    let venues = venues();
    let window = venues.window(1, 4).expect("a window");
    let owner = window.window_by("venue", false).expect("windows");
    let windows: Vec<_> = owner.iter().collect();
    // Every window is over the serie itself, at its offset in the serie.
    for (_, cut) in &windows {
        assert!(std::ptr::eq(cut.serie(), &venues));
    }
    let xnas = Scalar::from_sequence([Scalar::from("XNAS")]);
    let xnys = Scalar::from_sequence([Scalar::from("XNYS")]);
    // The XNAS run that opens before the window is cut at its edge, and the
    // one after it never reaches in.
    assert_eq!(
        cuts(&window.window_by("venue", false).expect("windows")),
        vec![(xnas.clone(), 1, 2), (xnys.clone(), 3, 2)]
    );
    assert_eq!(
        windows[0].1.rows().into_owned(),
        vec![Scalar::from("XNAS"); 2]
    );

    // What the serie's own windows answer, cut to the window.
    let restricted: Vec<_> = cuts(&venues.window_by("venue", false).expect("windows"))
        .into_iter()
        .filter_map(|(key, offset, len)| {
            let start = offset.max(window.offset());
            let end = (offset + len).min(window.offset() + window.len());
            (start < end).then_some((key, start, end - start))
        })
        .collect();
    assert_eq!(
        cuts(&window.window_by("venue", false).expect("windows")),
        restricted
    );

    // A window over the whole serie answers what the serie answers.
    let whole = venues.window(0, venues.len()).expect("the whole");
    assert_eq!(
        cuts(&whole.window_by("venue", false).expect("windows")),
        vec![(xnas.clone(), 0, 3), (xnys, 3, 2), (xnas, 5, 2)]
    );
    assert_eq!(
        cuts(&whole.window_by("venue", false).expect("windows")),
        cuts(&venues.window_by("venue", false).expect("windows"))
    );
    // An empty window cuts no window, and still binds first.
    let empty = venues.window(3, 0).expect("an empty window");
    for sorted in [false, true] {
        assert!(
            empty
                .window_by("venue", sorted)
                .expect("windows")
                .is_empty()
        );
        assert!(empty.window_by("tier", sorted).is_err());
    }

    // A mutable window answers what the read-only one does.
    let mut held = venues.clone();
    let mutable = held.window_mut(1, 4).expect("a window");
    assert_eq!(
        cuts(&mutable.window_by("venue", false).expect("windows")),
        cuts(&window.window_by("venue", false).expect("windows"))
    );

    // A window over a run has no field for a term to read, and the refusal
    // names the serie, sorted or not.
    let run = run(&[1, 1, 2]);
    for sorted in [false, true] {
        match run
            .window(1, 2)
            .expect("a window")
            .window_by("price", sorted)
        {
            Err(Error::InvalidRecord { path, reason }) => assert_eq!(
                (path.as_str(), reason.as_str()),
                ("$", "a schema-free run windows by no term")
            ),
            other => panic!("expected the run refused, got {other:?}"),
        }
        // So is a key naming no column, under the serie's name.
        let refused = window.window_by("*", sorted).unwrap_err().to_string();
        assert!(
            refused.contains("venue: expected at least one column to window by"),
            "{refused}"
        );
    }
}

#[test]
fn a_mutable_window_narrows_onto_the_same_serie() {
    let mut prices = column((0..8).map(Some).collect());
    {
        let mut outer = prices.window_mut(1, 6).expect("a window");
        let mut middle = outer.window_mut(1, 4).expect("a window of it");
        assert_eq!((middle.offset(), middle.len()), (2, 4));
        let mut inner = middle.window_mut(2, 2).expect("a window of that");
        // Three levels are one window of the serie, its offsets summed.
        assert_eq!((inner.offset(), inner.len()), (4, 2));
        assert_eq!(inner.scalar(0).expect("a row"), Scalar::from(4_i64));
        inner.set(1, Scalar::from(99_i64)).expect("a write");
        // A narrower window never reaches past the one it narrows.
        let refused = inner.window_mut(1, 2).unwrap_err().to_string();
        assert!(
            refused.contains("rows 1..3 reach past the 2 rows price holds"),
            "{refused}"
        );
        let refused = inner.window_mut(usize::MAX, 2).unwrap_err().to_string();
        assert!(
            refused.contains("reaches past the 2 rows price holds"),
            "{refused}"
        );
        // The parent reads the write once the narrower window is gone.
        assert_eq!(middle.scalar(3).expect("a row"), Scalar::from(99_i64));
    }
    assert_eq!(prices.rows().to_vec(), i64s(&[0, 1, 2, 3, 4, 99, 6, 7]));
}

#[test]
fn serie_windows_lends_its_windows_again_and_again() {
    let venues = venues();
    let windows = venues.window_by("venue", false).expect("windows");
    assert_eq!((windows.len(), windows.is_empty()), (3, false));
    // Walked twice, the same windows each time.
    assert_eq!(cuts(&windows), cuts(&windows));
    // And lent by reference in a `for`.
    let mut lengths = Vec::new();
    for (_, window) in &windows {
        assert!(std::ptr::eq(window.serie(), windows.serie()));
        lengths.push(window.len());
    }
    assert_eq!(lengths, [3, 2, 2]);

    // Each walk is exact and fused.
    let mut walk = windows.iter();
    assert_eq!(walk.size_hint(), (3, Some(3)));
    let copy = walk.clone();
    assert!(walk.next().is_some());
    assert_eq!((walk.len(), walk.size_hint()), (2, (2, Some(2))));
    assert!(walk.next().is_some());
    let (_, last) = walk.next().expect("a last window");
    assert_eq!((last.offset(), last.len()), (5, 2));
    assert_eq!(walk.len(), 0);
    assert!(walk.next().is_none());
    assert!(walk.next().is_none());
    // A clone walks from where it was taken, on its own.
    assert_eq!(copy.len(), 3);
    assert_eq!(copy.count(), 3);

    // A gathered owner lends the same way, and its walk is exact and fused.
    let mixed = column(vec![Some(2), Some(1), Some(2), Some(1)]);
    let gathered = mixed.window_by("price", true).expect("windows");
    assert_eq!((gathered.len(), gathered.is_empty()), (2, false));
    assert_eq!(cuts(&gathered), cuts(&gathered));
    let mut walk = (&gathered).into_iter();
    assert_eq!(walk.size_hint(), (2, Some(2)));
    assert!(walk.next().is_some());
    assert_eq!(walk.len(), 1);
    assert!(walk.next().is_some());
    assert!(walk.next().is_none());
    assert!(walk.next().is_none());
    // The owner clones with its gathered holder.
    let copy = gathered.clone();
    assert_eq!(keyed_rows(&copy), keyed_rows(&gathered));
}

#[test]
fn serie_windows_owns_a_gathered_holder_and_borrows_otherwise() {
    // Keys in order: sorted or not, the windows borrow the serie itself.
    let venues = column(vec![Some(1), Some(1), Some(2), None, None]);
    let unsorted = venues.window_by("price", false).expect("windows");
    let sorted = venues.window_by("price", true).expect("windows");
    assert!(std::ptr::eq(unsorted.serie(), &venues));
    assert!(std::ptr::eq(sorted.serie(), &venues));
    assert_eq!(cuts(&sorted), cuts(&unsorted));

    // Keys out of order: sorted gathers the rows once into key order -
    // ascending, absent keys last, the rows of one key in arrival order -
    // and every window views that copy, which the windows own.
    let prices = column(vec![Some(2), None, Some(1), Some(2), Some(1)]);
    let sorted = prices.window_by("price", true).expect("windows");
    assert!(!std::ptr::eq(sorted.serie(), &prices));
    assert_eq!(
        sorted.serie().rows().to_vec(),
        vec![
            Scalar::from(1_i64),
            Scalar::from(1_i64),
            Scalar::from(2_i64),
            Scalar::from(2_i64),
            Scalar::Null,
        ]
    );
    assert_eq!(sorted.serie().field(), prices.field());
    let key = |value: Scalar| Scalar::from_sequence([value]);
    assert_eq!(
        cuts(&sorted),
        vec![
            (key(Scalar::from(1_i64)), 0, 2),
            (key(Scalar::from(2_i64)), 2, 2),
            (key(Scalar::Null), 4, 1),
        ]
    );
    for (_, window) in &sorted {
        assert!(std::ptr::eq(window.serie(), sorted.serie()));
    }
    // The same answer as the unsorted cut of the rows taken in that order.
    let taken = prices
        .into_taken(
            &Serie::from_scalars(
                Field::new("index", DataType::UInt32, false),
                [2_u32, 4, 0, 3, 1].map(Scalar::from),
            )
            .expect("indices"),
        )
        .expect("taken");
    assert_eq!(
        keyed_rows(&sorted),
        keyed_rows(&taken.window_by("price", false).expect("windows"))
    );
}

#[test]
fn a_window_windows_sorted_over_its_own_rows() {
    let venues = venues();
    let xnas = Scalar::from_sequence([Scalar::from("XNAS")]);
    let xnys = Scalar::from_sequence([Scalar::from("XNYS")]);

    // Keys in order inside the window: sorted borrows the serie, at
    // absolute offsets, as unsorted does.
    let window = venues.window(1, 4).expect("a window");
    let sorted = window.window_by("venue", true).expect("windows");
    assert!(std::ptr::eq(sorted.serie(), &venues));
    assert_eq!(
        cuts(&sorted),
        cuts(&window.window_by("venue", false).expect("windows"))
    );
    assert_eq!(
        cuts(&sorted),
        vec![(xnas.clone(), 1, 2), (xnys.clone(), 3, 2)]
    );

    // Keys out of order: the gather takes the window's rows alone, and
    // every window lies in that copy from offset 0.
    let window = venues.window(2, 4).expect("a window");
    let sorted = window.window_by("venue", true).expect("windows");
    assert!(!std::ptr::eq(sorted.serie(), &venues));
    assert_eq!(sorted.serie().len(), 4);
    assert_eq!(cuts(&sorted), vec![(xnas, 0, 2), (xnys, 2, 2)]);
    assert_eq!(
        sorted.serie().rows().to_vec(),
        ["XNAS", "XNAS", "XNYS", "XNYS"].map(Scalar::from).to_vec()
    );
    // What the window's own rows answer as a serie of their own.
    assert_eq!(
        keyed_rows(&sorted),
        keyed_rows(
            &window
                .into_serie()
                .window_by("venue", true)
                .expect("windows")
        )
    );

    // A mutable window answers what the read-only one does.
    let mut held = venues.clone();
    let mutable = held.window_mut(2, 4).expect("a window");
    assert_eq!(
        keyed_rows(&mutable.window_by("venue", true).expect("windows")),
        keyed_rows(&sorted)
    );
}

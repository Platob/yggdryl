//! `rust/src/serie/datatype.rs`: the schema-free run a row is, and the serie
//! family's datatype it sits beside.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use yggdryl::{DataType, DataTypeId, Field, NestedValue, Run, Scalar, Serie, SortOptions, Value};

/// Two prices as a run.
fn prices() -> Run {
    Run::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)])
}

/// The prices `first..first + count`, each its own row number.
fn numbered(first: i64, count: i64) -> Vec<Scalar> {
    (first..first + count).map(Scalar::from).collect()
}

/// What `value` writes into a fresh hasher.
fn hashed(value: &impl Hash) -> u64 {
    let mut state = DefaultHasher::new();
    value.hash(&mut state);
    state.finish()
}

/// The run a serie holds; every serie here is one.
fn run_of(serie: &Serie) -> &Run {
    serie.as_run().expect("a run")
}

/// Whether `view`'s rows lie inside `holder`'s allocation.
fn lies_in(view: &Serie, holder: &Serie) -> bool {
    run_of(holder)
        .as_slice()
        .as_ptr_range()
        .contains(&run_of(view).as_slice().as_ptr())
}

#[test]
fn a_run_is_one_shared_slice_it_lends_as_it_is() {
    let run = prices();
    assert_eq!(run.as_slice(), &[Scalar::from(1_i64), Scalar::from(2_i64)]);
    assert_eq!(NestedValue::len(&run), 2);
    assert!(!run.is_empty());
    assert_eq!(run.children().count(), 2);
    assert!(matches!(run.children().next(), Some(Cow::Borrowed(_))));
    assert_eq!(run.to_string(), format!("{:?}", run.as_slice()));

    // The slice a run is built over is the slice it holds: nothing between
    // the two, so a run costs one allocation and a run over shared storage
    // costs none.
    let shared: Arc<[Scalar]> = Arc::from(run.as_slice());
    let over = Run::new(Arc::clone(&shared));
    assert_eq!(over, run);
    assert!(std::ptr::eq(over.as_slice().as_ptr(), shared.as_ptr()));
    assert_eq!(over.as_slice().len(), shared.len());
}

#[test]
fn the_empty_run_holds_nothing_and_every_empty_sequence_is_the_one_shared_slice() {
    assert_eq!(Run::default().as_slice().len(), 0);
    assert!(Run::default().is_empty());
    assert_eq!(Run::default(), Run::new(Vec::<Scalar>::new()));

    // `Scalar::from_sequence` of nothing is one static slice, shared by
    // every empty sequence rather than allocated per row.
    let first = Scalar::from_sequence([]);
    let second = Scalar::from_sequence(std::iter::empty());
    let (Some(first), Some(second)) = (Run::from_scalar(&first), Run::from_scalar(&second)) else {
        panic!("an empty sequence is a run");
    };
    assert!(first.is_empty());
    assert!(std::ptr::eq(
        first.as_slice().as_ptr(),
        second.as_slice().as_ptr()
    ));
    assert!(std::ptr::eq(
        first.as_slice().as_ptr(),
        Run::default().as_slice().as_ptr()
    ));
    assert_eq!(first, &Run::default());
}

#[test]
fn an_empty_run_slice_holds_no_holder() {
    // Refusals first: a window past the end, or one whose end overflows, is
    // refused naming the run and both counts rather than viewing nothing.
    let rows = Serie::new(numbered(0, 4));
    let past = rows.slice(3, 2).unwrap_err().to_string();
    assert!(past.contains("reach past the 4 rows"), "{past}");
    assert!(rows.slice(usize::MAX, 1).is_err());

    // An empty window keeps nothing alive: it is the one shared empty run,
    // wherever it was cut, and never a pointer into the rows it came from.
    let shared = Run::default().as_slice().as_ptr();
    for offset in [0, 2, 4] {
        let empty = rows.slice(offset, 0).unwrap();
        assert!(empty.is_empty());
        assert!(std::ptr::eq(run_of(&empty).as_slice().as_ptr(), shared));
    }
    let inner = rows.slice(1, 2).unwrap().slice(1, 0).unwrap();
    assert!(std::ptr::eq(run_of(&inner).as_slice().as_ptr(), shared));
}

#[test]
fn a_run_slice_is_a_view_of_its_holder_and_a_slice_of_a_slice_merges_onto_it() {
    let rows = Serie::new(numbered(0, 8));
    let holder = run_of(&rows).as_slice().as_ptr();

    let middle = rows.slice(2, 4).unwrap();
    assert!(std::ptr::eq(
        run_of(&middle).as_slice().as_ptr(),
        holder.wrapping_add(2)
    ));
    // A slice of a slice is one window over the original allocation, its
    // offsets summed: 2 + 1 = 3, never a window over the first window.
    let inner = middle.slice(1, 2).unwrap();
    assert!(std::ptr::eq(
        run_of(&inner).as_slice().as_ptr(),
        holder.wrapping_add(3)
    ));
    assert_eq!(run_of(&inner).as_slice(), numbered(3, 2).as_slice());
    assert_eq!(inner.len(), 2);
    assert_eq!(inner.scalar(1).unwrap(), Scalar::from(4_i64));
    assert!(
        inner.scalar(2).is_err(),
        "a row past the window, not the holder"
    );

    // A view is refused against its own window, never the holder around it.
    let refusal = middle.slice(3, 2).unwrap_err().to_string();
    assert!(refusal.contains("reach past the 4 rows"), "{refusal}");

    // The view outlives the serie it was cut from: the holder stays alive.
    drop(rows);
    drop(middle);
    assert_eq!(run_of(&inner).as_slice(), numbered(3, 2).as_slice());
}

#[test]
fn a_run_view_is_its_window_alone_to_eq_ord_hash_serde_and_debug() {
    let rows = Serie::new(numbered(0, 6));
    let sliced = rows.slice(2, 3).unwrap();
    let view = run_of(&sliced);
    let built = Run::new(numbered(2, 3));
    assert!(!std::ptr::eq(
        view.as_slice().as_ptr(),
        built.as_slice().as_ptr()
    ));

    // Equal, ordered and hashed by the window's rows alone.
    assert_eq!(view, &built);
    assert_eq!(view.cmp(&built), std::cmp::Ordering::Equal);
    assert_eq!(hashed(view), hashed(&built));
    assert_ne!(view, run_of(&rows));
    assert_eq!(
        view.cmp(&Run::new(numbered(2, 4))),
        view.as_slice().cmp(numbered(2, 4).as_slice())
    );
    assert!(view < &Run::new(numbered(3, 3)));
    // Two views of the same window are equal however they were reached.
    assert_eq!(
        run_of(&rows.slice(1, 4).unwrap().slice(1, 3).unwrap()),
        view
    );

    // ... and the hash is the one a column of those rows writes.
    let column = Serie::from_scalars(Field::new("n", DataType::Int64, false), numbered(2, 3))
        .expect("an int64 column");
    assert_eq!(hashed(view), hashed(&column));
    assert_eq!(hashed(&sliced), hashed(&column));

    // Written as the rows it views: no offset, no holder.
    assert_eq!(format!("{view:?}"), format!("{built:?}"));
    assert_eq!(format!("{view:#?}"), format!("{built:#?}"));
    assert!(format!("{view:?}").starts_with("Run(["), "{view:?}");
    assert_eq!(view.to_string(), built.to_string());
    let document = serde_json::to_string(view).unwrap();
    assert_eq!(document, serde_json::to_string(&built).unwrap());
    assert_eq!(
        document,
        serde_json::to_string(numbered(2, 3).as_slice()).unwrap()
    );
    let back: Run = serde_json::from_str(&document).unwrap();
    assert_eq!(&back, view);
    assert_eq!(back.as_slice().len(), 3);
}

#[test]
fn a_write_through_a_shared_run_view_copies_the_window_alone() {
    let rows = Serie::new(numbered(0, 6));

    // A write through a view whose holder is shared copies the window, and
    // only the window: the holder's rows stay as they were.
    let mut view = rows.slice(1, 3).unwrap();
    assert!(lies_in(&view, &rows));
    view.set(0, Scalar::from(10_i64)).unwrap();
    assert_eq!(run_of(&view).as_slice(), &[10_i64, 2, 3].map(Scalar::from));
    assert_eq!(run_of(&rows).as_slice(), numbered(0, 6).as_slice());
    assert!(!lies_in(&view, &rows));

    // A sort through a shared view copies the window too.
    let mut view = rows.slice(2, 3).unwrap();
    view.as_sorted(SortOptions::descending()).unwrap();
    assert_eq!(run_of(&view).as_slice(), &[4_i64, 3, 2].map(Scalar::from));
    assert_eq!(run_of(&rows).as_slice(), numbered(0, 6).as_slice());
    assert!(!lies_in(&view, &rows));
    assert_eq!(view.len(), 3);

    // A run holding its slice alone sorts where it stands: the whole run,
    // and a window whose holder was dropped, each at the pointer it had.
    let mut alone = Serie::new(numbered(0, 4));
    let before = run_of(&alone).as_slice().as_ptr();
    alone.as_sorted(SortOptions::descending()).unwrap();
    assert_eq!(
        run_of(&alone).as_slice(),
        &[3_i64, 2, 1, 0].map(Scalar::from)
    );
    assert!(std::ptr::eq(run_of(&alone).as_slice().as_ptr(), before));

    let mut window = Serie::new(numbered(0, 6)).slice(1, 3).unwrap();
    let before = run_of(&window).as_slice().as_ptr();
    window.as_reversed().unwrap();
    assert_eq!(run_of(&window).as_slice(), &[3_i64, 2, 1].map(Scalar::from));
    assert!(std::ptr::eq(run_of(&window).as_slice().as_ptr(), before));
}

#[test]
fn a_run_widens_to_a_sequence_scalar_and_narrows_back_only_from_a_run() {
    let run = prices();
    let scalar = run.clone().into_scalar();
    assert_eq!(
        scalar,
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)])
    );
    assert_eq!(Run::from_scalar(&scalar), Some(&run));
    assert_eq!(scalar.as_sequence(), Some(run.as_slice()));
    assert_eq!(
        Value::dtype(&run).unwrap(),
        DataType::serie(Field::new("item", DataType::Int64, false))
    );
    assert_eq!(Value::dtype(&run).unwrap().id(), DataTypeId::Serie);

    // A column is the same scalar variant and not this leaf.
    let column = Serie::from_scalars(
        Field::new("n", DataType::Int64, false),
        [Scalar::from(1_i64), Scalar::from(2_i64)],
    )
    .unwrap();
    let held = Scalar::Serie(column);
    assert_eq!(Run::from_scalar(&held), None);
    assert_eq!(held.as_sequence(), None);
    assert_eq!(Run::from_scalar(&Scalar::from(1_i64)), None);

    // The serie root holds it as its schema-free leaf, and answers it back.
    let serie = Serie::from(run.clone());
    assert_eq!(serie.as_run(), Some(&run));
    assert_eq!(serie.as_slice(), Some(run.as_slice()));
    assert_eq!(serie.field(), None);
    assert_eq!(serie.into_run(), run);
}

#[test]
fn a_run_serializes_as_a_transparent_list_and_reads_back_as_one() {
    let run = prices();
    let document = serde_json::to_string(&run).unwrap();
    assert_eq!(document, serde_json::to_string(run.as_slice()).unwrap());
    let back: Run = serde_json::from_str(&document).unwrap();
    assert_eq!(back, run);
}

#[test]
fn a_write_through_the_serie_root_copies_the_run_once_and_the_shared_slice_stays() {
    let run = prices();
    let mut serie = Serie::from(run.clone());
    serie.push(Scalar::from(3_i64)).unwrap();
    assert_eq!(serie.len(), 3);
    assert_eq!(run.as_slice().len(), 2, "the run copied, not grown");
    assert_eq!(serie.as_run().map(|held| held.as_slice().len()), Some(3));

    // A push onto a view grows the window alone, never the rows past it.
    let rows = Serie::new(numbered(0, 6));
    let mut view = rows.slice(1, 2).unwrap();
    view.push(Scalar::from(9_i64)).unwrap();
    assert_eq!(run_of(&view).as_slice(), &[1_i64, 2, 9].map(Scalar::from));
    assert_eq!(run_of(&rows).as_slice(), numbered(0, 6).as_slice());
}

#[test]
fn a_sequence_datatype_holds_one_item_field_in_five_layouts() {
    let item = Field::new("item", DataType::Int32, true);
    let sequence = DataType::serie(item.clone())
        .as_serie_type()
        .expect("a serie");
    assert_eq!(sequence.item(), &item);
    assert_eq!(sequence.fixed_length(), None);
    assert!(!sequence.is_view());
    assert!(!sequence.is_large());
    assert!(
        DataType::large_serie_view(item.clone())
            .as_serie_type()
            .unwrap()
            .is_view()
    );
    assert_eq!(
        DataType::fixed_size_serie(item, 3)
            .unwrap()
            .as_serie_type()
            .unwrap()
            .fixed_length(),
        Some(3)
    );
}

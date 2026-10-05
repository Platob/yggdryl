//! `rust/src/excel/journal.rs`: `Journal` - undo and redo over applied edits, bounded in entries and in bytes, a new edit clearing redo and an edit it cannot keep clearing undo.

use yggdryl::Scalar;
use yggdryl::excel::{Edit, Journal, Workbook};

fn at(text: &str) -> yggdryl::excel::CellRef {
    text.parse().unwrap()
}

/// Typing `text` into `cell` of `Sheet1`.
fn typing(cell: &str, text: &str) -> Edit {
    Edit::SetEntries {
        sheet: "Sheet1".into(),
        entries: vec![(at(cell), text.into())],
    }
}

fn book() -> Workbook {
    let mut workbook = Workbook::new();
    workbook.add_sheet("Sheet1").unwrap();
    workbook
}

fn value(workbook: &Workbook, cell: &str) -> Scalar {
    workbook.sheet("Sheet1").unwrap().scalar(at(cell))
}

/// Apply `edit` and keep it in `journal`, answering whether it is undoable.
fn edit(workbook: &mut Workbook, journal: &mut Journal, edit: Edit) -> bool {
    let label = edit.label();
    let mut applied = workbook.apply(edit).unwrap();
    let touched = applied.touched.clone();
    let undoable = journal.record(label, &mut applied);
    // The journal takes the inverse alone: the answer stays the caller's.
    assert!(applied.inverse.is_none());
    assert_eq!(applied.touched, touched);
    undoable
}

#[test]
fn undo_and_redo_walk_the_edits_back_and_forth_under_their_labels() {
    let mut workbook = book();
    let mut journal = Journal::new(100, 1 << 20);
    assert!(journal.is_empty());
    assert_eq!(journal.labels(), (None, None));
    assert!(journal.undo(&mut workbook).unwrap().is_none());
    assert!(journal.redo(&mut workbook).unwrap().is_none());
    assert!(edit(&mut workbook, &mut journal, typing("A1", "1")));
    assert!(edit(&mut workbook, &mut journal, typing("A1", "2")));
    assert!(edit(
        &mut workbook,
        &mut journal,
        Edit::InsertRows {
            sheet: "Sheet1".into(),
            at: 0,
            count: 1
        }
    ));
    assert_eq!(journal.labels(), (Some("Insert Rows"), None));
    assert_eq!(value(&workbook, "A2"), Scalar::from(2.0));
    journal.undo(&mut workbook).unwrap().unwrap();
    assert_eq!(value(&workbook, "A1"), Scalar::from(2.0));
    assert_eq!(
        journal.labels(),
        (Some("Typing '2' in A1"), Some("Insert Rows"))
    );
    journal.undo(&mut workbook).unwrap().unwrap();
    assert_eq!(value(&workbook, "A1"), Scalar::from(1.0));
    journal.redo(&mut workbook).unwrap().unwrap();
    assert_eq!(value(&workbook, "A1"), Scalar::from(2.0));
    journal.redo(&mut workbook).unwrap().unwrap();
    assert_eq!(value(&workbook, "A2"), Scalar::from(2.0));
    assert_eq!(journal.len(), (3, 0));
    // A new edit clears what could be redone.
    journal.undo(&mut workbook).unwrap();
    assert_eq!(journal.len(), (2, 1));
    assert!(edit(&mut workbook, &mut journal, typing("B1", "x")));
    assert_eq!(journal.len(), (3, 0));
    journal.clear();
    assert!(journal.is_empty());
}

#[test]
fn the_oldest_edits_go_past_the_entry_bound() {
    let mut workbook = book();
    let mut journal = Journal::new(2, 1 << 20);
    for (cell, text) in [("A1", "1"), ("A2", "2"), ("A3", "3")] {
        edit(&mut workbook, &mut journal, typing(cell, text));
    }
    assert_eq!(journal.len(), (2, 0));
    journal.undo(&mut workbook).unwrap();
    journal.undo(&mut workbook).unwrap();
    assert!(journal.undo(&mut workbook).unwrap().is_none());
    // The first edit went past the bound: it stays.
    assert_eq!(value(&workbook, "A1"), Scalar::from(1.0));
    assert_eq!(value(&workbook, "A2"), Scalar::Null);
}

#[test]
fn the_oldest_edits_go_past_the_byte_bound_and_one_past_it_alone_is_not_undoable() {
    let mut workbook = book();
    let size = workbook.apply(typing("Z9", "probe")).unwrap().bytes;
    assert!(size > 0);
    // Room for two inverses of this size, not three.
    let mut journal = Journal::new(100, size * 2 + size / 2);
    for (cell, text) in [("A1", "1"), ("A2", "2"), ("A3", "3")] {
        edit(&mut workbook, &mut journal, typing(cell, text));
    }
    assert_eq!(journal.len(), (2, 0));
    // An inverse larger than the whole bound is not kept, and nothing
    // before it can be undone past it.
    let mut small = Journal::new(100, size / 2);
    assert!(!edit(&mut workbook, &mut small, typing("B1", "big")));
    assert!(small.is_empty());
    assert!(edit(&mut workbook, &mut journal, typing("B2", "x")));
    let mut tight = Journal::new(100, size * 4);
    edit(&mut workbook, &mut tight, typing("C1", "1"));
    let wide = || Edit::SetEntries {
        sheet: "Sheet1".into(),
        entries: (0..200)
            .map(|row| (yggdryl::excel::CellRef::new(row, 5), "wide".into()))
            .collect(),
    };
    // Typed over two hundred cells holding something, the inverse holds
    // them all.
    workbook.apply(wide()).unwrap();
    assert!(!edit(&mut workbook, &mut tight, wide()));
    assert_eq!(tight.len(), (0, 0));
}

#[test]
fn a_refused_undo_keeps_the_edit_to_undo() {
    let mut workbook = book();
    workbook.add_sheet("Other").unwrap();
    let mut journal = Journal::new(100, 1 << 20);
    edit(&mut workbook, &mut journal, typing("A1", "1"));
    // The sheet the inverse names is gone behind the journal's back.
    workbook.remove_sheet("Sheet1").unwrap();
    assert!(journal.undo(&mut workbook).is_err());
    assert_eq!(journal.len(), (1, 0));
}

/// A loaded gradient is retained by a derived, newly interned bold style.
/// The inverse holds only its style ID in each cell and one descriptor.
fn retained_style_book(cells: u32, large: bool) -> (Workbook, yggdryl::excel::CellStyle, usize) {
    use crate::excel_package::{
        NS, content_types, package, root_relationships, workbook, workbook_relationships, worksheet,
    };
    use yggdryl::excel::{CellRange, CellRef, Fill, StylePatch};

    let font = if large {
        "F".repeat(31)
    } else {
        "F".to_owned()
    };
    let code = format!(
        "0\"{}\"",
        if large {
            "C".repeat(192)
        } else {
            "C".to_owned()
        }
    );
    let gradient = format!(
        "<gradientFill degree=\"90\"><!--{}--><stop position=\"0\"><color rgb=\"FFFF0000\"/></stop><stop position=\"1\"><color rgb=\"FF0000FF\"/></stop></gradientFill>",
        if large {
            "g".repeat(8_192)
        } else {
            "g".to_owned()
        }
    );
    let styles = format!(
        "<styleSheet xmlns=\"{NS}\"><numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"{}\"/></numFmts><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"{font}\"/></font></fonts><fills count=\"3\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill><fill>{gradient}</fill></fills><borders count=\"1\"><border/></borders><cellXfs count=\"2\"><xf/><xf numFmtId=\"164\" fillId=\"2\"/></cellXfs></styleSheet>",
        code.replace('"', "&quot;")
    );
    let rows: String = (1..=cells)
        .map(|row| format!("<row r=\"{row}\"><c r=\"A{row}\" s=\"1\"><v>{row}</v></c></row>"))
        .collect();
    let mut book = Workbook::from_bytes(package(&[
        ("[Content_Types].xml", &content_types(1, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Sheet1"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, true),
        ),
        ("xl/worksheets/sheet1.xml", &worksheet(&rows)),
        ("xl/styles.xml", &styles),
    ]))
    .unwrap();
    let range = CellRange::new(CellRef::new(0, 0), CellRef::new(cells - 1, 0));
    book.set_style(
        "Sheet1",
        &[range],
        &StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    )
    .unwrap();
    let style = book.cell_style("Sheet1", at("A1")).unwrap();
    let Fill::Gradient(bytes) = &style.fill else {
        panic!("the derived style retains the gradient")
    };
    let payload = style.font.name.len() + style.number_format.len() + bytes.len();
    (book, style, payload)
}

#[test]
fn retained_style_payload_counts_once_per_distinct_style_and_controls_journal_eviction() {
    use yggdryl::excel::{CellRange, CellRef, Clear};

    let mut first_delta = None;
    for cells in [1, 128] {
        let (mut small, _, short_payload) = retained_style_book(cells, false);
        let (mut large, style, long_payload) = retained_style_book(cells, true);
        let range = CellRange::new(CellRef::new(0, 0), CellRef::new(cells - 1, 0));
        let clear = || Edit::Clear {
            sheet: "Sheet1".into(),
            ranges: vec![range],
            what: Clear::All,
        };
        let short = small.apply(clear()).unwrap();
        let mut long = large.apply(clear()).unwrap();
        let delta = long.bytes - short.bytes;
        assert_eq!(delta, long_payload - short_payload, "{cells} cells");
        assert_eq!(*first_delta.get_or_insert(delta), delta);

        let mut too_small = Journal::new(10, long.bytes - 1);
        let inverse = long.inverse.clone();
        assert!(!too_small.record("Clear styled cells", &mut long));
        assert!(too_small.is_empty());
        assert!(long.inverse.is_none());
        long.inverse = inverse;
        let mut journal = Journal::new(10, long.bytes);
        assert!(journal.record("Clear styled cells", &mut long));
        journal.undo(&mut large).unwrap().unwrap();
        assert_eq!(large.cell_style("Sheet1", at("A1")).unwrap(), style);
        assert_eq!(
            large
                .cell_style("Sheet1", CellRef::new(cells - 1, 0))
                .unwrap(),
            style
        );
        assert_eq!(large.sheet("Sheet1").unwrap().cell_count(), cells as usize);
        journal.redo(&mut large).unwrap().unwrap();
        assert!(large.sheet("Sheet1").unwrap().is_empty());
    }
}

#[test]
fn journal_byte_bound_discards_an_oversized_redo_after_successful_undo() {
    let mut workbook = book();
    let text = "x".repeat(16_384);
    let mut applied = workbook.apply(typing("A1", &text)).unwrap();
    let cap = applied.bytes;
    let mut journal = Journal::new(100, cap);
    assert!(journal.record("Large entry over blank", &mut applied));
    let undone = journal.undo(&mut workbook).unwrap().unwrap();
    assert!(
        undone.bytes > cap,
        "the generated redo retains the large value"
    );
    assert_eq!(value(&workbook, "A1"), Scalar::Null);
    assert!(undone.inverse.is_none());
    assert_eq!(journal.len(), (0, 0));
    assert_eq!(journal.labels(), (None, None));
    assert!(journal.redo(&mut workbook).unwrap().is_none());
}

fn journal_byte_bound_sizes(first: &str, second: &str) -> (usize, usize, usize, usize) {
    let mut sample = book();
    let first = sample.apply(typing("A1", first)).unwrap();
    let second = sample.apply(typing("B1", second)).unwrap();
    let small_first = first.bytes;
    let small_second = second.bytes;
    let big_second = sample.apply(second.inverse.unwrap()).unwrap().bytes;
    let big_first = sample.apply(first.inverse.unwrap()).unwrap().bytes;
    (small_first, small_second, big_first, big_second)
}

#[test]
fn journal_byte_bound_counts_undo_and_redo_together_and_evicts_oldest_undo() {
    let first = "a".repeat(2_048);
    let second = "b".repeat(4_096);
    let (small_first, small_second, _, big_second) = journal_byte_bound_sizes(&first, &second);
    let cap = big_second + small_first - 1;
    assert!(small_first + small_second <= cap && big_second <= cap);
    let mut workbook = book();
    let mut journal = Journal::new(100, cap);
    assert!(edit(&mut workbook, &mut journal, typing("A1", &first)));
    assert!(edit(&mut workbook, &mut journal, typing("B1", &second)));
    assert_eq!(journal.len(), (2, 0));
    let undone = journal.undo(&mut workbook).unwrap().unwrap();
    assert_eq!(undone.bytes, big_second);
    assert!(undone.bytes + small_first > cap);
    assert_eq!(
        journal.len(),
        (0, 1),
        "the oldest undo must yield space to the new redo"
    );
    assert_eq!(value(&workbook, "A1"), Scalar::from(first.as_str()));
    assert_eq!(value(&workbook, "B1"), Scalar::Null);
    assert!(journal.undo(&mut workbook).unwrap().is_none());
    journal.redo(&mut workbook).unwrap().unwrap();
    assert_eq!(value(&workbook, "B1"), Scalar::from(second.as_str()));
    assert_eq!(journal.len(), (1, 0));
}

#[test]
fn journal_byte_bound_keeps_the_next_redo_and_evicts_the_farthest_future() {
    let first = "a".repeat(2_048);
    let second = "b".repeat(4_096);
    let (small_first, small_second, big_first, big_second) =
        journal_byte_bound_sizes(&first, &second);
    let cap = big_first + big_second - 1;
    assert!(small_first + small_second <= cap);
    assert!(small_first + big_second <= cap && big_first <= cap && big_second <= cap);
    let mut workbook = book();
    let mut journal = Journal::new(100, cap);
    assert!(edit(&mut workbook, &mut journal, typing("A1", &first)));
    assert!(edit(&mut workbook, &mut journal, typing("B1", &second)));
    journal.undo(&mut workbook).unwrap().unwrap();
    assert_eq!(journal.len(), (1, 1));
    let undone = journal.undo(&mut workbook).unwrap().unwrap();
    assert_eq!(undone.bytes, big_first);
    assert_eq!(journal.len(), (0, 1), "only the next reachable redo fits");
    journal.redo(&mut workbook).unwrap().unwrap();
    assert_eq!(value(&workbook, "A1"), Scalar::from(first.as_str()));
    assert_eq!(value(&workbook, "B1"), Scalar::Null);
    assert!(journal.redo(&mut workbook).unwrap().is_none());
}

#[test]
fn journal_byte_bound_discards_both_histories_when_redo_becomes_nonundoable() {
    let mut workbook = book();
    let mut journal = Journal::new(100, 4_096);
    assert!(edit(
        &mut workbook,
        &mut journal,
        Edit::Clear {
            sheet: "Sheet1".into(),
            ranges: vec!["A1:A64".parse().unwrap()],
            what: yggdryl::excel::Clear::All,
        }
    ));
    assert!(edit(&mut workbook, &mut journal, typing("B1", "second")));
    journal.undo(&mut workbook).unwrap().unwrap();
    journal.undo(&mut workbook).unwrap().unwrap();
    assert_eq!(journal.len(), (0, 2));
    // Restoring the originally empty range must capture all intervening cells,
    // not just the small state held when the redo entry was first recorded.
    for row in 1..=64 {
        workbook
            .set_entry("Sheet1", at(&format!("A{row}")), "1")
            .unwrap();
    }
    let redone = journal.redo(&mut workbook).unwrap().unwrap();
    assert!(
        redone.bytes > 4_096,
        "the inverse captures 64 intervening rows"
    );
    for row in 1..=64 {
        assert_eq!(value(&workbook, &format!("A{row}")), Scalar::Null);
    }
    assert_eq!(value(&workbook, "B1"), Scalar::Null);
    assert_eq!(journal.len(), (0, 0));
    assert!(journal.undo(&mut workbook).unwrap().is_none());
    assert!(journal.redo(&mut workbook).unwrap().is_none());
}

#[test]
fn journal_byte_bound_zero_entries_reports_the_applied_edit_as_nonundoable() {
    let mut workbook = book();
    let mut journal = Journal::new(0, 1 << 20);
    let mut applied = workbook.apply(typing("A1", "saved")).unwrap();
    assert!(!journal.record("Disabled undo", &mut applied));
    assert_eq!(value(&workbook, "A1"), Scalar::from("saved"));
    assert!(applied.inverse.is_none());
    assert!(journal.is_empty());
}

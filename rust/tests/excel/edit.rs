//! `rust/src/excel/edit.rs`: `Edit`, `Applied` and `Workbook::apply` - every edit all or nothing, answered with the edit undoing it exactly, the inverse of which redoes it exactly; and the JSON shape the workbook service reads an edit from.

use yggdryl::excel::{
    Cell, CellRange, CellRef, Clear, DateSystem, Edit, FillMode, FindOptions, FindScope, Frozen,
    Landing, Paste, Sheet, SheetState, SortKey, StylePatch, Within, Workbook,
};
use yggdryl::{Error, Scalar, from_json_scalar};

use crate::excel_package::rich_package;

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

fn range(text: &str) -> CellRange {
    text.parse().unwrap()
}

/// The rich package with every sheet parsed.
fn book() -> Workbook {
    let workbook = Workbook::from_bytes(rich_package()).unwrap();
    workbook.parse_all().unwrap();
    workbook
}

/// One of every edit over the rich package, each changing something.
fn edits() -> Vec<Edit> {
    let data = || "Data".into();
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    let mut landed = Sheet::new("Scratch").unwrap();
    landed.set_cell(at("A1"), "id").unwrap();
    landed.set_cell(at("A2"), 7.0).unwrap();
    vec![
        Edit::SetEntries {
            sheet: data(),
            entries: vec![
                (at("B2"), "=A2*2".into()),
                (at("C3"), "12%".into()),
                (at("H9"), "hello".into()),
            ],
        },
        Edit::FillEntry {
            sheet: data(),
            ranges: vec![range("G1:G5")],
            text: "=B1+1".into(),
            at: at("G1"),
        },
        Edit::Clear {
            sheet: data(),
            ranges: vec![range("A1:B3")],
            what: Clear::All,
        },
        Edit::Clear {
            sheet: data(),
            ranges: vec![range("B2:C4")],
            what: Clear::Contents,
        },
        Edit::Clear {
            sheet: data(),
            ranges: vec![range("A1:C5"), range("5:6")],
            what: Clear::Formats,
        },
        Edit::SetStyle {
            sheet: data(),
            ranges: vec![range("B2:C3")],
            patch: bold.clone(),
        },
        Edit::SetStyle {
            sheet: data(),
            ranges: vec![range("B:C"), range("4:5")],
            patch: StylePatch {
                italic: Some(true),
                ..StylePatch::default()
            },
        },
        Edit::InsertRows {
            sheet: data(),
            at: 2,
            count: 2,
        },
        Edit::RemoveRows {
            sheet: data(),
            start: 2,
            count: 1,
        },
        Edit::RemoveRows {
            sheet: data(),
            start: 4,
            count: 2,
        },
        Edit::InsertColumns {
            sheet: data(),
            at: 1,
            count: 1,
        },
        Edit::RemoveColumns {
            sheet: data(),
            start: 3,
            count: 1,
        },
        Edit::InsertRows {
            sheet: "Report".into(),
            at: 0,
            count: 1,
        },
        // The chart is anchored at column H and row 2, its offsets zero: a
        // removal starting there leaves it where it is, and the insertion
        // undoing it would move it. Column H holds nothing else, so a cell
        // typed after says the edit did something.
        Edit::Batch(vec![
            Edit::RemoveColumns {
                sheet: data(),
                start: 7,
                count: 1,
            },
            Edit::SetEntries {
                sheet: data(),
                entries: vec![(at("H1"), "after".into())],
            },
        ]),
        Edit::RemoveRows {
            sheet: data(),
            start: 1,
            count: 1,
        },
        Edit::RowHeight {
            sheet: data(),
            start: 1,
            count: 2,
            height: Some(30.0),
        },
        Edit::ColumnWidth {
            sheet: data(),
            start: 0,
            count: 2,
            width: None,
        },
        Edit::HideRows {
            sheet: data(),
            start: 0,
            count: 3,
            hidden: true,
        },
        Edit::HideColumns {
            sheet: data(),
            start: 1,
            count: 1,
            hidden: true,
        },
        Edit::Merge {
            sheet: data(),
            range: range("A1:B1"),
            center: true,
            across: false,
        },
        Edit::Merge {
            sheet: data(),
            range: range("D3:E4"),
            center: false,
            across: true,
        },
        Edit::Unmerge {
            sheet: data(),
            range: range("B6"),
        },
        Edit::Freeze {
            sheet: data(),
            frozen: Some(Frozen {
                rows: 2,
                columns: 0,
            }),
        },
        Edit::Freeze {
            sheet: data(),
            frozen: None,
        },
        Edit::Fill {
            sheet: data(),
            source: range("B2:B3"),
            target: range("B2:B8"),
            mode: FillMode::Series,
        },
        Edit::Fill {
            sheet: data(),
            source: range("C2:C3"),
            target: range("C2:C6"),
            mode: FillMode::Copy,
        },
        Edit::Sort {
            sheet: data(),
            range: range("A1:C4"),
            keys: vec![SortKey {
                column: 1,
                descending: true,
            }],
            header: true,
        },
        Edit::Paste {
            from: (data(), range("B2:C3")),
            to: (data(), at("D8")),
            what: Paste::All,
            cut: false,
        },
        Edit::Paste {
            from: ("Report".into(), range("A1:B1")),
            to: (data(), at("B3")),
            what: Paste::Values,
            cut: false,
        },
        Edit::Paste {
            from: ("Report".into(), range("A1:B1")),
            to: (data(), at("H3")),
            what: Paste::Formulas,
            cut: false,
        },
        Edit::Paste {
            from: (data(), range("A1:C1")),
            to: (data(), at("A3")),
            what: Paste::Formats,
            cut: false,
        },
        Edit::Paste {
            from: (data(), range("B2:C3")),
            to: (data(), at("H8")),
            what: Paste::All,
            cut: true,
        },
        Edit::PasteText {
            sheet: data(),
            anchor: at("G2"),
            text: "1\t2\n3\t\"x\ty\"\n".into(),
        },
        Edit::Replace {
            options: FindOptions {
                text: "Apple".into(),
                scope: FindScope::Workbook,
                sheet: "Report".into(),
                within: Within::Formulas,
                ..FindOptions::default()
            },
            replacement: "Kiwi".into(),
        },
        Edit::AddSheet {
            name: None,
            at: Some(1),
        },
        Edit::RenameSheet {
            name: "Data".into(),
            to: "Q1 Data".into(),
        },
        Edit::RemoveSheet {
            name: "Report".into(),
        },
        Edit::RemoveSheet {
            name: "Data".into(),
        },
        Edit::MoveSheet {
            name: "Data".into(),
            to: 2,
        },
        Edit::SheetState {
            name: "Pivot".into(),
            state: SheetState::Hidden,
        },
        Edit::Land {
            destination: Landing::NewSheet("Imported".into()),
            cells: Box::new(landed.clone()),
        },
        Edit::Land {
            destination: Landing::At {
                sheet: data(),
                anchor: at("G10"),
            },
            cells: Box::new(landed),
        },
        Edit::Batch(vec![
            Edit::SetEntries {
                sheet: data(),
                entries: vec![(at("A2"), "Plum".into())],
            },
            Edit::InsertColumns {
                sheet: data(),
                at: 0,
                count: 2,
            },
            Edit::SetStyle {
                sheet: data(),
                ranges: vec![range("C2")],
                patch: bold,
            },
        ]),
        Edit::SetCells {
            sheet: data(),
            cells: vec![
                (
                    at("A1"),
                    Some(
                        Cell::from_scalar(at("A1"), Scalar::from(1.0), DateSystem::Year1900)
                            .unwrap(),
                    ),
                ),
                (at("C2"), None),
            ],
        },
    ]
}

#[test]
fn every_edit_undone_gives_back_the_sheets_and_the_names_it_found() {
    for edit in edits() {
        let mut workbook = book();
        let label = edit.label();
        let sheets = |workbook: &Workbook| -> Vec<(String, Option<Sheet>)> {
            workbook
                .sheet_names()
                .into_iter()
                .map(|name| {
                    (
                        name.to_owned(),
                        workbook.get_sheet(name).ok().flatten().cloned(),
                    )
                })
                .collect()
        };
        let names = |workbook: &Workbook| -> Vec<(String, String)> {
            workbook
                .defined_names()
                .map(|name| (name.name().to_owned(), name.text()))
                .collect()
        };
        let before = (sheets(&workbook), names(&workbook));
        let applied = workbook
            .apply(edit)
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        assert!(applied.bytes > 0, "{label}");
        workbook.apply(applied.inverse.expect("an undo")).unwrap();
        assert!(
            (sheets(&workbook), names(&workbook)) == before,
            "{label} undone"
        );
        // The undone workbook still saves and opens again.
        Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    }
}

#[test]
fn pasted_text_is_undone_over_every_row_its_line_breaks_make() {
    // A lone CR ends a row as a line feed does; a quoted break does not.
    for (text, span) in [
        ("a\rb\rc", "H1:H3"),
        ("a\r\nb\r\nc\r\n", "H1:H3"),
        ("\"a\nb\"\tx\ry\nz", "H1:I3"),
    ] {
        let mut workbook = book();
        let before = workbook.sheet("Data").unwrap().clone();
        let applied = workbook
            .apply(Edit::PasteText {
                sheet: "Data".into(),
                anchor: at("H1"),
                text: text.into(),
            })
            .unwrap();
        assert_eq!(applied.touched, [("Data".into(), range(span))], "{text:?}");
        assert_ne!(workbook.sheet("Data").unwrap(), &before);
        workbook.apply(applied.inverse.expect("an undo")).unwrap();
        assert_eq!(workbook.sheet("Data").unwrap(), &before, "{text:?}");
    }
}

#[test]
fn a_cut_s_undo_holds_what_it_changed_and_nothing_it_left() {
    let mut workbook = Workbook::new();
    workbook.add_sheet("Moved").unwrap();
    workbook.add_sheet("Formulas").unwrap();
    workbook.set_entry("Moved", at("A1"), "1").unwrap();
    // Five thousand formulas the cut does not reach.
    let entries: Vec<(CellRef, yggdryl::excel::Formula)> = (0..5_000)
        .map(|row| {
            let host = CellRef::new(row, 0);
            (host, yggdryl::excel::Formula::from_file("B1+1", host))
        })
        .collect();
    let sheet = workbook.sheet_mut("Formulas").unwrap();
    for (host, formula) in entries {
        sheet
            .insert_cell(
                Cell::from_scalar(host, Scalar::from(0.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(formula),
            )
            .unwrap();
    }
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Moved".into(), range("A1")),
            to: ("Moved".into(), at("C3")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    assert!(applied.bytes < 4_096, "{}", applied.bytes);
    workbook.apply(applied.inverse.expect("an undo")).unwrap();
    assert_eq!(
        workbook.sheet("Moved").unwrap().scalar(at("A1")),
        Scalar::from(1.0)
    );
}

#[test]
fn a_refused_edit_changes_nothing() {
    let mut workbook = book();
    let before = workbook.sheet("Data").unwrap().clone();
    // The second entry refuses: the first is taken back.
    let error = workbook
        .apply(Edit::SetEntries {
            sheet: "Data".into(),
            entries: vec![(at("A1"), "changed".into()), (at("A2"), "=SUM(A1".into())],
        })
        .unwrap_err();
    assert!(matches!(error, Error::Parse { .. }), "{error}");
    assert_eq!(workbook.sheet("Data").unwrap(), &before);
    // A batch failing on its second edit takes the first back.
    let error = workbook
        .apply(Edit::Batch(vec![
            Edit::InsertRows {
                sheet: "Data".into(),
                at: 1,
                count: 1,
            },
            Edit::Merge {
                sheet: "Data".into(),
                range: range("A1"),
                center: false,
                across: false,
            },
        ]))
        .unwrap_err();
    assert!(error.to_string().contains("more than one cell"), "{error}");
    assert_eq!(workbook.sheet("Data").unwrap(), &before);
    // Removing the last visible worksheet is refused.
    let mut one = Workbook::new();
    one.add_sheet("Only").unwrap();
    assert!(
        one.apply(Edit::RemoveSheet {
            name: "Only".into()
        })
        .is_err()
    );
    assert_eq!(one.sheet_names(), ["Only"]);
}

#[test]
fn an_applied_edit_says_what_it_touched_and_what_it_answers() {
    let mut workbook = book();
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), range("A1:B2")),
            to: ("Data".into(), at("E8")),
            what: Paste::All,
            cut: false,
        })
        .unwrap();
    assert_eq!(applied.touched, [("Data".into(), range("E8:F9"))]);
    assert_eq!(
        applied.result,
        Scalar::from_struct([("range", Scalar::from("E8:F9"))]).unwrap()
    );
    assert!(!applied.structural && !applied.sheets);
    let applied = workbook
        .apply(Edit::InsertRows {
            sheet: "Data".into(),
            at: 0,
            count: 1,
        })
        .unwrap();
    assert!(applied.structural);
    let applied = workbook
        .apply(Edit::AddSheet {
            name: Some("Notes".into()),
            at: None,
        })
        .unwrap();
    assert!(applied.sheets);
    assert_eq!(
        applied.result.get_key_str("name"),
        Some(&Scalar::from("Notes"))
    );
    let applied = workbook
        .apply(Edit::SetStyle {
            sheet: "Data".into(),
            ranges: vec![range("A1")],
            patch: StylePatch {
                strike: Some(true),
                ..StylePatch::default()
            },
        })
        .unwrap();
    assert!(applied.styles);
    let applied = workbook
        .apply(Edit::Replace {
            options: FindOptions {
                text: "Pear".into(),
                sheet: "Data".into(),
                within: Within::Formulas,
                ..FindOptions::default()
            },
            replacement: "Quince".into(),
        })
        .unwrap();
    assert_eq!(
        applied.result,
        Scalar::from_struct([("replaced", Scalar::from(1_i64))]).unwrap()
    );
}

#[test]
fn a_label_names_the_edit_as_excel_s_menus_do() {
    for (edit, label) in [
        (
            Edit::SetEntries {
                sheet: "Data".into(),
                entries: vec![(at("B4"), "=SUM(A1:A3)".into())],
            },
            "Typing '=SUM(A1:A3)' in B4",
        ),
        (
            Edit::InsertRows {
                sheet: "Data".into(),
                at: 0,
                count: 1,
            },
            "Insert Rows",
        ),
        (
            Edit::RemoveColumns {
                sheet: "Data".into(),
                start: 0,
                count: 1,
            },
            "Delete Columns",
        ),
        (
            Edit::Paste {
                from: ("Data".into(), range("A1")),
                to: ("Data".into(), at("B1")),
                what: Paste::Formats,
                cut: false,
            },
            "Paste Formats",
        ),
        (
            Edit::Merge {
                sheet: "Data".into(),
                range: range("A1:B1"),
                center: true,
                across: false,
            },
            "Merge & Center",
        ),
    ] {
        assert_eq!(edit.label(), label);
    }
}

/// The edit the JSON `json` states over the rich package.
fn read(workbook: &Workbook, json: &str) -> yggdryl::Result<Edit> {
    Edit::from_scalar(&from_json_scalar(json).unwrap(), workbook)
}

#[test]
fn an_edit_reads_from_json_its_sheets_by_key() {
    let workbook = book();
    let data = workbook.sheet_key("Data").unwrap().as_u32();
    let report = workbook.sheet_key("Report").unwrap().as_u32();
    let spelled = |json: String| format!("{:?}", read(&workbook, &json).unwrap());
    assert_eq!(
        spelled(format!(
            r#"{{"op":"setEntries","sheet":{data},"entries":[{{"ref":"B2","text":"1"}}]}}"#
        )),
        format!(
            "{:?}",
            Edit::SetEntries {
                sheet: "Data".into(),
                entries: vec![(at("B2"), "1".into())]
            }
        )
    );
    for (json, expected) in [
        (
            format!(
                r#"{{"op":"fillEntry","sheet":{data},"ranges":["A1:B2","D4"],"text":"=A1","ref":"B2"}}"#
            ),
            Edit::FillEntry {
                sheet: "Data".into(),
                ranges: vec![range("A1:B2"), range("D4")],
                text: "=A1".into(),
                at: at("B2"),
            },
        ),
        (
            format!(r#"{{"op":"removeRows","sheet":{data},"start":3,"count":2}}"#),
            Edit::RemoveRows {
                sheet: "Data".into(),
                start: 3,
                count: 2,
            },
        ),
        (
            format!(r#"{{"op":"columnWidth","sheet":{data},"start":0,"count":1,"size":null}}"#),
            Edit::ColumnWidth {
                sheet: "Data".into(),
                start: 0,
                count: 1,
                width: None,
            },
        ),
        (
            format!(r#"{{"op":"freeze","sheet":{data},"rows":0,"columns":0}}"#),
            Edit::Freeze {
                sheet: "Data".into(),
                frozen: None,
            },
        ),
        (
            format!(
                r#"{{"op":"sort","sheet":{data},"range":"A1:C4","header":true,"keys":[{{"column":"B","descending":true}}]}}"#
            ),
            Edit::Sort {
                sheet: "Data".into(),
                range: range("A1:C4"),
                keys: vec![SortKey {
                    column: 1,
                    descending: true,
                }],
                header: true,
            },
        ),
        (
            format!(
                r#"{{"op":"paste","from":{{"sheet":{report},"range":"A1:B2"}},"to":{{"sheet":{data},"ref":"C3"}},"what":"formats","cut":false}}"#
            ),
            Edit::Paste {
                from: ("Report".into(), range("A1:B2")),
                to: ("Data".into(), at("C3")),
                what: Paste::Formats,
                cut: false,
            },
        ),
        (
            format!(
                r#"{{"op":"replace","scope":"workbook","sheet":{data},"text":"a*b","replacement":"c","in":"formulas","matchCase":true,"entireCell":false}}"#
            ),
            Edit::Replace {
                options: FindOptions {
                    text: "a*b".into(),
                    scope: FindScope::Workbook,
                    sheet: "Data".into(),
                    within: Within::Formulas,
                    match_case: true,
                    entire_cell: false,
                },
                replacement: "c".into(),
            },
        ),
        (
            format!(
                r#"{{"op":"batch","edits":[{{"op":"removeSheet","sheet":{report}}},{{"op":"sheetState","sheet":{data},"state":"hidden"}}]}}"#
            ),
            Edit::Batch(vec![
                Edit::RemoveSheet {
                    name: "Report".into(),
                },
                Edit::SheetState {
                    name: "Data".into(),
                    state: SheetState::Hidden,
                },
            ]),
        ),
    ] {
        assert_eq!(spelled(json), format!("{expected:?}"));
    }
    // A patch: an absent member skipped, `null` clearing.
    let Edit::SetStyle { patch, .. } = read(
        &workbook,
        &format!(
            r##"{{"op":"setStyle","sheet":{data},"ranges":["A1"],"patch":{{"bold":true,"fill":null,"fontColor":"#FF0000","borders":{{"preset":"outside","style":"thick"}},"numberFormat":"0.00%","horizontal":null}}}}"##
        ),
    )
    .unwrap() else {
        panic!("a style edit");
    };
    assert_eq!(patch.bold, Some(true));
    assert_eq!(patch.fill, Some(None));
    assert_eq!(
        patch.font_color,
        Some(Some(yggdryl::excel::Color::Rgb(0xFFFF_0000)))
    );
    assert_eq!(patch.number_format.as_deref(), Some("0.00%"));
    assert_eq!(patch.horizontal, Some(yggdryl::excel::Horizontal::General));
    assert_eq!(patch.italic, None);
}

#[test]
fn a_json_edit_is_refused_naming_the_member_at_fault() {
    let workbook = book();
    let data = workbook.sheet_key("Data").unwrap().as_u32();
    for (json, expected) in [
        (
            r#"{"op":"explode"}"#.to_owned(),
            "invalid record value at $.edit.op: expected an edit the service takes",
        ),
        (
            r#"[1]"#.to_owned(),
            "invalid record value at $.edit: expected an edit object, got serie",
        ),
        (
            format!(
                r#"{{"op":"setEntries","sheet":{data},"entries":[{{"ref":"B2","text":"x"}},{{"ref":"ZZZZ9"}}]}}"#
            ),
            "invalid record value at $.edit.entries[1].ref: expected a cell such as B2, got \"ZZZZ9\"",
        ),
        (
            format!(r#"{{"op":"setEntries","sheet":{data},"entries":[{{"ref":"B2"}}]}}"#),
            "invalid record value at $.edit.entries[0].text: expected a value, got none",
        ),
        (
            r#"{"op":"insertRows","sheet":99,"at":1,"count":1}"#.to_owned(),
            "expected a worksheet at \"$.edit.sheet (no sheet has the key 99)\", got nothing",
        ),
        (
            format!(r#"{{"op":"insertRows","sheet":{data},"at":-1,"count":1}}"#),
            "invalid record value at $.edit.at: expected a whole number of at least 0, got -1",
        ),
        (
            format!(r#"{{"op":"clear","sheet":{data},"ranges":["A1"],"what":"everything"}}"#),
            "invalid record value at $.edit.what: expected one of all, contents, formats, got \"everything\"",
        ),
        (
            format!(r#"{{"op":"sort","sheet":{data},"range":"A1:B2","keys":[{{"column":"1"}}]}}"#),
            "invalid record value at $.edit.keys[0].column: expected column letters such as B, got \"1\"",
        ),
        (
            format!(
                r#"{{"op":"setStyle","sheet":{data},"ranges":["A1"],"patch":{{"fill":"red"}}}}"#
            ),
            "invalid record value at $.edit.patch.fill: expected a colour such as #FF0000, got \"red\"",
        ),
        (
            format!(r#"{{"op":"replace","sheet":{data},"text":"","replacement":"x"}}"#),
            "invalid record value at $.edit.text: expected text to find, got the empty text",
        ),
        (
            format!(r#"{{"op":"rowHeight","sheet":{data},"start":0,"count":1}}"#),
            "invalid record value at $.edit.size: expected a value, got none",
        ),
        (
            format!(r#"{{"op":"columnWidth","sheet":{data},"start":0,"count":1,"size":"wide"}}"#),
            "invalid record value at $.edit.size: expected a number, got",
        ),
        (
            format!(
                r#"{{"op":"batch","edits":[{{"op":"removeSheet","sheet":{data}}},{{"op":"merge","sheet":{data},"range":"A0"}}]}}"#
            ),
            "invalid record value at $.edit.edits[1].range: expected a range such as A1:C3",
        ),
    ] {
        let error = read(&workbook, &json).unwrap_err().to_string();
        assert!(error.starts_with(expected), "{json}: {error}");
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::excel::{Journal, Workbook};
    use yggdryl::internals::excel_edit::{frame_edit, has_frame, state};

    /// A failed compound edit restores original overlay insertions, not merely
    /// equal bytes: an older in-flight save must still see the later insertion.
    #[test]
    fn atomicity_rich_batch_keeps_prior_overlays_and_pending_save_identity() {
        use yggdryl::excel::Edit;
        use yggdryl::internals::excel_edit::parts_edit;
        let mut book = super::book();
        saved(&mut book);
        const MEMBER: &str = "xl/charts/chart1.xml";
        let bytes = retained_style_parts(&book).remove(MEMBER).unwrap();
        book.apply(parts_edit(&book, &[(MEMBER, Some(&bytes))]))
            .unwrap();
        let older = book.into_package().unwrap();
        // Same bytes, a distinct logical insertion after the pending save.
        book.apply(parts_edit(&book, &[(MEMBER, Some(&bytes))]))
            .unwrap();
        let before = state(&book);
        let parts = retained_style_parts(&book);
        let revisions: Vec<_> = book
            .sheet_names()
            .iter()
            .map(|name| book.sheet(name).unwrap().revision())
            .collect();
        let error = book
            .apply(Edit::Batch(vec![
                Edit::InsertRows {
                    sheet: "Data".into(),
                    at: 1,
                    count: 1,
                },
                Edit::SetEntries {
                    sheet: "Missing".into(),
                    entries: vec![(super::at("A1"), "9".into())],
                },
            ]))
            .unwrap_err();
        assert!(matches!(error, yggdryl::Error::Absent { .. }), "{error}");
        assert_eq!(state(&book), before);
        assert_eq!(
            book.sheet_names()
                .iter()
                .map(|name| book.sheet(name).unwrap().revision())
                .collect::<Vec<_>>(),
            revisions
        );
        assert!(
            retained_style_parts(&book) == parts,
            "a failed row edit changed a rich package member"
        );
        assert!(book.is_dirty());
        book.rebase(older).unwrap();
        assert!(
            book.is_dirty(),
            "the older save cleared a later equal-byte overlay insertion"
        );
        assert!(
            retained_style_parts(&book) == parts,
            "rebase changed the restored logical package"
        );
    }

    /// Restoring a sheet also replaces names and views that its removal did
    /// not rewrite. Its inverse must capture their current values explicitly.
    #[test]
    fn restored_sheet_inverse_restores_intervening_names_and_views_after_saves() {
        use crate::excel_package::{
            content_types, package, root_relationships, workbook, workbook_relationships, worksheet,
        };
        use yggdryl::excel::Edit;
        let document = workbook(&["Gone", "Keep", "Other"], false)
            .replace("<sheets>", "<bookViews><workbookView activeTab=\"1\" firstSheet=\"1\"/></bookViews><sheets>")
            .replace("</workbook>", "<definedNames><definedName name=\"Kept\">Keep!$A$1</definedName></definedNames></workbook>");
        let mut book = Workbook::from_bytes(package(&[
            ("[Content_Types].xml", &content_types(3, false, false)),
            ("_rels/.rels", &root_relationships()),
            ("xl/workbook.xml", &document),
            (
                "xl/_rels/workbook.xml.rels",
                &workbook_relationships(3, false, false),
            ),
            ("xl/worksheets/sheet1.xml", &worksheet("")),
            ("xl/worksheets/sheet2.xml", &worksheet("")),
            ("xl/worksheets/sheet3.xml", &worksheet("")),
        ]))
        .unwrap();
        book.parse_all().unwrap();
        let restore = book
            .apply(Edit::RemoveSheet {
                name: "Gone".into(),
            })
            .unwrap()
            .inverse
            .unwrap();
        book.rename_sheet("Keep", "Renamed").unwrap();
        book.move_sheet("Other", 0).unwrap();
        saved(&mut book);
        assert_eq!(book.sheet_names(), ["Other", "Renamed"]);
        assert_eq!(book.active_tab(), 1);
        let before = state(&book);
        let parts = retained_style_parts(&book);
        let applied = book.apply(restore).unwrap();
        saved(&mut book);
        let restored = state(&book);
        let restored_parts = retained_style_parts(&book);
        let undone = book.apply(applied.inverse.unwrap()).unwrap();
        saved(&mut book);
        assert_eq!(state(&book), before);
        assert!(
            retained_style_parts(&book) == parts,
            "names or views changed across restored-sheet undo"
        );
        book.apply(undone.inverse.unwrap()).unwrap();
        saved(&mut book);
        assert_eq!(state(&book), restored);
        assert!(
            retained_style_parts(&book) == restored_parts,
            "names or views changed across restored-sheet redo"
        );
    }

    /// Compare logical package content, not compression or central-directory bytes.
    fn retained_style_parts(book: &Workbook) -> std::collections::BTreeMap<String, Vec<u8>> {
        use yggdryl::holder::{Buffer, Holder};
        use yggdryl::zip::ZipArchive;
        let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            book.into_bytes().unwrap(),
        ))));
        archive
            .entries()
            .unwrap()
            .into_iter()
            .map(|member| {
                let name = member.name().to_owned();
                let bytes = archive.read_member(&name).unwrap();
                (name, bytes)
            })
            .collect()
    }

    #[test]
    fn retained_style_restore_refuses_a_missing_later_sheet_before_changing_any_step() {
        use yggdryl::excel::{Edit, Paste, StylePatch};
        let mut book = Workbook::new();
        for (name, value) in [("From", 1.0), ("To", 2.0)] {
            book.add_sheet(name)
                .unwrap()
                .set_cell(super::at("A1"), value)
                .unwrap();
        }
        book.set_style(
            "From",
            &[super::range("A1")],
            &StylePatch {
                bold: Some(true),
                ..StylePatch::default()
            },
        )
        .unwrap();
        book.set_style(
            "To",
            &[super::range("A1")],
            &StylePatch {
                italic: Some(true),
                ..StylePatch::default()
            },
        )
        .unwrap();
        let missing = book.sheet_key("From").unwrap();
        let mut applied = book
            .apply(Edit::Paste {
                from: ("From".into(), super::range("A1")),
                to: ("To".into(), super::at("A1")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        // cut_paste's generated Restore first holds To, then From. The
        // first step could change To before encountering the missing key.
        let inverse = applied.inverse.as_ref().unwrap().clone();
        book.remove_sheet("From").unwrap();
        let state_before = state(&book);
        let parts_before = retained_style_parts(&book);
        let revision = book.sheet("To").unwrap().revision();
        let dirty = book.is_dirty();
        let styles: Vec<_> = (0..book.style_sheet().unwrap().len())
            .map(|id| {
                book.style_sheet()
                    .unwrap()
                    .style(yggdryl::excel::StyleId::new(u16::try_from(id).unwrap()))
                    .unwrap()
                    .clone()
            })
            .collect();
        let error = yggdryl::internals::excel_edit::restore_direct(&mut book, inverse).unwrap_err();
        assert!(matches!(error, yggdryl::Error::Absent { .. }));
        assert!(
            error.to_string().contains(&missing.as_u32().to_string()),
            "{error}"
        );
        assert_eq!(state(&book), state_before);
        assert_eq!(book.sheet("To").unwrap().revision(), revision);
        assert_eq!(book.is_dirty(), dirty);
        assert_eq!(book.style_sheet().unwrap().len(), styles.len());
        for (id, style) in styles.iter().enumerate() {
            assert_eq!(
                book.style_sheet()
                    .unwrap()
                    .style(yggdryl::excel::StyleId::new(u16::try_from(id).unwrap())),
                Some(style)
            );
        }
        assert_eq!(retained_style_parts(&book), parts_before);
        let mut journal = Journal::new(10, applied.bytes);
        assert!(journal.record("Cut cells", &mut applied));
        assert!(journal.undo(&mut book).is_err());
        assert_eq!(journal.labels(), (Some("Cut cells"), None));
        assert_eq!(journal.len(), (1, 0));
        assert_eq!(state(&book), state_before);
        assert_eq!(retained_style_parts(&book), parts_before);
    }

    #[test]
    fn restored_sheet_inverse_restores_an_intervening_formula_after_saves() {
        use yggdryl::excel::Edit;
        let mut book = Workbook::new();
        book.add_sheet("Gone").unwrap();
        book.add_sheet("Keep").unwrap();
        book.set_entry("Keep", super::at("A1"), "=Gone!A1").unwrap();
        let restore = book
            .apply(Edit::RemoveSheet {
                name: "Gone".into(),
            })
            .unwrap()
            .inverse
            .unwrap();
        book.set_entry("Keep", super::at("A1"), "=1+2").unwrap();
        saved(&mut book);
        let before = (state(&book), retained_style_parts(&book));

        let applied = book.apply(restore).unwrap();
        assert_eq!(
            book.entry_text("Keep", super::at("A1")).unwrap().as_deref(),
            Some("=Gone!A1")
        );
        saved(&mut book);
        let restored = (state(&book), retained_style_parts(&book));
        let undone = book.apply(applied.inverse.unwrap()).unwrap();
        saved(&mut book);
        assert_eq!((state(&book), retained_style_parts(&book)), before);

        book.apply(undone.inverse.unwrap()).unwrap();
        saved(&mut book);
        assert_eq!((state(&book), retained_style_parts(&book)), restored);
    }

    /// Every forward edit and its generated inverse must leave no trace when a
    /// later child refuses, including either side of a nested Batch boundary.
    #[test]
    fn atomicity_every_edit_and_inverse_survives_a_later_batch_refusal() {
        use yggdryl::excel::Edit;

        let mut checked = 0;
        for edit in super::edits() {
            let label = edit.label();
            for inverse in [false, true] {
                for nesting in 0..3 {
                    let mut book = super::book();
                    let mut control = super::book();
                    let target = if inverse {
                        let applied = book.apply(edit.clone()).unwrap();
                        control.apply(edit.clone()).unwrap();
                        applied.inverse.expect("the existing corpus is undoable")
                    } else {
                        edit.clone()
                    };
                    saved(&mut book);
                    saved(&mut control);
                    let before = (
                        state(&book),
                        book.sheet_names()
                            .iter()
                            .map(|name| (name.to_string(), book.sheet(name).unwrap().revision()))
                            .collect::<Vec<_>>(),
                        book.style_sheet().unwrap().len(),
                        book.is_dirty(),
                        retained_style_parts(&book),
                    );
                    let prefix = Edit::SetEntries {
                        sheet: book.sheet_names()[0].into(),
                        entries: vec![(super::at("Z100"), "before nested failure".into())],
                    };
                    let missing = Edit::SetEntries {
                        sheet: "AtomicMissing".into(),
                        entries: vec![(super::at("A1"), "9".into())],
                    };
                    let attempted = match nesting {
                        0 => Edit::Batch(vec![target, missing]),
                        1 => Edit::Batch(vec![Edit::Batch(vec![target]), missing]),
                        2 => Edit::Batch(vec![prefix, Edit::Batch(vec![target, missing])]),
                        _ => unreachable!(),
                    };
                    let context = format!("{label}, inverse={inverse}, nesting={nesting}");
                    let error = book.apply(attempted).unwrap_err();
                    assert!(
                        matches!(error, yggdryl::Error::Absent { .. })
                            && error.to_string().contains("AtomicMissing"),
                        "{context}: expected the final child's refusal, got {error}"
                    );
                    let actual_state = state(&book);
                    assert_eq!(
                        actual_state
                            .lines()
                            .zip(before.0.lines())
                            .position(|(actual, expected)| actual != expected),
                        None,
                        "{context}: first changed state line"
                    );
                    assert_eq!(
                        actual_state.len(),
                        before.0.len(),
                        "{context}: state length"
                    );
                    assert_eq!(
                        book.sheet_names()
                            .iter()
                            .map(|name| {
                                (name.to_string(), book.sheet(name).unwrap().revision())
                            })
                            .collect::<Vec<_>>(),
                        before.1,
                        "{context}: revisions"
                    );
                    assert_eq!(
                        book.style_sheet().unwrap().len(),
                        before.2,
                        "{context}: style count"
                    );
                    assert_eq!(book.is_dirty(), before.3, "{context}: dirty");
                    let actual_parts = retained_style_parts(&book);
                    assert_eq!(
                        actual_parts.keys().collect::<Vec<_>>(),
                        before.4.keys().collect::<Vec<_>>(),
                        "{context}: members"
                    );
                    for (member, expected) in &before.4 {
                        let actual = &actual_parts[member];
                        assert_eq!(
                            actual.iter().zip(expected).position(|(a, b)| a != b),
                            None,
                            "{context}: first changed byte in {member}"
                        );
                        assert_eq!(
                            actual.len(),
                            expected.len(),
                            "{context}: byte count in {member}"
                        );
                    }

                    // The same next insertion proves both the private counters
                    // and stable worksheet-part reservation survived failure.
                    book.add_sheet("AfterFailure").unwrap();
                    control.add_sheet("AfterFailure").unwrap();
                    assert_eq!(
                        book.sheet_key("AfterFailure"),
                        control.sheet_key("AfterFailure"),
                        "{context}"
                    );
                    assert_eq!(
                        retained_style_parts(&book),
                        retained_style_parts(&control),
                        "{context}: next sheetId/part"
                    );
                    checked += 1;
                }
            }
        }
        eprintln!("atomicity rich corpus: {checked} forward/inverse/nesting cases");
    }

    #[test]
    fn atomicity_guarded_failure_restores_styles_revisions_and_saved_package() {
        use yggdryl::excel::Edit;
        let mut book = Workbook::new();
        let sheet = book.add_sheet("Sheet1").unwrap();
        sheet.set_cell(super::at("A1"), 1.0).unwrap();
        sheet.set_cell(super::at("A2"), 2.0).unwrap();
        saved(&mut book);
        let before = (
            state(&book),
            book.sheet("Sheet1").unwrap().revision(),
            book.style_sheet().unwrap().len(),
            book.is_dirty(),
            retained_style_parts(&book),
        );
        let error = book
            .apply(Edit::SetEntries {
                sheet: "Sheet1".into(),
                entries: vec![
                    (super::at("A1"), "12%".into()),
                    (super::at("A2"), "=SUM(A1".into()),
                ],
            })
            .unwrap_err();
        assert!(matches!(error, yggdryl::Error::Parse { .. }), "{error}");
        assert_eq!(
            (
                state(&book),
                book.sheet("Sheet1").unwrap().revision(),
                book.style_sheet().unwrap().len(),
                book.is_dirty(),
                retained_style_parts(&book)
            ),
            before
        );
    }

    #[test]
    fn atomicity_invalid_add_position_preserves_formulas_naming_the_absent_sheet() {
        use yggdryl::excel::Edit;
        let mut book = Workbook::new();
        book.add_sheet("Ref").unwrap();
        book.set_entry("Ref", super::at("A1"), "=Future!A1")
            .unwrap();
        saved(&mut book);
        let before = (
            state(&book),
            book.entry_text("Ref", super::at("A1")).unwrap(),
            book.sheet("Ref").unwrap().revision(),
            book.is_dirty(),
            retained_style_parts(&book),
        );
        assert!(
            book.apply(Edit::AddSheet {
                name: Some("Future".into()),
                at: Some(usize::MAX)
            })
            .is_err()
        );
        assert_eq!(
            (
                state(&book),
                book.entry_text("Ref", super::at("A1")).unwrap(),
                book.sheet("Ref").unwrap().revision(),
                book.is_dirty(),
                retained_style_parts(&book)
            ),
            before
        );
    }

    #[test]
    fn atomicity_journal_failed_compound_undo_restores_the_removed_sheet_and_metadata() {
        use yggdryl::excel::Edit;
        let mut book = Workbook::new();
        book.add_sheet("Ref").unwrap();
        book.add_sheet("Keep").unwrap();
        book.set_entry("Ref", super::at("A1"), "=Future!A1")
            .unwrap();
        let mut journal = Journal::new(10, 1 << 20);
        let mut applied = book
            .apply(Edit::AddSheet {
                name: Some("Future".into()),
                at: None,
            })
            .unwrap();
        assert!(journal.record("Add Future", &mut applied));
        book.remove_sheet("Ref").unwrap();
        saved(&mut book);
        let key = book.sheet_key("Future").unwrap();
        let before = (
            state(&book),
            book.sheet("Future").unwrap().revision(),
            book.is_dirty(),
            retained_style_parts(&book),
        );
        let error = journal.undo(&mut book).unwrap_err();
        assert!(matches!(error, yggdryl::Error::Absent { .. }), "{error}");
        assert_eq!(book.sheet_key("Future"), Some(key));
        assert_eq!(
            (
                state(&book),
                book.sheet("Future").unwrap().revision(),
                book.is_dirty(),
                retained_style_parts(&book)
            ),
            before
        );
        assert_eq!(journal.len(), (1, 0));
        assert_eq!(journal.labels(), (Some("Add Future"), None));
    }

    #[test]
    fn atomicity_journal_failed_compound_redo_restores_the_earlier_row_insertion() {
        use yggdryl::excel::Edit;
        let mut book = Workbook::new();
        for (name, value) in [("A", 1.0), ("B", 2.0)] {
            book.add_sheet(name)
                .unwrap()
                .set_cell(super::at("A1"), value)
                .unwrap();
        }
        let mut journal = Journal::new(10, 1 << 20);
        let mut applied = book
            .apply(Edit::Batch(vec![
                Edit::InsertRows {
                    sheet: "B".into(),
                    at: 0,
                    count: 1,
                },
                Edit::SetEntries {
                    sheet: "A".into(),
                    entries: vec![(super::at("A1"), "8".into())],
                },
            ]))
            .unwrap();
        assert!(journal.record("Rows and value", &mut applied));
        journal.undo(&mut book).unwrap().unwrap();
        book.remove_sheet("A").unwrap();
        saved(&mut book);
        let before = (
            state(&book),
            book.sheet("B").unwrap().revision(),
            book.is_dirty(),
            retained_style_parts(&book),
        );
        let error = journal.redo(&mut book).unwrap_err();
        assert!(matches!(error, yggdryl::Error::Absent { .. }), "{error}");
        assert_eq!(
            (
                state(&book),
                book.sheet("B").unwrap().revision(),
                book.is_dirty(),
                retained_style_parts(&book)
            ),
            before
        );
        assert_eq!(journal.len(), (0, 1));
        assert_eq!(journal.labels(), (None, Some("Rows and value")));
    }

    #[test]
    fn retained_style_batch_refusal_restores_the_style_table_and_serialized_value() {
        use yggdryl::excel::{Edit, StylePatch};
        let mut book = Workbook::new();
        book.add_sheet("Sheet1")
            .unwrap()
            .set_cell(super::at("A1"), 7.0)
            .unwrap();
        saved(&mut book);
        let before = retained_style_parts(&book);
        let revision = book.sheet("Sheet1").unwrap().revision();
        let styles = book.style_sheet().unwrap().len();
        assert!(!book.is_dirty());
        let error = book
            .apply(Edit::Batch(vec![
                Edit::SetStyle {
                    sheet: "Sheet1".into(),
                    ranges: vec![super::range("A1")],
                    patch: StylePatch {
                        bold: Some(true),
                        ..StylePatch::default()
                    },
                },
                Edit::SetEntries {
                    sheet: "Missing".into(),
                    entries: vec![(super::at("A1"), "9".into())],
                },
            ]))
            .unwrap_err();
        assert!(matches!(error, yggdryl::Error::Absent { .. }));
        assert_eq!(
            book.sheet("Sheet1").unwrap().scalar(super::at("A1")),
            7.0.into()
        );
        assert!(
            !book
                .cell_style("Sheet1", super::at("A1"))
                .unwrap()
                .font
                .bold
        );
        // Aggregate these independent state checks so the red identifies
        // both retained styles and dirtiness instead of stopping at one.
        assert_eq!(
            (
                book.style_sheet().unwrap().len(),
                book.sheet("Sheet1").unwrap().revision(),
                book.is_dirty(),
                retained_style_parts(&book)
            ),
            (styles, revision, false, before),
        );
    }

    /// A worksheet whose inherited namespace, envelope and root prefix
    /// distinguish the complete frame from its carried children alone.
    fn frame_document(
        prefix: &str,
        declaration: &str,
        attributes: &str,
        epilogue: &str,
        value: u32,
    ) -> String {
        let namespace = crate::excel_package::NS;
        format!(
            "{declaration}<{prefix}:worksheet xmlns:{prefix}=\"{namespace}\" {attributes}><{prefix}:sheetPr v:flag='kept'/><{prefix}:dimension ref=\"A1\"/><{prefix}:sheetData><{prefix}:row r=\"1\"><{prefix}:c r=\"A1\"><{prefix}:v>{value}</{prefix}:v></{prefix}:c></{prefix}:row></{prefix}:sheetData></{prefix}:worksheet>{epilogue}"
        )
    }

    fn frame_book(xml: &str) -> Workbook {
        use crate::excel_package::{
            content_types, package, root_relationships, workbook, workbook_relationships,
        };

        let types = content_types(1, false, false);
        let root = root_relationships();
        let book = workbook(&["Sheet1"], false);
        let relationships = workbook_relationships(1, false, false);
        Workbook::from_bytes(package(&[
            ("[Content_Types].xml", types.as_str()),
            ("_rels/.rels", root.as_str()),
            ("xl/workbook.xml", book.as_str()),
            ("xl/_rels/workbook.xml.rels", relationships.as_str()),
            ("xl/worksheets/sheet1.xml", xml),
        ]))
        .unwrap()
    }

    fn frame_xml(workbook: &Workbook) -> String {
        use yggdryl::holder::{Buffer, Holder};
        use yggdryl::zip::ZipArchive;

        let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            workbook.into_bytes().unwrap(),
        ))));
        String::from_utf8(archive.read_member("xl/worksheets/sheet1.xml").unwrap()).unwrap()
    }

    #[test]
    fn frame_inverse_restores_the_whole_envelope_across_saves() {
        let original = frame_document(
            "s",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- original -->\n",
            "xmlns:v='urn:original'",
            "\n<?tail original?>\n",
            7,
        );
        let declaration = "<?xml version='1.0' encoding='UTF-8'?>\r\n<!-- replacement -->\r\n";
        let attributes = "xmlns:v='urn:vendor:&quot;quoted&quot;&amp;scope'";
        let epilogue = "\r\n<!-- replacement tail -->\r\n<?tail replacement?>";
        let donor = frame_book(&frame_document("q", declaration, attributes, epilogue, 999));
        let expected = frame_document("q", declaration, attributes, epilogue, 7);
        let mut workbook = frame_book(&original);
        let key = workbook.sheet_key("Sheet1").unwrap();
        assert_eq!(frame_xml(&workbook), original);
        let applied = workbook
            .apply(frame_edit(&workbook, key, donor.sheet("Sheet1").unwrap()))
            .unwrap();
        assert_eq!(frame_xml(&workbook), expected);
        saved(&mut workbook);
        assert_eq!(frame_xml(&workbook), expected);
        let undone = workbook
            .apply(applied.inverse.expect("frame undo"))
            .unwrap();
        assert_eq!(frame_xml(&workbook), original);
        saved(&mut workbook);
        assert_eq!(frame_xml(&workbook), original);
        workbook.apply(undone.inverse.expect("frame redo")).unwrap();
        saved(&mut workbook);
        assert_eq!(frame_xml(&workbook), expected);
        assert_eq!(
            workbook.sheet("Sheet1").unwrap().scalar(super::at("A1")),
            7.0.into()
        );
    }

    #[test]
    fn frame_inverse_restores_fresh_absence_after_saving_and_redoing() {
        let mut workbook = Workbook::new();
        workbook
            .add_sheet("Sheet1")
            .unwrap()
            .set_cell(super::at("A1"), 7.0)
            .unwrap();
        saved(&mut workbook);
        assert!(!has_frame(workbook.sheet("Sheet1").unwrap()));
        let before = frame_xml(&workbook);
        let key = workbook.sheet_key("Sheet1").unwrap();
        let donor = frame_book(&frame_document("q", "", "xmlns:v='urn:donor'", "", 999));
        let expected = frame_document("q", "", "xmlns:v='urn:donor'", "", 7);
        let applied = workbook
            .apply(frame_edit(&workbook, key, donor.sheet("Sheet1").unwrap()))
            .unwrap();
        assert!(has_frame(workbook.sheet("Sheet1").unwrap()));
        assert_eq!(frame_xml(&workbook), expected);
        saved(&mut workbook);
        let undone = workbook
            .apply(applied.inverse.expect("frame undo"))
            .unwrap();
        assert!(!has_frame(workbook.sheet("Sheet1").unwrap()));
        assert_eq!(frame_xml(&workbook), before);
        saved(&mut workbook);
        assert!(!has_frame(workbook.sheet("Sheet1").unwrap()));
        workbook.apply(undone.inverse.expect("frame redo")).unwrap();
        saved(&mut workbook);
        assert!(has_frame(workbook.sheet("Sheet1").unwrap()));
        assert_eq!(frame_xml(&workbook), expected);
    }

    #[test]
    fn frame_inverse_keeps_the_carried_pane_distinct_from_the_model_pane() {
        use yggdryl::excel::Frozen;

        let document = |prefix: &str, rows| {
            frame_document(prefix, "", "xmlns:v='urn:pane'", "", 7).replace(
                &format!("<{prefix}:sheetData>"),
                &format!(
                    "<{prefix}:sheetViews><{prefix}:sheetView workbookViewId=\"0\"><{prefix}:pane ySplit=\"{rows}\" topLeftCell=\"A{}\" activePane=\"bottomLeft\" state=\"frozen\"/></{prefix}:sheetView></{prefix}:sheetViews><{prefix}:sheetData>",
                    rows + 1
                ),
            )
        };
        let donor = frame_book(&document("q", 1));
        let mut workbook = frame_book(&document("s", 2));
        let model_pane = Some(Frozen {
            rows: 3,
            columns: 0,
        });
        workbook
            .sheet_mut("Sheet1")
            .unwrap()
            .set_frozen(model_pane)
            .unwrap();
        let before = frame_xml(&workbook);
        let key = workbook.sheet_key("Sheet1").unwrap();
        let applied = workbook
            .apply(frame_edit(&workbook, key, donor.sheet("Sheet1").unwrap()))
            .unwrap();
        assert_eq!(workbook.sheet("Sheet1").unwrap().frozen(), model_pane);
        let after = frame_xml(&workbook);
        assert!(after.contains("ySplit=\"3\""), "{after}");
        assert!(!after.contains("ySplit=\"1\""), "{after}");
        saved(&mut workbook);
        workbook
            .apply(applied.inverse.expect("frame undo"))
            .unwrap();
        assert_eq!(workbook.sheet("Sheet1").unwrap().frozen(), model_pane);
        assert_eq!(frame_xml(&workbook), before);
    }

    #[test]
    fn frame_inverse_counts_each_envelope_buffer_in_its_byte_budget() {
        let padding = "x".repeat(8_192);
        let donor = frame_book(&frame_document("q", "", "xmlns:v='urn:donor'", "", 999));
        for location in ["declaration", "root", "epilogue"] {
            let declaration = if location == "declaration" {
                format!("<?xml version=\"1.0\"?>\n<!--{padding}-->\n")
            } else {
                String::new()
            };
            let attributes = if location == "root" {
                format!("xmlns:v='urn:{padding}'")
            } else {
                "xmlns:v='urn:original'".to_owned()
            };
            let epilogue = if location == "epilogue" {
                format!("\n<!--{padding}-->\n")
            } else {
                String::new()
            };
            let mut workbook = frame_book(&frame_document(
                "s",
                &declaration,
                &attributes,
                &epilogue,
                7,
            ));
            let key = workbook.sheet_key("Sheet1").unwrap();
            let mut applied = workbook
                .apply(frame_edit(&workbook, key, donor.sheet("Sheet1").unwrap()))
                .unwrap();
            assert!(
                applied.bytes >= padding.len(),
                "{location}: {} bytes omitted the retained envelope",
                applied.bytes
            );
            let mut journal = Journal::new(4, padding.len() - 1);
            assert!(!journal.record("Frame", &mut applied), "{location}");
            assert!(journal.is_empty(), "{location}");
        }
    }

    /// The first line two states differ on, for a readable failure.
    fn first_difference(before: &str, after: &str) -> String {
        before
            .lines()
            .zip(after.lines())
            .find(|(first, second)| first != second)
            .map_or_else(
                || {
                    format!(
                        "{} lines against {}",
                        before.lines().count(),
                        after.lines().count()
                    )
                },
                |(first, second)| format!("{first}\n    became\n{second}"),
            )
    }

    /// Save `workbook` and read from the package it wrote: every part an
    /// edit rewrote is the member from now on.
    fn saved(workbook: &mut Workbook) {
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        workbook.parse_all().unwrap();
    }

    #[test]
    fn every_edit_undone_after_a_save_is_the_workbook_it_found_and_redone_after_another_the_one_it_made()
     {
        for edit in super::edits() {
            let mut workbook: Workbook = super::book();
            let label = edit.label();
            let before = state(&workbook);
            let applied = workbook
                .apply(edit)
                .unwrap_or_else(|error| panic!("{label}: {error}"));
            saved(&mut workbook);
            let after = state(&workbook);
            let undone = workbook
                .apply(applied.inverse.expect("an undo"))
                .unwrap_or_else(|error| panic!("{label} undone: {error}"));
            let found = state(&workbook);
            assert!(
                found == before,
                "{label} undone after a save: {}",
                first_difference(&before, &found)
            );
            saved(&mut workbook);
            workbook
                .apply(undone.inverse.expect("a redo"))
                .unwrap_or_else(|error| panic!("{label} redone: {error}"));
            // Compare persisted identities and membership on both sides: a
            // fresh sheet now owns a part, and deleted sidecars have gone.
            saved(&mut workbook);
            let made = state(&workbook);
            assert!(
                made == after,
                "{label} redone after a save: {}",
                first_difference(&after, &made)
            );
        }
    }

    #[test]
    fn every_edit_undone_is_the_workbook_it_found_and_redone_the_one_it_made() {
        for edit in super::edits() {
            let mut workbook: Workbook = super::book();
            let label = edit.label();
            let before = state(&workbook);
            let applied = workbook
                .apply(edit)
                .unwrap_or_else(|error| panic!("{label}: {error}"));
            let after = state(&workbook);
            assert_ne!(before, after, "{label} changed nothing");
            let undone = workbook
                .apply(applied.inverse.expect("an undo"))
                .unwrap_or_else(|error| panic!("{label} undone: {error}"));
            let found = state(&workbook);
            assert!(
                found == before,
                "{label} undone: {}",
                first_difference(&before, &found)
            );
            workbook
                .apply(undone.inverse.expect("a redo"))
                .unwrap_or_else(|error| panic!("{label} redone: {error}"));
            let made = state(&workbook);
            assert!(
                made == after,
                "{label} redone: {}",
                first_difference(&after, &made)
            );
        }
    }
}

/// Sheet keys identify a sheet only inside one workbook. An opaque inverse
/// must not turn a coincident key in another workbook into its target.
#[test]
fn foreign_restore_refuses_a_matching_sheet_key_without_styles() {
    let mut source = Workbook::new();
    source
        .add_sheet("Data")
        .unwrap()
        .set_cell(at("A1"), 7.0)
        .unwrap();
    let inverse = source
        .apply(Edit::Clear {
            sheet: "Data".into(),
            ranges: vec![range("A1")],
            what: Clear::All,
        })
        .unwrap()
        .inverse
        .unwrap();
    let mut target = Workbook::new();
    target
        .add_sheet("Data")
        .unwrap()
        .set_cell(at("A1"), 99.0)
        .unwrap();
    assert_eq!(source.sheet_key("Data"), target.sheet_key("Data"));
    let before = target.into_package().unwrap();
    target.rebase(before).unwrap();
    let revision = target.sheet("Data").unwrap().revision();
    let error = target
        .apply(inverse)
        .expect_err("an opaque inverse belongs to its workbook");
    assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
    assert_eq!(
        target.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(99.0)
    );
    assert_eq!(target.sheet("Data").unwrap().revision(), revision);
    assert!(!target.is_dirty());
}

/// Original IDs are intentionally not copied into retained descriptors:
/// their meaning is stable inside their source workbook, never globally.
#[test]
fn foreign_restore_refuses_original_style_ids_with_another_meaning() {
    let loaded = |bold: bool| {
        let mut book = Workbook::new();
        book.add_sheet("Data")
            .unwrap()
            .set_cell(at("A1"), if bold { 7.0 } else { 99.0 })
            .unwrap();
        book.set_style(
            "Data",
            &[range("A1")],
            &StylePatch {
                bold: Some(bold),
                italic: Some(!bold),
                ..StylePatch::default()
            },
        )
        .unwrap();
        let book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
        book.parse_all().unwrap();
        book
    };
    let mut source = loaded(true);
    let mut target = loaded(false);
    assert_eq!(source.sheet_key("Data"), target.sheet_key("Data"));
    assert_eq!(
        source
            .sheet("Data")
            .unwrap()
            .cell(at("A1"))
            .unwrap()
            .style(),
        target
            .sheet("Data")
            .unwrap()
            .cell(at("A1"))
            .unwrap()
            .style()
    );
    assert!(source.cell_style("Data", at("A1")).unwrap().font.bold);
    let style = target.cell_style("Data", at("A1")).unwrap();
    assert!(style.font.italic);
    let revision = target.sheet("Data").unwrap().revision();
    let inverse = source
        .apply(Edit::Clear {
            sheet: "Data".into(),
            ranges: vec![range("A1")],
            what: Clear::All,
        })
        .unwrap()
        .inverse
        .unwrap();
    let error = target
        .apply(inverse)
        .expect_err("the original XF prefix belongs to one workbook");
    assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
    assert_eq!(
        target.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(99.0)
    );
    assert_eq!(target.cell_style("Data", at("A1")).unwrap(), style);
    assert_eq!(target.sheet("Data").unwrap().revision(), revision);
    assert!(!target.is_dirty());
}

/// A removed sheet also carries workbook-owned parts, names and original
/// XF IDs. Unused destination identities do not authorize a foreign undo.
#[test]
fn foreign_restore_sheet_refuses_unused_identities_and_original_style_aliases() {
    let mut source = Workbook::new();
    source.add_sheet("Keep").unwrap();
    source.add_sheet("Aux").unwrap();
    source
        .add_sheet("Foreign")
        .unwrap()
        .set_cell(at("A1"), 7.0)
        .unwrap();
    source
        .set_style(
            "Foreign",
            &[range("A1")],
            &StylePatch {
                bold: Some(true),
                ..StylePatch::default()
            },
        )
        .unwrap();
    let mut source = Workbook::from_bytes(source.into_bytes().unwrap()).unwrap();
    source.parse_all().unwrap();
    let inverse = source
        .apply(Edit::RemoveSheet {
            name: "Foreign".into(),
        })
        .unwrap()
        .inverse
        .unwrap();

    // Three source sheets put Foreign at sheet3/rId3; the destination's
    // sole sheet and styles use sheet1/rId1/rId2. No ordinary part, key,
    // sheetId, relationship-id or sheet-name collision can mask this test.
    let mut target = Workbook::new();
    target
        .add_sheet("Target")
        .unwrap()
        .set_cell(at("A1"), 99.0)
        .unwrap();
    target
        .set_style(
            "Target",
            &[range("A1")],
            &StylePatch {
                italic: Some(true),
                ..StylePatch::default()
            },
        )
        .unwrap();
    let mut target = Workbook::from_bytes(target.into_bytes().unwrap()).unwrap();
    target.parse_all().unwrap();
    let style = target.cell_style("Target", at("A1")).unwrap();
    let revision = target.sheet("Target").unwrap().revision();
    let error = target
        .apply(inverse)
        .expect_err("removed sheet state belongs to its source workbook");
    assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
    assert_eq!(target.sheet_names(), vec!["Target"]);
    assert_eq!(
        target.sheet("Target").unwrap().scalar(at("A1")),
        Scalar::from(99.0)
    );
    assert_eq!(target.cell_style("Target", at("A1")).unwrap(), style);
    assert_eq!(target.sheet("Target").unwrap().revision(), revision);
    assert!(!target.is_dirty());
}

/// An opaque foreign child is refused before any earlier batch edit can
/// append styles or change revisions, including through nested batches.
#[test]
fn foreign_restore_in_a_batch_refuses_before_the_preceding_style_edit() {
    let mut source = Workbook::new();
    source
        .add_sheet("Data")
        .unwrap()
        .set_cell(at("A1"), 7.0)
        .unwrap();
    let foreign = source
        .apply(Edit::Clear {
            sheet: "Data".into(),
            ranges: vec![range("A1")],
            what: Clear::All,
        })
        .unwrap()
        .inverse
        .unwrap();
    let mut target = Workbook::new();
    target
        .add_sheet("Data")
        .unwrap()
        .set_cell(at("A1"), 99.0)
        .unwrap();
    let package = target.into_package().unwrap();
    target.rebase(package).unwrap();
    let count = target.style_sheet().unwrap().len();
    let style = target.cell_style("Data", at("A1")).unwrap();
    let revision = target.sheet("Data").unwrap().revision();
    let error = target
        .apply(Edit::Batch(vec![
            Edit::SetStyle {
                sheet: "Data".into(),
                ranges: vec![range("A1")],
                patch: StylePatch {
                    bold: Some(true),
                    ..StylePatch::default()
                },
            },
            Edit::Batch(vec![foreign]),
        ]))
        .expect_err("foreign inverses are preflighted before any batch child");
    assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
    assert_eq!(target.style_sheet().unwrap().len(), count);
    assert_eq!(
        target.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(99.0)
    );
    assert_eq!(target.cell_style("Data", at("A1")).unwrap(), style);
    assert_eq!(target.sheet("Data").unwrap().revision(), revision);
    assert!(!target.is_dirty());
}

/// A rejected position must not consume a sheet identity: removing a
/// briefly inserted tab is observably different from refusing insertion.
#[test]
fn atomicity_invalid_add_position_keeps_the_next_sheet_key() {
    for position in [2, usize::MAX] {
        let mut book = Workbook::new();
        book.add_sheet("Keep").unwrap();
        let previous = book.sheet_key("Keep").unwrap().as_u32();
        let package = book.into_package().unwrap();
        book.rebase(package).unwrap();
        let error = book
            .apply(Edit::AddSheet {
                name: Some("Refused".into()),
                at: Some(position),
            })
            .unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error:?}");
        assert_eq!(book.sheet_names(), vec!["Keep"]);
        book.add_sheet("Next").unwrap();
        assert_eq!(
            book.sheet_key("Next").unwrap().as_u32(),
            previous + 1,
            "refusing position {position} consumed the next identity",
        );
    }
}

// Append at the outer scope of rust/tests/excel/edit.rs. No new helpers.
// Source-inspected candidates only; no Cargo execution by the author.

#[test]
fn atomicity_zero_count_bands_do_not_mutate_during_batch_rollback() {
    let edits = [
        Edit::InsertRows {
            sheet: "Data".into(),
            at: 0,
            count: 0,
        },
        Edit::RemoveRows {
            sheet: "Data".into(),
            start: 0,
            count: 0,
        },
        Edit::InsertColumns {
            sheet: "Data".into(),
            at: 0,
            count: 0,
        },
        Edit::RemoveColumns {
            sheet: "Data".into(),
            start: 0,
            count: 0,
        },
    ];
    for edit in edits {
        for nested in [false, true] {
            let mut initial = Workbook::new();
            initial
                .add_sheet("Data")
                .unwrap()
                .set_cell(at("A1"), 7.0)
                .unwrap();
            let mut workbook = Workbook::from_bytes(initial.into_bytes().unwrap()).unwrap();
            let revision = workbook.sheet("Data").unwrap().revision();
            assert!(!workbook.is_dirty());
            let label = edit.label();
            let target = if nested {
                Edit::Batch(vec![edit.clone()])
            } else {
                edit.clone()
            };
            let error = workbook
                .apply(Edit::Batch(vec![
                    target,
                    Edit::SetEntries {
                        sheet: "Missing".into(),
                        entries: vec![(at("A1"), "9".into())],
                    },
                ]))
                .unwrap_err();
            assert!(
                matches!(error, Error::Absent { .. }),
                "{label}, nested={nested}: {error}"
            );
            assert_eq!(
                workbook.sheet("Data").unwrap().revision(),
                revision,
                "{label}, nested={nested}: a no-op inverse changed the revision"
            );
            assert!(!workbook.is_dirty(), "{label}, nested={nested}");
            assert_eq!(workbook.sheet("Data").unwrap().scalar(at("A1")), 7.0.into());
        }
    }
}

#[test]
fn atomicity_noop_sheet_state_on_an_unparsed_sheet_needs_no_rollback_parse() {
    for nested in [false, true] {
        let mut initial = Workbook::new();
        initial
            .add_sheet("Data")
            .unwrap()
            .set_cell(at("A1"), 7.0)
            .unwrap();
        let mut workbook = Workbook::from_bytes(initial.into_bytes().unwrap()).unwrap();
        // Do not call sheet()/parse_all()/state() here: the loaded Slot must
        // still be lazy when the forward no-op returns before parsed().
        assert_eq!(workbook.sheet_state("Data"), Some(SheetState::Visible));
        assert!(!workbook.is_dirty());
        let target = Edit::SheetState {
            name: "Data".into(),
            state: SheetState::Visible,
        };
        let target = if nested {
            Edit::Batch(vec![target])
        } else {
            target
        };
        let error = workbook
            .apply(Edit::Batch(vec![
                target,
                Edit::SetEntries {
                    sheet: "Missing".into(),
                    entries: vec![(at("A1"), "9".into())],
                },
            ]))
            .unwrap_err();
        assert!(
            matches!(error, Error::Absent { .. }),
            "nested={nested}: {error}"
        );
        assert_eq!(workbook.sheet_state("Data"), Some(SheetState::Visible));
        assert!(!workbook.is_dirty());
        assert_eq!(workbook.sheet("Data").unwrap().scalar(at("A1")), 7.0.into());
    }
}

fn insertion_style_edit_book(columns: bool, date: bool) -> Workbook {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    sheet
        .set_cell(
            at("A1"),
            if date {
                Scalar::date32(19_723)
            } else {
                Scalar::from(1)
            },
        )
        .unwrap();
    sheet.set_cell(at("B2"), 2).unwrap();
    sheet
        .set_cell(if columns { at("C2") } else { at("B3") }, 3)
        .unwrap();
    sheet
        .set_cell(if columns { at("D2") } else { at("B4") }, 4)
        .unwrap();
    workbook
}

fn insertion_style_edit_formats(workbook: &mut Workbook, columns: bool) {
    use yggdryl::excel::{Borders, Color};
    workbook
        .set_style(
            "Data",
            &[range("B2")],
            &StylePatch {
                bold: Some(true),
                borders: Some(Borders {
                    color: Some(Color::Rgb(0xFF204080)),
                    ..Borders::default()
                }),
                ..StylePatch::default()
            },
        )
        .unwrap();
    workbook
        .set_style(
            "Data",
            &[if columns { range("C2") } else { range("B3") }],
            &StylePatch {
                italic: Some(true),
                ..StylePatch::default()
            },
        )
        .unwrap();
}

fn insertion_style_edit_save(workbook: &mut Workbook) {
    let image = workbook.into_package().unwrap();
    workbook.rebase(image).unwrap();
}

fn insertion_style_edit_remove(columns: bool) -> Edit {
    if columns {
        Edit::RemoveColumns {
            sheet: "Data".into(),
            start: 2,
            count: 1,
        }
    } else {
        Edit::RemoveRows {
            sheet: "Data".into(),
            start: 2,
            count: 1,
        }
    }
}

#[test]
fn inserted_style_bands_saved_removal_undo_never_interns_temporary_inheritance() {
    use crate::excel_package::table_member_map;
    for columns in [false, true] {
        let mut workbook = insertion_style_edit_book(columns, false);
        insertion_style_edit_formats(&mut workbook, columns);
        insertion_style_edit_save(&mut workbook);
        let before = table_member_map(&workbook);
        let styles = workbook.style_sheet().unwrap().len();
        let mut undo = workbook
            .apply(insertion_style_edit_remove(columns))
            .unwrap()
            .inverse
            .unwrap();
        insertion_style_edit_save(&mut workbook);
        let removed = table_member_map(&workbook);
        for _ in 0..2 {
            let redo = workbook.apply(undo).unwrap().inverse.unwrap();
            assert_eq!(workbook.style_sheet().unwrap().len(), styles);
            insertion_style_edit_save(&mut workbook);
            assert_eq!(table_member_map(&workbook), before, "columns={columns}");
            undo = workbook.apply(redo).unwrap().inverse.unwrap();
            insertion_style_edit_save(&mut workbook);
            assert_eq!(table_member_map(&workbook), removed, "columns={columns}");
        }
    }
}

#[test]
fn inserted_style_bands_stale_save_rebinds_removed_styles_without_derived_growth() {
    for columns in [false, true] {
        let mut workbook = insertion_style_edit_book(columns, true);
        // The pending image reserves a temporal XF that a later live style
        // may otherwise take: adopt must remap the retained removed payload.
        let pending = workbook.into_package().unwrap();
        insertion_style_edit_formats(&mut workbook, columns);
        let removed_at = if columns { at("C2") } else { at("B3") };
        let expected = workbook.cell_style("Data", removed_at).unwrap();
        let undo = workbook
            .apply(insertion_style_edit_remove(columns))
            .unwrap()
            .inverse
            .unwrap();
        workbook.rebase(pending).unwrap();
        let styles = workbook.style_sheet().unwrap().len();
        workbook.apply(undo).unwrap();
        assert_eq!(workbook.style_sheet().unwrap().len(), styles);
        assert_eq!(workbook.cell_style("Data", removed_at).unwrap(), expected);
        assert_eq!(
            workbook.sheet("Data").unwrap().scalar(removed_at),
            Scalar::from(3)
        );
        insertion_style_edit_save(&mut workbook);
        let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
        assert_eq!(reopened.cell_style("Data", removed_at).unwrap(), expected);
    }
}

#[test]
fn land_all_null_unheaded_records_clears_known_width_and_reports_new_sheet() {
    use yggdryl::{ArrowCastOptions, DataType, Serie, StructType};

    let field = DataType::from(
        StructType::from_fields([
            DataType::Float64.nullable_field("Left"),
            DataType::Float64.nullable_field("Right"),
        ])
        .unwrap(),
    )
    .required_field("record");
    let records = Serie::from_scalars(
        field.clone(),
        [
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
        ],
    )
    .unwrap();
    let cells = Sheet::from_serie("Imported", &records, yggdryl::RecordHeader::None).unwrap();
    assert!(cells.dimension().is_none());

    let mut destination = Workbook::new();
    let sheet = destination.add_sheet("Data").unwrap();
    for (at, value) in [("B2", 9), ("C2", 8), ("B3", 7), ("C3", 6)] {
        sheet.set_cell(at.parse().unwrap(), value).unwrap();
    }
    let applied = destination
        .apply(Edit::Land {
            destination: Landing::At {
                sheet: "Data".into(),
                anchor: at("B2"),
            },
            cells: Box::new(cells.clone()),
        })
        .unwrap();
    assert_eq!(applied.touched, [("Data".into(), range("B2:C3"))]);
    for at in ["B2", "C2", "B3", "C3"] {
        assert_eq!(
            destination
                .sheet("Data")
                .unwrap()
                .scalar(at.parse().unwrap()),
            Scalar::Null,
            "{at}"
        );
    }
    destination.apply(applied.inverse.unwrap()).unwrap();
    assert_eq!(
        destination.sheet("Data").unwrap().scalar(at("C3")),
        Scalar::from(6)
    );

    let mut new = Workbook::new();
    let applied = new
        .apply(Edit::Land {
            destination: Landing::NewSheet("Imported".into()),
            cells: Box::new(cells),
        })
        .unwrap();
    assert_eq!(applied.touched, [("Imported".into(), range("A1:B2"))]);
    let reopened = Workbook::from_bytes(new.into_bytes().unwrap()).unwrap();
    let restored = reopened
        .sheet("Imported")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            yggdryl::RecordHeader::None,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(restored.len(), 2);
}

#[test]
fn land_at_invalid_anchor_refuses_before_changing_destination() {
    use crate::excel_package::table_member_map;

    let mut cells = Sheet::new("Imported").unwrap();
    cells.set_cell(at("A1"), 7).unwrap();
    let mut workbook = Workbook::new();
    workbook
        .add_sheet("Data")
        .unwrap()
        .set_cell(at("B2"), 9)
        .unwrap();
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    let before = table_member_map(&workbook);
    let revision = workbook.sheet("Data").unwrap().revision();
    for anchor in [CellRef::new(u32::MAX, 0), CellRef::new(0, u32::MAX)] {
        let error = workbook
            .apply(Edit::Land {
                destination: Landing::At {
                    sheet: "Data".into(),
                    anchor,
                },
                cells: Box::new(cells.clone()),
            })
            .unwrap_err();
        let (path, reason) = match error {
            Error::InvalidRecord { path, reason } => (path, reason),
            other => panic!("expected a located grid refusal, got {other:?}"),
        };
        assert!(path.contains("Data"), "{path}: {reason}");
        assert!(reason.contains("landing anchor"), "{reason}");
        assert_eq!(workbook.sheet("Data").unwrap().revision(), revision);
        assert_eq!(table_member_map(&workbook), before);
        assert!(!workbook.is_dirty());
    }
}

#[test]
fn land_null_footprint_survives_save_and_redo_but_undo_restores_exact_prior_span() {
    use yggdryl::RecordHeader;
    use yggdryl::{DataType, Serie, StructType};

    let one = DataType::from(
        StructType::from_fields([DataType::Float64.nullable_field("Only")]).unwrap(),
    )
    .required_field("record");
    let two = DataType::from(
        StructType::from_fields([
            DataType::Float64.nullable_field("Left"),
            DataType::Float64.nullable_field("Right"),
        ])
        .unwrap(),
    )
    .required_field("record");
    let initial = Serie::from_scalars(
        one,
        [
            Scalar::from_sequence([Scalar::Null]),
            Scalar::from_sequence([Scalar::Null]),
        ],
    )
    .unwrap();
    let incoming = Serie::from_scalars(
        two,
        [
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
        ],
    )
    .unwrap();
    let mut workbook = Workbook::new();
    workbook
        .insert_sheet(Sheet::from_serie("Data", &initial, RecordHeader::None).unwrap())
        .unwrap();
    let touched = |sheet: Sheet| {
        let mut observer = Workbook::new();
        let applied = observer
            .apply(Edit::Land {
                destination: Landing::NewSheet("Observed".into()),
                cells: Box::new(sheet),
            })
            .unwrap();
        applied.touched.into_iter().next().map(|(_, span)| span)
    };
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:A2"))
    );
    let applied = workbook
        .apply(Edit::Land {
            destination: Landing::At {
                sheet: "Data".into(),
                anchor: at("B2"),
            },
            cells: Box::new(Sheet::from_serie("Imported", &incoming, RecordHeader::None).unwrap()),
        })
        .unwrap();
    assert_eq!(applied.touched, [("Data".into(), range("B2:C3"))]);
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:C3"))
    );
    let undone = workbook.apply(applied.inverse.unwrap()).unwrap();
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:A2"))
    );
    workbook.apply(undone.inverse.unwrap()).unwrap();
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:C3"))
    );
}

#[test]
fn land_null_footprint_follows_slice_and_structural_row_edits() {
    use yggdryl::RecordHeader;
    use yggdryl::{DataType, Serie, StructType};

    let field = DataType::from(
        StructType::from_fields([
            DataType::Float64.nullable_field("Left"),
            DataType::Float64.nullable_field("Right"),
        ])
        .unwrap(),
    )
    .required_field("record");
    let records = Serie::from_scalars(
        field,
        [
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
        ],
    )
    .unwrap();
    let source = Sheet::from_serie("Data", &records, RecordHeader::None).unwrap();
    let touched = |sheet: Sheet| {
        let mut observer = Workbook::new();
        let applied = observer
            .apply(Edit::Land {
                destination: Landing::NewSheet("Observed".into()),
                cells: Box::new(sheet),
            })
            .unwrap();
        applied.touched.into_iter().next().map(|(_, span)| span)
    };
    assert_eq!(touched(source.slice(range("B2:B3"))), Some(range("B2:B3")));
    let mut workbook = Workbook::new();
    workbook.insert_sheet(source).unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:B3"))
    );
    let inserted = workbook
        .apply(Edit::InsertRows {
            sheet: "Data".into(),
            at: 1,
            count: 1,
        })
        .unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:B4"))
    );
    workbook.apply(inserted.inverse.unwrap()).unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:B3"))
    );
    let removed = workbook
        .apply(Edit::RemoveRows {
            sheet: "Data".into(),
            start: 1,
            count: 1,
        })
        .unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:B2"))
    );
    workbook.apply(removed.inverse.unwrap()).unwrap();
    assert_eq!(
        touched(workbook.sheet("Data").unwrap().clone()),
        Some(range("A1:B3"))
    );
}

#[test]
fn inserted_style_bands_opaque_removal_prefix_refuses_a_foreign_workbook() {
    use crate::excel_package::table_member_map;
    for columns in [false, true] {
        let mut source = insertion_style_edit_book(columns, false);
        let inverse = source
            .apply(insertion_style_edit_remove(columns))
            .unwrap()
            .inverse
            .unwrap();
        let Edit::Batch(mut pieces) = inverse else {
            panic!("structural undo retains a prefix and payload")
        };
        let prefix = pieces.remove(0);
        let mut foreign = insertion_style_edit_book(columns, false);
        insertion_style_edit_save(&mut foreign);
        let before = table_member_map(&foreign);
        let revision = foreign.sheet("Data").unwrap().revision();
        let error = foreign.apply(Edit::Batch(vec![prefix])).unwrap_err();
        assert!(matches!(error, Error::Conflict { .. }), "{error}");
        assert_eq!(foreign.sheet("Data").unwrap().revision(), revision);
        assert!(!foreign.is_dirty());
        assert_eq!(table_member_map(&foreign), before);
    }
}

/// Land is a rectangular overwrite: absent imported cells are nulls, not a
/// request to retain unrelated destination values inside that footprint.
#[test]
fn land_at_null_records_replace_existing_values_and_round_trip_the_inverse() {
    use crate::excel_package::table_member_map;
    use yggdryl::RecordHeader;
    use yggdryl::{DataType, Serie, StructType};

    let field = DataType::from(
        StructType::from_fields([DataType::Float64.nullable_field("Amount")]).unwrap(),
    )
    .required_field("record");
    for (header, values, span, expected) in [
        (
            RecordHeader::None,
            vec![Some(1.0), None, Some(3.0)],
            "B2:B4",
            vec![Scalar::from(1.0), Scalar::Null, Scalar::from(3.0)],
        ),
        (
            RecordHeader::Source,
            vec![Some(1.0), None, None],
            "B2:B5",
            vec![
                Scalar::from("Amount"),
                Scalar::from(1.0),
                Scalar::Null,
                Scalar::Null,
            ],
        ),
        (
            RecordHeader::Source,
            vec![None, None, None],
            "B2:B5",
            vec![
                Scalar::from("Amount"),
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
            ],
        ),
    ] {
        let serie = Serie::from_scalars(
            field.clone(),
            values
                .into_iter()
                .map(|value| Scalar::from_sequence([value.map_or(Scalar::Null, Scalar::from)])),
        )
        .unwrap();
        let cells = Sheet::from_serie("Imported", &serie, header).unwrap();
        let mut workbook = Workbook::new();
        let held = workbook.add_sheet("Data").unwrap();
        for row in 1..6 {
            held.set_cell(CellRef::new(row, 1), 90.0 + f64::from(row))
                .unwrap();
        }
        held.set_cell(at("C3"), "outside").unwrap();
        // The saved inverse reuses the append-only shared string table: the
        // Source header must already be interned before the exact byte pin.
        held.set_cell(at("D1"), "Amount").unwrap();
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Land {
                destination: Landing::At {
                    sheet: "Data".into(),
                    anchor: at("B2"),
                },
                cells: Box::new(cells),
            })
            .unwrap();
        assert_eq!(
            applied.touched,
            [("Data".into(), range(span))],
            "{header:?}"
        );
        for (index, value) in expected.iter().enumerate() {
            assert_eq!(
                &workbook
                    .sheet("Data")
                    .unwrap()
                    .scalar(CellRef::new(index as u32 + 1, 1)),
                value,
                "{header:?}, imported row {index}"
            );
        }
        assert_eq!(
            workbook.sheet("Data").unwrap().scalar(at("C3")),
            Scalar::from("outside")
        );
        assert_eq!(
            workbook.sheet("Data").unwrap().scalar(at("D1")),
            Scalar::from("Amount")
        );
        assert_eq!(
            workbook.sheet("Data").unwrap().scalar(at("B6")),
            Scalar::from(95.0)
        );
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        let after = table_member_map(&workbook);
        let undone = workbook.apply(applied.inverse.unwrap()).unwrap();
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        assert_eq!(table_member_map(&workbook), before, "undo {header:?}");
        workbook.apply(undone.inverse.unwrap()).unwrap();
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        assert_eq!(table_member_map(&workbook), after, "redo {header:?}");
    }
}

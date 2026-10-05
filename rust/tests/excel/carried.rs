//! `rust/src/excel/carried.rs`: what a worksheet part states outside its cells - carried byte for byte, written back around the model's own elements in schema order, prefixed as the part is, and classed by what a structural edit owes it.

use yggdryl::excel::Workbook;
use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::ZipArchive;

use crate::excel_package::{
    NS, R_NS, content_types, package, rich_shared_strings, rich_styles, rich_worksheet,
    root_relationships, workbook, workbook_relationships,
};

/// A one-sheet package whose worksheet part is `sheet` exactly as spelled,
/// beside the shared strings and the styles of the rich package, which its
/// cells index.
fn book_with(sheet: &str) -> Vec<u8> {
    let types = content_types(1, true, true);
    let root = root_relationships();
    let relationships = workbook_relationships(1, true, true);
    let book = workbook(&["Sheet1"], false);
    let strings = rich_shared_strings();
    let styles = rich_styles();
    package(&[
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root.as_str()),
        ("xl/workbook.xml", book.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships.as_str()),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/sharedStrings.xml", strings.as_str()),
        ("xl/styles.xml", styles.as_str()),
    ])
}

fn member(bytes: &[u8], name: &str) -> String {
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))));
    String::from_utf8(archive.read_member(name).unwrap()).unwrap()
}

/// The worksheet part `sheet` becomes when `B2` of its sheet is set to 5.
fn edited(sheet: &str) -> String {
    let mut workbook = Workbook::from_bytes(book_with(sheet)).unwrap();
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("B2".parse().unwrap(), 5.0)
        .unwrap();
    member(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml")
}

#[test]
fn a_sheet_written_again_keeps_every_byte_outside_its_cells() {
    let original = rich_worksheet();
    // Only the edited cell and the rows' `spans`, which nothing keeps true,
    // differ: the declaration, the root and its namespaces, the pane, the
    // formats, the filters, the rules, the links, the drawings and the
    // extensions are the bytes the part held.
    // `set_cell` keeps the style of the cell it replaces, as typing into a
    // formatted cell keeps its format.
    let mut expected = original.replace(
        "<c r=\"B2\" s=\"2\"><v>1.5</v></c>",
        "<c r=\"B2\" s=\"2\"><v>5</v></c>",
    );
    expected = expected.replace(" spans=\"1:6\"", "");
    assert_eq!(edited(&original), expected);
}

#[test]
fn a_prefixed_part_has_the_model_s_elements_written_under_its_prefix() {
    let original = format!(
        "<x:worksheet xmlns:x=\"{NS}\" xmlns:r=\"{R_NS}\"><x:dimension ref=\"A1:B2\"/>\
         <x:sheetData><x:row r=\"1\"><x:c r=\"A1\"><x:v>1</x:v></x:c></x:row>\
         <x:row r=\"2\"><x:c r=\"B2\"><x:f>A1*2</x:f><x:v>2</x:v></x:c></x:row></x:sheetData>\
         <x:mergeCells count=\"1\"><x:mergeCell ref=\"A3:B3\"/></x:mergeCells>\
         <x:pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></x:worksheet>"
    );
    assert_eq!(
        edited(&original),
        original.replace("<x:f>A1*2</x:f><x:v>2</x:v>", "<x:v>5</x:v>")
    );
}

#[test]
fn text_comments_and_unknown_children_keep_their_place_between_the_model_s_elements() {
    let original = format!(
        "<?xml version=\"1.0\"?>\n<!-- made by hand -->\n<worksheet xmlns=\"{NS}\">\n  \
         <sheetPr/>\n  <dimension ref=\"B2\"/>\n  <x:custom xmlns:x=\"urn:x\">kept</x:custom>\n  \
         <sheetData><row r=\"2\"><c r=\"B2\"><v>1</v></c></row></sheetData>\n  <?pi data?>\n  \
         <pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>\n</worksheet>\n"
    );
    assert_eq!(edited(&original), original.replace("<v>1</v>", "<v>5</v>"));
}

#[test]
fn a_sheet_moved_to_another_workbook_leaves_behind_what_names_its_part_s_relationships() {
    let opened = Workbook::from_bytes(book_with(&rich_worksheet())).unwrap();
    let sheet = opened.sheet("Sheet1").unwrap().clone();
    let mut other = Workbook::new();
    other.insert_sheet(sheet).unwrap();
    let part = member(&other.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    // What is nothing without the old part's relationships goes whole; the
    // relationship alone goes from what stands without it.
    for gone in [
        "r:id",
        "<drawing",
        "<legacyDrawing",
        "<tableParts",
        "<hyperlink ref=\"A2\"",
    ] {
        assert!(!part.contains(gone), "{gone} in {part}");
    }
    for kept in [
        "<sheetPr>",
        "<autoFilter",
        "<dataValidations",
        "<pageMargins",
        "<pageSetup paperSize=\"9\" orientation=\"landscape\"/>",
        "<hyperlinks><hyperlink ref=\"A3\" location=\"Report!A1\" display=\"Report\" \
         xr:uid=\"{7E3F2C1D-0000-4000-8000-000000000004}\"/></hyperlinks>",
        "<extLst>",
        "<x14:sparklineGroups",
        "<mergeCells count=\"1\"><mergeCell ref=\"A6:C6\"/></mergeCells>",
    ] {
        assert!(part.contains(kept), "{kept} in {part}");
    }
    // A rule's `dxfId` indexed the old workbook's differential formats: the
    // rule stays, its format goes.
    assert!(
        part.contains(
            "<conditionalFormatting sqref=\"B2:B4\"><cfRule type=\"cellIs\" priority=\"2\" \
             operator=\"greaterThan\"><formula>1</formula></cfRule></conditionalFormatting>"
        ),
        "{part}"
    );
    assert!(!part.contains("dxfId"), "{part}");
}

#[test]
fn a_sheet_held_again_in_its_own_workbook_keeps_its_rules_formats() {
    // Removal validates the worksheet's table memberships, so this fixture
    // includes the relationships and table part, not just their references.
    let mut workbook = Workbook::from_bytes(crate::excel_package::rich_package()).unwrap();
    let sheet = workbook.remove_sheet("Data").unwrap().unwrap();
    workbook.insert_sheet(sheet).unwrap();
    let bytes = workbook.into_bytes().unwrap();
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.clone(),
    ))));
    let name = archive
        .entries()
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .find(|name| {
            name.starts_with("xl/worksheets/")
                && name.ends_with(".xml")
                && ![
                    "xl/worksheets/sheet1.xml",
                    "xl/worksheets/sheet2.xml",
                    "xl/worksheets/sheet3.xml",
                ]
                .contains(&name.as_str())
        })
        .unwrap();
    let part = member(&bytes, &name);
    assert!(
        part.contains("<cfRule type=\"cellIs\" dxfId=\"0\" priority=\"2\""),
        "{part}"
    );
    // The part is a new one, and holds none of the old one's relationships.
    assert!(!part.contains("r:id"), "{part}");
}

#[test]
fn a_relationship_is_found_under_any_prefix_and_any_spacing() {
    let original = format!(
        "<worksheet xmlns=\"{NS}\" xmlns:rel=\"{R_NS}\"><sheetData/>\
         <hyperlinks><hyperlink ref=\"A1\"\n\trel:id=\"rId1\"/><hyperlink ref=\"A2\" location=\"A9\"/></hyperlinks>\
         <pageSetup orientation=\"portrait\"\trel:id=\"rId2\"/>\
         <drawing\nrel:id=\"rId3\"/>\
         <extLst><ext uri=\"{{A}}\" xmlns:q=\"{R_NS}\"><x q:id=\"rId4\"/></ext><ext uri=\"{{B}}\"><y/></ext></extLst>\
         </worksheet>"
    );
    let opened = Workbook::from_bytes(book_with(&original)).unwrap();
    let mut other = Workbook::new();
    other
        .insert_sheet(opened.sheet("Sheet1").unwrap().clone())
        .unwrap();
    let part = member(&other.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(
        part.contains(
            "<hyperlinks><hyperlink ref=\"A2\" location=\"A9\"/></hyperlinks>\
             <pageSetup orientation=\"portrait\"/><extLst><ext uri=\"{B}\"><y/></ext></extLst>"
        ),
        "{part}"
    );
    assert!(!part.contains("id=\"rId"), "{part}");
}

#[test]
fn a_relationship_namespace_is_decoded_before_detaching_a_sheet() {
    for (main, relationship) in [
        (NS, R_NS),
        (
            yggdryl::excel::STRICT_NAMESPACE,
            yggdryl::excel::STRICT_RELATIONSHIPS_NAMESPACE,
        ),
    ] {
        for local in [false, true] {
            for replace in [false, true] {
                let encoded = relationship.replace("relationships", "relation&#115;hips");
                let declaration = format!(" xmlns:q=\"{encoded}\"");
                let (root, child) = if local {
                    ("", declaration.as_str())
                } else {
                    (declaration.as_str(), "")
                };
                let carried = "<extLst><ext uri=\"{keep}\"><x xmlns:q=\"urn:custom\" q:id=\"opaque\"/></ext></extLst>";
                let original = format!(
                    "<worksheet xmlns=\"{main}\"{root}><sheetData/><tableParts count=\"1\"><tablePart{child} q:id=\"rId1\"/></tableParts>{carried}</worksheet>"
                );
                let opened = Workbook::from_bytes(book_with(&original)).unwrap();
                let copy = opened.sheet("Sheet1").unwrap().clone();
                let mut target = if replace { opened } else { Workbook::new() };
                target.insert_sheet(copy).unwrap();
                let part = member(
                    &target.into_bytes().unwrap(),
                    if replace {
                        "xl/worksheets/sheet2.xml"
                    } else {
                        "xl/worksheets/sheet1.xml"
                    },
                );
                assert!(
                    !part.contains("<tableParts") && !part.contains("q:id=\"rId1\""),
                    "main={main}, local={local}, replace={replace}: {part}"
                );
                assert!(part.contains(carried), "{part}");
                // The new part owns none of the source part's relationships.
                target.rename_sheet("Sheet1", "Renamed").unwrap();
            }
        }
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::excel::{Frozen, Workbook};
    use yggdryl::internals::excel_carried::{blocking, items};

    /// Freeze `frozen` in the sheet's first view.
    fn set_frozen(sheet: &mut yggdryl::excel::Sheet, frozen: Option<Frozen>) {
        sheet.set_frozen(frozen).unwrap();
    }

    use super::{book_with, member};
    use crate::excel_package::{NS, R_NS, rich_worksheet};

    #[test]
    fn set_child_changes_only_the_captured_child_in_schema_order() {
        use yggdryl::internals::excel_carried::set_child;

        let foreign = "<q:tableParts q:id='opaque'><q:child/></q:tableParts>";
        let extension = "<x:extLst><x:ext uri='urn:test'><q:opaque/></x:ext></x:extLst>";
        let original = format!(
            "<?xml version='1.0'?><!--head--><x:worksheet xmlns:x='{NS}' xmlns:r='{R_NS}' xmlns:q='urn:foreign'><x:dimension ref=\"A1\"/><x:sheetData><x:row r=\"1\"><x:c r=\"A1\"><x:v>1</x:v></x:c></x:row></x:sheetData>{foreign}{extension}</x:worksheet><?tail kept?>"
        );
        let first = "<x:tableParts count='1'><x:tablePart r:id='rIdTable1'/></x:tableParts>";
        let second = "<x:tableParts count='1'><x:tablePart r:id='rIdTable2'/></x:tableParts>";
        let mut workbook = Workbook::from_bytes(book_with(&original)).unwrap();
        let sheet = workbook.sheet_mut("Sheet1").unwrap();
        let revision = sheet.revision();
        set_child(sheet, "tableParts", None, Some(first.as_bytes()));
        assert_eq!(sheet.revision(), revision + 1);
        assert_eq!(
            items(sheet)
                .iter()
                .filter(|(name, _, _)| name == "tableParts")
                .map(|(_, slot, class)| (*slot, *class))
                .collect::<Vec<_>>(),
            // Only SpreadsheetML tableParts owns this schema slot. The
            // foreign q:tableParts retains its qualified name and blocks edits.
            [(37, "shifted")]
        );
        assert!(
            items(sheet)
                .iter()
                .any(|(name, _, class)| name == "q:tableParts" && *class == "blocking")
        );
        assert_eq!(
            member(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml"),
            original.replace(extension, &format!("{first}{extension}"))
        );

        let sheet = workbook.sheet_mut("Sheet1").unwrap();
        let revision = sheet.revision();
        set_child(
            sheet,
            "tableParts",
            Some(first.as_bytes()),
            Some(first.as_bytes()),
        );
        assert_eq!(
            sheet.revision(),
            revision,
            "an unchanged membership is not an edit"
        );
        set_child(
            sheet,
            "tableParts",
            Some(first.as_bytes()),
            Some(second.as_bytes()),
        );
        assert_eq!(sheet.revision(), revision + 1);
        assert_eq!(
            member(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml"),
            original.replace(extension, &format!("{second}{extension}"))
        );
        set_child(
            workbook.sheet_mut("Sheet1").unwrap(),
            "tableParts",
            Some(second.as_bytes()),
            None,
        );
        assert_eq!(
            member(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml"),
            original,
            "the unrelated same-local-name child and complete envelope must survive"
        );
    }

    #[test]
    fn each_child_is_carried_at_its_schema_slot_and_classed_by_what_an_edit_owes_it() {
        let workbook = Workbook::from_bytes(book_with(&rich_worksheet())).unwrap();
        let sheet = workbook.sheet("Sheet1").unwrap();
        let listed: Vec<(String, u8, &str)> = items(sheet);
        let expected: Vec<(&str, u8, &str)> = vec![
            ("sheetPr", 0, "free"),
            ("dimension", 1, "regenerated"),
            ("sheetViews", 2, "modelled"),
            ("sheetFormatPr", 3, "free"),
            ("cols", 4, "regenerated"),
            ("sheetData", 5, "regenerated"),
            ("autoFilter", 10, "shifted"),
            ("mergeCells", 14, "regenerated"),
            ("conditionalFormatting", 16, "shifted"),
            ("dataValidations", 17, "shifted"),
            ("hyperlinks", 18, "shifted"),
            ("pageMargins", 20, "free"),
            ("pageSetup", 21, "free"),
            ("drawing", 29, "shifted"),
            ("legacyDrawing", 30, "shifted"),
            ("tableParts", 37, "shifted"),
            ("extLst", 38, "shifted"),
        ];
        assert_eq!(
            listed
                .iter()
                .map(|(name, slot, class)| (name.as_str(), *slot, *class))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(blocking(sheet).is_empty());
    }

    #[test]
    fn an_unknown_child_or_extension_blocks_a_structural_edit_and_keeps_its_place() {
        let workbook = Workbook::from_bytes(book_with(&format!(
            "<worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheetData/>\
             <pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>\
             <oleObjects/><mystery/>\
             <extLst><ext uri=\"{{00000000-0000-0000-0000-000000000000}}\"/></extLst></worksheet>"
        )))
        .unwrap();
        let sheet = workbook.sheet("Sheet1").unwrap();
        assert_eq!(
            items(sheet)
                .iter()
                .map(|(name, slot, class)| (name.as_str(), *slot, *class))
                .collect::<Vec<_>>(),
            [
                ("sheetData", 5, "regenerated"),
                ("pageMargins", 20, "free"),
                ("oleObjects", 34, "blocking"),
                ("mystery", 34, "blocking"),
                ("extLst", 38, "blocking"),
            ]
        );
        assert_eq!(blocking(sheet), ["oleObjects", "mystery", "extLst"]);
    }

    #[test]
    fn a_frozen_pane_changed_rewrites_the_first_view_s_pane_and_selection_alone() {
        let mut workbook = Workbook::from_bytes(book_with(&rich_worksheet())).unwrap();
        let sheet = workbook.sheet_mut("Sheet1").unwrap();
        assert_eq!(
            sheet.frozen(),
            Some(Frozen {
                rows: 1,
                columns: 1
            })
        );
        set_frozen(
            sheet,
            Some(Frozen {
                rows: 2,
                columns: 0,
            }),
        );
        let part = member(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
        assert!(
            part.contains(
                "<sheetViews><sheetView zoomScale=\"110\" zoomScaleNormal=\"110\" workbookViewId=\"0\">\
                 <pane ySplit=\"2\" topLeftCell=\"A3\" activePane=\"bottomLeft\" state=\"frozen\"/>\
                 <selection pane=\"bottomLeft\"/></sheetView></sheetViews>"
            ),
            "{part}"
        );
        let mut workbook = Workbook::from_bytes(book_with(&rich_worksheet())).unwrap();
        set_frozen(workbook.sheet_mut("Sheet1").unwrap(), None);
        let part = member(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
        assert!(
            part.contains(
                "<sheetViews><sheetView zoomScale=\"110\" zoomScaleNormal=\"110\" workbookViewId=\"0\"></sheetView></sheetViews>"
            ),
            "{part}"
        );
    }

    #[test]
    fn a_frozen_pane_on_a_part_stating_no_views_creates_them_where_the_schema_puts_them() {
        let original = format!(
            "<x:worksheet xmlns:x=\"{NS}\"><x:dimension ref=\"A1\"/><x:sheetFormatPr defaultRowHeight=\"15\"/>\
             <x:sheetData><x:row r=\"1\"><x:c r=\"A1\"><x:v>1</x:v></x:c></x:row></x:sheetData></x:worksheet>"
        );
        let mut workbook = Workbook::from_bytes(book_with(&original)).unwrap();
        set_frozen(
            workbook.sheet_mut("Sheet1").unwrap(),
            Some(Frozen {
                rows: 1,
                columns: 1,
            }),
        );
        let part = member(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
        assert_eq!(
            part,
            original.replace(
                "<x:sheetFormatPr",
                "<x:sheetViews><x:sheetView workbookViewId=\"0\">\
                 <x:pane xSplit=\"1\" ySplit=\"1\" topLeftCell=\"B2\" activePane=\"bottomRight\" state=\"frozen\"/>\
                 <x:selection pane=\"bottomRight\"/></x:sheetView></x:sheetViews><x:sheetFormatPr"
            )
        );
        // A sheet built in memory states its views the same way.
        let mut built = Workbook::new();
        let sheet = built.add_sheet("Built").unwrap();
        sheet.set_cell("A1".parse().unwrap(), 1.0).unwrap();
        set_frozen(
            sheet,
            Some(Frozen {
                rows: 3,
                columns: 0,
            }),
        );
        let part = member(&built.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
        assert!(
            part.contains(
                "<dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\">\
                 <pane ySplit=\"3\" topLeftCell=\"A4\" activePane=\"bottomLeft\" state=\"frozen\"/>\
                 <selection pane=\"bottomLeft\"/></sheetView></sheetViews><sheetData>"
            ),
            "{part}"
        );
    }
}

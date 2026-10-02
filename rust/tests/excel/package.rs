//! `rust/src/excel/package.rs`: the Open Packaging Conventions of a workbook - its part names, how a relationship's target resolves to a member, and the content types and relationships a write states.

use std::sync::Arc;

use yggdryl::excel::package::{
    CONTENT_TYPES_NAMESPACE, CONTENT_TYPES_PART, OFFICE_DOCUMENT_RELATIONSHIP,
    PACKAGE_RELATIONSHIPS_NAMESPACE, RELATIONSHIPS_CONTENT_TYPE, ROOT_RELATIONSHIPS_PART,
    SHARED_STRINGS_CONTENT_TYPE, SHARED_STRINGS_PART, SHARED_STRINGS_RELATIONSHIP,
    STYLES_CONTENT_TYPE, STYLES_PART, STYLES_RELATIONSHIP, WORKBOOK_CONTENT_TYPE, WORKBOOK_PART,
    WORKBOOK_RELATIONSHIPS_PART, WORKSHEET_CONTENT_TYPE, WORKSHEET_RELATIONSHIP, worksheet_part,
};
use yggdryl::excel::{
    ExcelOptions, NAMESPACE, RELATIONSHIPS_NAMESPACE, STRICT_NAMESPACE,
    STRICT_RELATIONSHIPS_NAMESPACE, SheetKind, Workbook, overwrite_arrow_reader,
};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::zip::ZipArchive;
use yggdryl::{DataType, Error, Field, IOBase, IOMedia, MimeType, Scalar, Serie, StructType};

use crate::excel_package::{
    NS, R_NS, content_types, package, root_relationships, shared_strings, styles, workbook,
    workbook_relationships, worksheet,
};

/// The archive `bytes` hold.
fn archive(bytes: &[u8]) -> Arc<ZipArchive> {
    Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))))
}

/// The member `name` of the package `bytes`.
fn member(bytes: &[u8], name: &str) -> Vec<u8> {
    archive(bytes).read_member(name).unwrap()
}

/// The member `name` of the package `bytes`, as text.
fn member_text(bytes: &[u8], name: &str) -> String {
    String::from_utf8(member(bytes, name)).unwrap()
}

/// Every member name of the package `bytes`, sorted.
fn member_names(bytes: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = archive(bytes)
        .entries()
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    names.sort();
    names
}

/// A relationships part stating `entries`, each `(Id, Type, Target)`.
fn relationships(entries: &[(&str, &str, &str)]) -> String {
    let mut text = format!("<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">");
    for (id, kind, target) in entries {
        text.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    text.push_str("</Relationships>");
    text
}

/// A workbook part listing `sheets`, each `(name, r:id)`.
fn workbook_of(sheets: &[(&str, &str)]) -> String {
    let mut text = format!("<workbook xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheets>");
    for (index, (name, id)) in sheets.iter().enumerate() {
        text.push_str(&format!(
            "<sheet name=\"{name}\" sheetId=\"{}\" r:id=\"{id}\"/>",
            index + 1
        ));
    }
    text.push_str("</sheets></workbook>");
    text
}

/// A worksheet whose `A1` holds the inline text `text`.
fn sheet_saying(text: &str) -> String {
    worksheet(&format!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>{text}</t></is></c></row>"
    ))
}

/// The inline text `A1` of the sheet `name` holds in the package `bytes`.
fn a1_of(bytes: Vec<u8>, name: &str) -> Scalar {
    let workbook = Workbook::from_bytes(bytes).unwrap();
    workbook.sheet(name).unwrap().scalar("A1".parse().unwrap())
}

/// A package whose workbook relationships name `Sheet1` at `target`, and
/// whose member `member` holds the sheet.
fn sheet_at(target: &str, member: &str) -> Vec<u8> {
    let root = root_relationships();
    let book = workbook(&["Sheet1"], false);
    let rels = relationships(&[("rId1", WORKSHEET_RELATIONSHIP, target)]);
    let sheet = sheet_saying("resolved");
    package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        (member, &sheet),
    ])
}

/// Two columns a record write lays out: `id` and `symbol`.
fn root() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])
        .unwrap(),
    )
    .required_field("row")
}

/// Write two rows under `options` over the package `bytes`, answering the
/// package written.
fn write_rows(bytes: Vec<u8>, options: ExcelOptions) -> Vec<u8> {
    let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
    let options = RecordOptions::from(options).with_field(root());
    handle
        .overwrite_records(
            [
                Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
                Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")]),
            ],
            &options,
        )
        .unwrap();
    handle.read_all_bytes().unwrap()
}

/// Write the same two rows through the medium's own door, which takes a
/// sheet the package lacks.
fn write_rows_to_sheet(bytes: Vec<u8>, sheet: &str) -> Vec<u8> {
    let rows = Serie::from_scalars(
        root(),
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")]),
        ],
    )
    .unwrap();
    let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
    overwrite_arrow_reader(
        &mut handle,
        rows.into_arrow_reader().unwrap(),
        &ExcelOptions::new().with_sheet(sheet),
    )
    .unwrap();
    handle.read_all_bytes().unwrap()
}

/// A complete one-sheet package - shared strings, styles - with the
/// members no workbook write speaks for beside it.
fn complete_package() -> Vec<u8> {
    let types = content_types(1, true, true).replace(
        "</Types>",
        "<Override PartName=\"/xl/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
         <Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>\
         </Types>",
    );
    let root = root_relationships();
    let book = workbook(&["Sheet1"], false);
    let rels = workbook_relationships(1, true, true);
    let sheet = worksheet(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>1</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>7</v></c><c r=\"B2\" t=\"s\"><v>2</v></c></row>",
    );
    let strings = shared_strings(&["id", "symbol", "kept"]);
    let formats = styles(&[], &[0]);
    package(&[
        (CONTENT_TYPES_PART, types.as_str()),
        (ROOT_RELATIONSHIPS_PART, root.as_str()),
        (WORKBOOK_PART, book.as_str()),
        (WORKBOOK_RELATIONSHIPS_PART, rels.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
        (SHARED_STRINGS_PART, strings.as_str()),
        (STYLES_PART, formats.as_str()),
        ("xl/theme/theme1.xml", THEME),
        ("docProps/app.xml", APP),
        ("customXml/item1.xml", CUSTOM),
    ])
}

const THEME: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
    <a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office Theme\">\
    <a:themeElements><a:clrScheme name=\"Office\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1></a:clrScheme></a:themeElements>\
    </a:theme>";

const APP: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
    <Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">\
    <Application>Microsoft Excel</Application><DocSecurity>0</DocSecurity>\
    </Properties>";

const CUSTOM: &str = "<root xmlns=\"urn:example\">  whitespace &amp; entities kept  </root>";

#[test]
fn the_package_documents_live_at_the_conventional_part_names() {
    assert_eq!(CONTENT_TYPES_PART, "[Content_Types].xml");
    assert_eq!(ROOT_RELATIONSHIPS_PART, "_rels/.rels");
    assert_eq!(WORKBOOK_PART, "xl/workbook.xml");
    assert_eq!(WORKBOOK_RELATIONSHIPS_PART, "xl/_rels/workbook.xml.rels");
    assert_eq!(SHARED_STRINGS_PART, "xl/sharedStrings.xml");
    assert_eq!(STYLES_PART, "xl/styles.xml");
}

#[test]
fn the_content_and_relationship_types_are_the_transitional_uris() {
    assert_eq!(
        RELATIONSHIPS_CONTENT_TYPE,
        "application/vnd.openxmlformats-package.relationships+xml"
    );
    assert_eq!(
        WORKBOOK_CONTENT_TYPE,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
    );
    assert_eq!(
        WORKSHEET_CONTENT_TYPE,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"
    );
    assert_eq!(
        SHARED_STRINGS_CONTENT_TYPE,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"
    );
    assert_eq!(
        STYLES_CONTENT_TYPE,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"
    );
    assert_eq!(
        PACKAGE_RELATIONSHIPS_NAMESPACE,
        "http://schemas.openxmlformats.org/package/2006/relationships"
    );
    assert_eq!(
        CONTENT_TYPES_NAMESPACE,
        "http://schemas.openxmlformats.org/package/2006/content-types"
    );
    assert_eq!(
        OFFICE_DOCUMENT_RELATIONSHIP,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
    );
    assert_eq!(
        WORKSHEET_RELATIONSHIP,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
    );
    assert_eq!(
        SHARED_STRINGS_RELATIONSHIP,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings"
    );
    assert_eq!(
        STYLES_RELATIONSHIP,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"
    );
    assert_eq!(
        NAMESPACE,
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    );
    assert_eq!(
        RELATIONSHIPS_NAMESPACE,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    );
    assert_eq!(
        STRICT_NAMESPACE,
        "http://purl.oclc.org/ooxml/spreadsheetml/main"
    );
    assert_eq!(
        STRICT_RELATIONSHIPS_NAMESPACE,
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    );
}

#[test]
fn worksheet_parts_are_numbered_from_one_under_xl_worksheets() {
    assert_eq!(worksheet_part(1), "xl/worksheets/sheet1.xml");
    assert_eq!(worksheet_part(2), "xl/worksheets/sheet2.xml");
    assert_eq!(worksheet_part(12), "xl/worksheets/sheet12.xml");
}

const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

#[test]
fn a_fresh_package_is_the_template_and_what_its_first_save_writes() {
    let mut book = Workbook::new();
    book.add_sheet("Trades").unwrap();
    book.add_sheet("Quotes").unwrap();
    let bytes = book.into_bytes().unwrap();

    // No text was written, so no shared strings; the styles are created
    // because a sheet was written from its cells.
    let mut expected = vec![
        CONTENT_TYPES_PART.to_owned(),
        ROOT_RELATIONSHIPS_PART.to_owned(),
        WORKBOOK_PART.to_owned(),
        WORKBOOK_RELATIONSHIPS_PART.to_owned(),
        worksheet_part(1).to_string(),
        worksheet_part(2).to_string(),
        STYLES_PART.to_owned(),
    ];
    expected.sort();
    assert_eq!(member_names(&bytes), expected);

    assert_eq!(
        member_text(&bytes, ROOT_RELATIONSHIPS_PART),
        format!(
            "{DECLARATION}<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
             <Relationship Id=\"rId1\" Type=\"{OFFICE_DOCUMENT_RELATIONSHIP}\" Target=\"xl/workbook.xml\"/>\
             </Relationships>"
        )
    );
    // Sheets take `rId1..=n` in tab order, the styles the next id.
    assert_eq!(
        member_text(&bytes, WORKBOOK_RELATIONSHIPS_PART),
        format!(
            "{DECLARATION}<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
             <Relationship Id=\"rId1\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet1.xml\"/>\
             <Relationship Id=\"rId2\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet2.xml\"/>\
             <Relationship Id=\"rId3\" Type=\"{STYLES_RELATIONSHIP}\" Target=\"styles.xml\"/>\
             </Relationships>"
        )
    );
    assert_eq!(
        member_text(&bytes, CONTENT_TYPES_PART),
        format!(
            "{DECLARATION}<Types xmlns=\"{CONTENT_TYPES_NAMESPACE}\">\
             <Default Extension=\"rels\" ContentType=\"{RELATIONSHIPS_CONTENT_TYPE}\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"{WORKBOOK_CONTENT_TYPE}\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>\
             <Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>\
             <Override PartName=\"/xl/styles.xml\" ContentType=\"{STYLES_CONTENT_TYPE}\"/>\
             </Types>"
        )
    );
    // The workbook part lists the tabs, and asks Excel to recalculate.
    assert_eq!(
        member_text(&bytes, WORKBOOK_PART),
        format!(
            "{DECLARATION}<workbook xmlns=\"{NAMESPACE}\" xmlns:r=\"{RELATIONSHIPS_NAMESPACE}\">\
             <workbookPr/><sheets><sheet name=\"Trades\" sheetId=\"1\" r:id=\"rId1\"/>\
             <sheet name=\"Quotes\" sheetId=\"2\" r:id=\"rId2\"/></sheets>\
             <calcPr fullCalcOnLoad=\"1\"/></workbook>"
        )
    );

    // A sheet holding text gets the shared strings, related and typed.
    let mut book = Workbook::new();
    book.add_sheet("Words")
        .unwrap()
        .set_cell("A1".parse().unwrap(), "hello")
        .unwrap();
    let bytes = book.into_bytes().unwrap();
    assert!(member_names(&bytes).contains(&SHARED_STRINGS_PART.to_owned()));
    assert!(
        member_text(&bytes, WORKBOOK_RELATIONSHIPS_PART).contains(&format!(
            "<Relationship Id=\"rId2\" Type=\"{SHARED_STRINGS_RELATIONSHIP}\" Target=\"sharedStrings.xml\"/>\
             <Relationship Id=\"rId3\" Type=\"{STYLES_RELATIONSHIP}\" Target=\"styles.xml\"/>"
        ))
    );
    assert!(
        member_text(&bytes, CONTENT_TYPES_PART).contains(&format!(
            "<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"{SHARED_STRINGS_CONTENT_TYPE}\"/>"
        ))
    );
}

#[test]
fn a_relative_target_resolves_against_the_folder_of_the_part_stating_it() {
    let bytes = sheet_at("sub/one.xml", "xl/sub/one.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
}

#[test]
fn an_absolute_target_names_the_member_from_the_package_root() {
    // `/data/one.xml` is outside `xl/`: a target starting with `/` is never
    // joined onto the folder of the workbook part.
    let bytes = sheet_at("/data/one.xml", "data/one.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
    let bytes = sheet_at("/xl/worksheets/sheet1.xml", "xl/worksheets/sheet1.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
}

#[test]
fn a_root_relationship_target_may_be_absolute() {
    let root = relationships(&[("rId1", OFFICE_DOCUMENT_RELATIONSHIP, "/xl/workbook.xml")]);
    let book = workbook(&["Sheet1"], false);
    let rels = workbook_relationships(1, false, false);
    let sheet = sheet_saying("absolute root");
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("absolute root"));
}

#[test]
fn dot_segments_fold_out_of_a_relative_target() {
    let bytes = sheet_at("../data/./one.xml", "data/one.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
    let bytes = sheet_at(
        "./worksheets/../worksheets/sheet1.xml",
        "xl/worksheets/sheet1.xml",
    );
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
}

#[test]
fn a_dot_dot_past_the_package_root_stays_at_the_root() {
    let bytes = sheet_at("../../../one.xml", "one.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
}

#[test]
fn a_percent_encoded_target_names_the_member_it_decodes_to() {
    let bytes = sheet_at("worksheets/sheet%201.xml", "xl/worksheets/sheet 1.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
    // An escaped multi-byte character decodes to its UTF-8 bytes.
    let bytes = sheet_at("worksheets/%E2%82%AC.xml", "xl/worksheets/\u{20ac}.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
}

#[test]
fn a_percent_sign_that_starts_no_escape_is_kept_as_it_stands() {
    let bytes = sheet_at("worksheets/a%25b%zz%4", "xl/worksheets/a%b%zz%4");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
}

#[test]
fn an_entity_in_a_target_is_unescaped_before_it_resolves() {
    let bytes = sheet_at("worksheets/a&amp;b.xml", "xl/worksheets/a&b.xml");
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("resolved"));
}

#[test]
fn an_external_target_is_no_part_of_the_package() {
    // The external office document names a member that exists; it is still
    // no part, so the workbook is the conventional one.
    let root = format!(
        "<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
         <Relationship Id=\"rId1\" Type=\"{OFFICE_DOCUMENT_RELATIONSHIP}\" Target=\"book/wb.xml\" TargetMode=\"External\"/>\
         </Relationships>"
    );
    let conventional = workbook(&["Conventional"], false);
    let elsewhere = workbook(&["Elsewhere"], false);
    let rels = workbook_relationships(1, false, false);
    let sheet = sheet_saying("internal");
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &conventional),
        ("book/wb.xml", &elsewhere),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.sheet_names(), vec!["Conventional"]);
    assert_eq!(
        opened
            .sheet("Conventional")
            .unwrap()
            .scalar("A1".parse().unwrap()),
        Scalar::from("internal")
    );
}

#[test]
fn a_workbook_at_another_part_is_reached_through_the_root_relationships() {
    let root = relationships(&[("rId1", OFFICE_DOCUMENT_RELATIONSHIP, "book/wb.xml")]);
    let book = workbook_of(&[("Trades", "rId7")]);
    let rels = relationships(&[
        ("rId7", WORKSHEET_RELATIONSHIP, "sheets/one.xml"),
        ("rId8", SHARED_STRINGS_RELATIONSHIP, "strings.xml"),
    ]);
    let sheet = worksheet("<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>");
    let strings = shared_strings(&["from book/strings.xml"]);
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        ("book/wb.xml", &book),
        ("book/_rels/wb.xml.rels", &rels),
        ("book/sheets/one.xml", &sheet),
        ("book/strings.xml", &strings),
    ]);
    assert_eq!(
        a1_of(bytes, "Trades"),
        Scalar::from("from book/strings.xml")
    );
}

#[test]
fn the_strict_namespaces_read_as_the_transitional_ones() {
    let root = relationships(&[(
        "rId1",
        "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
        "xl/workbook.xml",
    )]);
    let book = format!(
        "<workbook xmlns=\"{STRICT_NAMESPACE}\" xmlns:r=\"{STRICT_RELATIONSHIPS_NAMESPACE}\">\
         <workbookPr date1904=\"1\"/><sheets><sheet name=\"Strict\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
    );
    let rels = relationships(&[(
        "rId1",
        "http://purl.oclc.org/ooxml/officeDocument/relationships/worksheet",
        "worksheets/sheet1.xml",
    )]);
    let sheet = format!(
        "<worksheet xmlns=\"{STRICT_NAMESPACE}\" xmlns:r=\"{STRICT_RELATIONSHIPS_NAMESPACE}\"><sheetData>\
         <row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>strict</t></is></c><c r=\"B1\"><v>2.5</v></c></row>\
         </sheetData></worksheet>"
    );
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.sheet_names(), vec!["Strict"]);
    assert_eq!(opened.sheet_kind("Strict"), Some(SheetKind::Worksheet));
    assert_eq!(opened.date_system(), yggdryl::excel::DateSystem::Year1904);
    let sheet = opened.sheet("Strict").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from("strict"));
    assert_eq!(sheet.scalar("B1".parse().unwrap()), Scalar::from(2.5));
}

#[test]
fn prefixed_package_elements_read_by_their_local_names() {
    let root = format!(
        "<pr:Relationships xmlns:pr=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
         <pr:Relationship Id=\"rId1\" Type=\"{OFFICE_DOCUMENT_RELATIONSHIP}\" Target=\"xl/workbook.xml\"/>\
         </pr:Relationships>"
    );
    let book = format!(
        "<x:workbook xmlns:x=\"{NS}\" xmlns:r=\"{R_NS}\">\
         <x:sheets><x:sheet name=\"Prefixed\" sheetId=\"1\" r:id=\"rId1\"/></x:sheets></x:workbook>"
    );
    let rels = format!(
        "<pr:Relationships xmlns:pr=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
         <pr:Relationship Id=\"rId1\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet1.xml\"/>\
         </pr:Relationships>"
    );
    let sheet = sheet_saying("prefixed");
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    assert_eq!(a1_of(bytes, "Prefixed"), Scalar::from("prefixed"));
}

#[test]
fn root_relationships_naming_no_office_document_leave_the_conventional_workbook_part() {
    let root = relationships(&[(
        "rId1",
        "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
        "docProps/core.xml",
    )]);
    let book = workbook(&["Sheet1"], false);
    let rels = workbook_relationships(1, false, false);
    let sheet = sheet_saying("conventional");
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("conventional"));
}

#[test]
fn a_package_without_root_relationships_reads_the_conventional_workbook_part() {
    let book = workbook(&["Sheet1"], false);
    let rels = workbook_relationships(1, false, false);
    let sheet = sheet_saying("no root rels");
    let bytes = package(&[
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    assert_eq!(a1_of(bytes, "Sheet1"), Scalar::from("no root rels"));
}

#[test]
fn a_package_without_its_workbook_part_is_refused_naming_the_part_and_listing_the_members() {
    let types = content_types(1, false, false);
    let root = root_relationships();
    let sheet = sheet_saying("orphan");
    let bytes = package(&[
        (CONTENT_TYPES_PART, &types),
        (ROOT_RELATIONSHIPS_PART, &root),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let refusal = Workbook::from_bytes(bytes).unwrap_err();
    assert!(
        matches!(refusal, Error::InvalidRecord { .. }),
        "{refusal:?}"
    );
    assert_eq!(
        refusal.to_string(),
        "invalid record value at xl/workbook.xml: expected the workbook part in the package, \
         got the members [[Content_Types].xml, _rels/.rels, xl/worksheets/sheet1.xml]"
    );
}

#[test]
fn an_office_document_naming_a_missing_member_is_refused_naming_that_member() {
    let root = relationships(&[("rId1", OFFICE_DOCUMENT_RELATIONSHIP, "book/wb.xml")]);
    let book = workbook(&["Sheet1"], false);
    let bytes = package(&[(ROOT_RELATIONSHIPS_PART, &root), (WORKBOOK_PART, &book)]);
    assert_eq!(
        Workbook::from_bytes(bytes).unwrap_err().to_string(),
        "invalid record value at book/wb.xml: expected the workbook part in the package, \
         got the members [_rels/.rels, xl/workbook.xml]"
    );
}

#[test]
fn a_sheet_relationship_naming_a_missing_member_is_refused_naming_the_part() {
    let root = root_relationships();
    let book = workbook(&["Sheet1"], false);
    let rels = relationships(&[("rId1", WORKSHEET_RELATIONSHIP, "worksheets/sheet9.xml")]);
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
    ]);
    // Opening reads the package documents only; the sheet's part is asked
    // for when the sheet is.
    let opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.sheet_names(), vec!["Sheet1"]);
    let refusal = opened.sheet("Sheet1").unwrap_err();
    assert!(matches!(refusal, Error::Absent { .. }), "{refusal:?}");
    assert_eq!(
        refusal.to_string(),
        "expected a workbook part at \"xl/worksheets/sheet9.xml\", got nothing"
    );
}

#[test]
fn a_relationship_type_is_read_by_its_last_segment() {
    let root = root_relationships();
    let book = workbook_of(&[("Data", "rId1"), ("Chart1", "rId2"), ("Dialog1", "rId3")]);
    let rels = relationships(&[
        ("rId1", WORKSHEET_RELATIONSHIP, "worksheets/sheet1.xml"),
        (
            "rId2",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet",
            "chartsheets/sheet1.xml",
        ),
        (
            "rId3",
            "http://purl.oclc.org/ooxml/officeDocument/relationships/dialogsheet",
            "dialogsheets/sheet1.xml",
        ),
    ]);
    let sheet = sheet_saying("cells");
    let bytes = package(&[
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.sheet_kind("Data"), Some(SheetKind::Worksheet));
    assert_eq!(opened.sheet_kind("Chart1"), Some(SheetKind::Chartsheet));
    assert_eq!(opened.sheet_kind("Dialog1"), Some(SheetKind::Dialogsheet));
    assert_eq!(
        opened.sheet("Chart1").unwrap_err().to_string(),
        "invalid record value at $.Chart1: expected a worksheet, got the chartsheet `Chart1`, which holds no cells"
    );
}

#[test]
fn a_relationships_part_that_is_not_well_formed_is_refused_as_xlsx_data() {
    let root = format!(
        "<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\"><Relationship Id=\"rId1\"></Relationships>"
    );
    let book = workbook(&["Sheet1"], false);
    let bytes = package(&[(ROOT_RELATIONSHIPS_PART, &root), (WORKBOOK_PART, &book)]);
    let refusal = Workbook::from_bytes(bytes).unwrap_err();
    assert!(
        matches!(refusal, Error::Codec { format: "xlsx", .. }),
        "{refusal:?}"
    );
    // The position is where the mismatched end tag starts.
    assert_eq!(
        refusal.to_string(),
        "invalid xlsx data at byte 108: ill-formed document: expected `</Relationship>`, \
         but `</Relationships>` was found"
    );
}

#[test]
fn bytes_that_are_no_zip_package_are_refused_naming_the_xlsx_media_type() {
    let refusal = Workbook::from_bytes(b"symbol,price\nAAPL,187.23\n".to_vec()).unwrap_err();
    assert!(
        matches!(refusal, Error::Codec { format: "xlsx", .. }),
        "{refusal:?}"
    );
    assert_eq!(
        refusal.to_string(),
        "invalid xlsx data at byte 0: expected a ZIP package \
         (application/vnd.openxmlformats-officedocument.spreadsheetml.sheet), \
         got: expected an end of central directory record in the archive's last 64 KiB"
    );
}

#[test]
fn a_record_write_carries_every_member_it_does_not_speak_for_byte_for_byte() {
    let before = complete_package();
    let after = write_rows(before.clone(), ExcelOptions::new());

    for name in [
        "xl/theme/theme1.xml",
        "docProps/app.xml",
        "customXml/item1.xml",
        // The sheet list did not change, no part came or went and the write
        // added no text or style, so these are carried over too.
        CONTENT_TYPES_PART,
        ROOT_RELATIONSHIPS_PART,
        WORKBOOK_RELATIONSHIPS_PART,
        SHARED_STRINGS_PART,
        STYLES_PART,
    ] {
        assert_eq!(member(&after, name), member(&before, name), "{name}");
    }
    assert_eq!(member_names(&after), member_names(&before));
    // The workbook part gains the one fact a written sheet asks for: Excel
    // recalculates it on load.
    assert_eq!(
        member_text(&after, WORKBOOK_PART),
        member_text(&before, WORKBOOK_PART)
            .replace("</workbook>", "<calcPr fullCalcOnLoad=\"1\"/></workbook>")
    );

    // The sheet itself is the one part the write replaced.
    assert_ne!(
        member(&after, "xl/worksheets/sheet1.xml"),
        member(&before, "xl/worksheets/sheet1.xml")
    );
    let opened = Workbook::from_bytes(after).unwrap();
    let sheet = opened.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from("id"));
    assert_eq!(sheet.scalar("B2".parse().unwrap()), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar("B3".parse().unwrap()), Scalar::from("MSFT"));
}

#[test]
fn a_record_write_to_a_sheet_the_package_lacks_states_its_part_in_the_content_types_and_relationships()
 {
    let before = complete_package();
    let after = write_rows_to_sheet(before.clone(), "New");

    let types = member_text(&before, CONTENT_TYPES_PART);
    assert_eq!(
        member_text(&after, CONTENT_TYPES_PART),
        types.replace(
            "</Types>",
            &format!(
                "<Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/></Types>"
            )
        )
    );
    // Every relationship stays where it was, and the new sheet's takes the
    // id past them.
    let rels = member_text(&before, WORKBOOK_RELATIONSHIPS_PART);
    assert_eq!(
        member_text(&after, WORKBOOK_RELATIONSHIPS_PART),
        rels.replace(
            "</Relationships>",
            &format!(
                "<Relationship Id=\"rId4\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet2.xml\"/>\
                 </Relationships>"
            )
        )
    );
    let book = member_text(&after, WORKBOOK_PART);
    assert!(
        book.contains(
            "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"New\" sheetId=\"2\" r:id=\"rId4\"/></sheets>\
             <calcPr fullCalcOnLoad=\"1\"/>"
        ),
        "{book}"
    );
    // The sheet already there is carried over as it was.
    assert_eq!(
        member(&after, "xl/worksheets/sheet1.xml"),
        member(&before, "xl/worksheets/sheet1.xml")
    );

    let opened = Workbook::from_bytes(after).unwrap();
    assert_eq!(opened.sheet_names(), vec!["Sheet1", "New"]);
    assert_eq!(
        opened
            .sheet("Sheet1")
            .unwrap()
            .scalar("B2".parse().unwrap()),
        Scalar::from("kept")
    );
    let new = opened.sheet("New").unwrap();
    assert_eq!(new.scalar("B1".parse().unwrap()), Scalar::from("symbol"));
    assert_eq!(new.scalar("A2".parse().unwrap()), Scalar::from(1.0));
    assert_eq!(new.scalar("B2".parse().unwrap()), Scalar::from("AAPL"));
}

#[test]
fn a_new_sheet_beside_a_workbook_at_another_part_is_targeted_from_the_package_root() {
    let root = relationships(&[("rId1", OFFICE_DOCUMENT_RELATIONSHIP, "book/wb.xml")]);
    let book = workbook_of(&[("One", "rId1")]);
    let rels = relationships(&[("rId1", WORKSHEET_RELATIONSHIP, "sheets/one.xml")]);
    let sheet = sheet_saying("first");
    let types = format!(
        "<Types xmlns=\"{CONTENT_TYPES_NAMESPACE}\">\
         <Default Extension=\"rels\" ContentType=\"{RELATIONSHIPS_CONTENT_TYPE}\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/book/wb.xml\" ContentType=\"{WORKBOOK_CONTENT_TYPE}\"/>\
         <Override PartName=\"/book/sheets/one.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>\
         </Types>"
    );
    let before = package(&[
        (CONTENT_TYPES_PART, &types),
        (ROOT_RELATIONSHIPS_PART, &root),
        ("book/wb.xml", &book),
        ("book/_rels/wb.xml.rels", &rels),
        ("book/sheets/one.xml", &sheet),
    ]);
    let after = write_rows_to_sheet(before, "New");

    // The new sheet takes the first free conventional part, and the styles
    // the package lacked take theirs; the rows are inline text, so no
    // shared strings are created.
    assert_eq!(
        member_text(&after, CONTENT_TYPES_PART),
        types.replace(
            "</Types>",
            &format!(
                "<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>\
                 <Override PartName=\"/xl/styles.xml\" ContentType=\"{STYLES_CONTENT_TYPE}\"/>\
                 </Types>"
            )
        )
    );

    // A part outside the workbook's folder is reached up from it, and
    // resolves back to the member written.
    assert_eq!(
        member_text(&after, "book/_rels/wb.xml.rels"),
        rels.replace(
            "</Relationships>",
            &format!(
                "<Relationship Id=\"rId2\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"../xl/worksheets/sheet1.xml\"/>\
                 <Relationship Id=\"rId3\" Type=\"{STYLES_RELATIONSHIP}\" Target=\"../xl/styles.xml\"/>\
                 </Relationships>"
            )
        )
    );
    let opened = Workbook::from_bytes(after).unwrap();
    assert_eq!(opened.sheet_names(), vec!["One", "New"]);
    assert_eq!(
        opened.sheet("One").unwrap().scalar("A1".parse().unwrap()),
        Scalar::from("first")
    );
    assert_eq!(
        opened.sheet("New").unwrap().scalar("B2".parse().unwrap()),
        Scalar::from("AAPL")
    );
}

#[test]
fn a_part_cut_short_inside_an_element_is_refused_where_a_save_rewrites_it() {
    // Each document a save edits, cut before its root's end tag: every edit
    // due there - a relationship, an override, a sheet, an entry - would be
    // lost, so the save names the element it never saw close.
    for (cut_part, root) in [
        (CONTENT_TYPES_PART, "Types"),
        (WORKBOOK_RELATIONSHIPS_PART, "Relationships"),
        (WORKBOOK_PART, "workbook"),
        (STYLES_PART, "styleSheet"),
    ] {
        let mut parts = vec![
            (CONTENT_TYPES_PART, content_types(1, false, true)),
            (ROOT_RELATIONSHIPS_PART, root_relationships()),
            (WORKBOOK_PART, workbook(&["Sheet1"], false)),
            (
                WORKBOOK_RELATIONSHIPS_PART,
                workbook_relationships(1, false, true),
            ),
            ("xl/worksheets/sheet1.xml", worksheet("")),
            (STYLES_PART, styles(&[], &[0])),
        ];
        for (name, text) in &mut parts {
            if *name == cut_part {
                let end = format!("</{root}>");
                assert!(text.ends_with(&end), "{name}");
                text.truncate(text.len() - end.len());
            }
        }
        let bytes = package(
            &parts
                .iter()
                .map(|(name, text)| (*name, text.as_str()))
                .collect::<Vec<_>>(),
        );
        let mut opened = Workbook::from_bytes(bytes).unwrap();
        // A sheet added and a date written: every document is rewritten.
        opened
            .add_sheet("Added")
            .unwrap()
            .set_cell("A1".parse().unwrap(), Scalar::date32(19_723))
            .unwrap();
        let refusal = opened.into_bytes().unwrap_err();
        assert!(
            matches!(refusal, Error::Codec { .. }),
            "{cut_part}: {refusal:?}"
        );
        assert!(
            refusal.to_string().ends_with(&format!(
                "expected the end tag of <{root}>, got the end of the part"
            )),
            "{cut_part}: {refusal}"
        );
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::excel_package::edit_document;

    #[test]
    fn exact_attribute_decodes_entities_and_ignores_qualified_lookalikes() {
        use yggdryl::internals::excel_package::exact_attribute;
        let encoded = r#"<ext uri="{&#x37;8C0D931-6437-407d-A8EE-F0AAD7539E65}"/>"#;
        assert_eq!(
            exact_attribute(encoded, "uri").unwrap().as_deref(),
            Some("{78C0D931-6437-407d-A8EE-F0AAD7539E65}")
        );
        for xml in [
            r#"<ext xmlns:v="urn:vendor" v:uri="wrong" uri="right"/>"#,
            r#"<ext uri="right" xmlns:v="urn:vendor" v:uri="wrong"/>"#,
        ] {
            assert_eq!(
                exact_attribute(xml, "uri").unwrap().as_deref(),
                Some("right")
            );
        }
        assert_eq!(
            exact_attribute(r#"<ext xmlns:v="urn:vendor" v:uri="only"/>"#, "uri").unwrap(),
            None
        );
    }

    #[test]
    fn exact_attribute_refuses_duplicate_or_malformed_names() {
        use yggdryl::internals::excel_package::exact_attribute;
        for xml in [
            r#"<ext uri="one" uri="two"/>"#,
            r#"<ext uri=unquoted/>"#,
            r#"<ext uri="good" later=unquoted/>"#,
        ] {
            let error = exact_attribute(xml, "uri").unwrap_err();
            assert!(
                matches!(
                    error,
                    yggdryl::Error::Codec {
                        format: "xlsx",
                        position: 0,
                        ..
                    }
                ),
                "{error}"
            );
        }
    }

    fn edited(
        bytes: &str,
        set: &[(&str, &str, Option<&str>)],
        texts: &[(&str, &str)],
        dropped: &[&str],
    ) -> Option<String> {
        edit_document(bytes.as_bytes(), set, texts, dropped, None)
            .unwrap()
            .map(|bytes| String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn document_attributes_select_the_exact_qualified_name() {
        let document = "<c xmlns:x=\"urn:custom\" x:s=\"71\" s=\"1\"/>";
        assert_eq!(
            edited(document, &[("c", "s", Some("2"))], &[], &[]).as_deref(),
            Some("<c xmlns:x=\"urn:custom\" x:s=\"71\" s=\"2\"/>")
        );
        assert_eq!(
            edited(document, &[("c", "x:s", Some("72"))], &[], &[]).as_deref(),
            Some("<c xmlns:x=\"urn:custom\" x:s=\"72\" s=\"1\"/>")
        );
    }

    #[test]
    fn document_attributes_add_a_missing_name_without_replacing_its_namesake() {
        assert_eq!(
            edited(
                "<c xmlns:x=\"urn:custom\" x:s=\"71\"/>",
                &[("c", "s", Some("2"))],
                &[],
                &[],
            )
            .as_deref(),
            Some("<c xmlns:x=\"urn:custom\" x:s=\"71\" s=\"2\"/>")
        );
        assert_eq!(
            edited(
                "<c xmlns:x=\"urn:custom\" s=\"1\"/>",
                &[("c", "x:s", Some("72"))],
                &[],
                &[],
            )
            .as_deref(),
            Some("<c xmlns:x=\"urn:custom\" s=\"1\" x:s=\"72\"/>")
        );
    }

    #[test]
    fn document_attributes_remove_only_the_exact_name() {
        let document = "<c xmlns:x=\"urn:custom\" x:s=\"71\" s=\"1\"/>";
        assert_eq!(
            edited(document, &[("c", "s", None)], &[], &[]).as_deref(),
            Some("<c xmlns:x=\"urn:custom\" x:s=\"71\"/>")
        );
        assert_eq!(
            edited(document, &[("c", "x:s", None)], &[], &[]).as_deref(),
            Some("<c xmlns:x=\"urn:custom\" s=\"1\"/>")
        );
    }

    #[test]
    fn document_attributes_preserve_every_other_lexical_byte() {
        for ending in [" />", "><f>SUM(A1,&#x32;)</f><v>1.0000000</v></s:c>"] {
            let document = format!(
                "<?xml version='1.0'?>\r\n<!--kept--><s:c xmlns:s = 'urn:sheet'\n\t\
                 xmlns:x='urn:custom'  r = 'B2' x:s=\"71\" title='été &#38; tea'  s \t= '1' \
                 data=\"a&apos;b&#x20;c\"{ending}<!--tail-->"
            );
            let expected = document.replace("s \t= '1'", "s \t= '20'");
            assert_eq!(
                edited(&document, &[("c", "s", Some("20"))], &[], &[]).as_deref(),
                Some(expected.as_str())
            );
        }
    }

    #[test]
    fn document_attributes_escape_changed_values_inside_the_original_quotes() {
        let document = "<c  s = '1' title=\"keep &#x26; spelling\" />";
        assert_eq!(
            edited(document, &[("c", "s", Some("a'\"&<\t\n\r"))], &[], &[]).as_deref(),
            Some(
                "<c  s = 'a&apos;&quot;&amp;&lt;&#9;&#10;&#13;' title=\"keep &#x26; spelling\" />"
            )
        );
    }

    #[test]
    fn document_attributes_leave_equal_values_and_absent_removals_unwritten() {
        let document = "<c xmlns:x='urn:custom' x:s = '71' s = '&#49;' />";
        assert_eq!(edited(document, &[("c", "s", Some("1"))], &[], &[]), None);
        assert_eq!(edited(document, &[("c", "missing", None)], &[], &[]), None);
        // The last assignment applies to its exact name only; returning to
        // the original decoded value keeps its original entity spelling.
        assert_eq!(
            edited(
                document,
                &[("c", "s", Some("2")), ("c", "s", Some("1"))],
                &[],
                &[],
            ),
            None
        );
    }

    fn edited_in_namespace(bytes: &str, namespace: &str, dropped: &[&str]) -> Option<String> {
        edit_document(
            bytes.as_bytes(),
            &[("target", "n", Some("1"))],
            &[],
            dropped,
            Some(namespace),
        )
        .unwrap()
        .map(|bytes| String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn document_namespace_resolves_inherited_and_encoded_declarations() {
        for document in [
            "<root xmlns='urn:main'><target n='0'/></root>",
            "<p:root xmlns:p='urn:main'><p:target n='0'/></p:root>",
            "<p:root xmlns:p='urn:ma&#x69;n'><p:target n='0'/></p:root>",
            "<p:root xmlns:p='urn:foreign'><target xmlns='urn:main' n='0'/></p:root>",
        ] {
            let expected = document.replace("n='0'", "n='1'");
            assert_eq!(
                edited_in_namespace(document, "urn:main", &[]).as_deref(),
                Some(expected.as_str()),
                "{document}"
            );
        }
    }

    #[test]
    fn document_namespace_distinguishes_foreign_unbound_and_unknown_elements() {
        for document in [
            "<root xmlns='urn:foreign'><target n='0'/></root>",
            "<root><target n='0'/></root>",
            "<root xmlns='urn:main'><x:target xmlns:x='urn:foreign' n='0'/></root>",
            "<root xmlns='urn:main'><target xmlns='' n='0'/></root>",
            "<root><x:target n='0'/></root>",
        ] {
            assert_eq!(
                edited_in_namespace(document, "urn:main", &[]),
                None,
                "{document}"
            );
        }
    }

    #[test]
    fn document_namespace_restores_scope_after_empty_and_closed_elements() {
        let document = concat!(
            "<root xmlns:p='urn:main'>",
            "<p:target xmlns:p='urn:foreign' n='0'/>",
            "<branch xmlns:p='urn:foreign'><p:target n='0'/></branch>",
            "<p:target n='2'/></root>"
        );
        let expected = document.replace("n='2'", "n='1'");
        assert_eq!(
            edited_in_namespace(document, "urn:main", &[]).as_deref(),
            Some(expected.as_str())
        );
        let document = concat!(
            "<root xmlns='urn:main'><empty xmlns=''/>",
            "<branch xmlns='urn:foreign'><target n='0'/></branch>",
            "<target n='2'/></root>"
        );
        let expected = document.replace("n='2'", "n='1'");
        assert_eq!(
            edited_in_namespace(document, "urn:main", &[]).as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn document_namespace_restores_scope_after_dropped_subtrees_and_empty_nodes() {
        let document = concat!(
            "<root xmlns='urn:main' xmlns:p='urn:outer'>",
            "<drop xmlns:p='urn:main'><branch xmlns='urn:foreign'>",
            "<target n='0'/></branch></drop>",
            "<drop xmlns:p='urn:main'/><p:target n='0'/><target n='2'/></root>"
        );
        let expected = concat!(
            "<root xmlns='urn:main' xmlns:p='urn:outer'>",
            "<p:target n='0'/><target n='1'/></root>"
        );
        assert_eq!(
            edited_in_namespace(document, "urn:main", &["drop"]).as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn document_namespace_does_not_apply_the_default_namespace_to_attributes() {
        let document = "<target xmlns='urn:main' xmlns:x='urn:main' x:n='7' n='0'/>";
        assert_eq!(
            edited_in_namespace(document, "urn:main", &[]).as_deref(),
            Some("<target xmlns='urn:main' xmlns:x='urn:main' x:n='7' n='1'/>")
        );
    }

    #[test]
    fn a_document_no_edit_changes_is_answered_as_none() {
        let document = "<?xml version=\"1.0\"?>\r\n<!-- kept --><a  x = 'one'><b>t&amp;u</b><![CDATA[<raw>]]><c/></a>";
        assert_eq!(edited(document, &[], &[], &[]), None);
        // An edit naming no element there changes nothing either.
        assert_eq!(
            edited(document, &[("zz", "x", Some("1"))], &[("zz", "t")], &["zz"]),
            None
        );
    }

    #[test]
    fn only_the_spans_an_edit_changes_are_written_again() {
        let document = "<?xml version=\"1.0\"?>\r\n<!-- kept --><p:a xmlns:p=\"urn:p\"  x = 'one' y=\"two\"><p:b>t&amp;u</p:b><c k=\"v\"/><d>keep  me</d></p:a>";
        // Only the selected attribute value changes: unrelated attribute
        // spelling, spacing and quotes stay byte-exact, as does the rest
        // of the document.
        assert_eq!(
            edited(document, &[("a", "y", Some("a&b\"c<"))], &[], &[]).as_deref(),
            Some(
                "<?xml version=\"1.0\"?>\r\n<!-- kept --><p:a xmlns:p=\"urn:p\"  x = 'one' y=\"a&amp;b&quot;c&lt;\"><p:b>t&amp;u</p:b><c k=\"v\"/><d>keep  me</d></p:a>"
            )
        );
        assert_eq!(
            edited(
                "<a x='say \"hi\"' y=\"1\"/>",
                &[("a", "y", Some("2"))],
                &[],
                &[]
            )
            .as_deref(),
            Some("<a x='say \"hi\"' y=\"2\"/>")
        );
        // A text replaced is escaped as a producer escapes one; one added
        // to an empty element keeps it self-closed; one removed goes.
        assert_eq!(
            edited(
                document,
                &[("c", "k", None), ("c", "n", Some("1"))],
                &[("b", "x<y&z")],
                &[]
            )
            .as_deref(),
            Some(
                "<?xml version=\"1.0\"?>\r\n<!-- kept --><p:a xmlns:p=\"urn:p\"  x = 'one' y=\"two\"><p:b>x&lt;y&amp;z</p:b><c n=\"1\"/><d>keep  me</d></p:a>"
            )
        );
    }

    #[test]
    fn an_element_left_out_takes_what_it_holds_with_it() {
        let document = "<a><b><c>1</c><![CDATA[x]]></b><d/><b/></a>";
        assert_eq!(
            edited(document, &[], &[], &["b"]).as_deref(),
            Some("<a><d/></a>")
        );
        assert_eq!(
            edited(document, &[], &[], &["d"]).as_deref(),
            Some("<a><b><c>1</c><![CDATA[x]]></b><b/></a>")
        );
        // A CDATA text is read as its characters and written escaped.
        assert_eq!(
            edited("<a><![CDATA[<x>]]></a>", &[], &[("a", "<y>")], &[]).as_deref(),
            Some("<a>&lt;y&gt;</a>")
        );
    }

    #[test]
    fn bytes_that_are_not_xml_are_refused() {
        for broken in ["<a><b></a>", "<a>", "<a x=\"1></a>"] {
            assert!(
                edit_document(broken.as_bytes(), &[], &[], &[], None).is_err(),
                "{broken}"
            );
        }
    }

    #[test]
    fn document_reader_keeps_insertion_order_across_tiny_reads_and_empty_reads() {
        use std::io::Read;
        use std::sync::Arc;
        use yggdryl::internals::excel_package::document_reader;

        let source = "<!--before--><a><row n='1'><b old='x'>old</b><gone/></row><row n='2'><b>old</b></row></a><!--after-->";
        let expected = "<!--before--><a><row n='1'><insert/><b n=\"2\">\u{e9}&amp;&lt;new</b><kept/></row><row n='2'><insert/><b n=\"2\">\u{e9}&amp;&lt;new</b></row></a><!--after-->";
        for width in [1, 2, 7, 1024] {
            let mut reader = document_reader(
                Arc::from(source.as_bytes()),
                &[("b", "old", None), ("b", "n", Some("2"))],
                &[("b", "\u{e9}&<new")],
                &["gone"],
                &[("b", "<insert/>"), ("gone", "<kept/>")],
            )
            .unwrap();
            let mut actual = Vec::new();
            let mut buffer = vec![0; width];
            loop {
                assert_eq!(reader.read(&mut []).unwrap(), 0);
                let count = reader.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                actual.extend_from_slice(&buffer[..count]);
            }
            assert_eq!(actual, expected.as_bytes(), "chunk size {width}");
            assert_eq!(reader.read(&mut buffer).unwrap(), 0);
            assert_eq!(reader.read(&mut []).unwrap(), 0);
            assert_eq!(reader.read(&mut buffer).unwrap(), 0);
        }
    }

    #[test]
    fn document_reader_handles_unchanged_empty_and_deleted_documents_without_early_eof() {
        use std::io::Read;
        use std::sync::Arc;
        use yggdryl::internals::excel_package::document_reader;

        for (source, dropped, inserted, expected) in [
            (
                "<a><b/>tail</a>",
                &[][..],
                &[("b", "")][..],
                "<a><b/>tail</a>",
            ),
            ("<a/>", &["a"][..], &[][..], ""),
            (
                "<a><b/>tail</a>",
                &["b"][..],
                &[("b", "")][..],
                "<a>tail</a>",
            ),
            ("", &[][..], &[][..], ""),
        ] {
            let mut reader =
                document_reader(Arc::from(source.as_bytes()), &[], &[], dropped, inserted).unwrap();
            let mut bytes = Vec::new();
            assert_eq!(reader.read(&mut []).unwrap(), 0);
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, expected.as_bytes());
            assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
        }
    }

    #[test]
    fn document_reader_refuses_malformed_tail_before_returning_a_reader() {
        use std::sync::Arc;
        use yggdryl::internals::excel_package::document_reader;
        for source in ["<a><b>old</b><tail>", "<a><b>old</b></wrong>"] {
            assert!(
                document_reader(
                    Arc::from(source.as_bytes()),
                    &[],
                    &[("b", "new")],
                    &[],
                    &[("b", "<insert/>")]
                )
                .is_err()
            );
        }
    }

    #[test]
    fn raw_subtree_capture_keeps_complete_empty_and_nested_bytes_and_inherited_scope() {
        use yggdryl::internals::excel_package::raw_subtree_capture;
        let nested = "<p:keep  a = 'x&amp;y'><p:inside xmlns:p='urn:foreign'/><!--raw--><![CDATA[a<b]]></p:keep>";
        let empty = "<p:keep a='empty'/>";
        let source = format!(
            "<r:root xmlns:r='urn:main' xmlns:p='urn:main'>{nested}{empty}<p:keep xmlns:p='urn:foreign'/><p:after value='old'/></r:root>"
        );
        let expected = source
            .replace(nested, &format!("<marker/>{nested}"))
            .replace(empty, &format!("<marker/>{empty}"))
            .replace("value='old'", "value='new'");
        assert_eq!(
            raw_subtree_capture(source.as_bytes(), None).unwrap(),
            [
                format!("root|p:keep|{nested}"),
                format!("root|p:keep|{empty}"),
                expected,
            ]
        );
    }

    #[test]
    fn raw_subtree_capture_refuses_late_errors_and_unclosed_input_before_a_reader_exists() {
        use yggdryl::internals::excel_package::raw_subtree_capture;
        let source = b"<root xmlns='urn:main'><keep>first</keep><keep/></root>";
        let error = raw_subtree_capture(source, Some(1)).unwrap_err();
        assert!(
            matches!(error, yggdryl::Error::InvalidRecord { ref path, ref reason }
            if path == "raw-capture" && reason == "injected late capture refusal"),
            "{error}"
        );
        for broken in [
            "<root xmlns='urn:main'><keep>first</keep><keep>",
            "<root xmlns='urn:main'><keep><child></keep></root>",
        ] {
            assert!(
                matches!(
                    raw_subtree_capture(broken.as_bytes(), None),
                    Err(yggdryl::Error::Codec { format: "xlsx", .. })
                ),
                "{broken}"
            );
        }
    }
}

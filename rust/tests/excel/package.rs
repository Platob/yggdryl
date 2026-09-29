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
        (CONTENT_TYPES_PART, &types),
        (ROOT_RELATIONSHIPS_PART, &root),
        (WORKBOOK_PART, &book),
        (WORKBOOK_RELATIONSHIPS_PART, &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
        (SHARED_STRINGS_PART, &strings),
        (STYLES_PART, &formats),
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

#[test]
fn a_fresh_package_states_every_part_it_writes() {
    let mut book = Workbook::new();
    book.add_sheet("Trades").unwrap();
    book.add_sheet("Quotes").unwrap();
    let bytes = book.into_bytes().unwrap();

    let mut expected = vec![
        CONTENT_TYPES_PART.to_owned(),
        ROOT_RELATIONSHIPS_PART.to_owned(),
        WORKBOOK_PART.to_owned(),
        WORKBOOK_RELATIONSHIPS_PART.to_owned(),
        worksheet_part(1).to_string(),
        worksheet_part(2).to_string(),
        SHARED_STRINGS_PART.to_owned(),
        STYLES_PART.to_owned(),
    ];
    expected.sort();
    assert_eq!(member_names(&bytes), expected);

    assert_eq!(
        member_text(&bytes, ROOT_RELATIONSHIPS_PART),
        format!(
            "<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
             <Relationship Id=\"rId1\" Type=\"{OFFICE_DOCUMENT_RELATIONSHIP}\" Target=\"xl/workbook.xml\"/>\
             </Relationships>"
        )
    );
    // Sheets take `rId1..=n`, the styles `rId{n + 1}`, the strings `rId{n + 2}`.
    assert_eq!(
        member_text(&bytes, WORKBOOK_RELATIONSHIPS_PART),
        format!(
            "<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
             <Relationship Id=\"rId1\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet1.xml\"/>\
             <Relationship Id=\"rId2\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet2.xml\"/>\
             <Relationship Id=\"rId3\" Type=\"{STYLES_RELATIONSHIP}\" Target=\"styles.xml\"/>\
             <Relationship Id=\"rId4\" Type=\"{SHARED_STRINGS_RELATIONSHIP}\" Target=\"sharedStrings.xml\"/>\
             </Relationships>"
        )
    );
    assert_eq!(
        member_text(&bytes, CONTENT_TYPES_PART),
        format!(
            "<Types xmlns=\"{CONTENT_TYPES_NAMESPACE}\">\
             <Default Extension=\"rels\" ContentType=\"{RELATIONSHIPS_CONTENT_TYPE}\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"{WORKBOOK_CONTENT_TYPE}\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>\
             <Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>\
             <Override PartName=\"/xl/styles.xml\" ContentType=\"{STYLES_CONTENT_TYPE}\"/>\
             <Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"{SHARED_STRINGS_CONTENT_TYPE}\"/>\
             </Types>"
        )
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
        // The sheet list did not change and the strings and styles parts
        // exist, so the package documents are carried over too.
        CONTENT_TYPES_PART,
        ROOT_RELATIONSHIPS_PART,
        WORKBOOK_PART,
        WORKBOOK_RELATIONSHIPS_PART,
    ] {
        assert_eq!(member(&after, name), member(&before, name), "{name}");
    }
    assert_eq!(member_names(&after), member_names(&before));

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
    // Every worksheet relationship is stated again past the ids the part
    // already holds; the others stay where they were.
    let rels = member_text(&before, WORKBOOK_RELATIONSHIPS_PART);
    let sheet_one = format!(
        "<Relationship Id=\"rId1\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet1.xml\"/>"
    );
    assert!(rels.contains(&sheet_one), "{rels}");
    assert_eq!(
        member_text(&after, WORKBOOK_RELATIONSHIPS_PART),
        rels.replace(&sheet_one, "").replace(
            "</Relationships>",
            &format!(
                "<Relationship Id=\"rId4\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet1.xml\"/>\
                 <Relationship Id=\"rId5\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"worksheets/sheet2.xml\"/>\
                 </Relationships>"
            )
        )
    );
    let book = member_text(&after, WORKBOOK_PART);
    assert!(
        book.contains(
            "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId4\"/><sheet name=\"New\" sheetId=\"2\" r:id=\"rId5\"/></sheets>"
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

    // The new sheet takes the first free conventional part, and the strings
    // and styles the package lacked take theirs.
    assert_eq!(
        member_text(&after, CONTENT_TYPES_PART),
        types.replace(
            "</Types>",
            &format!(
                "<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>\
                 <Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"{SHARED_STRINGS_CONTENT_TYPE}\"/>\
                 <Override PartName=\"/xl/styles.xml\" ContentType=\"{STYLES_CONTENT_TYPE}\"/>\
                 </Types>"
            )
        )
    );

    // A part outside the workbook's folder is spelled absolute, one inside
    // it relative, and both resolve back to the member written.
    let written = member_text(&after, "book/_rels/wb.xml.rels");
    assert!(
        written.contains(&format!(
            "<Relationship Id=\"rId2\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"sheets/one.xml\"/>\
             <Relationship Id=\"rId3\" Type=\"{WORKSHEET_RELATIONSHIP}\" Target=\"/xl/worksheets/sheet1.xml\"/>"
        )),
        "{written}"
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

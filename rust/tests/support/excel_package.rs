//! A hand-written `.xlsx` package, for the tests that feed the Excel medium
//! what another writer produces: parts spelled by the test, zipped by the
//! crate's own ZIP backend, so what is pinned is the reading of the parts
//! and never the crate's own writer.

#![allow(dead_code)]

use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::ZipArchive;
use yggdryl::{Codec, IOBase};

/// The main SpreadsheetML namespace every part below declares.
pub const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// The relationships namespace the workbook's `r:id` attributes use.
pub const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A package holding exactly `parts`, each deflated.
pub fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    let archive = ZipArchive::new(Holder::buffer(Buffer::new()));
    for (name, text) in parts {
        archive
            .write_member_with(name, text.as_bytes(), Codec::Deflate)
            .expect("the part writes");
    }
    archive.flush().expect("the directory publishes");
    let handle = archive.into_handle().expect("the handle");
    handle.read_all_bytes().expect("the package bytes")
}

/// The `[Content_Types].xml` part naming `sheets` worksheets, plus the
/// shared strings and styles parts when `strings` and `styles` say so.
pub fn content_types(sheets: usize, strings: bool, styles: bool) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
    );
    for number in 1..=sheets {
        text.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{number}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
    }
    if strings {
        text.push_str("<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>");
    }
    if styles {
        text.push_str("<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    }
    text.push_str("</Types>");
    text
}

/// The root relationships part, pointing at `xl/workbook.xml`.
pub fn root_relationships() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
     <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
     <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
     </Relationships>"
        .to_owned()
}

/// The workbook relationships part: one worksheet per name in order, then
/// the shared strings and styles parts when asked for.
pub fn workbook_relationships(sheets: usize, strings: bool, styles: bool) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    let mut id = 1;
    for number in 1..=sheets {
        text.push_str(&format!(
            "<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{number}.xml\"/>"
        ));
        id += 1;
    }
    if strings {
        text.push_str(&format!(
            "<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/>"
        ));
        id += 1;
    }
    if styles {
        text.push_str(&format!(
            "<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>"
        ));
    }
    text.push_str("</Relationships>");
    text
}

/// The workbook part naming `sheets` in order, `rId1..`, under the 1900
/// date system unless `date1904`.
pub fn workbook(sheets: &[&str], date1904: bool) -> String {
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <workbook xmlns=\"{NS}\" xmlns:r=\"{R_NS}\">"
    );
    if date1904 {
        text.push_str("<workbookPr date1904=\"1\"/>");
    }
    text.push_str("<sheets>");
    for (index, name) in sheets.iter().enumerate() {
        text.push_str(&format!(
            "<sheet name=\"{name}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
            index + 1,
            index + 1
        ));
    }
    text.push_str("</sheets></workbook>");
    text
}

/// A worksheet part around `sheet_data`, the `<sheetData>` element's rows.
pub fn worksheet(sheet_data: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheetData>{sheet_data}</sheetData></worksheet>"
    )
}

/// A shared strings part holding `items`, each as one `t`.
pub fn shared_strings(items: &[&str]) -> String {
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <sst xmlns=\"{NS}\" count=\"{}\" uniqueCount=\"{}\">",
        items.len(),
        items.len()
    );
    for item in items {
        text.push_str("<si><t xml:space=\"preserve\">");
        text.push_str(item);
        text.push_str("</t></si>");
    }
    text.push_str("</sst>");
    text
}

/// A styles part whose cell formats are `cell_xfs`, each a `numFmtId`, with
/// the custom `num_fmts` (id, code) declared beside the built-ins.
pub fn styles(num_fmts: &[(u32, &str)], cell_xfs: &[u32]) -> String {
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <styleSheet xmlns=\"{NS}\">"
    );
    if !num_fmts.is_empty() {
        text.push_str(&format!("<numFmts count=\"{}\">", num_fmts.len()));
        for (id, code) in num_fmts {
            text.push_str(&format!(
                "<numFmt numFmtId=\"{id}\" formatCode=\"{code}\"/>"
            ));
        }
        text.push_str("</numFmts>");
    }
    text.push_str(
        "<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>",
    );
    text.push_str("<fills count=\"1\"><fill><patternFill patternType=\"none\"/></fill></fills>");
    text.push_str(
        "<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>",
    );
    text.push_str("<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");
    text.push_str(&format!("<cellXfs count=\"{}\">", cell_xfs.len()));
    for id in cell_xfs {
        text.push_str(&format!(
            "<xf numFmtId=\"{id}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>"
        ));
    }
    text.push_str("</cellXfs>");
    text.push_str("<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>");
    text.push_str("</styleSheet>");
    text
}

/// A complete one-sheet package: `Sheet1` holding `sheet_data`, with the
/// shared strings `strings` (none for an empty list) and the styles part
/// declared by `num_fmts` and `cell_xfs` (none for an empty `cell_xfs`).
pub fn one_sheet(
    sheet_data: &str,
    strings: &[&str],
    num_fmts: &[(u32, &str)],
    cell_xfs: &[u32],
) -> Vec<u8> {
    let has_strings = !strings.is_empty();
    let has_styles = !cell_xfs.is_empty();
    let content_types = content_types(1, has_strings, has_styles);
    let root = root_relationships();
    let relationships = workbook_relationships(1, has_strings, has_styles);
    let workbook = workbook(&["Sheet1"], false);
    let sheet = worksheet(sheet_data);
    let sst = shared_strings(strings);
    let styles = styles(num_fmts, cell_xfs);
    let mut parts: Vec<(&str, &str)> = vec![
        ("[Content_Types].xml", &content_types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &relationships),
        ("xl/worksheets/sheet1.xml", &sheet),
    ];
    if has_strings {
        parts.push(("xl/sharedStrings.xml", &sst));
    }
    if has_styles {
        parts.push(("xl/styles.xml", &styles));
    }
    package(&parts)
}

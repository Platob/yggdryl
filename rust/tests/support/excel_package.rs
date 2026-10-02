//! Excel packages shared by medium, model, and cost tests. Raw parts
//! are authored here and zipped through the public ZIP backend. The
//! structural-edit fixtures also normalize their initial cells through
//! the public Workbook writer so inverse checks start from saved state.

#![allow(dead_code)]

use yggdryl::excel::{Cell, CellRange, CellRef, DateSystem, Formula, Workbook};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::ZipArchive;
use yggdryl::{Codec, IOBase};

/// The main SpreadsheetML namespace every part below declares.
pub const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// The relationships namespace the workbook's `r:id` attributes use.
pub const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A package holding exactly `parts`, each deflated.
pub fn package<B: AsRef<[u8]>>(parts: &[(&str, B)]) -> Vec<u8> {
    package_coded(parts, Codec::Deflate)
}

/// A package holding exactly `parts`, each stored under `codec` - `Identity`
/// for members a writer that re-encoded them would give away by deflating.
pub fn package_coded<B: AsRef<[u8]>>(parts: &[(&str, B)], codec: Codec) -> Vec<u8> {
    let archive = ZipArchive::new(Holder::buffer(Buffer::new()));
    for (name, text) in parts {
        archive
            .write_member_with(name, text.as_ref(), codec)
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

/// The worksheet of [`rich_package`]'s `Data` sheet, in Excel's spelling:
/// a frozen pane with its selections, `<cols>`, rows stating heights,
/// hidden flags, styles and `x14ac:dyDescent` - one with no cell - merged
/// cells, legacy and x14 conditional formatting, a data validation,
/// hyperlinks by relationship and by location, an autofilter with a filter
/// column and a sort state, a drawing, comments behind a legacy drawing, a
/// table, an x14 sparkline group, rich, phonetic and duplicate shared
/// strings and a rich inline string.
pub fn rich_worksheet() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"x14ac xr xr2 xr3\" xmlns:x14ac=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\" xmlns:xr2=\"http://schemas.microsoft.com/office/spreadsheetml/2015/revision2\" xmlns:xr3=\"http://schemas.microsoft.com/office/spreadsheetml/2016/revision3\" xr:uid=\"{{00000000-0001-0000-0000-000000000000}}\">\
<sheetPr><tabColor theme=\"4\" tint=\"0.39997558519241921\"/></sheetPr>\
<dimension ref=\"A1:F5\"/>\
<sheetViews><sheetView zoomScale=\"110\" zoomScaleNormal=\"110\" workbookViewId=\"0\"><pane xSplit=\"1\" ySplit=\"1\" topLeftCell=\"B2\" activePane=\"bottomRight\" state=\"frozen\"/><selection pane=\"topRight\" activeCell=\"B1\" sqref=\"B1\"/><selection pane=\"bottomLeft\" activeCell=\"A2\" sqref=\"A2\"/><selection pane=\"bottomRight\" activeCell=\"C3\" sqref=\"C3\"/></sheetView></sheetViews>\
<sheetFormatPr defaultRowHeight=\"14.5\" x14ac:dyDescent=\"0.35\"/>\
<cols><col min=\"1\" max=\"1\" width=\"14.7265625\" customWidth=\"1\"/><col min=\"2\" max=\"3\" width=\"10.54296875\" style=\"2\" customWidth=\"1\"/></cols>\
<sheetData>\
<row r=\"1\" spans=\"1:6\" x14ac:dyDescent=\"0.35\"><c r=\"A1\" s=\"3\" t=\"s\"><v>0</v></c><c r=\"B1\" s=\"3\" t=\"s\"><v>1</v></c><c r=\"C1\" s=\"3\" t=\"s\"><v>2</v></c><c r=\"E1\" t=\"s\"><v>9</v></c><c r=\"F1\" t=\"s\"><v>10</v></c></row>\
<row r=\"2\" spans=\"1:6\" ht=\"20\" customHeight=\"1\" x14ac:dyDescent=\"0.35\"><c r=\"A2\" t=\"s\"><v>5</v></c><c r=\"B2\" s=\"2\"><v>1.5</v></c><c r=\"C2\"><v>10</v></c><c r=\"E2\" t=\"s\"><v>7</v></c><c r=\"F2\"><v>3</v></c></row>\
<row r=\"3\" spans=\"1:6\" x14ac:dyDescent=\"0.35\"><c r=\"A3\" t=\"s\"><v>3</v></c><c r=\"B3\" s=\"2\"><v>2.25</v></c><c r=\"C3\"><v>20</v></c><c r=\"E3\" t=\"s\"><v>5</v></c><c r=\"F3\"><v>4</v></c></row>\
<row r=\"4\" spans=\"1:6\" x14ac:dyDescent=\"0.35\"><c r=\"A4\" t=\"s\" ph=\"1\"><v>4</v></c><c r=\"B4\" s=\"2\"><v>0.5</v></c><c r=\"C4\" t=\"s\"><v>6</v></c></row>\
<row r=\"5\" spans=\"1:6\" s=\"1\" customFormat=\"1\" hidden=\"1\" x14ac:dyDescent=\"0.35\"><c r=\"A5\" t=\"inlineStr\"><is><r><rPr><b/><sz val=\"11\"/><color theme=\"1\"/><rFont val=\"Calibri\"/></rPr><t>in</t></r><r><rPr><sz val=\"11\"/><color rgb=\"FFFF0000\"/><rFont val=\"Calibri\"/></rPr><t xml:space=\"preserve\">line </t></r></is></c><c r=\"B5\" s=\"1\"><v>0.25</v></c><c r=\"C5\" s=\"4\"><v>45292</v></c></row>\
<row r=\"7\" spans=\"1:6\" ht=\"30\" hidden=\"1\" customHeight=\"1\" x14ac:dyDescent=\"0.35\"/>\
</sheetData>\
<autoFilter ref=\"A1:C4\" xr:uid=\"{{A5A1F3B2-0000-4000-8000-000000000001}}\"><filterColumn colId=\"1\"><customFilters><customFilter operator=\"greaterThan\" val=\"1\"/></customFilters></filterColumn><sortState xmlns:xlrd2=\"http://schemas.microsoft.com/office/spreadsheetml/2017/richdata2\" ref=\"A2:C4\"><sortCondition ref=\"B2:B4\"/></sortState></autoFilter>\
<mergeCells count=\"1\"><mergeCell ref=\"A6:C6\"/></mergeCells>\
<conditionalFormatting sqref=\"B2:B4\"><cfRule type=\"cellIs\" dxfId=\"0\" priority=\"2\" operator=\"greaterThan\"><formula>1</formula></cfRule></conditionalFormatting>\
<dataValidations count=\"1\"><dataValidation type=\"whole\" allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\" sqref=\"C2:C4\" xr:uid=\"{{0B6C3C7A-0000-4000-8000-000000000002}}\"><formula1>0</formula1><formula2>100</formula2></dataValidation></dataValidations>\
<hyperlinks><hyperlink ref=\"A2\" r:id=\"rId1\" xr:uid=\"{{7E3F2C1D-0000-4000-8000-000000000003}}\"/><hyperlink ref=\"A3\" location=\"Report!A1\" display=\"Report\" xr:uid=\"{{7E3F2C1D-0000-4000-8000-000000000004}}\"/></hyperlinks>\
<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>\
<pageSetup paperSize=\"9\" orientation=\"landscape\" r:id=\"rId2\"/>\
<drawing r:id=\"rId3\"/>\
<legacyDrawing r:id=\"rId4\"/>\
<tableParts count=\"1\"><tablePart r:id=\"rId5\"/></tableParts>\
<extLst><ext uri=\"{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:conditionalFormattings><x14:conditionalFormatting xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\"><x14:cfRule type=\"dataBar\" id=\"{{00000000-000E-0000-0000-000001000000}}\"><x14:dataBar minLength=\"0\" maxLength=\"100\" gradient=\"0\"><x14:cfvo type=\"autoMin\"/><x14:cfvo type=\"autoMax\"/><x14:negativeFillColor rgb=\"FFFF0000\"/><x14:axisColor rgb=\"FF000000\"/></x14:dataBar></x14:cfRule><xm:sqref>C2:C4</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext>\
<ext uri=\"{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:sparklineGroups xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\"><x14:sparklineGroup displayEmptyCellsAs=\"gap\"><x14:colorSeries theme=\"4\" tint=\"-0.499984740745262\"/><x14:sparklines><x14:sparkline><xm:f>Data!B2:C2</xm:f><xm:sqref>D2</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst>\
</worksheet>"
    )
}

/// The worksheet of [`rich_package`]'s `Report` sheet: a formula over the
/// other sheet, one over a defined name, one a later Excel added, a shared
/// formula group, an array formula, a dynamic array anchor, a data table and
/// a volatile formula.
pub fn report_worksheet() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"x14ac xr xr2 xr3\" xmlns:x14ac=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\" xmlns:xr2=\"http://schemas.microsoft.com/office/spreadsheetml/2015/revision2\" xmlns:xr3=\"http://schemas.microsoft.com/office/spreadsheetml/2016/revision3\" xr:uid=\"{{00000000-0001-0000-0100-000000000000}}\">\
<dimension ref=\"A1:E4\"/>\
<sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\"><selection activeCell=\"A1\" sqref=\"A1\"/></sheetView></sheetViews>\
<sheetFormatPr defaultRowHeight=\"14.5\" x14ac:dyDescent=\"0.35\"/>\
<sheetData>\
<row r=\"1\" spans=\"1:5\" x14ac:dyDescent=\"0.35\"><c r=\"A1\"><f>SUM(Data!B2:B4)</f><v>4.25</v></c><c r=\"B1\"><f>Rate*A1</f><v>0.29749999999999999</v></c><c r=\"C1\" t=\"str\"><f>_xlfn.CONCAT(Data!A2,\"-\",Data!E2)</f><v>Apple-Pear</v></c><c r=\"E1\"><f ca=\"1\">TODAY()</f><v>45292</v></c></row>\
<row r=\"2\" spans=\"1:5\" x14ac:dyDescent=\"0.35\"><c r=\"A2\"><f>Data!B2*Data!C2</f><v>15</v></c><c r=\"B2\"><f t=\"array\" ref=\"B2:B4\">Data!B2:B4*2</f><v>3</v></c><c r=\"C2\" cm=\"1\"><f t=\"array\" ref=\"C2:C4\">_xlfn._xlws.SORT(Data!B2:B4)</f><v>0.5</v></c><c r=\"D2\"><f t=\"dataTable\" ref=\"D2:D3\" dtr=\"1\" r1=\"A1\"/><v>1</v></c><c r=\"E2\"><f t=\"shared\" ref=\"E2:E4\" si=\"1\">A2*2</f><v>30</v></c></row>\
<row r=\"3\" spans=\"1:5\" x14ac:dyDescent=\"0.35\"><c r=\"A3\"><f>Data!B3*Data!C3</f><v>45</v></c><c r=\"B3\"><v>4.5</v></c><c r=\"C3\"><v>1.5</v></c><c r=\"D3\"><v>2</v></c><c r=\"E3\"><f t=\"shared\" si=\"1\"/><v>90</v></c></row>\
<row r=\"4\" spans=\"1:5\" x14ac:dyDescent=\"0.35\"><c r=\"A4\"><f>Data!B4*Data!C4</f><v>5</v></c><c r=\"B4\"><v>1</v></c><c r=\"C4\"><v>2.25</v></c><c r=\"E4\"><f t=\"shared\" si=\"1\"/><v>10</v></c></row>\
</sheetData>\
<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>\
</worksheet>"
    )
}

/// The local shared formula each dependent of [`report_worksheet`] holds.
pub const REPORT_SHARED: [(&str, &str); 2] = [("E3", "A3*2"), ("E4", "A4*2")];

/// The styles of [`rich_package`]: fonts, fills, borders, alignment,
/// percent, currency and date formats, theme colours with tints.
pub fn rich_styles() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<styleSheet xmlns=\"{NS}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"x14ac x16r2 xr\" xmlns:x14ac=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac\" xmlns:x16r2=\"http://schemas.microsoft.com/office/spreadsheetml/2015/02/main\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\">\
<numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"&quot;$&quot;#,##0.00\"/></numFmts>\
<fonts count=\"2\" x14ac:knownFonts=\"1\"><font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font><font><b/><sz val=\"11\"/><color theme=\"4\" tint=\"-0.249977111117893\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font></fonts>\
<fills count=\"3\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill><fill><patternFill patternType=\"solid\"><fgColor theme=\"4\" tint=\"0.79998168889431442\"/><bgColor indexed=\"64\"/></patternFill></fill></fills>\
<borders count=\"2\"><border><left/><right/><top/><bottom/><diagonal/></border><border><left style=\"thin\"><color indexed=\"64\"/></left><right style=\"thin\"><color indexed=\"64\"/></right><top style=\"thin\"><color indexed=\"64\"/></top><bottom style=\"thin\"><color indexed=\"64\"/></bottom><diagonal/></border></borders>\
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
<cellXfs count=\"5\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"10\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"1\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\" applyAlignment=\"1\"><alignment horizontal=\"center\" vertical=\"center\" wrapText=\"1\"/></xf><xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>\
<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
<dxfs count=\"1\"><dxf><font><color rgb=\"FF9C0006\"/></font><fill><patternFill><bgColor rgb=\"FFFFC7CE\"/></patternFill></fill></dxf></dxfs>\
<tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium2\" defaultPivotStyle=\"PivotStyleLight16\"/>\
</styleSheet>"
    )
}

/// The shared strings of [`rich_package`], which its sheets index: plain
/// items, a rich one, a phonetic one and a duplicate.
pub fn rich_shared_strings() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<sst xmlns=\"{NS}\" count=\"15\" uniqueCount=\"11\">\
<si><t>Name</t></si><si><t>Price</t></si><si><t>Qty</t></si>\
<si><r><rPr><b/><sz val=\"11\"/><color theme=\"1\"/><rFont val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></rPr><t>be</t></r><r><rPr><sz val=\"11\"/><color theme=\"1\"/><rFont val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></rPr><t>ta</t></r></si>\
<si><t>東京</t><rPh sb=\"0\" eb=\"2\"><t>トウキョウ</t></rPh><phoneticPr fontId=\"1\"/></si>\
<si><t>Apple</t></si><si><t>Apple</t></si><si><t>Pear</t></si>\
<si><t>Sum of Price</t></si><si><t>Item</t></si><si><t>Cost</t></si>\
</sst>"
    )
}

/// A package hand-built in Excel's spelling that carries every fact a
/// round trip through the crate must keep: the `Data` sheet of
/// [`rich_worksheet`] with its drawing and chart, comments and their VML,
/// table, printer settings and external hyperlink; the `Report` sheet of
/// [`report_worksheet`]; a `Pivot` sheet holding a pivot table over a cache
/// the workbook lists; fonts, fills, borders, alignment, percent, currency
/// and date formats with theme colours and tints; rich, phonetic and
/// duplicate shared strings; defined names scoped to a sheet and to the
/// workbook; the active tab; a calculation chain; dynamic array metadata; a
/// theme and the document properties.
pub fn rich_package() -> Vec<u8> {
    let parts = rich_parts();
    let parts: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    package(&parts)
}

/// The members of [`rich_package`], with binary printer settings kept as
/// bytes, in the order the package lists them.
pub fn rich_parts() -> Vec<(&'static str, Vec<u8>)> {
    let main = "application/vnd.openxmlformats-officedocument";
    let types = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"bin\" ContentType=\"{main}.spreadsheetml.printerSettings\"/>\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"vml\" ContentType=\"{main}.vmlDrawing\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"{main}.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"{main}.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"{main}.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet3.xml\" ContentType=\"{main}.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/theme/theme1.xml\" ContentType=\"{main}.theme+xml\"/>\
<Override PartName=\"/xl/styles.xml\" ContentType=\"{main}.spreadsheetml.styles+xml\"/>\
<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"{main}.spreadsheetml.sharedStrings+xml\"/>\
<Override PartName=\"/xl/drawings/drawing1.xml\" ContentType=\"{main}.drawing+xml\"/>\
<Override PartName=\"/xl/charts/chart1.xml\" ContentType=\"{main}.drawingml.chart+xml\"/>\
<Override PartName=\"/xl/tables/table1.xml\" ContentType=\"{main}.spreadsheetml.table+xml\"/>\
<Override PartName=\"/xl/comments1.xml\" ContentType=\"{main}.spreadsheetml.comments+xml\"/>\
<Override PartName=\"/xl/pivotTables/pivotTable1.xml\" ContentType=\"{main}.spreadsheetml.pivotTable+xml\"/>\
<Override PartName=\"/xl/pivotCache/pivotCacheDefinition1.xml\" ContentType=\"{main}.spreadsheetml.pivotCacheDefinition+xml\"/>\
<Override PartName=\"/xl/pivotCache/pivotCacheRecords1.xml\" ContentType=\"{main}.spreadsheetml.pivotCacheRecords+xml\"/>\
<Override PartName=\"/xl/calcChain.xml\" ContentType=\"{main}.spreadsheetml.calcChain+xml\"/>\
<Override PartName=\"/xl/metadata.xml\" ContentType=\"{main}.spreadsheetml.sheetMetadata+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"{main}.extended-properties+xml\"/>\
</Types>"
    );
    let relationship = |id: &str, kind: &str, target: &str| {
        format!(
            "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>"
        )
    };
    let relationships = |entries: &[String]| {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>",
            entries.concat()
        )
    };
    let root = relationships(&[
        relationship("rId3", "extended-properties", "docProps/app.xml"),
        "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>".to_owned(),
        relationship("rId1", "officeDocument", "xl/workbook.xml"),
    ]);
    let workbook_rels = relationships(&[
        relationship("rId8", "pivotCacheDefinition", "pivotCache/pivotCacheDefinition1.xml"),
        relationship("rId3", "worksheet", "worksheets/sheet3.xml"),
        relationship("rId7", "calcChain", "calcChain.xml"),
        relationship("rId2", "worksheet", "worksheets/sheet2.xml"),
        relationship("rId1", "worksheet", "worksheets/sheet1.xml"),
        relationship("rId6", "sharedStrings", "sharedStrings.xml"),
        relationship("rId5", "styles", "styles.xml"),
        relationship("rId4", "theme", "theme/theme1.xml"),
        "<Relationship Id=\"rId9\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata\" Target=\"metadata.xml\"/>".to_owned(),
    ]);
    let book = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<workbook xmlns=\"{NS}\" xmlns:r=\"{R_NS}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"x15 xr xr6 xr10 xr2\" xmlns:x15=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\" xmlns:xr6=\"http://schemas.microsoft.com/office/spreadsheetml/2016/revision6\" xmlns:xr10=\"http://schemas.microsoft.com/office/spreadsheetml/2016/revision10\" xmlns:xr2=\"http://schemas.microsoft.com/office/spreadsheetml/2015/revision2\">\
<fileVersion appName=\"xl\" lastEdited=\"7\" lowestEdited=\"7\" rupBuild=\"27425\"/>\
<workbookPr defaultThemeVersion=\"202300\"/>\
<bookViews><workbookView xWindow=\"-110\" yWindow=\"-110\" windowWidth=\"25820\" windowHeight=\"15500\" activeTab=\"1\" xr2:uid=\"{{00000000-000D-0000-FFFF-FFFF00000000}}\"/></bookViews>\
<sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Report\" sheetId=\"2\" r:id=\"rId2\"/><sheet name=\"Pivot\" sheetId=\"4\" r:id=\"rId3\"/></sheets>\
<definedNames><definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">Data!$A$1:$C$4</definedName><definedName name=\"_xlnm.Print_Area\" localSheetId=\"1\">Report!$A$1:$E$4</definedName><definedName name=\"Prices\" comment=\"the price column\">Data!$B$2:$B$4</definedName><definedName name=\"Rate\">0.07</definedName></definedNames>\
<calcPr calcId=\"191029\"/>\
<pivotCaches><pivotCache cacheId=\"1\" r:id=\"rId8\"/></pivotCaches>\
<extLst><ext uri=\"{{140A7094-0E35-4892-8432-C4D2E57EDEB5}}\" xmlns:x15=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main\"><x15:workbookPr chartTrackingRefBase=\"1\"/></ext></extLst>\
</workbook>"
    );
    let styles = rich_styles();
    let strings = rich_shared_strings();
    let sheet1_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId6\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments\" Target=\"../comments1.xml\"/>\
<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"../drawings/drawing1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings\" Target=\"../printerSettings/printerSettings1.bin\"/>\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"https://example.com/apple\" TargetMode=\"External\"/>\
<Relationship Id=\"rId5\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/table\" Target=\"../tables/table1.xml\"/>\
<Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\"/>\
</Relationships>"
        .to_owned();
    let pivot_sheet = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><dimension ref=\"A3:B4\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"14.5\"/>\
<sheetData><row r=\"3\" spans=\"1:2\"><c r=\"A3\" t=\"s\"><v>0</v></c><c r=\"B3\" t=\"s\"><v>8</v></c></row><row r=\"4\" spans=\"1:2\"><c r=\"A4\" t=\"s\"><v>5</v></c><c r=\"B4\"><v>1.5</v></c></row></sheetData>\
<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/><pivotTableParts count=\"1\"><pivotTablePart r:id=\"rId1\"/></pivotTableParts></worksheet>"
    );
    let sheet3_rels = relationships(&[relationship(
        "rId1",
        "pivotTable",
        "../pivotTables/pivotTable1.xml",
    )]);
    let pivot_table = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<pivotTableDefinition xmlns=\"{NS}\" name=\"PivotTable1\" cacheId=\"1\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\" dataCaption=\"Values\" updatedVersion=\"8\" minRefreshableVersion=\"3\" createdVersion=\"8\" indent=\"0\" outline=\"1\" outlineData=\"1\" multipleFieldFilters=\"0\">\
<location ref=\"A3:B4\" firstHeaderRow=\"1\" firstDataRow=\"1\" firstDataCol=\"1\"/>\
<pivotFields count=\"2\"><pivotField axis=\"axisRow\" showAll=\"0\"><items count=\"2\"><item x=\"0\"/><item t=\"default\"/></items></pivotField><pivotField dataField=\"1\" showAll=\"0\"/></pivotFields>\
<rowFields count=\"1\"><field x=\"0\"/></rowFields><rowItems count=\"1\"><i><x/></i></rowItems><colItems count=\"1\"><i/></colItems>\
<dataFields count=\"1\"><dataField name=\"Sum of Price\" fld=\"1\" baseField=\"0\" baseItem=\"0\"/></dataFields>\
<pivotTableStyleInfo name=\"PivotStyleLight16\" showRowHeaders=\"1\" showColHeaders=\"1\" showRowStripes=\"0\" showColStripes=\"0\" showLastColumn=\"1\"/></pivotTableDefinition>"
    );
    let pivot_table_rels = relationships(&[relationship(
        "rId1",
        "pivotCacheDefinition",
        "../pivotCache/pivotCacheDefinition1.xml",
    )]);
    let cache = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<pivotCacheDefinition xmlns=\"{NS}\" xmlns:r=\"{R_NS}\" r:id=\"rId1\" refreshedBy=\"Author\" refreshedDate=\"45292.5\" createdVersion=\"8\" refreshedVersion=\"8\" minRefreshableVersion=\"3\" recordCount=\"1\">\
<cacheSource type=\"worksheet\"><worksheetSource ref=\"A1:B2\" sheet=\"Data\"/></cacheSource>\
<cacheFields count=\"2\"><cacheField name=\"Name\" numFmtId=\"0\"><sharedItems count=\"1\"><s v=\"Apple\"/></sharedItems></cacheField><cacheField name=\"Price\" numFmtId=\"0\"><sharedItems containsSemiMixedTypes=\"0\" containsString=\"0\" containsNumber=\"1\" minValue=\"1.5\" maxValue=\"1.5\"/></cacheField></cacheFields></pivotCacheDefinition>"
    );
    let cache_rels = relationships(&[relationship(
        "rId1",
        "pivotCacheRecords",
        "pivotCacheRecords1.xml",
    )]);
    let records = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<pivotCacheRecords xmlns=\"{NS}\" xmlns:r=\"{R_NS}\" count=\"1\"><r><x v=\"0\"/><n v=\"1.5\"/></r></pivotCacheRecords>"
    );
    let drawing = include_str!("fixtures/rich_drawing.xml").to_owned();
    let drawing_rels = relationships(&[relationship("rId1", "chart", "../charts/chart1.xml")]);
    let chart = include_str!("fixtures/rich_chart.xml").to_owned();
    let vml = include_str!("fixtures/rich_vml.xml").to_owned();
    let comments = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<comments xmlns=\"{NS}\"><authors><author>Author</author></authors><commentList><comment ref=\"B2\" authorId=\"0\"><text><r><rPr><b/><sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/><family val=\"2\"/></rPr><t>Author:</t></r><r><rPr><sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/><family val=\"2\"/></rPr><t xml:space=\"preserve\"> list price</t></r></text></comment></commentList></comments>"
    );
    let table = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<table xmlns=\"{NS}\" id=\"1\" name=\"Costs\" displayName=\"Costs\" ref=\"E1:F3\" totalsRowShown=\"0\"><autoFilter ref=\"E1:F3\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Item\"/><tableColumn id=\"2\" name=\"Cost\"/></tableColumns><tableStyleInfo name=\"TableStyleMedium2\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>"
    );
    let metadata = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<metadata xmlns=\"{NS}\" xmlns:xda=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\"><metadataTypes count=\"1\"><metadataType name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\" cellMeta=\"1\"/></metadataTypes><futureMetadata name=\"XLDAPR\" count=\"1\"><bk><extLst><ext uri=\"{{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}}\"><xda:dynamicArrayProperties fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata></metadata>"
    );
    let chain = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<calcChain xmlns=\"{NS}\"><c r=\"A1\" i=\"2\"/><c r=\"B1\"/><c r=\"C1\"/><c r=\"E1\"/><c r=\"A2\"/><c r=\"B2\"/><c r=\"C2\"/><c r=\"A3\"/><c r=\"A4\"/></calcChain>"
    );
    let theme = include_str!("fixtures/rich_theme.xml").to_owned();
    let core = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:creator>Author</dc:creator><dcterms:created xsi:type=\"dcterms:W3CDTF\">2024-01-01T00:00:00Z</dcterms:created></cp:coreProperties>"
        .to_owned();
    let app = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><Application>Microsoft Excel</Application><TitlesOfParts><vt:vector size=\"3\" baseType=\"lpstr\"><vt:lpstr>Data</vt:lpstr><vt:lpstr>Report</vt:lpstr><vt:lpstr>Pivot</vt:lpstr></vt:vector></TitlesOfParts></Properties>"
        .to_owned();
    let mut parts: Vec<(&'static str, Vec<u8>)> = vec![
        ("[Content_Types].xml", types),
        ("_rels/.rels", root),
        ("docProps/app.xml", app),
        ("docProps/core.xml", core),
        ("xl/workbook.xml", book),
        ("xl/_rels/workbook.xml.rels", workbook_rels),
        ("xl/worksheets/sheet1.xml", rich_worksheet()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet1_rels),
        ("xl/worksheets/sheet2.xml", report_worksheet()),
        ("xl/worksheets/sheet3.xml", pivot_sheet),
        ("xl/worksheets/_rels/sheet3.xml.rels", sheet3_rels),
        ("xl/theme/theme1.xml", theme),
        ("xl/styles.xml", styles),
        ("xl/sharedStrings.xml", strings),
        ("xl/metadata.xml", metadata),
        ("xl/calcChain.xml", chain),
        ("xl/drawings/drawing1.xml", drawing),
        ("xl/drawings/_rels/drawing1.xml.rels", drawing_rels),
        ("xl/charts/chart1.xml", chart),
        ("xl/drawings/vmlDrawing1.vml", vml),
        ("xl/comments1.xml", comments),
        ("xl/tables/table1.xml", table),
        ("xl/pivotTables/pivotTable1.xml", pivot_table),
        (
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            pivot_table_rels,
        ),
        ("xl/pivotCache/pivotCacheDefinition1.xml", cache),
        (
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            cache_rels,
        ),
        ("xl/pivotCache/pivotCacheRecords1.xml", records),
    ]
    .into_iter()
    .map(|(name, text)| (name, text.into_bytes()))
    .collect();
    let printer = parts
        .iter()
        .position(|(name, _)| *name == "xl/calcChain.xml")
        .unwrap()
        + 1;
    parts.insert(
        printer,
        (
            "xl/printerSettings/printerSettings1.bin",
            include_bytes!("fixtures/rich_printer.bin").to_vec(),
        ),
    );
    parts
}

/// `xml` in a canonical spelling two documents stating the same thing
/// share: attributes sorted, a self-closed element spelled open and
/// closed, entities resolved - with the changes a worksheet written from
/// its cells may make, which the design lists and nothing else: the
/// `dimension`'s span and each row's `spans` left out, a cell's attributes
/// beyond `r s t cm vm ph` and its `extLst` left out, a number's digits
/// read as the double they spell, and a shared formula group spelled as one
/// plain `<f>` per cell, `shared` naming the text each dependent holds.
pub fn normalized(xml: &str, shared: &[(&str, &str)]) -> String {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut skipping = 0_usize;
    let mut text = String::new();
    // A dependent's `<f/>`, whose text the group states.
    let mut dependent = false;
    let local = |name: &[u8]| -> String {
        let name = String::from_utf8_lossy(name).into_owned();
        name.rsplit(':').next().unwrap_or_default().to_owned()
    };
    loop {
        let event = reader.read_event().expect("well-formed XML");
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                if skipping > 0 {
                    if matches!(event, Event::Start(_)) {
                        skipping += 1;
                    }
                    continue;
                }
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let element = local(start.name().as_ref());
                if element == "extLst"
                    && stack
                        .last()
                        .is_some_and(|parent| local(parent.as_bytes()) == "c")
                {
                    if matches!(event, Event::Start(_)) {
                        skipping = 1;
                    }
                    continue;
                }
                let mut attributes: Vec<(String, String)> = start
                    .attributes()
                    .map(|attribute| {
                        let attribute = attribute.expect("an attribute");
                        (
                            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                            attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .expect("an attribute value")
                                .into_owned(),
                        )
                    })
                    .collect();
                let shared_formula = element == "f"
                    && attributes
                        .iter()
                        .any(|(key, value)| key == "t" && value == "shared");
                attributes.retain(|(key, _)| match element.as_str() {
                    "dimension" => key != "ref",
                    "row" => key != "spans",
                    "c" => ["r", "s", "t", "cm", "vm", "ph"].contains(&key.as_str()),
                    "f" if shared_formula => !["t", "ref", "si"].contains(&key.as_str()),
                    _ => true,
                });
                if element == "c" {
                    cell = attributes
                        .iter()
                        .find(|(key, _)| key == "r")
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default();
                }
                attributes.sort();
                out.push('<');
                out.push_str(&name);
                for (key, value) in &attributes {
                    out.push_str(&format!(" {key}={value:?}"));
                }
                out.push('>');
                dependent = shared_formula;
                text.clear();
                if matches!(event, Event::Empty(_)) {
                    if dependent {
                        if let Some((_, formula)) = shared.iter().find(|(at, _)| *at == cell) {
                            out.push_str(formula);
                        }
                    }
                    out.push_str(&format!("</{name}>"));
                    dependent = false;
                } else {
                    stack.push(name);
                }
            }
            Event::End(ref end) => {
                if skipping > 0 {
                    skipping -= 1;
                    continue;
                }
                let name = String::from_utf8_lossy(end.name().as_ref()).into_owned();
                let element = local(end.name().as_ref());
                let mut held = std::mem::take(&mut text);
                if element == "v" {
                    if let Ok(number) = held.trim().parse::<f64>() {
                        held = format!("{number}");
                    }
                }
                if element == "f" && dependent && held.is_empty() {
                    if let Some((_, formula)) = shared.iter().find(|(at, _)| *at == cell) {
                        held = (*formula).to_owned();
                    }
                }
                dependent = false;
                out.push_str(&held.replace('&', "&amp;").replace('<', "&lt;"));
                out.push_str(&format!("</{name}>"));
                stack.pop();
            }
            Event::Text(ref held) => {
                if skipping == 0 {
                    text.push_str(&held.xml10_content().expect("text"));
                }
            }
            Event::CData(ref held) => {
                if skipping == 0 {
                    text.push_str(&String::from_utf8_lossy(held));
                }
            }
            Event::GeneralRef(ref reference) => {
                if skipping == 0 {
                    let name = String::from_utf8_lossy(reference).into_owned();
                    let character = match name.as_str() {
                        "amp" => "&".to_owned(),
                        "lt" => "<".to_owned(),
                        "gt" => ">".to_owned(),
                        "quot" => "\"".to_owned(),
                        "apos" => "'".to_owned(),
                        other => reference
                            .resolve_char_ref()
                            .ok()
                            .flatten()
                            .map_or_else(|| format!("&{other};"), |c| c.to_string()),
                    };
                    text.push_str(&character);
                }
            }
            Event::Decl(ref declaration) => {
                out.push_str(&format!("<?{}?>", String::from_utf8_lossy(declaration)));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Plain numeric cells beside independent legacy note parts. Varying cells
/// does not change any XML registration; varying notes changes only payload.
pub fn note_cost_package(notes: u32, cells: u32) -> Vec<u8> {
    assert!(notes > 0);
    let rows: String = (1..=cells)
        .map(|row| format!("<row r=\"{row}\"><c r=\"C{row}\"><v>{row}</v></c></row>"))
        .collect();
    let sheet = worksheet(&rows).replace(
        "</worksheet>",
        "<legacyDrawing r:id=\"rIdVml\"/></worksheet>",
    );
    let comments: String = (0..notes)
        .map(|index| {
            format!(
                "<comment ref=\"B{}\" authorId=\"0\"><text><t>Note {index}</t></text></comment>",
                2 * index + 2
            )
        })
        .collect();
    let comments = format!(
        "<comments xmlns=\"{NS}\"><authors><author>Author</author></authors><commentList>{comments}</commentList></comments>"
    );
    let shapes: String = (0..notes).map(|index| {
        let row = 2 * index + 1;
        format!("<v:shape id=\"_x0000_s{}\" type=\"#_x0000_t202\" style=\"position:absolute;visibility:hidden\"><x:ClientData ObjectType=\"Note\"><x:Anchor>2, 15, {}, 2, 4, 15, {}, 16</x:Anchor><x:Row>{row}</x:Row><x:Column>1</x:Column></x:ClientData></v:shape>", 1025 + index, row - 1, row + 3)
    }).collect();
    let drawing = format!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout><v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"/>{shapes}</xml>"
    );
    let rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdNotes\" Type=\"{R_NS}/comments\" Target=\"../comments1.xml\"/><Relationship Id=\"rIdVml\" Type=\"{R_NS}/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\"/></Relationships>"
    );
    let types = content_types(1, false, false).replace("</Types>", "<Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/><Override PartName=\"/xl/comments1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml\"/></Types>");
    package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Data"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/worksheets/_rels/sheet1.xml.rels", &rels),
        ("xl/comments1.xml", &comments),
        ("xl/drawings/vmlDrawing1.vml", &drawing),
    ])
}

/// Cross-sheet carried ownership at two scales. The grid is deliberately
/// orthogonal to the registrations, so a per-cell scan is visible in costs.
pub fn carried_cost_package(kind: &str, registrations: u32, cells: u32) -> Vec<u8> {
    assert!(registrations > 0);
    let rows: String = (1..=cells)
        .map(|row| format!("<row r=\"{row}\"><c r=\"C{row}\"><v>{row}</v></c></row>"))
        .collect();
    let mut fragments = String::new();
    let mut links = String::new();
    for index in 0..registrations {
        let row = index + 2;
        match kind {
            "cf" => fragments.push_str(&format!(
                "<conditionalFormatting sqref=\"B{row}\"><cfRule type=\"expression\" priority=\"{}\"><formula>B{row}&gt;0</formula></cfRule></conditionalFormatting>",
                index + 1,
            )),
            "x14cf" => fragments.push_str(&format!(
                "<x14:conditionalFormatting><x14:cfRule type=\"expression\" priority=\"{}\"><xm:f>B{row}&gt;0</xm:f></x14:cfRule><xm:sqref>B{row}</xm:sqref></x14:conditionalFormatting>",
                index + 1,
            )),
            "x14dv" => fragments.push_str(&format!(
                "<x14:dataValidation type=\"whole\"><x14:formula1><xm:f>B{row}</xm:f></x14:formula1><xm:sqref>B{row}</xm:sqref></x14:dataValidation>"
            )),
            "x14spark" => fragments.push_str(&format!(
                "<x14:sparkline><xm:f>Data!C1:C3</xm:f><xm:sqref>B{row}</xm:sqref></x14:sparkline>"
            )),
            "dv" => fragments.push_str(&format!(
                "<dataValidation type=\"whole\" sqref=\"B{row}\"><formula1>B{row}</formula1></dataValidation>"
            )),
            "location" => fragments.push_str(&format!(
                "<hyperlink ref=\"B{row}\" location=\"Data!A1\"/>"
            )),
            "protected" => fragments.push_str(&format!(
                "<protectedRange name=\"Range{index}\" sqref=\"B{row}\" password=\"ABCD\"/>"
            )),
            "ignored" => fragments.push_str(&format!(
                "<ignoredError sqref=\"B{row}\" numberStoredAsText=\"1\"/>"
            )),
            "watch" => fragments.push_str(&format!("<cellWatch r=\"B{row}\"/>")),
            "hyperlink" | "hyperlink_existing" => {
                fragments.push_str(&format!("<hyperlink ref=\"B{row}\" r:id=\"rIdLink{index}\"/>"));
                links.push_str(&format!(
                    "<Relationship Id=\"rIdLink{index}\" Type=\"{R_NS}/hyperlink\" Target=\"https://example.invalid/{index}\" TargetMode=\"External\"/>"
                ));
            }
            _ => panic!("expected a carried-registration corpus kind"),
        }
    }
    let after = match kind {
        "cf" => fragments,
        "dv" => format!("<dataValidations count=\"{registrations}\">{fragments}</dataValidations>"),
        "x14cf" | "x14dv" | "x14spark" => {
            let (uri, container, child) = match kind {
                "x14cf" => (
                    "{78C0D931-6437-407d-A8EE-F0AAD7539E65}",
                    "conditionalFormattings",
                    fragments,
                ),
                "x14dv" => (
                    "{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}",
                    "dataValidations",
                    fragments,
                ),
                "x14spark" => (
                    "{05C60535-1F16-4fd2-B633-F4F36F0B64E0}",
                    "sparklineGroups",
                    format!(
                        "<x14:sparklineGroup><x14:sparklines>{fragments}</x14:sparklines></x14:sparklineGroup>"
                    ),
                ),
                _ => unreachable!(),
            };
            let count = if kind == "x14dv" {
                format!(" count=\"{registrations}\"")
            } else {
                String::new()
            };
            format!(
                "<extLst><ext uri=\"{uri}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\"><x14:{container}{count}>{child}</x14:{container}></ext></extLst>"
            )
        }
        "location" | "hyperlink" | "hyperlink_existing" => {
            format!("<hyperlinks>{fragments}</hyperlinks>")
        }
        "protected" => format!("<protectedRanges>{fragments}</protectedRanges>"),
        "ignored" => format!("<ignoredErrors>{fragments}</ignoredErrors>"),
        "watch" => format!("<cellWatches>{fragments}</cellWatches>"),
        _ => unreachable!(),
    };
    let source = worksheet(&rows).replace("</worksheet>", &format!("{after}</worksheet>"));
    let destination = worksheet("");
    let source_rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{links}</Relationships>"
    );
    let book = workbook(&["Data", "Other"], false);
    let root_rels = root_relationships();
    let book_rels = workbook_relationships(2, false, false);
    let types = content_types(2, false, false);
    let mut parts = vec![
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root_rels.as_str()),
        ("xl/workbook.xml", book.as_str()),
        ("xl/_rels/workbook.xml.rels", book_rels.as_str()),
        ("xl/worksheets/sheet1.xml", source.as_str()),
        ("xl/worksheets/sheet2.xml", destination.as_str()),
    ];
    if matches!(kind, "hyperlink" | "hyperlink_existing") {
        parts.push(("xl/worksheets/_rels/sheet1.xml.rels", source_rels.as_str()));
    }
    let destination_rels = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>";
    if kind == "hyperlink_existing" {
        parts.push(("xl/worksheets/_rels/sheet2.xml.rels", destination_rels));
    }
    package(&parts)
}

/// Prove a metadata-only cost workload changed worksheet ownership.
/// Call outside the measured allocation or source-read interval.
pub fn assert_cost_carried_scope_moved(workbook: &Workbook, kind: &str, registrations: u32) {
    let (container, child, reference) = match kind {
        "protected" => ("protectedRanges", "protectedRange", "sqref"),
        "ignored" => ("ignoredErrors", "ignoredError", "sqref"),
        "watch" => ("cellWatches", "cellWatch", "r"),
        _ => panic!("expected a formula-free carried-registration kind"),
    };
    let source_xml = member(workbook, "xl/worksheets/sheet1.xml");
    let source = yggdryl::xml::from_bytes(source_xml.as_bytes()).unwrap();
    let source = yggdryl::xml::Element::root(&source).unwrap();
    assert!(source.child(Some(NS), container).is_none());
    let target_xml = member(workbook, "xl/worksheets/sheet2.xml");
    let target = yggdryl::xml::from_bytes(target_xml.as_bytes()).unwrap();
    let target = yggdryl::xml::Element::root(&target).unwrap();
    let container = target.child(Some(NS), container).unwrap();
    let entries = container.children_in(Some(NS), child);
    assert_eq!(entries.len(), registrations as usize);
    for (index, entry) in entries.into_iter().enumerate() {
        let expected = format!("J{}", index + 10);
        assert_eq!(entry.attribute_in(None, reference), Some(expected.as_str()));
    }
}

/// The note-only cut still changed the payload when it moved no stored Cell.
pub fn assert_cost_note_moved(workbook: &yggdryl::excel::Workbook, notes: u32) {
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        workbook.into_bytes().unwrap(),
    ))));
    let bytes = archive.read_member("xl/comments1.xml").unwrap();
    let doc = yggdryl::xml::from_bytes(&bytes).unwrap();
    let root = yggdryl::xml::Element::root(&doc).unwrap();
    let list = root.child(Some(NS), "commentList").unwrap();
    let comments = list.children_in(Some(NS), "comment");
    assert_eq!(comments.len(), notes as usize);
    assert!(
        comments
            .iter()
            .any(|entry| entry.attribute_in(None, "ref") == Some("F6"))
    );
    assert!(
        !comments
            .iter()
            .any(|entry| entry.attribute_in(None, "ref") == Some("B2"))
    );
}

/// A relationship: its id, the last segment of its type, its target.
pub type Related<'a> = (&'a str, &'a str, &'a str);

/// A worksheet part: `data` its rows, `after` what follows them.
pub fn sheet(data: &str, after: &str) -> String {
    format!(
        "<worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheetData>{data}</sheetData>{after}</worksheet>"
    )
}

pub fn member(workbook: &Workbook, name: &str) -> String {
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        workbook.into_bytes().unwrap(),
    ))));
    String::from_utf8(archive.read_member(name).unwrap()).unwrap()
}

/// A table with non-sequential column ids: its formulas belong to the second
/// column in document order, independent of the ids Excel assigned it.
pub fn table_formula_part(range: &str, filter: &str, calculated: &str, totals: &str) -> String {
    format!(
        "<table xmlns=\"{NS}\" id=\"1\" name=\"Costs\" displayName=\"Costs\" ref=\"{range}\" totalsRowCount=\"1\"><autoFilter ref=\"{filter}\"/><tableColumns count=\"2\"><tableColumn id=\"7\" name=\"Item\" totalsRowLabel=\"Total\"/><tableColumn id=\"42\" name=\"Cost\" totalsRowFunction=\"custom\"><calculatedColumnFormula array=\"1\">{calculated}</calculatedColumnFormula><totalsRowFormula array=\"0\">{totals}</totalsRowFormula></tableColumn></tableColumns></table>"
    )
}

/// Compare the logical package, independent of ZIP record order or compression.
pub fn table_member_map(workbook: &Workbook) -> std::collections::BTreeMap<String, Vec<u8>> {
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        workbook.into_bytes().unwrap(),
    ))));
    archive
        .entries()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry.name().to_owned(),
                archive.read_member(entry.name()).unwrap(),
            )
        })
        .collect()
}

/// A table's ranges in document order: table, filter, sort and sort column.
pub fn cut_table_part(name: &str, id: u32, ranges: [&str; 4], formulas: [&str; 2]) -> String {
    let [range, filter, sort, condition] = ranges;
    let [calculated, totals] = formulas;
    format!(
        "<table xmlns=\"{NS}\" id=\"{id}\" name=\"{name}\" displayName=\"{name}\" ref=\"{range}\" totalsRowCount=\"1\"><autoFilter ref=\"{filter}\"><filterColumn colId=\"1\"><customFilters><customFilter operator=\"greaterThan\" val=\"1\"/></customFilters></filterColumn></autoFilter><sortState ref=\"{sort}\" caseSensitive=\"0\"><sortCondition ref=\"{condition}\" descending=\"1\"/></sortState><tableColumns count=\"2\"><tableColumn id=\"7\" name=\"Item\" totalsRowLabel=\"Total\"/><tableColumn id=\"42\" name=\"Cost\" totalsRowFunction=\"custom\"><calculatedColumnFormula array=\"1\">{calculated}</calculatedColumnFormula><totalsRowFormula array=\"0\">{totals}</totalsRowFormula></tableColumn></tableColumns><tableStyleInfo name=\"TableStyleMedium2\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>"
    )
}

/// Each table keeps its own part and relationship, including tables on Other.
/// Reopen the populated package so exact undo comparisons begin at a clean,
/// already serialized workbook rather than at a synthetic worksheet spelling.
pub fn with_cut_tables(tables: &[(&str, String)]) -> Workbook {
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let names: Vec<_> = (1..=tables.len())
        .map(|index| format!("xl/tables/table{index}.xml"))
        .collect();
    let ids: Vec<_> = (1..=tables.len())
        .map(|index| format!("rIdTable{index}"))
        .collect();
    let targets: Vec<_> = (1..=tables.len())
        .map(|index| format!("../tables/table{index}.xml"))
        .collect();
    let relations: Vec<Vec<Related<'_>>> = ["Data", "Other"]
        .iter()
        .map(|owner| {
            tables
                .iter()
                .enumerate()
                .filter(|(_, (sheet, _))| sheet == owner)
                .map(|(index, _)| (ids[index].as_str(), "table", targets[index].as_str()))
                .collect()
        })
        .collect();
    let frames: Vec<_> = relations
        .iter()
        .map(|relations| {
            let listed: String = relations
                .iter()
                .map(|(id, _, _)| format!("<tablePart r:id=\"{id}\"/>"))
                .collect();
            let after = if relations.is_empty() {
                String::new()
            } else {
                format!(
                    "<tableParts count=\"{}\">{listed}</tableParts>",
                    relations.len()
                )
            };
            sheet("", &after)
        })
        .collect();
    let parts: Vec<_> = names
        .iter()
        .zip(tables)
        .map(|(name, (_, xml))| (name.as_str(), xml.clone()))
        .collect();
    let mut workbook = sheets_book(
        &[
            ("Data", frames[0].clone(), &relations[0]),
            ("Other", frames[1].clone(), &relations[1]),
        ],
        &[],
        &parts,
    );
    let data = workbook.sheet_mut("Data").unwrap();
    for (reference, value) in [
        ("A1", 9.0),
        ("E3", 3.0),
        ("F4", 4.0),
        ("G4", 44.0),
        ("H4", 8.0),
    ] {
        data.set_cell(at(reference), value).unwrap();
    }
    data.insert_cell(
        Cell::from_scalar(at("F3"), 12.0.into(), DateSystem::Year1900)
            .unwrap()
            .with_formula(Formula::from_file("E3+$A$1", at("F3"))),
    )
    .unwrap();
    workbook
        .sheet_mut("Other")
        .unwrap()
        .set_cell(at("B3"), 30.0)
        .unwrap();
    Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap()
}

pub fn costs_cut_part() -> String {
    cut_table_part(
        "Costs",
        11,
        ["E2:F6", "E2:F5", "E3:F5", "F3:F5"],
        [
            "E3+$E$3+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
            "SUM(E3:E5)+SUM($E$3:$E$5)+Costs[Cost]+$A$1",
        ],
    )
}

/// A package of the worksheets `sheets` - each its name as the workbook
/// part spells it (escaped), its part's text and its relationships - the
/// workbook's own relationships `related` beside theirs, and the members
/// `parts`.
pub fn sheets_book(
    sheets: &[(&str, String, &[Related<'_>])],
    related: &[(&str, &str, &str)],
    parts: &[(&str, String)],
) -> Workbook {
    let relationship = |(id, kind, target): &(&str, &str, &str)| {
        let family = if kind.starts_with("slicer") {
            "http://schemas.microsoft.com/office/2007/relationships"
        } else if kind.starts_with("timeline") {
            "http://schemas.microsoft.com/office/2011/relationships"
        } else {
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        };
        format!("<Relationship Id=\"{id}\" Type=\"{family}/{kind}\" Target=\"{target}\"/>")
    };
    let rels = |entries: String| {
        format!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{entries}</Relationships>"
        )
    };
    let listed: String = sheets
        .iter()
        .enumerate()
        .map(|(index, (name, _, _))| {
            format!(
                "<sheet name=\"{name}\" sheetId=\"{}\" r:id=\"rIdS{}\"/>",
                index + 1,
                index + 1
            )
        })
        .collect();
    let mut book_rels: String = (1..=sheets.len())
        .map(|index| {
            relationship(&(
                format!("rIdS{index}").as_str(),
                "worksheet",
                format!("worksheets/sheet{index}.xml").as_str(),
            ))
        })
        .collect();
    book_rels.extend(related.iter().map(relationship));
    let mut members: Vec<(String, String)> = vec![
        (
            "[Content_Types].xml".into(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>".into(),
        ),
        (
            "_rels/.rels".into(),
            rels("<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>".into()),
        ),
        (
            "xl/workbook.xml".into(),
            format!("<workbook xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheets>{listed}</sheets></workbook>"),
        ),
        ("xl/_rels/workbook.xml.rels".into(), rels(book_rels)),
    ];
    for (index, (_, part, relationships)) in sheets.iter().enumerate() {
        members.push((
            format!("xl/worksheets/sheet{}.xml", index + 1),
            part.clone(),
        ));
        if !relationships.is_empty() {
            members.push((
                format!("xl/worksheets/_rels/sheet{}.xml.rels", index + 1),
                rels(relationships.iter().map(relationship).collect()),
            ));
        }
    }
    for (name, text) in parts {
        members.push(((*name).to_owned(), text.clone()));
    }
    let borrowed: Vec<(&str, &str)> = members
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let workbook = Workbook::from_bytes(package(&borrowed)).unwrap();
    workbook.parse_all().unwrap();
    workbook
}

/// A package whose one sheet `Sheet1` has the part `sheet`, its
/// relationships `relationships` and the members `parts` beside it.
pub fn book(sheet: &str, relationships: &[(&str, &str, &str)], parts: &[(&str, &str)]) -> Workbook {
    let mut members: Vec<(String, String)> = vec![
        (
            "[Content_Types].xml".into(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>".into(),
        ),
        (
            "_rels/.rels".into(),
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>".into(),
        ),
        (
            "xl/workbook.xml".into(),
            format!("<workbook xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"),
        ),
        (
            "xl/_rels/workbook.xml.rels".into(),
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>".into(),
        ),
        ("xl/worksheets/sheet1.xml".into(), sheet.to_owned()),
        ("xl/sharedStrings.xml".into(), rich_shared_strings()),
        ("xl/styles.xml".into(), rich_styles()),
    ];
    if !relationships.is_empty() {
        let entries: String = relationships
            .iter()
            .map(|(id, kind, target)| {
                format!(
                    "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>"
                )
            })
            .collect();
        members.push((
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{entries}</Relationships>"),
        ));
    }
    for (name, text) in parts {
        members.push(((*name).to_owned(), (*text).to_owned()));
    }
    let borrowed: Vec<(&str, &str)> = members
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let workbook = Workbook::from_bytes(package(&borrowed)).unwrap();
    workbook.parse_all().unwrap();
    workbook
}

pub fn stationary_cut_table(name: &str, id: u32, range: &str) -> String {
    let parsed: yggdryl::excel::CellRange = range.parse().unwrap();
    let start = parsed.start();
    let end = parsed.end();
    let filter = yggdryl::excel::CellRange::new(start, CellRef::new(end.row() - 1, end.column()))
        .to_string();
    let sort = yggdryl::excel::CellRange::new(
        CellRef::new(start.row() + 1, start.column()),
        CellRef::new(end.row() - 1, end.column()),
    )
    .to_string();
    let condition = yggdryl::excel::CellRange::new(
        CellRef::new(start.row() + 1, end.column()),
        CellRef::new(end.row() - 1, end.column()),
    )
    .to_string();
    cut_table_part(name, id, [range, &filter, &sort, &condition], ["0", "0"])
}

/// The legacy cost fixture with one root and one reply per note, plus the
/// workbook-owned persons registry. Cell count does not change these parts.
pub fn threaded_cost_package(threads: u32, cells: u32) -> Vec<u8> {
    const T_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
    const T_REL: &str = "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment";
    const P_REL: &str = "http://schemas.microsoft.com/office/2017/10/relationships/person";
    const PERSON: &str = "{00010000-0000-4000-8000-000000000001}";
    let mut parts =
        table_member_map(&Workbook::from_bytes(note_cost_package(threads, cells)).unwrap());
    let mut comments = String::from_utf8(parts["xl/comments1.xml"].clone()).unwrap();
    let mut authors = String::from("<authors>");
    let mut messages = String::new();
    for index in 0..threads {
        let root = format!("{{00000000-0000-4000-8000-{:012x}}}", 2 * index + 1);
        let reply = format!("{{00000000-0000-4000-8000-{:012x}}}", 2 * index + 2);
        let row = 2 * index + 2;
        authors.push_str(&format!("<author>tc={root}</author>"));
        comments = comments.replace(
            &format!("<comment ref=\"B{row}\" authorId=\"0\">"),
            &format!("<comment ref=\"B{row}\" authorId=\"{index}\">"),
        );
        messages.push_str(&format!("<threadedComment ref=\"B{row}\" personId=\"{PERSON}\" id=\"{root}\"><text>Thread {index}</text></threadedComment><threadedComment personId=\"{PERSON}\" id=\"{reply}\" parentId=\"{root}\"><text>Reply {index}</text></threadedComment>"));
    }
    authors.push_str("</authors>");
    comments = comments.replace("<authors><author>Author</author></authors>", &authors);
    parts.insert("xl/comments1.xml".into(), comments.into_bytes());
    parts.insert(
        "xl/threadedComments/threadedComment1.xml".into(),
        format!("<ThreadedComments xmlns=\"{T_NS}\">{messages}</ThreadedComments>").into_bytes(),
    );
    parts.insert("xl/persons/person.xml".into(), format!("<personList xmlns=\"{T_NS}\"><person id=\"{PERSON}\" displayName=\"Author\" userId=\"Author\" providerId=\"None\"/></personList>").into_bytes());
    for (member, registration) in [
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            format!(
                "<Relationship Id=\"rIdThreads\" Type=\"{T_REL}\" Target=\"../threadedComments/threadedComment1.xml\"/>"
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                "<Relationship Id=\"rIdPersons\" Type=\"{P_REL}\" Target=\"persons/person.xml\"/>"
            ),
        ),
    ] {
        let xml = std::str::from_utf8(&parts[member]).unwrap().replace(
            "</Relationships>",
            &format!("{registration}</Relationships>"),
        );
        parts.insert(member.into(), xml.into_bytes());
    }
    let types = std::str::from_utf8(&parts["[Content_Types].xml"]).unwrap().replace("</Types>", "<Override PartName=\"/xl/threadedComments/threadedComment1.xml\" ContentType=\"application/vnd.ms-excel.threadedcomments+xml\"/><Override PartName=\"/xl/persons/person.xml\" ContentType=\"application/vnd.ms-excel.person+xml\"/></Types>");
    parts.insert("[Content_Types].xml".into(), types.into_bytes());
    let borrowed: Vec<_> = parts
        .iter()
        .map(|(name, bytes)| (name.as_str(), std::str::from_utf8(bytes).unwrap()))
        .collect();
    package(&borrowed)
}

pub fn assert_cost_thread_moved(workbook: &Workbook, threads: u32) {
    const T_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
    assert_cost_note_moved(workbook, threads);
    let bytes = member(workbook, "xl/threadedComments/threadedComment1.xml");
    let doc = yggdryl::xml::from_bytes(bytes.as_bytes()).unwrap();
    let root = yggdryl::xml::Element::root(&doc).unwrap();
    let messages = root.children_in(Some(T_NS), "threadedComment");
    assert_eq!(messages.len(), 2 * threads as usize);
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.attribute_in(None, "ref") == Some("F6"))
            .count(),
        1
    );
    assert!(
        !messages
            .iter()
            .any(|message| message.attribute_in(None, "ref") == Some("B2"))
    );
    for message in messages
        .iter()
        .filter(|message| message.attribute_in(None, "parentId").is_some())
    {
        assert!(message.attribute_in(None, "ref").is_none());
        let parent = message.attribute_in(None, "parentId");
        assert!(
            messages
                .iter()
                .any(|root| root.attribute_in(None, "id") == parent)
        );
    }
}

/// A parsed-name cost corpus: only one relevant local name; other registry
/// entries are global or belong to an unrelated third worksheet. Formula
/// spelling deliberately differs in case from its declaration.
pub fn scoped_name_cost_package(
    cells: u32,
    name: &str,
    occurrences: usize,
    unrelated: usize,
    shadow: bool,
) -> Vec<u8> {
    let mut names = String::from("<definedNames>");
    let scope = usize::from(shadow);
    names.push_str(&format!(
        "<definedName name=\"{name}\" localSheetId=\"{scope}\">11</definedName>"
    ));
    if shadow {
        names.push_str(&format!("<definedName name=\"{name}\">33</definedName>"));
    }
    for index in 0..unrelated {
        names.push_str(&format!(
            "<definedName name=\"IgnoredGlobal_{index}\">1</definedName><definedName name=\"IgnoredLocal_{index}\" localSheetId=\"2\">2</definedName>"
        ));
    }
    names.push_str("</definedNames>");
    let xml = workbook(&["Data", "Other", "Third"], false)
        .replace("</workbook>", &format!("{names}</workbook>"));
    let bytes = package(&[
        ("[Content_Types].xml", &content_types(3, false, false)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &xml),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(3, false, false),
        ),
        ("xl/worksheets/sheet1.xml", &worksheet("")),
        ("xl/worksheets/sheet2.xml", &worksheet("")),
        ("xl/worksheets/sheet3.xml", &worksheet("")),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    let token = name.to_ascii_lowercase();
    let formula = Formula::from_file(
        &vec![token.as_str(); occurrences].join("+"),
        CellRef::new(2, 2),
    );
    let source = opened.sheet_mut("Data").unwrap();
    for row in 0..cells {
        source
            .insert_cell(
                Cell::from_scalar(CellRef::new(row + 2, 2), 0.into(), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(formula.clone()),
            )
            .unwrap();
    }
    opened.into_bytes().unwrap()
}

/// Two registered tables in one worksheet, plus an empty second worksheet.
/// Raw metadata remains unparsed so the media observer's streaming path is used.
pub fn named_table_parts() -> Vec<(&'static str, String)> {
    let mut types = content_types(2, false, false);
    types = types.replace("</Types>", "<Override PartName=\"/xl/tables/table1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/><Override PartName=\"/xl/tables/table2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/></Types>");
    let data = "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>id</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>name</t></is></c><c r=\"D1\" t=\"inlineStr\"><is><t>year</t></is></c><c r=\"E1\" t=\"inlineStr\"><is><t>qty</t></is></c></row><row r=\"2\"><c r=\"A2\"><v>1</v></c><c r=\"B2\" t=\"inlineStr\"><is><t>one</t></is></c><c r=\"D2\"><v>2024</v></c><c r=\"E2\"><v>3</v></c></row><row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\" t=\"inlineStr\"><is><t>two</t></is></c><c r=\"D3\"><v>2025</v></c><c r=\"E3\"><v>4</v></c></row><row r=\"4\"><c r=\"D4\" t=\"inlineStr\"><is><t>Total</t></is></c><c r=\"E4\"><v>7</v></c></row>";
    vec![
        ("[Content_Types].xml", types),
        ("_rels/.rels", root_relationships()),
        ("xl/workbook.xml", workbook(&["Data", "Other"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(2, false, false),
        ),
        (
            "xl/worksheets/sheet1.xml",
            sheet(
                data,
                "<tableParts count=\"2\"><tablePart r:id=\"rIdT1\"/><tablePart r:id=\"rIdT2\"/></tableParts>",
            ),
        ),
        ("xl/worksheets/sheet2.xml", worksheet("")),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            format!(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdT1\" Type=\"{R_NS}/table\" Target=\"../tables/table1.xml\"/><Relationship Id=\"rIdT2\" Type=\"{R_NS}/table\" Target=\"../tables/table2.xml\"/></Relationships>"
            ),
        ),
        (
            "xl/tables/table1.xml",
            format!(
                "<table xmlns=\"{NS}\" id=\"1\" name=\"Names\" displayName=\"Names\" ref=\"A1:B3\"><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"id\"/><tableColumn id=\"2\" name=\"name\"/></tableColumns></table>"
            ),
        ),
        (
            "xl/tables/table2.xml",
            format!(
                "<table xmlns=\"{NS}\" id=\"2\" name=\"Quantities\" displayName=\"Quantities\" ref=\"D1:E4\" totalsRowCount=\"1\"><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"year\"/><tableColumn id=\"2\" name=\"qty\" totalsRowFunction=\"sum\"/></tableColumns></table>"
            ),
        ),
    ]
}

/// Serialize raw named-table facts, allowing each test to alter one fact.
pub fn named_table_package(parts: &[(&str, String)]) -> Vec<u8> {
    package(
        &parts
            .iter()
            .map(|(name, text)| (*name, text.as_str()))
            .collect::<Vec<_>>(),
    )
}

/// The selected totals table plus an arbitrarily long outside tail. The
/// extra rows remain raw through its streamed body-height overlay.
pub fn named_totals_cost_package(
    unrelated_rows: u32,
    unrelated_cells: u32,
    local_namespace: bool,
    old_row_cells: u32,
) -> Vec<u8> {
    let mut parts = named_table_parts();
    let worksheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *worksheet = worksheet.replace(
        "<c r=\"E4\"><v>7</v></c>",
        "<c r=\"E4\"><f>SUBTOTAL(109,[qty])</f><v>7</v></c>",
    );
    let mut before = String::new();
    let mut after = String::new();
    for index in 0..old_row_cells {
        let column = if index < 3 { index } else { index + 2 };
        let at = CellRef::new(3, column);
        let cell = format!("<c r=\"{at}\"><v>{index}</v></c>");
        if index < 3 {
            before.push_str(&cell)
        } else {
            after.push_str(&cell)
        }
    }
    *worksheet = worksheet
        .replace(
            "<row r=\"4\"><c r=\"D4\"",
            &format!("<row r=\"4\">{before}<c r=\"D4\""),
        )
        .replace("<v>7</v></c></row>", &format!("<v>7</v></c>{after}</row>"));
    let mut extra = String::from("<row r=\"5\"><c r=\"G5\"><v>0</v></c></row>");
    for row in 6..6 + unrelated_rows {
        if local_namespace {
            extra.push_str(&format!("<row r=\"{row}\" xmlns:x=\"urn:other\">"));
        } else {
            extra.push_str(&format!("<row r=\"{row}\">"));
        }
        for column in 0..unrelated_cells {
            let at = CellRef::new(row - 1, 7 + column);
            extra.push_str(&format!("<c r=\"{at}\"><v>{row}</v></c>"));
        }
        extra.push_str("</row>");
    }
    *worksheet = worksheet.replace("</sheetData>", &format!("{extra}</sheetData>"));
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table2.xml")
        .unwrap()
        .1;
    *table = table.replace(
        "totalsRowCount=\"1\"><tableColumns",
        "totalsRowCount=\"1\"><autoFilter ref=\"D1:E3\"/><tableColumns",
    );
    named_table_package(&parts)
}

/// A selected two-column table beside another table and unrelated long cells.
pub fn named_table_cost_package(table_rows: u32, unrelated_rows: u32) -> Vec<u8> {
    assert!(table_rows >= 2);
    let mut parts = named_table_parts();
    let first = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *first = first.replace("ref=\"A1:B3\"", &format!("ref=\"A1:B{}\"", table_rows + 1));
    let mut rows = String::new();
    for row in 1..=table_rows.saturating_add(1).max(4) {
        rows.push_str(&format!("<row r=\"{row}\">"));
        if row == 1 {
            rows.push_str("<c r=\"A1\" t=\"inlineStr\"><is><t>id</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>name</t></is></c><c r=\"D1\" t=\"inlineStr\"><is><t>year</t></is></c><c r=\"E1\" t=\"inlineStr\"><is><t>qty</t></is></c>");
        } else {
            if row <= table_rows + 1 {
                rows.push_str(&format!("<c r=\"A{row}\"><v>{}</v></c><c r=\"B{row}\" t=\"inlineStr\"><is><t>selected-{row}</t></is></c>", row - 1));
            }
            match row {
                2 | 3 => rows.push_str(&format!(
                    "<c r=\"D{row}\"><v>{}</v></c><c r=\"E{row}\"><v>{row}</v></c>",
                    2022 + row
                )),
                4 => rows.push_str(
                    "<c r=\"D4\" t=\"inlineStr\"><is><t>Total</t></is></c><c r=\"E4\"><v>5</v></c>",
                ),
                _ => {}
            }
        }
        rows.push_str("</row>");
    }
    // Long inline text forces an allocation if the metadata observer builds
    // RawCell.content, making that slope measurable independently of output.
    for extra in 0..unrelated_rows {
        let row = table_rows.saturating_add(2).max(5) + extra;
        rows.push_str(&format!("<row r=\"{row}\"><c r=\"G{row}\" t=\"inlineStr\"><is><t>unrelated-cell-value-{row:08}-outside-all-tables</t></is></c></row>"));
    }
    let worksheet_part = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *worksheet_part = sheet(
        &rows,
        "<tableParts count=\"2\"><tablePart r:id=\"rIdT1\"/><tablePart r:id=\"rIdT2\"/></tableParts>",
    );
    named_table_package(&parts)
}

/// Numeric occupied rows partitioned into `regions` vertical components.
/// One absent row separates components; only source height and returned
/// rectangle count change, with no long strings or style metadata involved.
pub fn regions_cost_package(rows: u32, regions: u32, restarts: bool) -> Vec<u8> {
    use std::fmt::Write as _;

    assert!(regions > 0 && rows % regions == 0);
    let height = rows / regions;
    assert!(height > 0 && rows + regions - 1 <= yggdryl::excel::MAX_ROWS);
    let mut data = String::new();
    for component in 0..regions {
        let first = component * (height + 1) + 1;
        for row in first..first + height {
            write!(data, "<row r=\"{row}\"><c r=\"A{row}\"><v>1</v></c></row>").unwrap();
        }
    }
    if restarts {
        return one_sheet(&data, &[], &[], &[]);
    }
    // Hold transport metadata constant across the two heights. The default
    // ZIP writer adds a restart map when decoded XML crosses 64 KiB; parsing
    // that new map costs a Vec plus an Arc, independent of discovery rows.
    let parts = [
        ("[Content_Types].xml", content_types(1, false, false)),
        ("_rels/.rels", root_relationships()),
        ("xl/workbook.xml", workbook(&["Sheet1"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", worksheet(&data)),
    ];
    let archive = ZipArchive::new(Holder::buffer(Buffer::new())).with_restart_stride(0);
    for (name, text) in &parts {
        archive
            .write_member(name, text.as_bytes())
            .expect("a solid deflated fixture member");
    }
    archive.flush().expect("the directory publishes");
    archive
        .into_handle()
        .expect("the fixture handle")
        .read_all_bytes()
        .expect("fixture bytes")
}

/// Worksheet filter criteria vary independently from unrelated resident cells.
/// The selected rectangle contains no physical cells, so only metadata moves.
pub fn worksheet_filter_cost_package(columns: u32, cells: u32) -> Vec<u8> {
    assert!((1..=16).contains(&columns));
    let rows: String = (1..=cells)
        .map(|row| format!("<row r=\"{row}\"><c r=\"AZ{row}\"><v>{row}</v></c></row>"))
        .collect();
    let criteria: String = (0..columns).map(|column|
        format!("<filterColumn colId=\"{column}\"><filters><filter val=\"choice-{column}\"/></filters></filterColumn>")).collect();
    let end = CellRef::new(4, columns);
    let filter = format!("<autoFilter ref=\"B2:{end}\">{criteria}</autoFilter>");
    let names = format!(
        "<definedNames><definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">Data!B2:{end}</definedName></definedNames>"
    );
    let book =
        workbook(&["Data", "Other"], false).replace("</sheets>", &format!("</sheets>{names}"));
    let types = content_types(2, false, false);
    let root_rels = root_relationships();
    let book_rels = workbook_relationships(2, false, false);
    let source = sheet(&rows, &filter);
    let target = worksheet("");
    package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root_rels),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &book_rels),
        ("xl/worksheets/sheet1.xml", &source),
        ("xl/worksheets/sheet2.xml", &target),
    ])
}

/// One measured native placement; the partial cases leave every filter edge
/// occupied by an unselected coordinate and land outside the source filter.
pub fn worksheet_filter_cut_case(mode: &str, columns: u32) -> (CellRange, &'static str) {
    match mode {
        "partial-interior" => (
            CellRange::new(CellRef::new(2, 1), CellRef::new(3, 1)),
            "Data",
        ),
        "partial-header" => (
            CellRange::new(CellRef::new(1, 1), CellRef::new(2, 1)),
            "Other",
        ),
        "same" | "full" | "body" => {
            let first = if mode == "body" { 2 } else { 1 };
            (
                CellRange::new(CellRef::new(first, 1), CellRef::new(4, columns)),
                if mode == "same" { "Data" } else { "Other" },
            )
        }
        _ => panic!("expected a measured worksheet filter cut mode"),
    }
}

/// Prove the measured metadata edit performed the native filter/name behavior.
pub fn assert_cost_worksheet_filter_cut(book: &Workbook, columns: u32, mode: &str) {
    let source = member(book, "xl/worksheets/sheet1.xml");
    let document = yggdryl::xml::from_bytes(source.as_bytes()).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let filter = root.child(Some(NS), "autoFilter");
    let expected_name = match mode {
        "same" => {
            let expected = format!("J10:{}", CellRef::new(12, columns + 8));
            let filter = filter.unwrap();
            assert_eq!(filter.attribute_in(None, "ref"), Some(expected.as_str()));
            assert!(filter.children_in(Some(NS), "filterColumn").is_empty());
            format!("Data!{expected}")
        }
        "full" => {
            assert!(filter.is_none());
            format!("Other!J10:{}", CellRef::new(12, columns + 8))
        }
        "body" => {
            let filter = filter.unwrap();
            let expected = format!("B2:{}", CellRef::new(4, columns));
            assert_eq!(filter.attribute_in(None, "ref"), Some(expected.as_str()));
            assert_eq!(
                filter.children_in(Some(NS), "filterColumn").len(),
                columns as usize
            );
            format!("Data!B2:{}", CellRef::new(1, columns))
        }
        "partial-interior" | "partial-header" => {
            let filter = filter.unwrap();
            let expected = format!("B2:{}", CellRef::new(4, columns));
            assert_eq!(filter.attribute_in(None, "ref"), Some(expected.as_str()));
            assert_eq!(
                filter.children_in(Some(NS), "filterColumn").len(),
                columns as usize
            );
            format!("Data!{expected}")
        }
        _ => panic!("expected a measured filter cut placement"),
    };
    let names: Vec<_> = book.defined_names().collect();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].text(), expected_name);
    assert_eq!(book.sheet_by_key(names[0].scope().unwrap()), Some("Data"));
    let target = member(book, "xl/worksheets/sheet2.xml");
    let document = yggdryl::xml::from_bytes(target.as_bytes()).unwrap();
    assert!(
        yggdryl::xml::Element::root(&document)
            .unwrap()
            .child(Some(NS), "autoFilter")
            .is_none()
    );
}

/// Sparse ranges whose covered cell count changes while their XML/part graph
/// and selected two-by-two rectangle keep the same shape.
pub fn partial_carried_cost_package(kind: &str, last_row: u32, registrations: u32) -> Vec<u8> {
    if kind == "hyperlink" {
        let mut entries = String::new();
        for index in 0..registrations {
            let cell = CellRef::new(0, index + 1).to_string();
            let column = cell.trim_end_matches('1');
            entries.push_str(&format!(
                "<hyperlink ref=\"{column}2:{column}{last_row}\" location=\"Data!A1\" tooltip=\"Link{index}\"/>"
            ));
        }
        let after = format!("<hyperlinks>{entries}</hyperlinks>");
        let source = sheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, &after);
        return sheets_book(
            &[("Data", source, &[]), ("Other", sheet("", ""), &[])],
            &[],
            &[],
        )
        .into_bytes()
        .unwrap();
    }
    let (container, child) = match kind {
        "protected" => ("protectedRanges", "protectedRange"),
        "ignored" => ("ignoredErrors", "ignoredError"),
        _ => panic!("expected a divisible formula-free registration kind"),
    };
    let mut items = String::new();
    for index in 0..registrations {
        let metadata = if kind == "protected" {
            format!("name=\"Range{index}\" password=\"ABCD\"")
        } else {
            "numberStoredAsText=\"1\"".to_owned()
        };
        items.push_str(&format!("<{child} sqref=\"B2:E{last_row}\" {metadata}/>"));
    }
    let after = format!("<{container}>{items}</{container}>");
    let source = sheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, &after);
    let target = sheet("", "");
    sheets_book(&[("Data", source, &[]), ("Other", target, &[])], &[], &[])
        .into_bytes()
        .unwrap()
}

/// The selected two-row stripe spans exactly the disjoint hyperlink anchors.
pub fn partial_carried_cost_edit(kind: &str, registrations: u32) -> (CellRange, CellRef) {
    if kind == "hyperlink" {
        (
            CellRange::new(CellRef::new(2, 1), CellRef::new(3, registrations)),
            CellRef::new(9, 19),
        )
    } else {
        ("C3:D4".parse().unwrap(), "J10".parse().unwrap())
    }
}

/// Check that the measured metadata operation performed both partitions.
pub fn assert_partial_carried_cost_split(
    workbook: &Workbook,
    kind: &str,
    registrations: u32,
    last_row: u32,
) {
    if kind == "hyperlink" {
        for (part, source) in [
            ("xl/worksheets/sheet1.xml", true),
            ("xl/worksheets/sheet2.xml", false),
        ] {
            let xml = member(workbook, part);
            let document = yggdryl::xml::from_bytes(xml.as_bytes()).unwrap();
            let root = yggdryl::xml::Element::root(&document).unwrap();
            let links = root.child(Some(NS), "hyperlinks").unwrap();
            let entries = links.children_in(Some(NS), "hyperlink");
            assert_eq!(entries.len(), registrations as usize);
            for (index, entry) in entries.into_iter().enumerate() {
                let range: CellRange = entry.attribute_in(None, "ref").unwrap().parse().unwrap();
                let column = index as u32 + 1;
                if source {
                    assert_eq!(
                        range,
                        CellRange::new(CellRef::new(1, column), CellRef::new(last_row - 1, column))
                    );
                } else {
                    assert_eq!(
                        range,
                        CellRange::new(
                            CellRef::new(9, 19 + index as u32),
                            CellRef::new(10, 19 + index as u32)
                        )
                    );
                }
                assert_eq!(entry.attribute_in(None, "location"), Some("Data!A1"));
            }
        }
        return;
    }
    let (container, child) = if kind == "protected" {
        ("protectedRanges", "protectedRange")
    } else {
        ("ignoredErrors", "ignoredError")
    };
    for (part, source) in [
        ("xl/worksheets/sheet1.xml", true),
        ("xl/worksheets/sheet2.xml", false),
    ] {
        let xml = member(workbook, part);
        let document = yggdryl::xml::from_bytes(xml.as_bytes()).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        let held = root.child(Some(NS), container).unwrap();
        let entries = held.children_in(Some(NS), child);
        assert_eq!(entries.len(), registrations as usize);
        for entry in entries {
            let ranges = entry.attribute_in(None, "sqref").unwrap();
            if source {
                assert_eq!(ranges.split_whitespace().count(), 4);
            } else {
                assert_eq!(ranges, "J10:K11");
            }
        }
    }
    assert_eq!(workbook.sheet("Data").unwrap().cell_count(), 1);
    assert_eq!(workbook.sheet("Other").unwrap().cell_count(), 0);
}

/// Equivalent CF rules share one source host or arrive as separate hosts.
/// Every rule must follow the same source-cell dependency during a cut.
pub fn carried_formula_rules_cost_package(rules: u32, together: bool) -> Vec<u8> {
    let mut carried = String::new();
    if together {
        carried.push_str("<conditionalFormatting sqref=\"H8:J10\">");
    }
    for index in 1..=rules {
        if !together {
            carried.push_str("<conditionalFormatting sqref=\"H8:J10\">");
        }
        carried.push_str(&format!(
            "<cfRule type=\"expression\" priority=\"{index}\"><formula>A1</formula></cfRule>"
        ));
        if !together {
            carried.push_str("</conditionalFormatting>");
        }
    }
    if together {
        carried.push_str("</conditionalFormatting>");
    }
    sheets_book(
        &[
            ("Data", sheet("", &carried), &[]),
            ("Other", sheet("", ""), &[]),
        ],
        &[],
        &[],
    )
    .into_bytes()
    .unwrap()
}

/// The measured work rewrites every rule, without parsing inside its counter.
pub fn assert_carried_formula_rules_followed(workbook: &Workbook, expected: u32) {
    let xml = member(workbook, "xl/worksheets/sheet1.xml");
    let document = yggdryl::xml::from_bytes(xml.as_bytes()).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let mut count = 0;
    for host in root.children_in(Some(NS), "conditionalFormatting") {
        assert_eq!(host.attribute_in(None, "sqref"), Some("H8:J10"));
        for rule in host.children_in(Some(NS), "cfRule") {
            assert_eq!(
                rule.child(Some(NS), "formula").unwrap().text(),
                Some("Other!J10")
            );
            count += 1;
        }
    }
    assert_eq!(count, expected);
}

/// Sparse numeric cells plus long, scoped lookup keys for the formula resolver.
/// Payloads are parsed before the measured read loop; unrelated name growth
/// must not add a lowercase allocation or rescan on each reference lookup.
pub fn reference_resolver_cost_package(rows: u32, names: usize) -> Vec<u8> {
    let mut data = String::new();
    for row in 1..=rows {
        data.push_str(&format!(
            r#"<row r="{row}"><c r="A{row}"><v>{row}</v></c></row>"#
        ));
    }
    let mut declared = String::from("<definedNames>");
    for index in 0..names {
        declared.push_str(&format!(r#"<definedName name="ResolverNameWithMoreThanInlineStorage_{index:04}">Data!$A:$A</definedName>"#));
    }
    declared.push_str("</definedNames></workbook>");
    let book = workbook(&["Data"], false).replace("</workbook>", &declared);
    package(&[
        ("[Content_Types].xml", content_types(1, false, false)),
        ("_rels/.rels", root_relationships()),
        ("xl/workbook.xml", book),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", worksheet(&data)),
    ])
}

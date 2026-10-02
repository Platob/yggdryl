//! `rust/src/excel/records.rs`: shared physical-header public behavior.

use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, StructType};

fn record(children: impl IntoIterator<Item = Field>) -> Field {
    DataType::from(StructType::from_fields(children).unwrap()).required_field("row")
}

fn json_rows(serie: &Serie) -> Vec<String> {
    (0..serie.len())
        .map(|index| serie.scalar(index).unwrap().into_json().unwrap())
        .collect()
}

#[test]
fn flat_owner_source_header_whitespace_matches_stream_and_model() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t> qty </t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>7</v></c></row>"
    );
    let handle = Buffer::from_bytes(crate::excel_package::one_sheet(rows, &[], &[], &[]))
        .with_media_type(MimeType::XLSX.into());
    let options = RecordOptions::from(ExcelOptions::new());
    let streamed = handle.read_arrow_field(&options).unwrap();
    assert_eq!(streamed.fields()[0].name(), "qty");
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field().unwrap().fields(), streamed.fields());
    assert_eq!(json_rows(&held), ["[7.0]"]);
}

#[test]
fn flat_owner_source_letter_fallback_matches_stream_and_model() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>id</t></is></c>",
        "<c r=\"B1\" t=\"inlineStr\"><is><t>other</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>1</v></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>x</t></is></c></row>"
    );
    let handle = Buffer::from_bytes(crate::excel_package::one_sheet(rows, &[], &[], &[]))
        .with_media_type(MimeType::XLSX.into());
    let field = record([DataType::utf8().required_field("B")]);
    let options = RecordOptions::from(ExcelOptions::new()).with_field(field.clone());
    let batch = handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let streamed = Serie::from_arrow_batch(None, &batch, Default::default()).unwrap();
    assert_eq!(json_rows(&streamed), [r#"["x"]"#]);
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::Source,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(json_rows(&held), json_rows(&streamed));
}

#[test]
fn flat_owner_none_declared_width_refuses_neighbor_column_in_both_readers() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c>",
        "<c r=\"C1\"><v>3</v></c></row>"
    );
    let handle = Buffer::from_bytes(crate::excel_package::one_sheet(rows, &[], &[], &[]))
        .with_media_type(MimeType::XLSX.into());
    let field = record([
        DataType::Float64.required_field("first"),
        DataType::Float64.required_field("second"),
        DataType::Float64.required_field("third"),
    ]);
    let options = RecordOptions::from(
        ExcelOptions::new()
            .with_range("A1:B1".parse().unwrap())
            .with_header(RecordHeader::None),
    )
    .with_field(field.clone());
    let stream_error = handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(stream_error.contains("$.third"), "{stream_error}");
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let model_error = workbook
        .sheet("Sheet1")
        .unwrap()
        .slice("A1:B1".parse().unwrap())
        .into_serie(
            Some(&field),
            RecordHeader::None,
            ArrowCastOptions::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(model_error.contains("$.third"), "{model_error}");
}

#[test]
fn flat_owner_blank_and_error_labels_use_column_letters_in_both_readers() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"e\"><v>#N/A</v></c>",
        "<c r=\"B1\" s=\"0\"/></row>",
        "<row r=\"2\"><c r=\"A2\"><v>1</v></c><c r=\"B2\"><v>2</v></c></row>"
    );
    let handle = Buffer::from_bytes(crate::excel_package::one_sheet(rows, &[], &[], &[]))
        .with_media_type(MimeType::XLSX.into());
    let options = RecordOptions::from(ExcelOptions::new());
    let streamed = handle.read_arrow_field(&options).unwrap();
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
        .unwrap();
    let names = |field: &Field| {
        field
            .fields()
            .iter()
            .map(|child| child.name().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&streamed), ["A", "B"]);
    assert_eq!(names(held.field().unwrap()), names(&streamed));
    assert_eq!(json_rows(&held), ["[1.0,2.0]"]);
}

// Append to rust/tests/excel/records.rs after RecordHeader::Rows is introduced.
// Public tests: no internals forwarder. They should fail at runtime before
// header-window/layout behavior is installed.

fn rows_two_package(rows: &str, merges: &str) -> Vec<u8> {
    use crate::excel_package as x;
    let content_types = x::content_types(1, false, false);
    let root_relationships = x::root_relationships();
    let workbook_relationships = x::workbook_relationships(1, false, false);
    let workbook = x::workbook(&["Sheet1"], false);
    let sheet = if merges.is_empty() {
        x::worksheet(rows)
    } else {
        x::worksheet(rows).replace(
            "</worksheet>",
            &format!("<mergeCells>{merges}</mergeCells></worksheet>"),
        )
    };
    x::package(&[
        ("[Content_Types].xml", &content_types),
        ("_rels/.rels", &root_relationships),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &workbook_relationships),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

fn rows_two_handle(rows: &str, merges: &str) -> yggdryl::holder::Buffer {
    use yggdryl::MimeType;
    yggdryl::holder::Buffer::from_bytes(rows_two_package(rows, merges))
        .with_media_type(MimeType::XLSX.into())
}

#[test]
fn rows_two_merged_group_and_vertical_leaf_roundtrip_both_readers() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{ArrowCastOptions, IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c>",
        "<c r=\"C1\" t=\"inlineStr\"><is><t>Person</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\"><v>3</v></c>",
        "<c r=\"C3\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/><mergeCell ref=\"C1:C2\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(
        field
            .fields()
            .iter()
            .map(|child| child.name())
            .collect::<Vec<_>>(),
        ["Sales", "Person"]
    );
    assert_eq!(
        field.fields()[0]
            .fields()
            .iter()
            .map(|child| child.name())
            .collect::<Vec<_>>(),
        ["Units", "Price"]
    );
    let batch = handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let streamed = Serie::from_arrow_batch(None, &batch, Default::default()).unwrap();
    assert_eq!(json_rows(&streamed), [r#"[[2.0,3.0],"Ann"]"#]);

    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Rows(2), ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field().unwrap(), &field);
    assert_eq!(json_rows(&held), json_rows(&streamed));
}

#[test]
fn rows_two_embedded_line_break_is_one_literal_label() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is>",
        "<t xml:space=\"preserve\">Gross&#10;Sales</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>USD</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>4</v></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(field.fields()[0].name(), "Gross\nSales");
    assert_eq!(field.fields()[0].fields()[0].name(), "USD");
}

#[test]
fn rows_two_refuses_merge_crossing_body_or_selected_edge() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Group</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Leaf</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>4</v></c></row>"
    );
    let body_crossing = rows_two_handle(rows, "<mergeCell ref=\"A1:A3\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let error = body_crossing
        .read_arrow_field(&options)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("A1:A3") && error.contains("header"),
        "{error}"
    );

    let edge_crossing = rows_two_handle(rows, "<mergeCell ref=\"A1:C1\"/>");
    let options = RecordOptions::from(
        ExcelOptions::new()
            .with_range("A1:B3".parse().unwrap())
            .with_header(RecordHeader::Rows(2)),
    );
    let error = edge_crossing
        .read_arrow_field(&options)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("A1:C1") && error.contains("range"),
        "{error}"
    );
}

#[test]
fn rows_two_refuses_duplicate_complete_path_with_both_cells() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Units</t></is></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let error = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(error.contains("A2") && error.contains("B2"), "{error}");
}

#[test]
fn rows_two_missing_physical_second_header_row_does_not_consume_data() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>4</v></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let error = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(error.contains("A2") && error.contains("header"), "{error}");
}

#[test]
fn rows_two_refuses_overlapping_merges_and_hidden_labels_with_coordinates() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c>",
        "<c r=\"B1\" t=\"inlineStr\"><is><t>Hidden</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c></row>"
    );
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let hidden = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/>");
    let error = hidden.read_arrow_field(&options).unwrap_err().to_string();
    assert!(error.contains("A1:B1") && error.contains("B1"), "{error}");

    let overlap = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/><mergeCell ref=\"B1:C1\"/>");
    // Both spans must lie inside the selection to exercise overlap itself.
    let options = RecordOptions::from(
        ExcelOptions::new()
            .with_header(RecordHeader::Rows(2))
            .with_range("A1:C2".parse().unwrap()),
    );
    let error = overlap.read_arrow_field(&options).unwrap_err().to_string();
    assert!(
        error.contains("A1:B1") && error.contains("B1:C1"),
        "{error}"
    );
}

#[test]
fn rows_many_levels_use_existing_field_depth_boundary_not_a_rows_cap() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    // A tall physical header can still describe one shallow leaf.
    let shallow = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Count</t></is></c></row>",
        "<row r=\"66\"><c r=\"A66\"><v>3</v></c></row>"
    );
    let handle = rows_two_handle(shallow, "<mergeCell ref=\"A1:A65\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(65)));
    assert_eq!(
        handle.read_arrow_field(&options).unwrap().fields()[0].name(),
        "Count"
    );

    let deep: String = (1..=64)
        .map(|row| {
            format!(
                "<row r=\"{row}\"><c r=\"A{row}\" t=\"inlineStr\"><is><t>L{row}</t></is></c></row>"
            )
        })
        .collect();
    let handle = rows_two_handle(&deep, "");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(64)));
    let error = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(
        error.contains("A64") && error.contains("Field limit"),
        "{error}"
    );
}

#[test]
fn rows_header_only_vertical_merge_extends_implicit_extent_but_not_explicit_range() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{ArrowCastOptions, IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Count</t></is></c></row>";
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:A65\"/>");
    let inferred = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(65)));
    let field = handle.read_arrow_field(&inferred).unwrap();
    assert_eq!(field.fields()[0].name(), "Count");
    assert_eq!(handle.read_arrow_reader(&inferred).unwrap().count(), 0);
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Rows(65), ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.len(), 0);
    assert_eq!(held.field(), Some(&field));

    let explicit = RecordOptions::from(
        ExcelOptions::new()
            .with_range("A1:A65".parse().unwrap())
            .with_header(RecordHeader::Rows(65)),
    );
    assert_eq!(handle.read_arrow_field(&explicit).unwrap(), field);
    let too_short = RecordOptions::from(
        ExcelOptions::new()
            .with_range("A1:A1".parse().unwrap())
            .with_header(RecordHeader::Rows(65)),
    );
    assert!(
        handle
            .read_arrow_field(&too_short)
            .unwrap_err()
            .to_string()
            .contains("$.header")
    );
}

fn rows_two_writer_serie() -> Serie {
    let sales = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("Units"),
            DataType::Date32.required_field("Day"),
        ])
        .unwrap(),
    )
    .required_field("Sales");
    let root = record([sales, DataType::utf8().required_field("Person")]);
    Serie::from_scalars(
        root,
        [Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(2.0), Scalar::date32(19_723)]),
            Scalar::from("Ann"),
        ])],
    )
    .unwrap()
}

#[test]
fn rows_two_model_writer_renders_nested_headers_merges_and_temporal_leaves() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{CellRef, ExcelOptions, Sheet, Workbook},
    };

    let serie = rows_two_writer_serie();
    let sheet = Sheet::from_serie("Sheet1", &serie, RecordHeader::Rows(2)).unwrap();
    assert_eq!(
        sheet
            .merges()
            .map(|merge| merge.to_string())
            .collect::<Vec<_>>(),
        ["A1:B1", "C1:C2"]
    );
    assert_eq!(
        sheet.scalar("A1".parse::<CellRef>().unwrap()),
        Scalar::from("Sales")
    );
    assert_eq!(
        sheet.scalar("B2".parse::<CellRef>().unwrap()),
        Scalar::from("Day")
    );
    let held = sheet
        .clone()
        .into_serie(None, RecordHeader::Rows(2), ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field(), serie.field());
    assert_eq!(json_rows(&held), json_rows(&serie));

    let mut workbook = Workbook::new();
    workbook.insert_sheet(sheet).unwrap();
    let model_bytes = workbook.into_bytes().unwrap();
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)))
        .with_field(serie.field().unwrap().clone());
    let model_handle = Buffer::from_bytes(model_bytes).with_media_type(MimeType::XLSX.into());
    let model_batch = model_handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let model_back = Serie::from_arrow_batch(None, &model_batch, Default::default()).unwrap();
    assert_eq!(json_rows(&model_back), json_rows(&serie));
}

#[test]
fn rows_two_stream_writer_matches_model_and_nested_temporal_style() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{CellRef, ExcelOptions, Workbook},
    };

    let serie = rows_two_writer_serie();
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)))
        .with_field(serie.field().unwrap().clone());
    let mut stream_handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    stream_handle
        .overwrite_arrow_batch(serie.clone().into_arrow_batch().unwrap(), &options)
        .unwrap();
    let xml = Workbook::from_bytes(stream_handle.read_all_bytes().unwrap()).unwrap();
    let written = xml.sheet("Sheet1").unwrap();
    assert_eq!(
        written
            .merges()
            .map(|merge| merge.to_string())
            .collect::<Vec<_>>(),
        ["A1:B1", "C1:C2"]
    );
    assert_eq!(
        written.scalar("B3".parse::<CellRef>().unwrap()),
        Scalar::date32(19_723)
    );
    let stream_batch = stream_handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let stream_back = Serie::from_arrow_batch(None, &stream_batch, Default::default()).unwrap();
    assert_eq!(json_rows(&stream_back), json_rows(&serie));
}

#[test]
fn rows_two_writer_refuses_ambiguous_nullable_parent_atomically() {
    use yggdryl::{RecordHeader, excel::Sheet};

    let sales = DataType::from(
        StructType::from_fields([DataType::Float64.nullable_field("Units")]).unwrap(),
    )
    .nullable_field("Sales");
    let root = record([sales]);
    let serie = Serie::from_scalars(root, [Scalar::from_sequence([Scalar::Null])]).unwrap();
    let mut sheet = Sheet::new("Sheet1").unwrap();
    sheet.set_cell("Z9".parse().unwrap(), "keep").unwrap();
    let before = sheet.clone();
    let error = sheet
        .write_serie("A1".parse().unwrap(), &serie, RecordHeader::Rows(2))
        .unwrap_err()
        .to_string();
    assert!(error.contains("Sales") && error.contains("A3"), "{error}");
    assert_eq!(sheet, before);
}

#[test]
fn rows_two_model_writer_reused_merge_clears_hidden_header_cells() {
    use yggdryl::{
        RecordHeader,
        excel::{CellRef, Sheet},
    };

    let serie = rows_two_writer_serie();
    let mut sheet = Sheet::from_serie("Sheet1", &serie, RecordHeader::Rows(2)).unwrap();
    sheet
        .set_cell("B1".parse().unwrap(), "stale under Sales merge")
        .unwrap();
    sheet
        .set_cell("C2".parse().unwrap(), "stale under Person merge")
        .unwrap();
    sheet
        .write_serie("A1".parse().unwrap(), &serie, RecordHeader::Rows(2))
        .unwrap();
    for at in ["B1", "C2"] {
        assert_eq!(
            sheet.scalar(at.parse::<CellRef>().unwrap()),
            Scalar::Null,
            "{at}"
        );
    }
    let back = sheet
        .into_serie(None, RecordHeader::Rows(2), ArrowCastOptions::default())
        .unwrap();
    assert_eq!(json_rows(&back), json_rows(&serie));
}

#[test]
fn rows_two_model_writer_refuses_out_of_grid_anchor_before_mutation() {
    use yggdryl::{
        RecordHeader,
        excel::{CellRef, MAX_COLUMNS, Sheet},
    };

    let serie = rows_two_writer_serie();
    let mut sheet = Sheet::new("Sheet1").unwrap();
    let before = sheet.clone();
    let error = sheet
        .write_serie(CellRef::new(0, MAX_COLUMNS), &serie, RecordHeader::Rows(2))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("grid") || error.contains("column"),
        "{error}"
    );
    assert_eq!(sheet, before);
}

#[test]
fn rows_two_stream_writer_refuses_nullable_parent_without_replacing_bytes() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia, MimeType};
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let sales = DataType::from(
        StructType::from_fields([DataType::Float64.nullable_field("Units")]).unwrap(),
    )
    .nullable_field("Sales");
    let serie =
        Serie::from_scalars(record([sales]), [Scalar::from_sequence([Scalar::Null])]).unwrap();
    let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)))
        .with_field(serie.field().unwrap().clone());
    let error = handle
        .overwrite_arrow_batch(serie.into_arrow_batch().unwrap(), &options)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Sales") && error.contains("A3"), "{error}");
    assert!(handle.read_all_bytes().unwrap().is_empty());
}

#[test]
fn rows_model_writer_late_bad_text_and_merge_collision_leave_sheet_unchanged() {
    use yggdryl::RecordHeader;

    let group = DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("Label"),
            DataType::Float64.required_field("Units"),
        ])
        .unwrap(),
    )
    .required_field("Sales");
    let field = record([group]);
    let row = |text: &str| {
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(text),
            Scalar::from(1.0),
        ])])
    };
    let long = "x".repeat(yggdryl::excel::MAX_CELL_TEXT + 1);
    let series = Serie::from_scalars(field.clone(), [row("fine"), row(&long)]).unwrap();
    let mut sheet = yggdryl::excel::Sheet::new("Sheet1").unwrap();
    sheet.set_cell("Z9".parse().unwrap(), "keep").unwrap();
    let before = sheet.clone();
    assert!(
        sheet
            .write_serie("A1".parse().unwrap(), &series, RecordHeader::Rows(2))
            .is_err()
    );
    assert_eq!(sheet, before);

    let valid = Serie::from_scalars(field, [row("fine")]).unwrap();
    sheet.merge("A1:C1".parse().unwrap()).unwrap();
    let before = sheet.clone();
    let error = sheet
        .write_serie("A1".parse().unwrap(), &valid, RecordHeader::Rows(2))
        .unwrap_err()
        .to_string();
    assert!(error.contains("A1:C1"), "{error}");
    assert_eq!(sheet, before);
}

#[test]
fn rows_empty_occupancy_stream_and_model_keep_trailing_physical_records() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c></row>",
        "<row r=\"3\"/>",
        "<row r=\"4\"><c r=\"A4\"><v>7</v></c></row>",
        "<row r=\"5\"/>"
    );
    let handle = rows_two_handle(rows, "");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let field = handle.read_arrow_field(&options).unwrap();
    assert!(field.fields()[0].fields()[0].is_nullable());
    let streamed = handle
        .read_arrow_reader(&options)
        .unwrap()
        .flat_map(|batch| {
            let batch = batch.unwrap();
            json_rows(&Serie::from_arrow_batch(None, &batch, Default::default()).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(streamed, ["[[null]]", "[[7.0]]", "[[null]]"]);
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = book
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Rows(2), ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field().unwrap(), &field);
    assert_eq!(json_rows(&held), streamed);
}

#[test]
fn rows_empty_occupancy_model_write_save_and_reopen_preserve_null_records() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Sheet, Workbook},
    };

    let field = record([DataType::from(
        StructType::from_fields([DataType::Float64.nullable_field("Units")]).unwrap(),
    )
    .required_field("Sales")]);
    for values in [vec![None, None], vec![None, Some(7.0), None, None]] {
        let expected = Serie::from_scalars(
            field.clone(),
            values.into_iter().map(|value| {
                Scalar::from_sequence([Scalar::from_sequence([
                    value.map_or(Scalar::Null, Scalar::from)
                ])])
            }),
        )
        .unwrap();
        let sheet = Sheet::from_serie("Sheet1", &expected, RecordHeader::Rows(2)).unwrap();
        let before_save = sheet
            .clone()
            .into_serie(
                Some(&field),
                RecordHeader::Rows(2),
                ArrowCastOptions::default(),
            )
            .unwrap();
        assert_eq!(json_rows(&before_save), json_rows(&expected), "held model");
        let mut book = Workbook::new();
        book.insert_sheet(sheet).unwrap();
        let bytes = book.into_bytes().unwrap();
        let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)))
            .with_field(field.clone());
        let handle = Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
        let streamed = handle
            .read_arrow_reader(&options)
            .unwrap()
            .flat_map(|batch| {
                let batch = batch.unwrap();
                json_rows(&Serie::from_arrow_batch(None, &batch, Default::default()).unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(streamed, json_rows(&expected), "saved stream");
        let reopened = Workbook::from_bytes(bytes).unwrap();
        let held = reopened
            .sheet("Sheet1")
            .unwrap()
            .clone()
            .into_serie(
                Some(&field),
                RecordHeader::Rows(2),
                ArrowCastOptions::default(),
            )
            .unwrap();
        assert_eq!(json_rows(&held), json_rows(&expected), "reopened model");
    }
}

#[test]
fn rows_empty_occupancy_clearing_formats_keeps_rows_without_creating_absent_ones() {
    use yggdryl::{RecordHeader, excel::Workbook};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c></row>",
        "<row r=\"3\"/>",
        "<row r=\"4\"><c r=\"A4\"><v>7</v></c></row>",
        "<row r=\"5\" hidden=\"1\" ht=\"24\" customHeight=\"1\"/>"
    );
    let book = Workbook::from_bytes(rows_two_package(rows, "")).unwrap();
    let mut sheet = book.sheet("Sheet1").unwrap().clone();
    sheet.set_row_height(4..5, None).unwrap();
    sheet.set_rows_hidden(4..5, false).unwrap();
    sheet.set_row_height(8..9, None).unwrap();
    sheet.set_rows_hidden(8..9, false).unwrap();
    assert_eq!(sheet.dimension().unwrap().to_string(), "A1:A4");
    assert_eq!(sheet.cell_count(), 3);
    let held = sheet
        .into_serie(None, RecordHeader::Rows(2), ArrowCastOptions::default())
        .unwrap();
    assert_eq!(json_rows(&held), ["[[null]]", "[[7.0]]", "[[null]]"]);
}

#[test]
fn rows_empty_occupancy_source_consumes_the_first_explicit_row_in_both_readers() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };
    let handle = rows_two_handle(
        concat!(
            "<row r=\"1\"/>",
            "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>label</t></is></c></row>",
            "<row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>payload</t></is></c></row>"
        ),
        "",
    );
    let options = RecordOptions::from(ExcelOptions::new());
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(field.fields()[0].name(), "A");
    let streamed = handle
        .read_arrow_reader(&options)
        .unwrap()
        .flat_map(|batch| {
            json_rows(&Serie::from_arrow_batch(None, &batch.unwrap(), Default::default()).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(streamed, [r#"["label"]"#, r#"["payload"]"#]);
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = book
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field().unwrap(), &field);
    assert_eq!(json_rows(&held), streamed);
}

#[test]
fn rows_empty_occupancy_without_a_declared_field_preserves_zero_column_records() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };
    let handle = rows_two_handle("<row r=\"2\"/><row r=\"4\"/>", "");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::None));
    // Row-only XML proves two records and no physical columns. A declared
    // Field is required to recover any intended null column names/types.
    let field = handle.read_arrow_field(&options).unwrap();
    assert!(field.fields().is_empty());
    let streamed = handle
        .read_arrow_reader(&options)
        .unwrap()
        .flat_map(|batch| {
            json_rows(&Serie::from_arrow_batch(None, &batch.unwrap(), Default::default()).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(streamed, ["[]", "[]"]);
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = book
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::None, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field().unwrap(), &field);
    assert_eq!(json_rows(&held), streamed);
}

#[test]
fn rows_empty_occupancy_required_missing_values_name_the_physical_cell() {
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };
    let handle = rows_two_handle("<row r=\"5\"/>", "");
    let field = record([DataType::Float64.required_field("A")]);
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::None))
        .with_field(field.clone());
    let stream_error = handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(stream_error.contains("Sheet1!A5"), "{stream_error}");
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let error = book
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::None,
            ArrowCastOptions::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("Sheet1!A5"), "{error}");
}

#[test]
fn rows_model_writer_refuses_platform_maximum_length_without_overflow() {
    use arrow_array::{NullArray, RecordBatch};
    use std::sync::Arc;
    use yggdryl::{RecordHeader, excel::Sheet};

    let field = record([DataType::Null.nullable_field("Blank")]);
    let batch = RecordBatch::try_new(
        field.clone().into_arrow_schema().unwrap(),
        vec![Arc::new(NullArray::new(usize::MAX))],
    )
    .unwrap();
    let serie = Serie::from_arrow_batch(Some(&field), &batch, Default::default()).unwrap();
    let mut sheet = Sheet::new("Sheet1").unwrap();
    let before = sheet.clone();
    let error = sheet
        .write_serie((0, 0).into(), &serie, RecordHeader::Rows(2))
        .unwrap_err()
        .to_string();
    assert!(error.contains("grid"), "{error}");
    assert_eq!(sheet, before);
}

#[test]
fn rows_empty_source_header_keeps_zero_column_records_in_model_and_stream() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOMedia, MimeType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Sheet, Workbook},
    };

    let field = record(std::iter::empty::<Field>());
    for count in [0, 2] {
        let serie = Serie::from_scalars(
            field.clone(),
            (0..count).map(|_| Scalar::from_sequence(std::iter::empty::<Scalar>())),
        )
        .unwrap();
        let options = RecordOptions::from(ExcelOptions::new()).with_field(field.clone());
        let mut streamed = Buffer::new().with_media_type(MimeType::XLSX.into());
        streamed
            .overwrite_arrow_batch(serie.clone().into_arrow_batch().unwrap(), &options)
            .unwrap();
        let stream_count = streamed
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>();
        assert_eq!(stream_count, count, "stream fixture");

        let sheet = Sheet::from_serie("Sheet1", &serie, RecordHeader::Source).unwrap();
        let raw = sheet
            .clone()
            .into_serie(
                Some(&field),
                RecordHeader::None,
                ArrowCastOptions::default(),
            )
            .unwrap();
        assert_eq!(raw.len(), count + 1, "one physical empty Source header");
        let direct = sheet
            .clone()
            .into_serie(
                Some(&field),
                RecordHeader::Source,
                ArrowCastOptions::default(),
            )
            .unwrap();
        assert_eq!(
            direct.len(),
            count,
            "model Source does not consume its first record"
        );
        let mut book = Workbook::new();
        book.insert_sheet(sheet).unwrap();
        let saved =
            Buffer::from_bytes(book.into_bytes().unwrap()).with_media_type(MimeType::XLSX.into());
        let saved_count = saved
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>();
        assert_eq!(saved_count, count, "saved model");
    }
}

#[test]
fn infer_one_text_header_over_typed_body_without_losing_the_first_record() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Qty</t></is></c>",
        "<c r=\"B1\" t=\"inlineStr\"><is><t>Price&#10;USD</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><v>3</v></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>4</v></c><c r=\"B3\"><v>5</v></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["Qty", "Price\nUSD"]
    );
    let batches = handle.read_arrow_reader(&options).unwrap();
    let actual = batches
        .map(|batch| {
            let batch = batch.unwrap();
            Serie::from_arrow_batch(Some(&field), &batch, Default::default()).unwrap()
        })
        .flat_map(|serie| json_rows(&serie))
        .collect::<Vec<_>>();
    assert_eq!(actual, ["[2.0,3.0]", "[4.0,5.0]"]);
}

#[test]
fn infer_declared_nontrivial_merge_proves_two_header_levels() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c>",
        "<c r=\"C1\" t=\"inlineStr\"><is><t>Person</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\"><v>3</v></c>",
        "<c r=\"C3\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/><mergeCell ref=\"C1:C2\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["Sales", "Person"]
    );
    assert_eq!(
        field.fields()[0]
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["Units", "Price"]
    );
    let batch = handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let serie = Serie::from_arrow_batch(Some(&field), &batch, Default::default()).unwrap();
    assert_eq!(json_rows(&serie), [r#"[[2.0,3.0],"Ann"]"#]);
}

#[test]
fn infer_nested_group_merges_prove_three_header_levels() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Portfolio</t></is></c>",
        "<c r=\"E1\" t=\"inlineStr\"><is><t>Owner</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Revenue</t></is></c>",
        "<c r=\"C2\" t=\"inlineStr\"><is><t>Cost</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>North</t></is></c>",
        "<c r=\"B3\" t=\"inlineStr\"><is><t>South</t></is></c>",
        "<c r=\"C3\" t=\"inlineStr\"><is><t>North</t></is></c>",
        "<c r=\"D3\" t=\"inlineStr\"><is><t>South</t></is></c></row>",
        "<row r=\"4\"><c r=\"A4\"><v>1</v></c><c r=\"B4\"><v>2</v></c>",
        "<c r=\"C4\"><v>3</v></c><c r=\"D4\"><v>4</v></c>",
        "<c r=\"E4\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>"
    );
    let merges = concat!(
        "<mergeCell ref=\"A1:D1\"/><mergeCell ref=\"E1:E3\"/>",
        "<mergeCell ref=\"A2:B2\"/><mergeCell ref=\"C2:D2\"/>"
    );
    let handle = rows_two_handle(rows, merges);
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(field.fields()[0].name(), "Portfolio");
    assert_eq!(
        field.fields()[0]
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["Revenue", "Cost"]
    );
    assert_eq!(
        field.fields()[0].fields()[0]
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["North", "South"]
    );
    let batch = handle
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let serie = Serie::from_arrow_batch(Some(&field), &batch, Default::default()).unwrap();
    assert_eq!(json_rows(&serie), [r#"[[[1.0,2.0],[3.0,4.0]],"Ann"]"#]);
}

#[test]
fn infer_tall_vertical_merges_do_not_impose_a_physical_header_cap() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c>",
        "<c r=\"C1\" t=\"inlineStr\"><is><t>Person</t></is></c></row>",
        "<row r=\"65\"><c r=\"A65\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B65\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
        "<row r=\"66\"><c r=\"A66\"><v>2</v></c><c r=\"B66\"><v>3</v></c>",
        "<c r=\"C66\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>"
    );
    let handle = rows_two_handle(
        rows,
        "<mergeCell ref=\"A1:B64\"/><mergeCell ref=\"C1:C65\"/>",
    );
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["Sales", "Person"]
    );
    assert_eq!(
        field.fields()[0]
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["Units", "Price"]
    );
    let count: usize = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(count, 1);
}

#[test]
fn infer_numeric_merged_data_is_not_positive_header_evidence() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"C1\"><v>2</v></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>3</v></c><c r=\"B2\"><v>4</v></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>5</v></c><c r=\"B3\"><v>6</v></c>",
        "<c r=\"C3\"><v>7</v></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/><mergeCell ref=\"C1:C2\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let error = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(
        error.contains("A1") && error.contains("text label"),
        "{error}"
    );
}

#[test]
fn infer_all_string_merged_rows_still_need_explicit_header_depth() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Group</t></is></c>",
        "<c r=\"C1\" t=\"inlineStr\"><is><t>Other</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Left</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Right</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>a</t></is></c>",
        "<c r=\"B3\" t=\"inlineStr\"><is><t>b</t></is></c>",
        "<c r=\"C3\" t=\"inlineStr\"><is><t>c</t></is></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/><mergeCell ref=\"C1:C2\"/>");
    let inferred = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let error = handle.read_arrow_field(&inferred).unwrap_err().to_string();
    assert!(
        error.contains("ambiguous") && error.contains("Rows"),
        "{error}"
    );
    let explicit = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    assert_eq!(
        handle.read_arrow_field(&explicit).unwrap().fields()[0].name(),
        "Group"
    );
}

#[test]
fn infer_all_string_first_row_is_ambiguous_and_explicit_none_keeps_it() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Name</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>Bob</t></is></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let inferred = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let error = handle.read_arrow_field(&inferred).unwrap_err().to_string();
    assert!(
        error.contains("A1") && error.contains("ambiguous"),
        "{error}"
    );
    let explicit = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::None));
    let batch = handle
        .read_arrow_reader(&explicit)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(batch.num_rows(), 3);
}

#[test]
fn infer_full_width_merge_refuses_title_versus_parent_header() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Report</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\"><v>3</v></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:B1\"/>");
    let inferred = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let error = handle.read_arrow_field(&inferred).unwrap_err().to_string();
    assert!(
        error.contains("A1:B1") && error.contains("ambiguous"),
        "{error}"
    );
    let explicit = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    assert_eq!(
        handle.read_arrow_field(&explicit).unwrap().fields()[0].name(),
        "Report"
    );
}

#[test]
fn infer_named_table_uses_authoritative_columns_and_its_exact_body() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOMedia, MimeType};
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let mut parts = crate::excel_package::named_table_parts();
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *table = table.replace("name=\"id\"", "name=\"Quantity\"");
    let handle = Buffer::from_bytes(crate::excel_package::named_table_package(&parts))
        .with_media_type(MimeType::XLSX.into());
    let options = RecordOptions::from(
        ExcelOptions::new()
            .with_table("Names")
            .with_header(RecordHeader::Infer),
    );
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["Quantity", "name"]
    );
    let count: usize = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(count, 2);
}

#[test]
fn infer_whole_sheet_with_two_tables_refuses_selection_but_explicit_range_resolves() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOMedia, MimeType};
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let handle = Buffer::from_bytes(crate::excel_package::named_table_package(
        &crate::excel_package::named_table_parts(),
    ))
    .with_media_type(MimeType::XLSX.into());
    let whole = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let error = handle.read_arrow_field(&whole).unwrap_err().to_string();
    assert!(
        error.contains("Names") && error.contains("Quantities"),
        "{error}"
    );
    let chosen = RecordOptions::from(
        ExcelOptions::new()
            .with_range("A1:B3".parse().unwrap())
            .with_header(RecordHeader::Infer),
    );
    let field = handle.read_arrow_field(&chosen).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["id", "name"]
    );
    let count: usize = handle
        .read_arrow_reader(&chosen)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(count, 2);
}

// Append to rust/tests/excel/records.rs after Infer's public enum arm exists.

#[test]
fn infer_flat_header_field_and_rows_match_held_sheet() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{ArrowCastOptions, IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Qty</t></is></c>",
        "<c r=\"B1\" t=\"inlineStr\"><is><t>Price&#10;USD</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><v>3</v></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>4</v></c><c r=\"B3\"><v>5</v></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let source = handle.read_arrow_field(&options).unwrap();
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Infer, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field().unwrap().fields(), source.fields());
    assert_eq!(json_rows(&held), ["[2.0,3.0]", "[4.0,5.0]"]);
}

#[test]
fn infer_three_level_merge_field_and_rows_match_held_sheet() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{ArrowCastOptions, IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Portfolio</t></is></c>",
        "<c r=\"E1\" t=\"inlineStr\"><is><t>Owner</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Revenue</t></is></c>",
        "<c r=\"C2\" t=\"inlineStr\"><is><t>Cost</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>North</t></is></c>",
        "<c r=\"B3\" t=\"inlineStr\"><is><t>South</t></is></c>",
        "<c r=\"C3\" t=\"inlineStr\"><is><t>North</t></is></c>",
        "<c r=\"D3\" t=\"inlineStr\"><is><t>South</t></is></c></row>",
        "<row r=\"4\"><c r=\"A4\"><v>1</v></c><c r=\"B4\"><v>2</v></c>",
        "<c r=\"C4\"><v>3</v></c><c r=\"D4\"><v>4</v></c>",
        "<c r=\"E4\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>"
    );
    let merges = concat!(
        "<mergeCell ref=\"A1:D1\"/><mergeCell ref=\"E1:E3\"/>",
        "<mergeCell ref=\"A2:B2\"/><mergeCell ref=\"C2:D2\"/>"
    );
    let handle = rows_two_handle(rows, merges);
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let source = handle.read_arrow_field(&options).unwrap();
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Infer, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(held.field().unwrap().fields(), source.fields());
    assert_eq!(json_rows(&held), [r#"[[[1.0,2.0],[3.0,4.0]],"Ann"]"#]);
}

#[test]
fn infer_all_string_held_rows_remain_ambiguous() {
    use yggdryl::{ArrowCastOptions, IOBase};
    use yggdryl::{RecordHeader, excel::Workbook};
    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Name</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let error = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Infer, ArrowCastOptions::default())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("ambiguous") && error.contains("A1"),
        "{error}"
    );
}

// Public controls appended to rust/tests/excel/records.rs after the Infer API.

#[test]
fn infer_declared_numeric_uses_header_over_heterogeneous_wire_body() {
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Qty</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>1</v></c></row>",
        "<row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>2</t></is></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let field = record([DataType::Int64.required_field("Qty")]);
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer))
        .with_field(field.clone());
    let actual = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| {
            let batch = batch.unwrap();
            Serie::from_arrow_batch(Some(&field), &batch, Default::default()).unwrap()
        })
        .flat_map(|serie| json_rows(&serie))
        .collect::<Vec<_>>();
    assert_eq!(actual, ["[1]", "[2]"]);
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::Infer,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(json_rows(&held), actual);
}

#[test]
fn infer_declared_castable_text_first_row_requires_explicit_choice() {
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>1</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>2</v></c></row>"
    );
    let handle = rows_two_handle(rows, "");
    let field = record([DataType::Int64.required_field("value")]);
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer))
        .with_field(field.clone());
    // This name cannot bind to the text label "1", so only the positional
    // data reading remains viable and keeps both records.
    let actual = handle
        .read_arrow_reader(&options)
        .unwrap()
        .flat_map(|batch| {
            json_rows(
                &Serie::from_arrow_batch(Some(&field), &batch.unwrap(), Default::default())
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, ["[1]", "[2]"]);
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let held = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::Infer,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(json_rows(&held), actual);
    let field = record([DataType::Int64.required_field("1")]);
    let options = options.with_field(field.clone());
    let error = match handle.read_arrow_reader(&options) {
        Ok(_) => panic!("castable first row needs an explicit header choice"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("ambiguous") && error.contains("A1"),
        "{error}"
    );
    let error = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::Infer,
            ArrowCastOptions::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("ambiguous") && error.contains("A1"),
        "{error}"
    );
}

// Public column-size controls appended to rust/tests/excel/records.rs.

#[test]
fn infer_flat_column_size_follows_the_resolved_header() {
    use yggdryl::IOMedia;
    use yggdryl::{
        RecordHeader,
        excel::{Excel, ExcelOptions},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Qty</t></is></c>",
        "<c r=\"B1\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\"><v>1</v></c><c r=\"B2\"><v>2</v></c></row>"
    );
    let media = Excel::new(rows_two_handle(rows, ""))
        .with_options(ExcelOptions::new().with_header(RecordHeader::Infer));
    assert_eq!(media.column_size().unwrap(), 2);
    assert_eq!(media.row_size().unwrap(), 1);
}

#[test]
fn infer_merged_column_size_counts_leaf_columns() {
    use yggdryl::IOMedia;
    use yggdryl::{
        RecordHeader,
        excel::{Excel, ExcelOptions},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c>",
        "<c r=\"C1\" t=\"inlineStr\"><is><t>Person</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\"><v>3</v></c>",
        "<c r=\"C3\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>"
    );
    let media = Excel::new(rows_two_handle(
        rows,
        "<mergeCell ref=\"A1:B1\"/><mergeCell ref=\"C1:C2\"/>",
    ))
    .with_options(ExcelOptions::new().with_header(RecordHeader::Infer));
    assert_eq!(media.column_size().unwrap(), 3);
    assert_eq!(media.row_size().unwrap(), 1);
}

#[test]
fn infer_full_width_title_ignores_styled_blank_extent() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Report</t></is></c>",
        "<c r=\"D1\" s=\"0\"/></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Qty</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c>",
        "<c r=\"C2\" t=\"inlineStr\"><is><t>Flag</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>1</v></c><c r=\"B3\"><v>2</v></c>",
        "<c r=\"C3\"><v>3</v></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"A1:C1\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let error = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(
        error.contains("A1") && error.contains("full-width"),
        "{error}"
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let error = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Infer, ArrowCastOptions::default())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("A1") && error.contains("full-width"),
        "{error}"
    );
}

#[test]
fn infer_merge_touching_only_a_region_bounding_hole_still_refuses_missing_paths() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    // The A-column component bends across B:E only at row 5. Its bounding
    // rectangle touches D1:E1 even though no occupied cell does. The shared
    // No occupied cell in that component touches the D1:E1 merge.
    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Main</t></is></c>",
        "<c r=\"D1\" t=\"inlineStr\"><is><t>Other</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Value</t></is></c>",
        "<c r=\"D2\" t=\"inlineStr\"><is><t>Left</t></is></c>",
        "<c r=\"E2\" t=\"inlineStr\"><is><t>Right</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>1</v></c><c r=\"D3\"><v>2</v></c>",
        "<c r=\"E3\"><v>3</v></c></row>",
        "<row r=\"4\"><c r=\"A4\"><v>4</v></c></row>",
        "<row r=\"5\"><c r=\"A5\"><v>5</v></c><c r=\"B5\"><v>6</v></c>",
        "<c r=\"C5\"><v>7</v></c><c r=\"D5\"><v>8</v></c>",
        "<c r=\"E5\"><v>9</v></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"D1:E1\"/>");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let error = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(
        error.contains("$.selection") && error.contains("ambiguous"),
        "{error}"
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let error = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(None, RecordHeader::Infer, ArrowCastOptions::default())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("$.selection") && error.contains("ambiguous"),
        "{error}"
    );
}

#[test]
fn infer_declared_field_cannot_merge_regions_through_a_bounding_hole() {
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let rows = concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Main</t></is></c>",
        "<c r=\"D1\" t=\"inlineStr\"><is><t>Other</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Value</t></is></c>",
        "<c r=\"D2\" t=\"inlineStr\"><is><t>Left</t></is></c>",
        "<c r=\"E2\" t=\"inlineStr\"><is><t>Right</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>1</v></c><c r=\"D3\"><v>2</v></c>",
        "<c r=\"E3\"><v>3</v></c></row>",
        "<row r=\"4\"><c r=\"A4\"><v>4</v></c></row>",
        "<row r=\"5\"><c r=\"A5\"><v>5</v></c><c r=\"B5\"><v>6</v></c>",
        "<c r=\"C5\"><v>7</v></c><c r=\"D5\"><v>8</v></c>",
        "<c r=\"E5\"><v>9</v></c></row>"
    );
    let handle = rows_two_handle(rows, "<mergeCell ref=\"D1:E1\"/>");
    let main = DataType::from(
        StructType::from_fields([DataType::Float64.nullable_field("Value")]).unwrap(),
    )
    .required_field("Main");
    let other = DataType::from(
        StructType::from_fields([
            DataType::Float64.nullable_field("Left"),
            DataType::Float64.nullable_field("Right"),
        ])
        .unwrap(),
    )
    .required_field("Other");
    let field = record([main, other]);
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer))
        .with_field(field.clone());
    let error = match handle.read_arrow_reader(&options) {
        Ok(_) => panic!("declared field must not join distinct regions by bounding rectangles"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("$.selection") && error.contains("ambiguous"),
        "{error}"
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let error = workbook
        .sheet("Sheet1")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::Infer,
            ArrowCastOptions::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("$.selection") && error.contains("ambiguous"),
        "{error}"
    );
}

#[test]
fn infer_empty_styled_sheet_reports_no_header_evidence() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    for rows in [
        "<row r=\"1\"><c r=\"A1\" s=\"0\"/></row>",
        "<row r=\"1\"><c r=\"A1\"><f>1+1</f></c></row>",
    ] {
        let handle = rows_two_handle(rows, "");
        let error = handle.read_arrow_field(&options).unwrap_err().to_string();
        assert!(
            error.contains("$.header")
                && error.contains("selected cell values or labels")
                && error.contains("got no evidence")
                && !error.contains("yet"),
            "{error}"
        );
        let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
        let error = workbook
            .sheet("Sheet1")
            .unwrap()
            .clone()
            .into_serie(None, RecordHeader::Infer, ArrowCastOptions::default())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("$.header")
                && error.contains("selected cell values or labels")
                && error.contains("got no evidence")
                && !error.contains("yet"),
            "{error}"
        );
    }
}

fn inferred_null_tail_source() -> yggdryl::holder::Buffer {
    rows_two_handle(
        concat!(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c>",
            "<c r=\"C1\" t=\"inlineStr\"><is><t>Person</t></is></c></row>",
            "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
            "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
            "<row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\"><v>3</v></c>",
            "<c r=\"C3\" t=\"inlineStr\"><is><t>Ann</t></is></c></row>",
            "<row r=\"4\"/>"
        ),
        "<mergeCell ref=\"A1:B1\"/><mergeCell ref=\"C1:C2\"/>",
    )
}

#[test]
fn infer_merged_trailing_empty_row_preserves_streamed_record() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let handle = inferred_null_tail_source();
    let explicit = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let inferred = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
    let count = |options| {
        handle
            .read_arrow_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>()
    };
    assert_eq!(count(&explicit), 2);
    assert_eq!(
        count(&inferred),
        2,
        "Infer dropped a physical all-null body row"
    );
    assert_eq!(
        handle.read_arrow_field(&inferred).unwrap(),
        handle.read_arrow_field(&explicit).unwrap()
    );
}

#[test]
fn infer_merged_trailing_empty_row_preserves_held_record() {
    use yggdryl::IOBase;
    use yggdryl::{RecordHeader, excel::Workbook};

    let handle = inferred_null_tail_source();
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap().clone();
    let explicit = sheet
        .clone()
        .into_serie(None, RecordHeader::Rows(2), ArrowCastOptions::default())
        .unwrap();
    let inferred = sheet
        .into_serie(None, RecordHeader::Infer, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(explicit.len(), 2);
    assert_eq!(
        inferred.len(),
        2,
        "Infer dropped a physical all-null body row"
    );
    assert_eq!(inferred.field(), explicit.field());
    assert_eq!(json_rows(&inferred), json_rows(&explicit));
}

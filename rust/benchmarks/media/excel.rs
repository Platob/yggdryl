//! Office Open XML workbooks: the records a `.xlsx` handle writes and reads
//! back, and the random-access model beside them.
//!
//! The fixture is a small market row - an identifier, a nullable symbol, a
//! price, a flag and a date - so the numbers describe a sheet a caller
//! exports, not a synthetic best case. Every write renders the part as it is
//! read, every read streams the part, and the workbook doors parse the sheet
//! once into cells. The editing rows time what the workbook service does
//! per request: a format rendering a value, a style patched over a range,
//! a typed entry, and rows opened above a sheet of formulas - every
//! reference the workbook states moved with them, a whole table changing
//! worksheets, and an executed Batch rolled back after a later refusal.

#[path = "../../tests/support/excel_package.rs"]
mod excel_package;

use criterion::{BatchSize, Criterion, Throughput};
use std::hint::black_box;
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, Field, IOBase, IOMedia, MimeType, Scalar, Serie, StructType};
use yggdryl::{
    RecordHeader,
    excel::{
        Cell, CellRange, CellRef, Clock, DateSystem, Edit, FormatCode, Formula, Landing, Paste,
        Sheet, StylePatch, Workbook,
    },
};

use crate::bench_profile::corpus;

/// The record field every benchmark row is laid out under.
fn field() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
            DataType::Date32.required_field("traded"),
        ])
        .expect("a valid root"),
    )
    .required_field("row")
}

/// `rows` representative rows as one record column.
fn batch(field: &Field, rows: usize) -> Serie {
    Serie::from_scalars(
        field.clone(),
        (0..rows).map(|index| {
            Scalar::from_sequence([
                Scalar::from(index as i64),
                if index % 5 == 0 {
                    Scalar::Null
                } else {
                    Scalar::from(format!("SYM{index:04}"))
                },
                Scalar::from(index as f64 * 0.25),
                Scalar::from(index % 2 == 0),
                Scalar::date32(19_723 + (index % 365) as i32),
            ])
        }),
    )
    .expect("rows under the field")
}

/// A workbook whose `Data` sheet holds `rows` rows of four cells - a
/// number, then three formulas of one shape each (`A2*2`, a running
/// `SUM($A$1:A2)`, and one naming the absolute `$E$1` above every row) -
/// and whose `Other` sheet names a tenth of them across sheets: the sheet
/// a structural edit moves every reference of.
fn formula_workbook(rows: u32) -> Workbook {
    let mut workbook = Workbook::new();
    let shapes = [
        Formula::from_file("A1*2", CellRef::new(0, 1)),
        Formula::from_file("SUM($A$1:A1)", CellRef::new(0, 2)),
        Formula::from_file("A1+$E$1", CellRef::new(0, 3)),
    ];
    let data = workbook.add_sheet("Data").expect("a sheet");
    for row in 0..rows {
        data.set_cell(CellRef::new(row, 0), f64::from(row))
            .expect("a number");
        for (column, shape) in (1..).zip(&shapes) {
            let at = CellRef::new(row, column);
            data.insert_cell(
                Cell::from_scalar(at, Scalar::from(f64::from(row)), DateSystem::Year1900)
                    .expect("a cached value")
                    .with_formula(shape.clone()),
            )
            .expect("a formula cell");
        }
    }
    let across = Formula::from_file("Data!B1", CellRef::new(0, 0));
    let other = workbook.add_sheet("Other").expect("a sheet");
    for row in 0..rows / 10 {
        let at = CellRef::new(row, 0);
        other
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(0.0), DateSystem::Year1900)
                    .expect("a cached value")
                    .with_formula(across.clone()),
            )
            .expect("a formula cell");
    }
    workbook
}

/// A populated table and a separate stationary one. Build and serialize
/// once outside timing; each iteration opens and parses its own workbook.
fn table_cut_package(rows: u32) -> Vec<u8> {
    use std::sync::Arc;
    use yggdryl::zip::ZipArchive;

    let mut workbook = Workbook::new();
    let data = workbook.add_sheet("Data").expect("a sheet");
    for (reference, text) in [
        ("A1", "Item"),
        ("B1", "Cost"),
        ("H1", "Item"),
        ("I1", "Cost"),
    ] {
        data.set_cell(reference.parse().unwrap(), text).unwrap();
    }
    let shape = Formula::from_file("A2*2", CellRef::new(1, 1));
    for row in 1..=rows {
        data.set_cell(CellRef::new(row, 0), f64::from(row)).unwrap();
        data.insert_cell(
            Cell::from_scalar(
                CellRef::new(row, 1),
                (f64::from(row) * 2.0).into(),
                DateSystem::Year1900,
            )
            .unwrap()
            .with_formula(shape.clone()),
        )
        .unwrap();
    }
    for (reference, value) in [("H2", 7.0), ("I2", 14.0), ("H3", 8.0), ("I3", 16.0)] {
        data.set_cell(reference.parse().unwrap(), value).unwrap();
    }
    let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        workbook.into_bytes().unwrap(),
    ))));
    let sheet =
        String::from_utf8(archive.read_member("xl/worksheets/sheet1.xml").unwrap()).unwrap();
    archive.write_member("xl/worksheets/sheet1.xml", sheet.replace(
        "</worksheet>",
        "<tableParts count=\"2\"><tablePart r:id=\"rIdCosts\"/><tablePart r:id=\"rIdTaken\"/></tableParts></worksheet>",
    ).as_bytes()).unwrap();
    archive.write_member("xl/worksheets/_rels/sheet1.xml.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdCosts\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/table\" Target=\"../tables/table1.xml\"/><Relationship Id=\"rIdTaken\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/table\" Target=\"../tables/table2.xml\"/></Relationships>").unwrap();
    for (id, name, range, calculated) in [
        (
            1,
            "Costs",
            format!("A1:B{}", rows + 1),
            "<calculatedColumnFormula>A2*2</calculatedColumnFormula>",
        ),
        (2, "Taken", "H1:I3".to_owned(), ""),
    ] {
        let table = format!(
            "<table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"{id}\" name=\"{name}\" displayName=\"{name}\" ref=\"{range}\" totalsRowShown=\"0\"><autoFilter ref=\"{range}\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Item\"/><tableColumn id=\"2\" name=\"Cost\">{calculated}</tableColumn></tableColumns></table>"
        );
        archive
            .write_member(&format!("xl/tables/table{id}.xml"), table.as_bytes())
            .unwrap();
    }
    let types = String::from_utf8(archive.read_member("[Content_Types].xml").unwrap()).unwrap();
    let declarations: String = (1..=2).map(|id| format!("<Override PartName=\"/xl/tables/table{id}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/>")).collect();
    archive
        .write_member(
            "[Content_Types].xml",
            types
                .replace("</Types>", &format!("{declarations}</Types>"))
                .as_bytes(),
        )
        .unwrap();
    archive.flush().unwrap();
    Arc::try_unwrap(archive)
        .unwrap_or_else(|_| panic!("no retained archive readers"))
        .into_handle()
        .unwrap()
        .read_all_bytes()
        .unwrap()
}

/// A fresh in-memory `.xlsx` handle.
fn xlsx() -> Buffer {
    Buffer::new().with_media_type(MimeType::XLSX.into())
}

pub(crate) fn excel_benchmarks(criterion: &mut Criterion) {
    pivot_benchmarks(criterion);
    let rows = corpus(10_000, 64);
    let field = field();
    let serie = batch(&field, rows);
    let arrow = serie.clone().into_arrow_batch().expect("a batch");
    let declared = xlsx()
        .record_options()
        .expect("Excel options")
        .with_field(field.clone());
    let inferred = xlsx().record_options().expect("Excel options");

    // Proven once outside the timers: the rows round-trip under the field.
    let mut written = xlsx();
    written
        .overwrite_arrow_batch(arrow.clone(), &declared)
        .expect("the workbook writes");
    let bytes = written.read_all_bytes().expect("the package bytes");
    let read: usize = written
        .read_arrow_reader(&declared)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(read, rows);
    assert_eq!(
        Workbook::from_bytes(bytes.clone())
            .expect("the package opens")
            .sheet("Sheet1")
            .expect("the sheet")
            .len(),
        rows + 1
    );

    let mut group = criterion.benchmark_group("media/excel");
    group.throughput(Throughput::Elements(rows as u64));
    group.bench_function("write_records", |bencher| {
        bencher.iter(|| {
            let mut handle = xlsx();
            handle
                .overwrite_arrow_batch(black_box(arrow.clone()), &declared)
                .expect("writes");
            handle
        });
    });
    group.bench_function("read_records_declared", |bencher| {
        bencher.iter(|| {
            let handle = Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
            handle
                .read_arrow_reader(black_box(&declared))
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });
    group.bench_function("read_records_inferred", |bencher| {
        bencher.iter(|| {
            let handle = Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
            let root = handle
                .read_arrow_field(black_box(&inferred))
                .expect("a field");
            let read: usize = handle
                .read_arrow_reader(&inferred)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum();
            (root, read)
        });
    });
    group.bench_function("workbook_open_and_one_cell", |bencher| {
        let reference: CellRef = "C3".parse().expect("a reference");
        bencher.iter(|| {
            Workbook::from_bytes(black_box(bytes.clone()))
                .expect("the package opens")
                .sheet("Sheet1")
                .expect("the sheet")
                .scalar(reference)
        });
    });
    group.bench_function("sheet_into_serie", |bencher| {
        bencher.iter(|| {
            let mut workbook =
                Workbook::from_bytes(black_box(bytes.clone())).expect("the package opens");
            workbook
                .remove_sheet("Sheet1")
                .expect("the sheet parses")
                .expect("the sheet is there")
                .into_serie(Some(&field), RecordHeader::Source, Default::default())
                .expect("the rows lay out")
        });
    });
    group.bench_function("sheet_from_serie", |bencher| {
        bencher.iter(|| {
            Sheet::from_serie("Sheet1", black_box(&serie), RecordHeader::Source)
                .expect("the cells lay out")
        });
    });

    // A save writes what changed and copies every other member as it is
    // stored: a workbook only read copies its parts, one edited writes its
    // sheet from the cells and copies the rest.
    let read = Workbook::from_bytes(bytes.clone()).expect("the package opens");
    read.parse_all().expect("the sheets parse");
    group.bench_function("workbook_save_clean", |bencher| {
        bencher.iter(|| black_box(&read).into_bytes().expect("the package").len());
    });
    let mut edited = Workbook::from_bytes(bytes.clone()).expect("the package opens");
    edited
        .sheet_mut("Sheet1")
        .expect("the sheet")
        .set_cell("Z1".parse().expect("a reference"), "edited")
        .expect("the edit");
    group.bench_function("workbook_save_dirty", |bencher| {
        bencher.iter(|| black_box(&edited).into_bytes().expect("the package").len());
    });
    group.finish();

    editing_benchmarks(criterion);
    formula_parser_benchmarks(criterion);
    function_catalog_benchmarks(criterion);
    literal_array_benchmarks(criterion);
    mapped_array_benchmarks(criterion);
    constant_formula_recalculation_benchmarks(criterion);
    dependency_recalculation_benchmarks(criterion);
    range_recalculation_benchmarks(criterion);
    lazy_selector_benchmarks(criterion);
    multi_selector_benchmarks(criterion);
    geometry_benchmarks(criterion);
    indexed_reference_benchmarks(criterion);
    reference_algebra_benchmarks(criterion);
    lookup_axis_benchmarks(criterion);
    text_function_benchmarks(criterion);
    criteria_subtotal_benchmarks(criterion);
    typed_text_write_benchmarks(criterion);
    variance_exact_benchmarks(criterion);
    financial_payment_benchmarks(criterion);
    financial_npv_benchmarks(criterion);
    financial_annuity_benchmarks(criterion);
    text_value_format_benchmarks(criterion);
    text_character_benchmarks(criterion);
    text_search_benchmarks(criterion);
    text_casing_benchmarks(criterion);
    text_index_benchmarks(criterion);
    text_conversion_benchmarks(criterion);
    logical_reducer_benchmarks(criterion);
    comparison_recalculation_benchmarks(criterion);
    named_recalculation_benchmarks(criterion);
    clock_recalculation_benchmarks(criterion);
    named_table_benchmarks(criterion);
    named_table_write_scaling(criterion);
    named_totals_resize_benchmarks(criterion);
    regions_benchmarks(criterion);
    selection_benchmarks(criterion);
    rows_header_benchmarks(criterion);
    infer_header_benchmarks(criterion);
    worksheet_filter_benchmarks(criterion);
    partial_carried_benchmarks(criterion);
    carried_formula_rule_benchmarks(criterion);
    temporal_fill_benchmarks(criterion);
}

/// Date-format serials 59/60 and a sub-millisecond source, parsed before timing.
fn temporal_fill_benchmarks(criterion: &mut Criterion) {
    let data = "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>59</v></c>\
                <c r=\"B1\" s=\"1\"><v>45292.000000001</v></c></row>";
    let types = excel_package::content_types(1, false, true);
    let root = excel_package::root_relationships();
    let book = excel_package::workbook(&["Data"], false);
    let rels = excel_package::workbook_relationships(1, false, true);
    let sheet = excel_package::worksheet(data);
    let styles = excel_package::styles(&[], &[0, 14]);
    let bytes = excel_package::package(&[
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root.as_str()),
        ("xl/workbook.xml", book.as_str()),
        ("xl/_rels/workbook.xml.rels", rels.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ("xl/styles.xml", styles.as_str()),
    ]);
    let fresh = || {
        let workbook = Workbook::from_bytes(bytes.clone()).expect("the date package");
        workbook.parse_all().expect("the source date parses");
        workbook
    };
    let mut group = criterion.benchmark_group("media/excel/fill_temporal");
    for rows in [corpus(64, 16) as u32, corpus(4_096, 64) as u32] {
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(rows - 1, 0));
        let source = CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0));
        let mut proven = fresh();
        proven
            .fill("Data", source, target, yggdryl::excel::FillMode::Series)
            .expect("the daily serial series");
        assert!(
            proven
                .sheet("Data")
                .unwrap()
                .cell(CellRef::new(rows - 1, 0))
                .is_some()
        );
        group.throughput(Throughput::Elements(u64::from(rows - 1)));
        group.bench_function(format!("daily/{rows}"), |bencher| {
            bencher.iter_batched(
                &fresh,
                |mut workbook| {
                    workbook
                        .fill("Data", source, target, yggdryl::excel::FillMode::Series)
                        .expect("the daily serial series");
                    black_box(workbook)
                },
                BatchSize::LargeInput,
            );
        });
        let target = CellRange::new(CellRef::new(0, 1), CellRef::new(rows - 1, 1));
        let source = CellRange::new(CellRef::new(0, 1), CellRef::new(0, 1));
        let mut proven = fresh();
        proven
            .fill("Data", source, target, yggdryl::excel::FillMode::Copy)
            .expect("the exact serial copies");
        assert!(
            proven
                .sheet("Data")
                .unwrap()
                .cell(CellRef::new(rows - 1, 1))
                .is_some()
        );
        group.bench_function(format!("copy_submillisecond/{rows}"), |bencher| {
            bencher.iter_batched(
                &fresh,
                |mut workbook| {
                    workbook
                        .fill("Data", source, target, yggdryl::excel::FillMode::Copy)
                        .expect("the exact serial copies");
                    black_box(workbook)
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

/// The edits the workbook service applies per request.
fn editing_benchmarks(criterion: &mut Criterion) {
    let rows = corpus(25_000, 64) as u32;
    let mut group = criterion.benchmark_group("media/excel");

    // A number under an accounting format with a colour section, a date
    // under a built-in one: the display a tile renders per cell.
    let money = FormatCode::from_code("#,##0.00;[Red](#,##0.00)").expect("a format");
    let day = FormatCode::builtin(14).expect("a built-in format");
    let values: Vec<Scalar> = (0..rows)
        .map(|row| Scalar::from((f64::from(row) - f64::from(rows) / 2.0) * 1.25))
        .collect();
    assert_eq!(
        money
            .render(&Scalar::from(-1234.5), DateSystem::Year1900)
            .text,
        "(1,234.50)"
    );
    // Setup clones the already-imported null rows outside the timer; the
    // edit records one physical row per record and no cell per null field.
    let null_field = DataType::from(
        StructType::from_fields([
            DataType::Float64.nullable_field("left"),
            DataType::Float64.nullable_field("right"),
        ])
        .expect("a record"),
    )
    .required_field("record");
    let null_records = Serie::from_scalars(
        null_field,
        (0..rows).map(|_| Scalar::from_sequence([Scalar::Null, Scalar::Null])),
    )
    .expect("null records");
    let null_sheet = Sheet::from_serie("Imported", &null_records, RecordHeader::None)
        .expect("known null footprint");
    group.throughput(Throughput::Elements(u64::from(rows)));
    group.bench_function("workbook_land_null_records", |bencher| {
        bencher.iter_batched(
            || {
                let mut workbook = Workbook::new();
                workbook.add_sheet("Data").expect("a destination");
                (workbook, null_sheet.clone())
            },
            |(mut workbook, cells)| {
                workbook
                    .apply(Edit::Land {
                        destination: Landing::At {
                            sheet: "Data".into(),
                            anchor: CellRef::new(0, 0),
                        },
                        cells: Box::new(cells),
                    })
                    .expect("the null records land")
            },
            BatchSize::SmallInput,
        );
    });

    group.throughput(Throughput::Elements(u64::from(rows)));
    group.bench_function("format_render", |bencher| {
        bencher.iter(|| {
            values
                .iter()
                .map(|value| {
                    money
                        .render(black_box(value), DateSystem::Year1900)
                        .text
                        .len()
                        + day.render(value, DateSystem::Year1900).text.len()
                })
                .sum::<usize>()
        });
    });

    // Typed entries over one column, each read as Excel reads what is
    // typed: a number, a date, a percentage.
    let entries = ["1234.5", "1/2/2024", "12%"];
    let references: Vec<CellRef> = (0..rows).map(|row| CellRef::new(row, 0)).collect();
    let mut typed = Workbook::new();
    typed.add_sheet("Sheet1").expect("a sheet");
    group.bench_function("workbook_set_entry", |bencher| {
        bencher.iter(|| {
            for (at, text) in references.iter().zip(entries.iter().cycle()) {
                typed
                    .set_entry("Sheet1", *at, black_box(text))
                    .expect("an entry");
            }
        });
    });

    // Bold over every cell of the formula sheet: each distinct style the
    // cells hold is patched once, the cells cost the plan.
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    let all = [CellRange::new(
        CellRef::new(0, 0),
        CellRef::new(rows - 1, 3),
    )];
    group.throughput(Throughput::Elements(4 * u64::from(rows)));
    group.bench_function("workbook_set_style", |bencher| {
        bencher.iter_batched(
            || formula_workbook(rows),
            |mut workbook| {
                workbook
                    .set_style("Data", black_box(&all), &bold)
                    .expect("a patch");
                workbook
            },
            BatchSize::LargeInput,
        );
    });

    // Two rows opened above every row of the formula sheet: every cell
    // moves, every formula's references move with it - one rewrite per
    // shape and class - and the other sheet's references follow.
    let mut proven = formula_workbook(rows);
    proven.insert_rows("Data", 0, 2).expect("the rows open");
    let moved = proven
        .sheet("Data")
        .expect("the sheet")
        .cell(CellRef::new(rows + 1, 3))
        .expect("the last row moved");
    assert_eq!(
        moved
            .formula()
            .expect("a formula")
            .at(moved.reference())
            .to_string(),
        format!("A{}+$E$3", rows + 2)
    );
    group.bench_function("workbook_insert_rows_formulas", |bencher| {
        bencher.iter_batched(
            || formula_workbook(rows),
            |mut workbook| {
                workbook
                    .insert_rows("Data", black_box(0), 2)
                    .expect("the rows open");
                workbook
            },
            BatchSize::LargeInput,
        );
    });

    let table_bytes = table_cut_package(rows);
    let fresh = || {
        let workbook = Workbook::from_bytes(table_bytes.clone()).expect("the table package");
        workbook
            .parse_all()
            .expect("the cells parse outside timing");
        workbook
    };
    let source = CellRange::new(CellRef::new(0, 0), CellRef::new(rows, 1));
    let target = CellRef::new(0, 3);
    let mut proven = fresh();
    proven
        .paste(("Data", source), ("Data", target), Paste::All, true)
        .unwrap();
    let moved = proven.sheet("Data").unwrap();
    assert_eq!(moved.scalar(CellRef::new(rows, 3)), f64::from(rows).into());
    assert!(moved.cell(CellRef::new(rows, 0)).is_none());
    let formula = moved.cell(CellRef::new(1, 4)).unwrap();
    assert_eq!(
        formula
            .formula()
            .unwrap()
            .at(formula.reference())
            .to_string(),
        "D2*2"
    );
    let mut refused = fresh();
    assert!(matches!(
        refused.paste(
            ("Data", source),
            ("Data", CellRef::new(0, 7)),
            Paste::All,
            true
        ),
        Err(yggdryl::Error::InvalidRecord { .. })
    ));

    group.throughput(Throughput::Elements(2 * u64::from(rows)));
    group.bench_function("workbook_cut_whole_table", |bencher| {
        bencher.iter_batched(
            &fresh,
            |mut workbook| {
                workbook
                    .paste(
                        ("Data", black_box(source)),
                        ("Data", target),
                        Paste::All,
                        true,
                    )
                    .expect("the whole table moves");
                workbook
            },
            BatchSize::LargeInput,
        );
    });
    // Collision intake depends on two table extents, before visiting the
    // populated source cells; this benchmark measures one refused edit.
    group.throughput(Throughput::Elements(1));
    group.bench_function("workbook_cut_table_collision", |bencher| {
        bencher.iter_batched(
            &fresh,
            |mut workbook| {
                let error = workbook
                    .paste(
                        ("Data", black_box(source)),
                        ("Data", CellRef::new(0, 7)),
                        Paste::All,
                        true,
                    )
                    .expect_err("the distinct table refuses the cut");
                (workbook, error)
            },
            BatchSize::LargeInput,
        );
    });

    let parts = |workbook: &Workbook| {
        let archive = std::sync::Arc::new(yggdryl::zip::ZipArchive::new(Holder::buffer(
            Buffer::from_bytes(workbook.into_bytes().expect("the logical package")),
        )));
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
            .collect::<std::collections::BTreeMap<_, _>>()
    };

    // Destination setup is outside timing; the cut creates its table membership
    // and relationships while retaining the stationary table on the source.
    let across = || {
        let mut workbook = fresh();
        workbook.add_sheet("Other").expect("a fresh destination");
        workbook
    };
    let mut proven = across();
    proven
        .paste(("Data", source), ("Other", target), Paste::All, true)
        .expect("the table moves to the fresh sheet");
    assert!(
        proven
            .sheet("Data")
            .unwrap()
            .cell(CellRef::new(rows, 0))
            .is_none()
    );
    let moved = proven.sheet("Other").unwrap();
    assert_eq!(moved.scalar(CellRef::new(rows, 3)), f64::from(rows).into());
    let formula = moved.cell(CellRef::new(1, 4)).unwrap();
    assert_eq!(
        formula
            .formula()
            .unwrap()
            .at(formula.reference())
            .to_string(),
        "D2*2"
    );
    let moved_parts = parts(&proven);
    let xml = |name: &str| std::str::from_utf8(&moved_parts[name]).unwrap();
    let moved_range = format!("ref=\"D1:E{}\"", rows + 1);
    assert_eq!(
        xml("xl/tables/table1.xml")
            .matches(moved_range.as_str())
            .count(),
        2
    );
    assert!(
        xml("xl/tables/table1.xml")
            .contains("<calculatedColumnFormula>D2*2</calculatedColumnFormula>")
    );
    assert!(!xml("xl/worksheets/_rels/sheet1.xml.rels").contains("rIdCosts"));
    assert!(xml("xl/worksheets/_rels/sheet1.xml.rels").contains("rIdTaken"));
    assert!(xml("xl/worksheets/_rels/sheet2.xml.rels").contains("../tables/table1.xml"));
    group.throughput(Throughput::Elements(2 * u64::from(rows)));
    group.bench_function("workbook_cut_whole_table_cross_sheet", |bencher| {
        bencher.iter_batched(
            &across,
            |mut workbook| {
                workbook
                    .paste(
                        ("Data", black_box(source)),
                        ("Other", target),
                        Paste::All,
                        true,
                    )
                    .expect("the table changes owner");
                workbook
            },
            BatchSize::LargeInput,
        );
    });

    // The first edit visits populated cells and package referrers; the second
    // refuses, so the timed transaction includes actual inverse execution.
    let first = Edit::InsertRows {
        sheet: "Data".into(),
        at: 0,
        count: 1,
    };
    let batch = Edit::Batch(vec![
        first.clone(),
        Edit::InsertRows {
            sheet: "Missing".into(),
            at: 0,
            count: 1,
        },
    ]);
    let mut applied = fresh();
    applied.apply(first).expect("the first operation executes");
    assert_eq!(
        applied
            .sheet("Data")
            .unwrap()
            .scalar(CellRef::new(rows + 1, 0)),
        f64::from(rows).into()
    );
    let mut refused = fresh();
    let dirty = refused.is_dirty();
    let revision = refused.sheet("Data").unwrap().revision();
    let before = parts(&refused);
    refused
        .apply(batch.clone())
        .expect_err("the missing sheet refuses after an edit");
    assert_eq!(refused.is_dirty(), dirty);
    assert_eq!(refused.sheet("Data").unwrap().revision(), revision);
    assert_eq!(parts(&refused), before);
    group.throughput(Throughput::Elements(1));
    group.bench_function("workbook_failed_batch", |bencher| {
        bencher.iter_batched(
            || (fresh(), batch.clone()),
            |(mut workbook, edit)| {
                let error = workbook
                    .apply(black_box(edit))
                    .expect_err("the failed Batch restores its first edit");
                (workbook, error)
            },
            BatchSize::LargeInput,
        );
    });
    // Notes have no stored cells. Parsing the grid belongs to setup; the
    // timed cut moves the note metadata, VML and any complete reply chain.
    let note_count = corpus(1_024, 16) as u32;
    let note_source: CellRange = "B2".parse().unwrap();
    let note_target: CellRef = "F6".parse().unwrap();
    for (name, bytes, prove) in [
        (
            "workbook_cut_legacy_note",
            excel_package::note_cost_package(note_count, rows),
            excel_package::assert_cost_note_moved as fn(&Workbook, u32),
        ),
        (
            "workbook_cut_threaded_note",
            excel_package::threaded_cost_package(note_count, rows),
            excel_package::assert_cost_thread_moved as fn(&Workbook, u32),
        ),
    ] {
        let note_book = || {
            let workbook = Workbook::from_bytes(bytes.clone()).expect("the note package");
            workbook
                .parse_all()
                .expect("the grid parses outside timing");
            workbook
        };
        let mut proven = note_book();
        proven
            .paste(
                ("Data", note_source),
                ("Data", note_target),
                Paste::All,
                true,
            )
            .expect("the note moves without a stored source cell");
        prove(&proven, note_count);
        group.throughput(Throughput::Elements(1));
        group.bench_function(name, |bencher| {
            bencher.iter_batched(
                &note_book,
                |mut workbook| {
                    workbook
                        .paste(
                            ("Data", black_box(note_source)),
                            ("Data", note_target),
                            Paste::All,
                            true,
                        )
                        .expect("the note moves");
                    workbook
                },
                BatchSize::LargeInput,
            );
        });
    }

    // Registry intake is once per cut; token lookups visit only the relevant
    // local scope. Two dimensions expose a per-cell registry scan if one is
    // introduced. Existing defined-name formulas still need their own pass.
    for named_cells in [corpus(64, 4), corpus(4_096, 8)] {
        for unrelated in [corpus(64, 4), corpus(4_096, 8)] {
            let bytes = excel_package::scoped_name_cost_package(
                named_cells as u32,
                "LoNgMiXeDReferenceNameBeyondInlineCapacity",
                4,
                unrelated,
                false,
            );
            let ready = || {
                let book = Workbook::from_bytes(bytes.clone()).unwrap();
                book.parse_all().unwrap();
                book
            };
            let block = CellRange::new(CellRef::new(2, 2), CellRef::new(named_cells as u32 + 1, 2));
            let target = CellRef::new(7, 7);
            let mut proven = ready();
            proven
                .paste(("Data", block), ("Other", target), Paste::All, true)
                .unwrap();
            assert_eq!(proven.sheet("Other").unwrap().len(), named_cells);
            assert_eq!(
                proven.entry_text("Other", target).unwrap().as_deref(),
                Some(
                    "=Data!longmixedreferencenamebeyondinlinecapacity+Data!longmixedreferencenamebeyondinlinecapacity+Data!longmixedreferencenamebeyondinlinecapacity+Data!longmixedreferencenamebeyondinlinecapacity"
                )
            );
            group.throughput(Throughput::Elements(named_cells as u64));
            group.bench_function(
                format!("workbook_cut_scoped_name_{named_cells}_cells_{unrelated}_unrelated"),
                |bencher| {
                    bencher.iter_batched(
                        &ready,
                        |mut book| {
                            book.paste(
                                ("Data", black_box(block)),
                                ("Other", target),
                                Paste::All,
                                true,
                            )
                            .unwrap();
                            book
                        },
                        BatchSize::LargeInput,
                    )
                },
            );
        }
    }

    for kind in [
        "cf",
        "dv",
        "x14cf",
        "x14dv",
        "x14spark",
        "hyperlink",
        "protected",
        "ignored",
        "watch",
    ] {
        for registrations in [corpus(1, 1) as u32, corpus(256, 16) as u32] {
            for cells in [corpus(64, 4) as u32, corpus(4_096, 8) as u32] {
                let bytes = excel_package::carried_cost_package(kind, registrations, cells);
                let ready = || {
                    let workbook =
                        Workbook::from_bytes(bytes.clone()).expect("the carried package");
                    workbook
                        .parse_all()
                        .expect("the grid parses outside timing");
                    workbook
                };
                let source: CellRange = format!("B2:B{}", registrations + 1).parse().unwrap();
                let target: CellRef = "J10".parse().unwrap();
                let mut proven = ready();
                proven
                    .paste(("Data", source), ("Other", target), Paste::All, true)
                    .expect("the carried registrations move");
                let target_xml = excel_package::member(&proven, "xl/worksheets/sheet2.xml");
                assert!(
                    target_xml.contains("J10"),
                    "{kind} transfer materializes at target"
                );
                if matches!(kind, "protected" | "ignored" | "watch") {
                    excel_package::assert_cost_carried_scope_moved(&proven, kind, registrations);
                }
                group.throughput(Throughput::Elements(registrations as u64));
                group.bench_function(
                    format!(
                        "workbook_cut_carried_{kind}_{registrations}_registrations_{cells}_cells"
                    ),
                    |bencher| {
                        bencher.iter_batched(
                            &ready,
                            |mut workbook| {
                                workbook
                                    .paste(
                                        ("Data", black_box(source)),
                                        ("Other", target),
                                        Paste::All,
                                        true,
                                    )
                                    .expect("the carried registrations move");
                                workbook
                            },
                            BatchSize::LargeInput,
                        )
                    },
                );
            }
        }
    }

    group.finish();
}

/// Named-table resolution and selected record consumption.
fn named_table_benchmarks(criterion: &mut Criterion) {
    use yggdryl::excel::ExcelOptions;
    use yggdryl::media::RecordOptions;

    let mut group = criterion.benchmark_group("media/excel/named_table");
    group.sample_size(10);
    let large = corpus(256, 32) as u32;
    for table_rows in [16, large] {
        for unrelated_rows in [16, large] {
            let bytes = excel_package::named_table_cost_package(table_rows, unrelated_rows);
            let handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
            let field = DataType::from(
                StructType::from_fields([
                    DataType::Float64.required_field("id"),
                    DataType::utf8().required_field("name"),
                ])
                .expect("unique table columns"),
            )
            .required_field("row");
            let options =
                RecordOptions::from(ExcelOptions::new().with_table("Names")).with_field(field);

            let mut ids = Vec::new();
            for batch in handle.read_arrow_reader(&options).expect("a named reader") {
                let batch = batch.expect("a named batch");
                assert_eq!(batch.num_columns(), 2);
                let column = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow_array::Float64Array>()
                    .expect("declared numeric IDs");
                ids.extend(column.iter().map(|value| value.expect("an id")));
            }
            assert_eq!(ids, (1..=table_rows).map(f64::from).collect::<Vec<_>>());

            group.throughput(Throughput::Elements(u64::from(table_rows)));
            // One staged package write, selected body only; input batches and
            // the fixture are built outside each timed iteration.
            let schema = options
                .field()
                .expect("declared table field")
                .into_arrow_schema()
                .expect("Arrow schema");
            let incoming = arrow_array::RecordBatch::try_new(
                schema.clone(),
                vec![
                    std::sync::Arc::new(arrow_array::Float64Array::from_iter_values(
                        (1..=table_rows).map(f64::from),
                    )),
                    std::sync::Arc::new(arrow_array::StringArray::from_iter_values(
                        (1..=table_rows).map(|_| "written"),
                    )),
                ],
            )
            .expect("the replacement body");
            let write = ExcelOptions::new().with_table("Names");
            let source = handle.read_all_bytes().expect("the table package");
            group.bench_function(
                format!("write/{table_rows}_rows/{unrelated_rows}_other"),
                |bencher| {
                    bencher.iter(|| {
                        let mut output = Buffer::from_bytes(source.clone())
                            .with_media_type(MimeType::XLSX.into());
                        yggdryl::excel::overwrite_arrow_reader(
                            &mut output,
                            yggdryl::arrow::batch_reader(schema.clone(), [incoming.clone()]),
                            black_box(&write),
                        )
                        .expect("the named body writes");
                        black_box(output)
                    })
                },
            );

            group.bench_function(
                format!("resolve/{table_rows}_rows/{unrelated_rows}_other"),
                |bencher| {
                    bencher.iter(|| {
                        drop(black_box(
                            black_box(&handle)
                                .read_arrow_reader(black_box(&options))
                                .expect("a named reader"),
                        ));
                    })
                },
            );
            group.bench_function(
                format!("read/{table_rows}_rows/{unrelated_rows}_other"),
                |bencher| {
                    bencher.iter(|| {
                        black_box(
                            black_box(&handle)
                                .read_arrow_reader(black_box(&options))
                                .expect("a named reader")
                                .map(|batch| batch.expect("a named batch").num_rows())
                                .sum::<usize>(),
                        )
                    })
                },
            );
        }
    }
    group.finish();
}

/// Occupancy discovery, separate from named-table record selection.
fn regions_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel/regions");
    group.sample_size(10);
    for rows in [64_u32, corpus(4_096, 256) as u32] {
        for count in [1_u32, 32] {
            let handle = Buffer::from_bytes(excel_package::regions_cost_package(rows, count, true))
                .with_media_type(MimeType::XLSX.into());
            let regions = yggdryl::excel::regions(&handle, None).expect("discovered regions");
            assert_eq!(regions.len(), count as usize);
            assert_eq!(regions.first().unwrap().range.start(), CellRef::new(0, 0));
            assert_eq!(
                regions.last().unwrap().range.end(),
                CellRef::new(rows + count - 2, 0)
            );
            group.throughput(Throughput::Elements(u64::from(rows)));
            group.bench_function(format!("{rows}_rows/{count}_components"), |bencher| {
                bencher.iter(|| {
                    black_box(
                        yggdryl::excel::regions(black_box(&handle), None)
                            .expect("discovered regions"),
                    )
                })
            });
        }
    }
    group.finish();
}

fn selection_benchmarks(criterion: &mut Criterion) {
    use yggdryl::excel::ExcelSelection;

    let mut group = criterion.benchmark_group("media/excel/selection");
    for (label, json, expected) in [
        (
            "worksheet",
            r#"{"sheet":"Data","range":"B2:C4"}"#,
            ExcelSelection::Worksheet {
                sheet: Some("Data".into()),
                range: Some("B2:C4".parse().unwrap()),
            },
        ),
        (
            "table",
            r#"{"table":"Sales"}"#,
            ExcelSelection::Table {
                name: "Sales".into(),
            },
        ),
    ] {
        let input = yggdryl::from_json_scalar(json).expect("a selection object");
        assert_eq!(ExcelSelection::from_scalar(&input).unwrap(), expected);
        group.bench_function(label, |bencher| {
            bencher.iter(|| {
                black_box(ExcelSelection::from_scalar(black_box(&input)).expect("a selection"))
            })
        });
    }
    group.finish();
}

fn rows_header_benchmarks(criterion: &mut Criterion) {
    use yggdryl::excel::ExcelOptions;
    use yggdryl::media::RecordOptions;

    let mut group = criterion.benchmark_group("media/excel/rows2");
    for rows in [16, corpus(10_000, 64)] {
        let sales = DataType::from(
            StructType::from_fields([
                DataType::Float64.required_field("Units"),
                DataType::Date32.required_field("Day"),
            ])
            .expect("Sales fields"),
        )
        .required_field("Sales");
        let field = DataType::from(
            StructType::from_fields([sales, DataType::utf8().required_field("Person")])
                .expect("record fields"),
        )
        .required_field("row");
        let serie = Serie::from_scalars(
            field.clone(),
            (0..rows).map(|index| {
                Scalar::from_sequence([
                    Scalar::from_sequence([
                        Scalar::from(index as f64),
                        Scalar::date32(19_723 + (index % 365) as i32),
                    ]),
                    Scalar::from(format!("P{index:05}")),
                ])
            }),
        )
        .expect("nested rows");
        let arrow = serie.clone().into_arrow_batch().expect("one Arrow batch");
        let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)))
            .with_field(field.clone());

        // Prove geometry and selected values before timing either writer.
        let held =
            Sheet::from_serie("Sheet1", &serie, RecordHeader::Rows(2)).expect("held Rows writer");
        assert_eq!(held.merges().count(), 2);
        let held_back = held
            .clone()
            .into_serie(Some(&field), RecordHeader::Rows(2), Default::default())
            .expect("held Rows reader");
        assert_eq!(held_back.len(), rows);
        assert_eq!(held_back.scalar(0).unwrap(), serie.scalar(0).unwrap());
        assert_eq!(
            held_back.scalar(rows - 1).unwrap(),
            serie.scalar(rows - 1).unwrap()
        );
        let mut written = xlsx();
        written
            .overwrite_arrow_batch(arrow.clone(), &options)
            .expect("streamed Rows writer");
        let bytes = written.read_all_bytes().expect("package bytes");
        let stream_source = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
        let read: usize = stream_source
            .read_arrow_reader(&options)
            .expect("streamed Rows reader")
            .map(|batch| batch.expect("one batch").num_rows())
            .sum();
        assert_eq!(read, rows);
        let reopened = Workbook::from_bytes(stream_source.read_all_bytes().unwrap())
            .expect("streamed package opens");
        assert_eq!(reopened.sheet("Sheet1").unwrap().merges().count(), 2);

        let infer_options =
            RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
        let inferred = stream_source
            .read_arrow_field(&infer_options)
            .expect("merged inferred field");
        assert_eq!(inferred.fields().len(), field.fields().len());
        let inferred_count: usize = stream_source
            .read_arrow_reader(&infer_options)
            .expect("merged inferred reader")
            .map(|batch| batch.expect("batch").num_rows())
            .sum();
        assert_eq!(inferred_count, rows);
        let held_inferred = held
            .clone()
            .into_serie(None, RecordHeader::Infer, Default::default())
            .expect("merged held inference");
        assert_eq!(held_inferred.len(), rows);
        assert_eq!(held_inferred.field().unwrap().fields(), inferred.fields());

        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(format!("model_write/{rows}"), |bencher| {
            bencher.iter(|| {
                Sheet::from_serie("Sheet1", black_box(&serie), RecordHeader::Rows(2))
                    .expect("held rows")
            });
        });
        group.bench_function(format!("model_read/{rows}"), |bencher| {
            bencher.iter_batched(
                || held.clone(),
                |sheet| {
                    sheet
                        .into_serie(Some(&field), RecordHeader::Rows(2), Default::default())
                        .expect("held rows")
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("stream_write/{rows}"), |bencher| {
            bencher.iter(|| {
                let mut handle = xlsx();
                handle
                    .overwrite_arrow_batch(black_box(arrow.clone()), &options)
                    .expect("streamed rows");
                handle
            });
        });
        group.bench_function(format!("stream_read/{rows}"), |bencher| {
            bencher.iter(|| {
                stream_source
                    .read_arrow_reader(black_box(&options))
                    .expect("streamed rows")
                    .map(|batch| batch.expect("one batch").num_rows())
                    .sum::<usize>()
            });
        });
        group.bench_function(format!("infer_stream/{rows}"), |bencher| {
            bencher.iter(|| {
                stream_source
                    .read_arrow_reader(black_box(&infer_options))
                    .expect("inferred rows")
                    .map(|batch| batch.expect("batch").num_rows())
                    .sum::<usize>()
            });
        });
        group.bench_function(format!("infer_model/{rows}"), |bencher| {
            bencher.iter_batched(
                || held.clone(),
                |sheet| {
                    sheet
                        .into_serie(None, RecordHeader::Infer, Default::default())
                        .expect("inferred held rows")
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Opt-in header selection, including the metadata and type passes before
/// the actual row stream. Fixtures and expected values are proven outside
/// timing. The two row counts expose fixed-pass versus per-record work.
fn infer_header_benchmarks(criterion: &mut Criterion) {
    use yggdryl::excel::ExcelOptions;
    use yggdryl::media::RecordOptions;

    let mut group = criterion.benchmark_group("media/excel/infer");
    for rows in [16, corpus(10_000, 64)] {
        let root = field();
        let serie = batch(&root, rows);
        let arrow = serie.clone().into_arrow_batch().expect("one Arrow batch");
        let source_options = RecordOptions::from(ExcelOptions::new()).with_field(root.clone());
        let infer_options =
            RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
        let mut written = xlsx();
        written
            .overwrite_arrow_batch(arrow, &source_options)
            .expect("flat source package");
        let bytes = written.read_all_bytes().expect("package bytes");
        let source = Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
        let inferred = source
            .read_arrow_field(&infer_options)
            .expect("flat inferred field");
        assert_eq!(inferred.fields().len(), root.fields().len());
        let count: usize = source
            .read_arrow_reader(&infer_options)
            .expect("flat inferred reader")
            .map(|batch| batch.expect("batch").num_rows())
            .sum();
        assert_eq!(count, rows);
        let held = Workbook::from_bytes(bytes)
            .expect("workbook opens")
            .sheet("Sheet1")
            .expect("flat sheet")
            .clone();
        let held_back = held
            .clone()
            .into_serie(None, RecordHeader::Infer, Default::default())
            .expect("flat held inference");
        assert_eq!(held_back.len(), rows);
        assert_eq!(held_back.field().unwrap().fields(), inferred.fields());

        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(format!("flat_field/{rows}"), |bencher| {
            bencher.iter(|| {
                source
                    .read_arrow_field(black_box(&infer_options))
                    .expect("inferred field")
            });
        });
        group.bench_function(format!("flat_stream/{rows}"), |bencher| {
            bencher.iter(|| {
                source
                    .read_arrow_reader(black_box(&infer_options))
                    .expect("inferred reader")
                    .map(|batch| batch.expect("batch").num_rows())
                    .sum::<usize>()
            });
        });
        group.bench_function(format!("flat_model/{rows}"), |bencher| {
            bencher.iter_batched(
                || held.clone(),
                |sheet| {
                    sheet
                        .into_serie(None, RecordHeader::Infer, Default::default())
                        .expect("inferred held rows")
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn worksheet_filter_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel/worksheet_filter_cut");
    for mode in ["same", "full", "body", "partial-interior", "partial-header"] {
        let widths: &[u32] = if mode == "partial-header" {
            &[2, 16]
        } else {
            &[1, 16]
        };
        for &columns in widths {
            for cells in [corpus(64, 4), corpus(4_096, 8)] {
                let bytes = excel_package::worksheet_filter_cost_package(columns, cells as u32);
                let ready = || {
                    let book = Workbook::from_bytes(bytes.clone()).unwrap();
                    book.parse_all().unwrap();
                    book
                };
                let (block, destination) = excel_package::worksheet_filter_cut_case(mode, columns);
                let mut proven = ready();
                proven
                    .paste(
                        ("Data", block),
                        (destination, CellRef::new(9, 9)),
                        Paste::All,
                        true,
                    )
                    .unwrap();
                excel_package::assert_cost_worksheet_filter_cut(&proven, columns, mode);
                group.throughput(Throughput::Elements(columns as u64));
                group.bench_function(format!("{mode}/{columns}/{cells}"), |bencher| {
                    bencher.iter_batched(
                        &ready,
                        |mut book| {
                            book.paste(
                                ("Data", black_box(block)),
                                (destination, CellRef::new(9, 9)),
                                Paste::All,
                                true,
                            )
                            .unwrap();
                            book
                        },
                        BatchSize::LargeInput,
                    );
                });
            }
        }
    }
    group.finish();
}

/// A separate two-size throughput comparison exposes repeated scans of all
/// earlier replacements; allocation counts cannot detect that quadratic work.
fn named_table_write_scaling(criterion: &mut Criterion) {
    use arrow_array::{Float64Array, RecordBatch, StringArray};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};

    let mut group = criterion.benchmark_group("media/excel/named_table_write_scaling");
    group.sample_size(10);
    for rows in [corpus(256, 4) as u32, corpus(4_096, 16) as u32] {
        let bytes = excel_package::named_table_cost_package(rows, 16);
        let schema = DataType::from(
            StructType::from_fields([
                DataType::Float64.required_field("id"),
                DataType::utf8().required_field("name"),
            ])
            .unwrap(),
        )
        .required_field("row")
        .into_arrow_schema()
        .unwrap();
        let input = RecordBatch::try_new(
            schema.clone(),
            vec![
                std::sync::Arc::new(Float64Array::from_iter_values((1..=rows).map(f64::from))),
                std::sync::Arc::new(StringArray::from_iter_values((0..rows).map(|_| "written"))),
            ],
        )
        .unwrap();
        let options = ExcelOptions::new().with_table("Names");
        let prepare = || {
            (
                Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into()),
                yggdryl::arrow::batch_reader(schema.clone(), [input.clone()]),
            )
        };
        let (mut proof, reader) = prepare();
        overwrite_arrow_reader(&mut proof, reader, &options).unwrap();
        let book = Workbook::from_bytes(proof.into_bytes()).unwrap();
        assert_eq!(
            book.sheet("Data").unwrap().scalar(CellRef::new(rows, 1)),
            Scalar::from("written")
        );
        group.throughput(Throughput::Elements(2 * u64::from(rows)));
        group.bench_function(rows.to_string(), |bencher| {
            bencher.iter_batched(
                &prepare,
                |(mut output, reader)| {
                    overwrite_arrow_reader(&mut output, reader, black_box(&options)).unwrap();
                    black_box(output)
                },
                BatchSize::LargeInput,
            );
        });
        // The same package and selected-table writer also measure an empty
        // expansion corridor and clearing one departing body row.
        for (mode, count) in [("grow", rows + 1), ("shrink", rows - 1)] {
            let changed = RecordBatch::try_new(
                schema.clone(),
                vec![
                    std::sync::Arc::new(Float64Array::from_iter_values((1..=count).map(f64::from))),
                    std::sync::Arc::new(StringArray::from_iter_values(
                        (0..count).map(|_| "written"),
                    )),
                ],
            )
            .unwrap();
            let prepare = || {
                (
                    Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into()),
                    yggdryl::arrow::batch_reader(schema.clone(), [changed.clone()]),
                )
            };
            let (mut proof, reader) = prepare();
            overwrite_arrow_reader(&mut proof, reader, &options).unwrap();
            let book = Workbook::from_bytes(proof.into_bytes()).unwrap();
            assert_eq!(
                book.sheet("Data").unwrap().scalar(CellRef::new(count, 1)),
                Scalar::from("written")
            );
            if mode == "shrink" {
                assert_eq!(
                    book.sheet("Data").unwrap().scalar(CellRef::new(rows, 1)),
                    Scalar::Null
                );
            }
            group.throughput(Throughput::Elements(2 * u64::from(count)));
            group.bench_function(format!("{mode}/{rows}"), |bencher| {
                bencher.iter_batched(
                    &prepare,
                    |(mut output, reader)| {
                        overwrite_arrow_reader(&mut output, reader, black_box(&options)).unwrap();
                        black_box(output)
                    },
                    BatchSize::LargeInput,
                );
            });
        }
    }
    group.finish();
}

/// Stream one selected body replacement while retaining a formula totals row.
/// The two outside-tail sizes reveal scans of unrelated worksheet rows.
fn named_totals_resize_benchmarks(criterion: &mut Criterion) {
    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};

    let schema = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row")
    .into_arrow_schema()
    .unwrap();
    let options = ExcelOptions::new().with_table("Quantities");
    let mut group = criterion.benchmark_group("media/excel/named_totals_resize");
    group.sample_size(10);
    for outside_rows in [corpus(64, 4) as u32, corpus(4_096, 16) as u32] {
        let bytes = excel_package::named_totals_cost_package(outside_rows, 1, false, 0);
        for (mode, body_rows, table_range, filter_range) in [
            ("shrink", 1, "D1:E3", "D1:E2"),
            ("equal", 2, "D1:E4", "D1:E3"),
            ("grow", 3, "D1:E5", "D1:E4"),
        ] {
            let input = RecordBatch::try_new(
                schema.clone(),
                vec![
                    std::sync::Arc::new(Int64Array::from(
                        (0..body_rows).map(|row| 2030_i64 + row).collect::<Vec<_>>(),
                    )),
                    std::sync::Arc::new(Int64Array::from(
                        (0..body_rows).map(|row| 6_i64 + row).collect::<Vec<_>>(),
                    )),
                ],
            )
            .unwrap();
            let prepare = || {
                (
                    Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into()),
                    yggdryl::arrow::batch_reader(schema.clone(), [input.clone()]),
                )
            };
            let (mut proof, reader) = prepare();
            overwrite_arrow_reader(&mut proof, reader, &options).unwrap();
            let book = Workbook::from_bytes(proof.into_bytes()).unwrap();
            let part = excel_package::member(&book, "xl/tables/table2.xml");
            assert!(
                part.contains(&format!("ref=\"{table_range}\"")),
                "{mode}: {part}"
            );
            assert!(
                part.contains(&format!("<autoFilter ref=\"{filter_range}\"")),
                "{mode}: {part}"
            );
            let sheet = book.sheet("Data").unwrap();
            // OOXML numbers reopen as Float64, including an Int64 Arrow input.
            assert_eq!(
                sheet.scalar(CellRef::new(body_rows as u32, 3)),
                Scalar::from(2030.0 + body_rows as f64 - 1.0)
            );
            let total = sheet.cell(CellRef::new(body_rows as u32 + 1, 4)).unwrap();
            assert_eq!(
                total.formula().unwrap().at(total.reference()).to_string(),
                "SUBTOTAL(109,[qty])"
            );
            group.throughput(Throughput::Elements(2 * body_rows as u64));
            group.bench_function(format!("{mode}/{outside_rows}_outside_rows"), |bencher| {
                bencher.iter_batched(
                    &prepare,
                    |(mut output, reader)| {
                        overwrite_arrow_reader(&mut output, reader, black_box(&options)).unwrap();
                        black_box(output)
                    },
                    BatchSize::LargeInput,
                );
            });
        }
    }
    group.finish();
}

/// Partitions scale with metadata rectangles, independently of covered cells.
fn partial_carried_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel/partial_carried_cut");
    for kind in ["ignored", "hyperlink"] {
        for registrations in [corpus(1, 1) as u32, corpus(256, 16) as u32] {
            for last_row in [64_u32, 1_048_575] {
                let bytes =
                    excel_package::partial_carried_cost_package(kind, last_row, registrations);
                let ready = || {
                    let workbook = Workbook::from_bytes(bytes.clone()).unwrap();
                    workbook.parse_all().unwrap();
                    workbook
                };
                let (source, target) =
                    excel_package::partial_carried_cost_edit(kind, registrations);
                let mut proven = ready();
                proven
                    .paste(("Data", source), ("Other", target), Paste::All, true)
                    .unwrap();
                excel_package::assert_partial_carried_cost_split(
                    &proven,
                    kind,
                    registrations,
                    last_row,
                );
                group.throughput(Throughput::Elements(u64::from(registrations)));
                group.bench_function(
                    format!("{kind}_{registrations}_registrations_{last_row}_rows"),
                    |bencher| {
                        bencher.iter_batched(
                            &ready,
                            |mut workbook| {
                                workbook
                                    .paste(
                                        ("Data", black_box(source)),
                                        ("Other", target),
                                        Paste::All,
                                        true,
                                    )
                                    .unwrap();
                                workbook
                            },
                            BatchSize::LargeInput,
                        )
                    },
                );
            }
        }
    }
    group.finish();
}

/// Many rules sharing one host must scale with rules, not rule pairs.
fn carried_formula_rule_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel/carried_formula_rules");
    for rules in [corpus(32, 4) as u32, corpus(512, 16) as u32] {
        for together in [true, false] {
            let bytes = excel_package::carried_formula_rules_cost_package(rules, together);
            let ready = || {
                let workbook = Workbook::from_bytes(bytes.clone()).unwrap();
                workbook.parse_all().unwrap();
                workbook
            };
            let source: CellRange = "A1".parse().unwrap();
            let target: CellRef = "J10".parse().unwrap();
            let mut proven = ready();
            proven
                .paste(("Data", source), ("Other", target), Paste::All, true)
                .unwrap();
            excel_package::assert_carried_formula_rules_followed(&proven, rules);
            group.throughput(Throughput::Elements(u64::from(rules)));
            group.bench_function(
                format!(
                    "{rules}_rules_{}",
                    if together {
                        "one_parent"
                    } else {
                        "separate_parents"
                    }
                ),
                |bencher| {
                    bencher.iter_batched(
                        &ready,
                        |mut workbook| {
                            workbook
                                .paste(
                                    ("Data", black_box(source)),
                                    ("Other", target),
                                    Paste::All,
                                    true,
                                )
                                .unwrap();
                            workbook
                        },
                        BatchSize::LargeInput,
                    )
                },
            );
        }
    }
    group.finish();
}

/// Parse entry once and parse a file shape lazily at two expression sizes.
fn formula_parser_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel/formula_parser");
    let host = CellRef::new(2, 2);
    for terms in [corpus(64, 8), corpus(512, 32)] {
        let text = format!("{}1", "A1+".repeat(terms));
        let entry = Formula::from_entry(&text, host).expect("valid entry");
        assert!(entry.is_computed());
        let file = Formula::from_file(&text, host);
        assert!(file.is_computed());
        assert_eq!(entry.at(host).to_string(), file.at(host).to_string());
        group.throughput(Throughput::Elements(terms as u64));
        group.bench_function(format!("entry/{terms}"), |bencher| {
            bencher.iter(|| Formula::from_entry(black_box(&text), host).unwrap())
        });
        group.bench_function(format!("file_lazy/{terms}"), |bencher| {
            bencher.iter(|| {
                let formula = Formula::from_file(black_box(&text), host);
                black_box(formula.is_computed())
            })
        });
    }
    group.finish();
}

/// One parsed shape over many independent constant cells. Workbook construction
/// and first lazy arena compilation are outside the warm-pass timer.
fn constant_formula_recalculation_benchmarks(criterion: &mut Criterion) {
    fn workbook(rows: usize, expression: &str) -> Workbook {
        let mut book = Workbook::new();
        let formula = Formula::from_file(expression, CellRef::new(0, 0));
        let sheet = book.add_sheet("Data").expect("sheet");
        for row in 0..rows {
            let at = CellRef::new(row as u32, 0);
            sheet
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(0.0), DateSystem::Year1900)
                        .expect("cache")
                        .with_formula(formula.clone()),
                )
                .expect("formula cell");
        }
        book
    }

    let mut group = criterion.benchmark_group("media/excel_recalc/constant");
    for (name, expression) in [
        ("arithmetic", "1+2"),
        ("abs", "ABS(-7.25)"),
        ("year_serial", "YEAR(60)"),
        ("day_phantom", "DAY(60)"),
        ("weekday_phantom", "WEEKDAY(60,1)"),
        ("days", "_xlfn.DAYS(61,60)"),
        ("date_phantom", "DATE(1900,2,29)"),
        ("hour", "HOUR(60.5)"),
        ("second", "SECOND(1/86400)"),
        ("time", "TIME(12,34,56)"),
        ("edate_phantom", "EDATE(60,1)"),
        ("eomonth_phantom", "EOMONTH(61,-1)"),
        ("datevalue_iso", "DATEVALUE(\"2024-02-29\")"),
        ("timevalue", "TIMEVALUE(\"12:34:56\")"),
        ("sum", "SUM(1,2,3)"),
        ("count", r#"COUNT(1,"text",2)"#),
        ("counta", r#"COUNTA(1,"text",2)"#),
        ("min", "MIN(3,1,2)"),
        ("max", "MAX(3,1,2)"),
        ("average", "AVERAGE(1,2,3)"),
        ("averagea", "AVERAGEA(1,TRUE,2)"),
        ("mina", "MINA(2,FALSE)"),
        ("maxa", "MAXA(2,TRUE)"),
        ("product", "PRODUCT(2,3,4)"),
        ("round", "ROUND(2.15,1)"),
        ("roundup", "ROUNDUP(2.15,1)"),
        ("rounddown", "ROUNDDOWN(2.15,1)"),
        ("quotient", "QUOTIENT(0.3,0.1)"),
        ("even", "EVEN(2.5)"),
        ("odd", "ODD(-2.5)"),
        ("ceiling", "CEILING(0.3,0.1)"),
        ("floor", "FLOOR(0.3,0.1)"),
        ("mround", "MROUND(1.005,0.01)"),
        ("ceiling_math", "_xlfn.CEILING.MATH(-3.2,2,1)"),
        ("floor_math", "_xlfn.FLOOR.MATH(-3.2,2,1)"),
        ("sqrt", "SQRT(2)"),
        ("exp", "EXP(1)"),
        ("ln", "LN(2.5)"),
        ("log10", "LOG10(2.5)"),
        ("degrees", "DEGREES(1)"),
        ("radians", "RADIANS(1)"),
        ("cos", "COS(1)"),
        ("asin", "ASIN(0.5)"),
        ("sin", "SIN(0.5)"),
        ("tan", "TAN(0.5)"),
        ("acos", "ACOS(0.5)"),
        ("atan", "ATAN(0.5)"),
        ("atan2", "ATAN2(1,0.5)"),
        ("log", "LOG(3,2)"),
        ("mod", "MOD(-7,3)"),
        ("gcd", "GCD(12,18)"),
        ("lcm", "LCM(12,18)"),
        ("fact", "FACT(12)"),
        ("sign", "SIGN(-0.001)"),
        ("int", "INT(-3.2)"),
        ("trunc", "TRUNC(-3.14159,3)"),
        ("pi", "PI()"),
        ("not", "NOT(0)"),
        ("isnumber", "ISNUMBER(2)"),
        ("iserror", "ISERROR(NA())"),
        ("iseven", "ISEVEN(-3.7)"),
        ("n", "N(TRUE)"),
        ("na", "NA()"),
        ("isref_self", "ISREF(A1)"),
        ("isref_column", "ISREF(B:B)"),
    ] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut warm = workbook(rows, expression);
            assert_eq!(
                warm.calculate_all().expect("first pass").evaluated,
                rows as u64
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{name}/first/{rows}"), |bencher| {
                bencher.iter_batched(
                    || workbook(rows, expression),
                    |mut book| black_box(book.calculate_all().expect("first pass")),
                    BatchSize::SmallInput,
                );
            });
            group.bench_function(format!("{name}/warm_noop/{rows}"), |bencher| {
                bencher.iter(|| black_box(warm.calculate_all().expect("warm pass")));
            });
        }
    }
    group.finish();
}

/// Compare forced passes, no-change passes and a two-node dirty closure. Setup,
/// lazy parsing and graph construction are outside each warm measurement.
fn dependency_recalculation_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/dependencies");
    for rows in [corpus(64, 16), corpus(4_096, 64)] {
        let mut book = Workbook::new();
        let sheet = book.add_sheet("Data").unwrap();
        let first = Formula::from_file("A1+1", CellRef::new(0, 1));
        let second = Formula::from_file("B1*2", CellRef::new(0, 2));
        for row in 0..rows as u32 {
            sheet
                .set_cell(CellRef::new(row, 0), f64::from(row))
                .unwrap();
            for (column, formula) in [(1, &first), (2, &second)] {
                let at = CellRef::new(row, column);
                sheet
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-1.0), DateSystem::Year1900)
                            .unwrap()
                            .with_formula(formula.clone()),
                    )
                    .unwrap();
            }
        }
        assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64 * 2);
        assert_eq!(book.recalculate().unwrap().evaluated, 0);
        group.bench_function(format!("force_all/{rows}"), |bencher| {
            bencher.iter(|| black_box(book.calculate_all().unwrap()));
        });
        group.bench_function(format!("unchanged/{rows}"), |bencher| {
            bencher.iter(|| black_box(book.recalculate().unwrap()));
        });
        let mut value = 3.0;
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(CellRef::new(0, 0), value)
            .unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 2);
        group.bench_function(format!("point_change/{rows}"), |bencher| {
            bencher.iter(|| {
                value = if value == 3.0 { 4.0 } else { 3.0 };
                book.sheet_mut("Data")
                    .unwrap()
                    .set_cell(CellRef::new(0, 0), value)
                    .unwrap();
                black_box(book.recalculate().unwrap())
            });
        });
    }
    group.finish();
}

/// A warm volatile pass visits every formula, with the workbook graph and
/// parsed shape already held. `RAND` exercises host-seeded draws; TODAY uses
/// the same once-per-pass clock sample without per-cell system calls.
fn clock_recalculation_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/clock");
    for (name, expression) in [
        ("rand", "RAND()"),
        ("today", "TODAY()"),
        ("between", "RANDBETWEEN(-5,7)"),
    ] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book = Workbook::new().with_clock(Clock::fixed(
                -2_203_977_600_000_000_000,
                yggdryl::Timezone::UTC,
                73,
            ));
            let sheet = book.add_sheet("Cases").unwrap();
            let shape = Formula::from_file(expression, CellRef::new(0, 0));
            for row in 0..rows as u32 {
                let at = CellRef::new(row, 0);
                sheet
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(0.0), DateSystem::Year1900)
                            .unwrap()
                            .with_formula(shape.clone()),
                    )
                    .unwrap();
            }
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{name}/warm/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.recalculate().unwrap()));
            });
        }
    }
    group.finish();
}

/// The range iterator lends physical cells directly to the calculation context.
/// Force a warm pass so every iteration executes SUM over the complete range.
fn range_recalculation_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/range");
    for rows in [corpus(64, 16), corpus(4_096, 64)] {
        let mut book = Workbook::new();
        let sheet = book.add_sheet("Data").unwrap();
        for row in 0..rows as u32 {
            sheet
                .set_cell(CellRef::new(row, 0), f64::from(row + 1))
                .unwrap();
        }
        let at = CellRef::new(0, 1);
        book.set_entry("Data", at, &format!("=SUM(A1:A{rows})"))
            .unwrap();
        assert_eq!(book.calculate_all().unwrap().evaluated, 1);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at),
            Scalar::from((rows * (rows + 1) / 2) as f64)
        );
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(format!("numeric/{rows}"), |bencher| {
            bencher.iter(|| black_box(book.calculate_all().unwrap()));
        });
    }
    for function in ["MEDIAN", "MODE"] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book = Workbook::new();
            let sheet = book.add_sheet("Data").unwrap();
            for row in 0..rows as u32 {
                sheet
                    .set_cell(CellRef::new(row, 0), f64::from(row % 8))
                    .unwrap();
            }
            let at = CellRef::new(0, 1);
            book.set_entry("Data", at, &format!("={function}(A1:A{rows})"))
                .unwrap();
            assert_eq!(book.calculate_all().unwrap().evaluated, 1);
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at),
                Scalar::from(if function == "MEDIAN" { 3.5 } else { 0.0 })
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("rank/{function}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    for (formula, expected_at_rows) in [
        ("LARGE(A1:A{rows},1)", None),
        ("SMALL(A1:A{rows},1)", Some(0.0)),
        ("PERCENTILE(A1:A{rows},0.5)", Some(3.5)),
        ("_xlfn.PERCENTILE.INC(A1:A{rows},0.5)", Some(3.5)),
        ("QUARTILE(A1:A{rows},2)", Some(3.5)),
        ("_xlfn.QUARTILE.INC(A1:A{rows},2)", Some(3.5)),
        ("RANK(2,A1:A{rows})", None),
        ("_xlfn.RANK.EQ(2,A1:A{rows})", None),
    ] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book = Workbook::new();
            let sheet = book.add_sheet("Data").unwrap();
            for row in 0..rows as u32 {
                sheet
                    .set_cell(CellRef::new(row, 0), f64::from(row % 8))
                    .unwrap();
            }
            let at = CellRef::new(0, 1);
            book.set_entry(
                "Data",
                at,
                &format!("={}", formula.replace("{rows}", &rows.to_string())),
            )
            .unwrap();
            assert_eq!(book.calculate_all().unwrap().evaluated, 1);
            let expected = expected_at_rows.unwrap_or_else(|| {
                if formula.starts_with("LARGE") {
                    7.0
                } else {
                    (1 + 5 * rows / 8) as f64
                }
            });
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at),
                Scalar::from(expected)
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("order_statistic/{formula}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    for function in ["COUNTBLANK", "_xlfn.MAXIFS", "_xlfn.MINIFS"] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book = Workbook::new();
            let sheet = book.add_sheet("Data").unwrap();
            for row in 0..rows as u32 {
                if function == "COUNTBLANK" {
                    if row % 2 == 1 {
                        sheet.set_cell(CellRef::new(row, 0), "occupied").unwrap();
                    }
                } else {
                    sheet
                        .set_cell(CellRef::new(row, 0), f64::from(row % 8))
                        .unwrap();
                    sheet.set_cell(CellRef::new(row, 1), 1.0).unwrap();
                }
            }
            let at = CellRef::new(0, 2);
            let formula = if function == "COUNTBLANK" {
                format!("=COUNTBLANK(A1:A{rows})")
            } else {
                format!("={function}(A1:A{rows},B1:B{rows},1)")
            };
            book.set_entry("Data", at, &formula).unwrap();
            assert_eq!(book.calculate_all().unwrap().evaluated, 1);
            let expected = match function {
                "COUNTBLANK" => (rows / 2) as f64,
                "_xlfn.MAXIFS" => 7.0,
                _ => 0.0,
            };
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at),
                Scalar::from(expected)
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("blank_extrema/{function}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    let typed_factor = Scalar::from_sequence([
        Scalar::from("long text factor ".repeat(32)),
        Scalar::from(17_i64),
    ]);
    for (kind, text_source) in [("numeric", false), ("typed_text", true)] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book = Workbook::new();
            let sheet = book.add_sheet("Data").unwrap();
            for row in 0..rows as u32 {
                let source = if text_source {
                    typed_factor.clone()
                } else {
                    Scalar::from(f64::from(row % 8))
                };
                sheet.set_cell(CellRef::new(row, 0), source).unwrap();
                sheet.set_cell(CellRef::new(row, 1), 1.0).unwrap();
            }
            let at = CellRef::new(0, 2);
            book.set_entry("Data", at, &format!("=SUMPRODUCT(A1:A{rows},B1:B{rows})"))
                .unwrap();
            assert_eq!(book.calculate_all().unwrap().evaluated, 1);
            let expected = if text_source {
                0.0
            } else {
                (rows / 8 * 28) as f64
            };
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at),
                Scalar::from(expected)
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("sumproduct/{kind}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    // The borrowed Presence path must scale with source count, not text size.
    let long = "not a numeric value ".repeat(512);
    let typed = Scalar::from_sequence([Scalar::from(long.clone()), Scalar::from(17_i64)]);
    for (kind, value) in [("native_text", Scalar::from(long)), ("typed_text", typed)] {
        for function in [
            "COUNT", "COUNTA", "MIN", "MAX", "AVERAGE", "AVERAGEA", "MINA", "MAXA", "PRODUCT",
            "MEDIAN",
        ] {
            for rows in [corpus(64, 16), corpus(4_096, 64)] {
                let mut book = Workbook::new();
                let sheet = book.add_sheet("Data").unwrap();
                for row in 0..rows as u32 {
                    sheet.set_cell(CellRef::new(row, 0), value.clone()).unwrap();
                }
                let at = CellRef::new(0, 1);
                book.set_entry("Data", at, &format!("={function}(A1:A{rows},2)"))
                    .unwrap();
                assert_eq!(book.calculate_all().unwrap().evaluated, 1);
                let expected = match function {
                    "COUNT" => 1.0,
                    "COUNTA" => (rows + 1) as f64,
                    "AVERAGEA" => 2.0 / (rows + 1) as f64,
                    "MINA" => 0.0,
                    "MEDIAN" => 2.0,
                    _ => 2.0,
                };
                assert_eq!(
                    book.sheet("Data").unwrap().scalar(at),
                    Scalar::from(expected)
                );
                group.throughput(Throughput::Elements(rows as u64));
                group.bench_function(format!("{kind}/{function}/{rows}"), |bencher| {
                    bencher.iter(|| black_box(book.calculate_all().unwrap()));
                });
            }
        }
    }
    group.finish();
}

/// Names execute through the same evaluator and sparse source reader; these
/// force actual work after arena compilation and dependency intake are warm.
fn named_recalculation_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/names");
    for (label, formula) in [
        ("constant", "Constant"),
        ("aliases", "SecondAlias+SecondAlias"),
        ("range", "SUM(NamedColumn)"),
    ] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book =
                excel_package::defined_name_calculation_cost_book(rows as u32, 64, formula);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{label}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn comparison_recalculation_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/comparisons");
    let text = "AbC123".repeat(512);
    for (label, left, right) in [
        ("numeric", Scalar::from(1.0), Scalar::from(2.0)),
        (
            "long_ascii",
            Scalar::from(format!("{text}x")),
            Scalar::from(format!("{text}Y")),
        ),
    ] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book = excel_package::comparison_calculation_cost_book(
                rows as u32,
                left.clone(),
                right.clone(),
            );
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{label}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn logical_reducer_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/logical_reducers");
    for rows in [corpus(64, 16), corpus(4_096, 64)] {
        let mut book = excel_package::logical_scalar_cost_book(rows as u32);
        assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(format!("scalar/{rows}"), |bencher| {
            bencher.iter(|| black_box(book.calculate_all().unwrap()));
        });
    }
    let long = "not a logical value ".repeat(512);
    for (label, value) in [
        ("Boolean", Scalar::from(true)),
        ("long_text", Scalar::from(long)),
    ] {
        for rows in [corpus(64, 16), corpus(4_096, 64)] {
            let mut book = excel_package::logical_reducer_cost_book(rows as u32, value.clone());
            assert_eq!(book.calculate_all().unwrap().evaluated, 3);
            group.throughput(Throughput::Elements(3 * rows as u64));
            group.bench_function(format!("{label}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn lazy_selector_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/lazy_selectors");
    for rows in [corpus(64, 16), corpus(4096, 64)] {
        for (label, mut book, evaluated) in [
            (
                "scalar",
                excel_package::lazy_scalar_cost_book(rows as u32),
                rows as u64,
            ),
            ("range", excel_package::lazy_range_cost_book(rows as u32), 1),
            (
                "suspended",
                excel_package::lazy_chain_cost_book(rows as u32),
                rows as u64,
            ),
        ] {
            assert_eq!(book.calculate_all().unwrap().evaluated, evaluated);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{label}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn multi_selector_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/multi_selectors");
    for rows in [corpus(64, 16), corpus(4096, 64)] {
        for (label, base, text, column, formulas) in [
            (
                "ifs_scalar",
                excel_package::lazy_scalar_cost_book(rows as u32),
                "IFS(A1,C1,FALSE,D1)",
                1,
                rows as u32,
            ),
            (
                "switch_scalar",
                excel_package::lazy_scalar_cost_book(rows as u32),
                "SWITCH(A1,TRUE,C1,FALSE,D1,0)",
                1,
                rows as u32,
            ),
            (
                "ifs_range",
                excel_package::lazy_range_cost_book(rows as u32),
                "IFS(C1,SUM(B:B),TRUE,0)",
                0,
                1,
            ),
            (
                "switch_range",
                excel_package::lazy_range_cost_book(rows as u32),
                "SWITCH(C1,TRUE,SUM(B:B),FALSE,0,-1)",
                0,
                1,
            ),
            (
                "ifs_suspended",
                excel_package::lazy_chain_cost_book(rows as u32),
                "IFS(TRUE,A2,FALSE,0)",
                0,
                rows as u32,
            ),
            (
                "switch_suspended",
                excel_package::lazy_chain_cost_book(rows as u32),
                "SWITCH(1,1,A2,2,0,-1)",
                0,
                rows as u32,
            ),
        ] {
            let mut book = excel_package::selector_formula_cost_book(base, text, column, formulas);
            assert_eq!(book.calculate_all().unwrap().evaluated, u64::from(formulas));
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{label}/{rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn geometry_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/geometry");
    for formulas in [corpus(64, 16), corpus(4096, 64)] {
        for source_rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::geometry_cost_book(formulas as u32, source_rows as u32);
            assert_eq!(book.calculate_all().unwrap().evaluated, formulas as u64);
            group.throughput(Throughput::Elements(formulas as u64));
            group.bench_function(format!("{formulas}/{source_rows}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn text_function_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/text");
    for long in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::text_cost_book(rows as u32, long);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/long-{long}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn indexed_reference_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/indexed_reference");
    for offset in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book =
                excel_package::indexed_reference_cost_book(rows as u32, rows as u32, offset);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/offset-{offset}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn text_conversion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/text_conversion");
    for joins in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::text_conversion_cost_book(rows as u32, joins);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/joins-{joins}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn text_index_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/text_index");
    for long in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::text_index_cost_book(rows as u32, long);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/long-{long}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn text_casing_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/text_casing");
    for long in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::text_casing_cost_book(rows as u32, long);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/long-{long}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn text_search_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/text_search");
    for long in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::text_search_cost_book(rows as u32, long);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/long-{long}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn text_character_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/text_character");
    for long in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::text_character_cost_book(rows as u32, long);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/long-{long}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn text_value_format_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/text_value_format");
    for kind in 0..6 {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::text_value_format_cost_book(rows as u32, kind);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/kind-{kind}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

/// Exact early-key and paused-late-key lookup over source axes of two sizes.
fn lookup_axis_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/lookup_axis");
    for rows in [corpus(64, 16), corpus(4096, 64)] {
        for late in [false, true] {
            let mut book = Workbook::new();
            book.add_sheet("Cases").unwrap();
            book.add_sheet("Data").unwrap();
            let target = if late { rows } else { 1 };
            book.set_entry(
                "Cases",
                CellRef::new(0, 1),
                &format!("=MATCH({target},Data!A1:A{rows},0)"),
            )
            .unwrap();
            for row in 0..rows {
                let at = CellRef::new(row as u32, 0);
                let input = if late {
                    format!("={}", row + 1) // every key suspends before its first read
                } else {
                    (row + 1).to_string()
                };
                book.set_entry("Data", at, &input).unwrap();
            }
            book.calculate_all().unwrap();
            assert_eq!(
                book.sheet("Cases")
                    .unwrap()
                    .scalar(CellRef::new(0, 1))
                    .as_f64(),
                Some(target as f64)
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/late-{late}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()));
            });
        }
    }
    group.finish();
}

fn financial_annuity_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/financial_annuity");
    for long in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::financial_annuity_cost_book(rows as u32, long);
            assert_eq!(book.calculate_all().unwrap().evaluated, rows as u64);
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/long-{long}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn financial_npv_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/financial_npv");
    for range in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::financial_npv_cost_book(rows as u32, range);
            assert_eq!(
                book.calculate_all().unwrap().evaluated,
                if range { 1 } else { rows as u64 }
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/range-{range}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn financial_payment_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/financial_payment");
    for rows in [corpus(64, 16), corpus(4096, 64)] {
        let mut book = excel_package::financial_payment_cost_book(rows as u32);
        assert_eq!(
            book.calculate_all().unwrap().evaluated,
            (rows / 4 * 3) as u64
        );
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(format!("{rows}"), |bencher| {
            bencher.iter(|| black_box(book.calculate_all().unwrap()))
        });
    }
    group.finish();
}

fn variance_exact_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/variance_exact");
    for range in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = excel_package::variance_exact_cost_book(rows as u32, range);
            assert_eq!(
                book.calculate_all().unwrap().evaluated,
                if range { 8 } else { rows as u64 }
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/range-{range}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn typed_text_write_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_write/typed_text");
    for rows in [corpus(64, 16), corpus(4096, 64)] {
        let book = excel_package::typed_text_write_cost_book(rows as u32, true);
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(format!("{rows}"), |bencher| {
            bencher.iter(|| black_box(book.into_bytes().unwrap()))
        });
    }
    group.finish();
}

fn criteria_subtotal_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/criteria_subtotal");
    for subtotal in [false, true] {
        for rows in [corpus(64, 16), corpus(4096, 64)] {
            let mut book = if subtotal {
                excel_package::subtotal_cost_book(rows as u32)
            } else {
                excel_package::criteria_six_cost_book(rows as u32)
            };
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed),
                (if subtotal { 23 } else { 6 }, 0)
            );
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/subtotal-{subtotal}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn function_catalog_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_formula/catalog");
    for scans in [corpus(64, 16), corpus(4096, 64)] {
        group.throughput(Throughput::Elements((scans * 159) as u64));
        group.bench_function(format!("{scans}"), |bencher| {
            bencher.iter(|| {
                let mut names = 0;
                for _ in 0..scans {
                    for entry in Formula::functions() {
                        names += black_box(entry.name.len());
                    }
                }
                black_box(names)
            })
        });
    }
    group.finish();
}

fn literal_array_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/literal_arrays");
    for rows in [corpus(64, 16), corpus(4096, 64)] {
        for long in [false, true] {
            let mut book = excel_package::literal_array_cost_book(rows as u32, long);
            let report = book.calculate_all().unwrap();
            assert_eq!((report.evaluated, report.uncomputed), (rows as u64, 0));
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("{rows}/long-{long}"), |bencher| {
                bencher.iter(|| black_box(book.calculate_all().unwrap()))
            });
        }
    }
    group.finish();
}

fn reference_algebra_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/reference_algebra");
    for (formulas, source_rows) in [
        (corpus(64, 16), corpus(64, 16)),
        (corpus(4096, 64), corpus(64, 16)),
        (corpus(64, 16), corpus(4096, 64)),
    ] {
        let mut book =
            excel_package::reference_algebra_cost_book(formulas as u32, source_rows as u32);
        assert_eq!(book.calculate_all().unwrap().evaluated, formulas as u64);
        group.throughput(Throughput::Elements(formulas as u64));
        group.bench_function(format!("{formulas}/{source_rows}"), |bencher| {
            bencher.iter(|| black_box(book.calculate_all().unwrap()));
        });
    }
    group.finish();
}

fn mapped_array_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_recalc/mapped_arrays");
    for rows in [corpus(64, 16), corpus(4096, 64)] {
        let mut book = excel_package::mapped_array_cost_book(rows as u32);
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (rows as u64, 0));
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(format!("cells/{rows}"), |bencher| {
            bencher.iter(|| black_box(book.calculate_all().unwrap()))
        });
    }
    for rows in [1, 64] {
        let mut book = excel_package::mapped_array_broadcast_book(rows, 64);
        assert_eq!(book.calculate_all().unwrap().uncomputed, 0);
        group.throughput(Throughput::Elements((rows * 64) as u64));
        group.bench_function(format!("broadcast/{}", rows * 64), |bencher| {
            bencher.iter(|| black_box(book.calculate_all().unwrap()))
        });
    }
    group.finish();
}

fn pivot_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/excel_pivot");
    for rows in [corpus(1_000, 64), corpus(100_000, 256)] {
        for formatted in [false, true] {
            let (mut book, mut spec) = excel_package::pivot_cost_book(rows as u32);
            if formatted {
                spec.values[0].number_format = Some("#,##0.0000".into());
            }
            book.add_pivot(spec, "Report", CellRef::new(2, 0)).unwrap();
            book.refresh_pivot("Report", "CostPivot").unwrap();
            group.throughput(Throughput::Elements(rows as u64));
            group.bench_function(format!("refresh/{rows}/formatted-{formatted}"), |bencher| {
                bencher.iter(|| black_box(book.refresh_pivot("Report", "CostPivot").unwrap()));
            });
        }
    }
    group.finish();
}

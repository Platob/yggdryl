//! `rust/src/excel/cell.rs`: the A1 grammar of `CellRef` and `CellRange`, the `t` kinds, the serial-date rule both ways, and the `Cell` a value builds.

use yggdryl::excel::{
    Cell, CellKind, CellRange, CellRef, DateSystem, MAX_CELL_TEXT, MAX_COLUMNS, MAX_ROWS,
    NumberFormat,
};
use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

/// Milliseconds in one day.
const DAY: i64 = 86_400_000;

fn reference(text: &str) -> CellRef {
    text.parse().unwrap()
}

fn range(text: &str) -> CellRange {
    text.parse().unwrap()
}

fn reference_refusal(text: &str) -> String {
    text.parse::<CellRef>().unwrap_err().to_string()
}

fn range_refusal(text: &str) -> String {
    text.parse::<CellRange>().unwrap_err().to_string()
}

/// The date a serial names in `system`, as its ISO 8601 JSON text.
fn date_of(system: DateSystem, serial: f64) -> String {
    Scalar::date32(system.days_from_serial(serial).unwrap())
        .into_json()
        .unwrap()
}

fn naive_millis(count: i64) -> Scalar {
    Scalar::datetime64(count, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
}

#[test]
fn a_cell_reference_reads_every_a1_spelling_with_or_without_dollar_anchors() {
    for (text, row, column) in [
        ("A1", 0, 0),
        ("$B$2", 1, 1),
        ("$C3", 2, 2),
        ("C$3", 2, 2),
        ("b2", 1, 1),
        ("AB12", 11, 27),
        ("xfd1048576", 1_048_575, 16_383),
    ] {
        let cell = reference(text);
        assert_eq!((cell.row(), cell.column()), (row, column), "{text}");
        assert_eq!(cell, CellRef::new(row, column), "{text}");
    }
}

#[test]
fn a_cell_reference_writes_the_a1_spelling_it_reads_back() {
    for ((row, column), text) in [
        ((0, 0), "A1"),
        ((1, 27), "AB2"),
        ((9, 701), "ZZ10"),
        ((0, 702), "AAA1"),
        ((1_048_575, 16_383), "XFD1048576"),
    ] {
        let cell = CellRef::from((row, column));
        assert_eq!(cell.to_string(), text);
        assert_eq!(reference(text), cell);
    }
    assert_eq!(reference("$Z$9").to_string(), "Z9");
}

#[test]
fn column_names_and_column_indexes_are_inverse_from_a_to_xfd() {
    for (column, name) in [
        (0, "A"),
        (25, "Z"),
        (26, "AA"),
        (51, "AZ"),
        (701, "ZZ"),
        (702, "AAA"),
        (16_383, "XFD"),
    ] {
        assert_eq!(CellRef::column_name(column), name);
        assert_eq!(CellRef::column_index(name), Some(column), "{name}");
    }
    assert_eq!(CellRef::column_index("xfd"), Some(16_383));
    for refused in ["XFE", "", "AAAA", "A1", "$A"] {
        assert_eq!(CellRef::column_index(refused), None, "{refused}");
    }
}

#[test]
fn a_cell_reference_refuses_a_sheet_qualifier_a_column_past_xfd_and_a_row_outside_the_grid() {
    assert_eq!(
        reference_refusal("Sheet1!A1"),
        "invalid cell reference expression at byte 6: expected a reference within the sheet, \
         got the sheet-qualified \"Sheet1!A1\""
    );
    assert_eq!(
        reference_refusal("XFE1"),
        "invalid cell reference expression at byte 0: expected column letters A to XFD in \
         \"XFE1\", got \"XFE\""
    );
    let past_rows = reference_refusal("A1048577");
    assert!(
        past_rows
            .contains("expected a row number from 1 to 1048576 in \"A1048577\", got \"1048577\""),
        "{past_rows}"
    );
    let row_zero = reference_refusal("A0");
    assert!(
        row_zero.contains("expected a row number from 1 to 1048576 in \"A0\", got \"0\""),
        "{row_zero}"
    );
    let four_letters = reference_refusal("AAAA1");
    assert!(
        four_letters.contains("expected column letters A to XFD in \"AAAA1\", got \"AAAA\""),
        "{four_letters}"
    );
    for half in ["A", "7", ""] {
        let refusal = reference_refusal(half);
        assert!(
            refusal.contains(&format!(
                "expected a column and a row such as B2, got {half:?}"
            )),
            "{refusal}"
        );
    }
}

#[test]
fn the_grid_holds_1048576_rows_by_16384_columns_and_require_in_grid_names_a_cell_past_it() {
    assert_eq!((MAX_ROWS, MAX_COLUMNS), (1_048_576, 16_384));
    let last = CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 1);
    assert!(last.is_in_grid());
    assert_eq!(last.require_in_grid().unwrap(), last);
    assert!(!CellRef::new(MAX_ROWS, 0).is_in_grid());
    assert!(!CellRef::new(0, MAX_COLUMNS).is_in_grid());
    assert_eq!(
        CellRef::new(MAX_ROWS, 0)
            .require_in_grid()
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a cell within 1048576 rows and 16384 columns \
         (A1 to XFD1048576), got row 1048577 column 1"
    );
    assert_eq!(
        CellRef::new(0, MAX_COLUMNS)
            .require_in_grid()
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a cell within 1048576 rows and 16384 columns \
         (A1 to XFD1048576), got row 1 column 16385"
    );
}

#[test]
fn cell_references_order_by_row_then_by_column() {
    let mut cells = [
        reference("B2"),
        reference("A2"),
        reference("C1"),
        reference("A1"),
    ];
    cells.sort();
    let spelled: Vec<String> = cells.iter().map(ToString::to_string).collect();
    assert_eq!(spelled, ["A1", "C1", "A2", "B2"]);
}

#[test]
fn a_range_normalizes_its_corners_whichever_order_they_come_in() {
    let expected = (CellRef::new(0, 0), CellRef::new(2, 2));
    for (first, second) in [("A1", "C3"), ("C3", "A1"), ("A3", "C1"), ("C1", "A3")] {
        let built = CellRange::new(reference(first), reference(second));
        assert_eq!((built.start(), built.end()), expected, "{first}:{second}");
        assert_eq!(
            CellRange::from((reference(first), reference(second))),
            built
        );
        let parsed = range(&format!("{first}:{second}"));
        assert_eq!(parsed, built);
        assert_eq!(parsed.to_string(), "A1:C3");
    }
}

#[test]
fn a_single_cell_range_is_that_cell_and_displays_as_it() {
    let single = range("B2");
    assert_eq!(single.start(), reference("B2"));
    assert_eq!(single.end(), reference("B2"));
    assert_eq!((single.row_size(), single.column_size()), (1, 1));
    assert_eq!(single.cells().collect::<Vec<_>>(), [reference("B2")]);
    assert_eq!(single.to_string(), "B2");
    assert_eq!(range("B2:B2").to_string(), "B2");
}

#[test]
fn open_ranges_run_to_the_edge_of_the_grid_and_display_in_their_short_form() {
    let last_row = MAX_ROWS - 1;
    let last_column = MAX_COLUMNS - 1;
    for (text, start, end, shown) in [
        ("A:C", (0, 0), (last_row, 2), "A:C"),
        ("A:A", (0, 0), (last_row, 0), "A:A"),
        ("3:5", (2, 0), (4, last_column), "3:5"),
        ("3:3", (2, 0), (2, last_column), "3:3"),
        ("A3:F", (2, 0), (last_row, 5), "A3:F"),
        ("A1:F", (0, 0), (last_row, 5), "A:F"),
        ("$A$1:$B$2", (0, 0), (1, 1), "A1:B2"),
        ("1:1048576", (0, 0), (last_row, last_column), "A:XFD"),
    ] {
        let parsed = range(text);
        assert_eq!(parsed.start(), CellRef::from(start), "{text}");
        assert_eq!(parsed.end(), CellRef::from(end), "{text}");
        assert_eq!(parsed.to_string(), shown, "{text}");
        assert_eq!(range(shown), parsed, "{shown}");
    }
    assert_eq!(range("1:1048576"), CellRange::all());
    assert_eq!(CellRange::all().to_string(), "A:XFD");
}

#[test]
fn an_open_range_spans_every_row_or_column_to_the_edge() {
    let columns = range("A:C");
    assert_eq!((columns.row_size(), columns.column_size()), (MAX_ROWS, 3));
    assert!(columns.is_row_open());
    assert!(!columns.is_column_open());

    let rows = range("3:5");
    assert_eq!((rows.row_size(), rows.column_size()), (3, MAX_COLUMNS));
    assert!(rows.is_column_open());
    assert!(!rows.is_row_open());

    let from_row = range("B3:F");
    assert_eq!(
        (from_row.row_size(), from_row.column_size()),
        (MAX_ROWS - 2, 5)
    );
    assert!(from_row.is_row_open());

    let closed = range("B2:C3");
    assert!(!closed.is_row_open());
    assert!(!closed.is_column_open());
    let all = CellRange::all();
    assert!(all.is_row_open() && all.is_column_open());
    assert_eq!((all.row_size(), all.column_size()), (MAX_ROWS, MAX_COLUMNS));
}

#[test]
fn a_range_contains_exactly_the_rows_and_columns_between_its_corners() {
    let square = range("B2:C3");
    for inside in ["B2", "C2", "B3", "C3"] {
        assert!(square.contains(reference(inside)), "{inside}");
    }
    for outside in ["A2", "B1", "D3", "C4", "A1"] {
        assert!(!square.contains(reference(outside)), "{outside}");
    }
    assert_eq!(
        (0..5)
            .map(|row| square.contains_row(row))
            .collect::<Vec<_>>(),
        [false, true, true, false, false]
    );
    assert_eq!(
        (0..5)
            .map(|column| square.contains_column(column))
            .collect::<Vec<_>>(),
        [false, true, true, false, false]
    );
    let columns = range("A:C");
    assert!(columns.contains(CellRef::new(MAX_ROWS - 1, 2)));
    assert!(!columns.contains(CellRef::new(0, 3)));
}

#[test]
fn cells_walk_the_range_row_by_row() {
    let spelled: Vec<String> = range("B2:C3")
        .cells()
        .map(|cell| cell.to_string())
        .collect();
    assert_eq!(spelled, ["B2", "C2", "B3", "C3"]);
    assert_eq!(range("A1:C1").cells().count(), 3);
    assert_eq!(range("A1:A4").cells().count(), 4);
}

#[test]
fn a_range_refuses_a_side_neither_corner_names_and_a_sheet_qualifier() {
    assert_eq!(
        range_refusal(":"),
        "invalid cell reference expression at byte 0: expected a cell range such as A1:C10, \
         A:C, 3:5 or A3:F, got \":\""
    );
    assert_eq!(
        range_refusal("A1:"),
        "invalid cell reference expression at byte 2: expected a cell range such as A1:C10, \
         A:C, 3:5 or A3:F, got \"A1:\""
    );
    assert_eq!(
        range_refusal("Sheet1!A1:B2"),
        "invalid cell reference expression at byte 6: expected a reference within the sheet, \
         got the sheet-qualified \"Sheet1!A1\""
    );
    let past_columns = range_refusal("C3:XFE9");
    assert!(
        past_columns.contains("expected column letters A to XFD in \"XFE9\", got \"XFE\""),
        "{past_columns}"
    );
}

#[test]
fn a_cell_kind_reads_and_writes_the_t_attribute_the_schema_lists() {
    assert_eq!(CellKind::default(), CellKind::Number);
    for (kind, spelled, attribute, is_text) in [
        (CellKind::Number, None, "n", false),
        (CellKind::SharedString, Some("s"), "s", true),
        (CellKind::FormulaString, Some("str"), "str", true),
        (CellKind::InlineString, Some("inlineStr"), "inlineStr", true),
        (CellKind::Boolean, Some("b"), "b", false),
        (CellKind::Date, Some("d"), "d", false),
        (CellKind::Error, Some("e"), "e", false),
    ] {
        assert_eq!(kind.as_str(), spelled, "{kind:?}");
        assert_eq!(
            CellKind::from_attribute(attribute).unwrap(),
            kind,
            "{attribute}"
        );
        assert_eq!(kind.is_text(), is_text, "{kind:?}");
    }
}

#[test]
fn a_cell_kind_refuses_an_attribute_the_schema_does_not_list() {
    assert_eq!(
        CellKind::from_attribute("x").unwrap_err().to_string(),
        "invalid cell type expression at byte 0: expected one of n, s, str, inlineStr, b, d, \
         e for a cell's `t`, got \"x\""
    );
    for refused in ["S", "inlinestr", "N", ""] {
        let refusal = CellKind::from_attribute(refused).unwrap_err().to_string();
        assert!(refusal.ends_with(&format!("got {refused:?}")), "{refusal}");
    }
}

#[test]
fn the_1900_system_is_the_default_and_reads_serials_across_the_phantom_leap_day() {
    assert_eq!(DateSystem::default(), DateSystem::Year1900);
    assert_eq!(DateSystem::Year1900.to_string(), "1900");
    assert_eq!(DateSystem::Year1904.to_string(), "1904");
    let system = DateSystem::Year1900;
    for (serial, date) in [
        (0.0, "1899-12-31"),
        (1.0, "1900-01-01"),
        (59.0, "1900-02-28"),
        (60.0, "1900-02-28"),
        (61.0, "1900-03-01"),
        (25_569.0, "1970-01-01"),
        (45_292.0, "2024-01-01"),
        (2_958_465.0, "9999-12-31"),
    ] {
        assert_eq!(date_of(system, serial), format!("\"{date}\""), "{serial}");
    }
    assert_eq!(system.millis_from_serial(25_569.0).unwrap(), 0);
    assert_eq!(system.millis_from_serial(1.0).unwrap(), -25_567 * DAY);
    assert_eq!(
        system.millis_from_serial(60.0).unwrap(),
        system.millis_from_serial(59.0).unwrap()
    );
}

#[test]
fn the_1904_system_counts_from_1904_01_01_with_no_phantom_day_both_ways() {
    let system = DateSystem::Year1904;
    for (serial, date) in [
        (0.0, "1904-01-01"),
        (1.0, "1904-01-02"),
        (59.0, "1904-02-29"),
        (60.0, "1904-03-01"),
        (61.0, "1904-03-02"),
        (25_569.0, "1974-01-02"),
        (2_957_003.0, "9999-12-31"),
    ] {
        assert_eq!(date_of(system, serial), format!("\"{date}\""), "{serial}");
    }
    assert_eq!(system.serial_from_millis(-24_107 * DAY).unwrap(), 0.0);
    assert_eq!(system.serial_from_millis(-24_048 * DAY).unwrap(), 59.0);
    assert_eq!(system.serial_from_millis(1_462 * DAY).unwrap(), 25_569.0);
    assert_eq!(
        system.serial_from_millis(2_932_896 * DAY).unwrap(),
        2_957_003.0
    );
    assert_eq!(
        system
            .serial_from_millis(-24_108 * DAY)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected an instant the 1904 date system spells, from its \
         first day to 9999-12-31, got 1903-12-31T00:00:00"
    );
}

#[test]
fn writing_a_1900_date_skips_the_phantom_day_and_refuses_a_day_the_system_does_not_spell() {
    let system = DateSystem::Year1900;
    for (days, serial) in [
        (-25_567, 1.0),
        (-25_509, 59.0),
        (-25_508, 61.0),
        (0, 25_569.0),
        (19_723, 45_292.0),
        (2_932_896, 2_958_465.0),
    ] {
        assert_eq!(
            system.serial_from_millis(days * DAY).unwrap(),
            serial,
            "{days}"
        );
    }
    for days in -25_567..=-25_508 {
        assert_ne!(
            system.serial_from_millis(days * DAY).unwrap(),
            60.0,
            "{days}"
        );
    }
    assert_eq!(
        system
            .serial_from_millis(-25_569 * DAY)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected an instant the 1900 date system spells, from its \
         first day to 9999-12-31, got 1899-12-30T00:00:00"
    );
    let past = system
        .serial_from_millis(2_932_897 * DAY)
        .unwrap_err()
        .to_string();
    assert!(
        past.contains(
            "expected an instant the 1900 date system spells, from its first day to 9999-12-31"
        ),
        "{past}"
    );
}

#[test]
fn every_serial_but_the_1900_phantom_day_round_trips() {
    let system = DateSystem::Year1900;
    for serial in (1..=200_u32).chain([25_569, 45_292, 2_958_465]) {
        let serial = f64::from(serial);
        let millis = system.millis_from_serial(serial).unwrap();
        let written = system.serial_from_millis(millis).unwrap();
        if serial == 60.0 {
            assert_eq!(written, 59.0);
        } else {
            assert_eq!(written, serial);
        }
    }
    let system = DateSystem::Year1904;
    for serial in (0..=200_u32).chain([25_569, 2_957_003]) {
        let serial = f64::from(serial);
        let millis = system.millis_from_serial(serial).unwrap();
        assert_eq!(system.serial_from_millis(millis).unwrap(), serial);
    }
}

#[test]
fn a_serial_outside_the_system_is_refused_naming_the_systems_range() {
    let system = DateSystem::Year1900;
    for (serial, spelled) in [
        (-1.0, "-1"),
        (-0.5, "-0.5"),
        (2_958_466.0, "2958466"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "inf"),
    ] {
        assert_eq!(
            system.millis_from_serial(serial).unwrap_err().to_string(),
            format!(
                "invalid record value at $: expected a serial date between 0 and 2958465 in the \
                 1900 date system, got {spelled}"
            )
        );
    }
    assert_eq!(
        DateSystem::Year1904
            .millis_from_serial(2_957_004.0)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a serial date between 0 and 2957003 in the 1904 \
         date system, got 2957004"
    );
}

#[test]
fn a_serial_fraction_is_the_clock_read_at_the_millisecond() {
    let system = DateSystem::Year1900;
    let new_year = 19_723 * DAY;
    assert_eq!(
        system.millis_from_serial(45_292.5).unwrap(),
        new_year + 43_200_000
    );
    assert_eq!(
        system
            .millis_from_serial(45_292.0 + 1.0 / 86_400.0)
            .unwrap(),
        new_year + 1_000
    );
    assert_eq!(
        system.millis_from_serial(2_958_465.999).unwrap(),
        2_932_896 * DAY + 86_313_600
    );
    assert_eq!(
        system.millis_from_serial(59.5).unwrap(),
        -25_509 * DAY + 43_200_000
    );
    assert_eq!(
        system.millis_from_serial(60.5).unwrap(),
        -25_509 * DAY + 43_200_000
    );

    let instant = new_year + 43_200_123;
    let serial = system.serial_from_millis(instant).unwrap();
    assert_eq!(serial, 45_292.0 + 43_200_123.0 / 86_400_000.0);
    assert_eq!(system.millis_from_serial(serial).unwrap(), instant);
}

#[test]
fn days_from_serial_refuses_a_clock_a_date_cannot_hold() {
    let system = DateSystem::Year1900;
    assert_eq!(system.days_from_serial(45_292.0).unwrap(), 19_723);
    assert_eq!(
        system.days_from_serial(45_292.5).unwrap_err().to_string(),
        "invalid record value at $: expected a whole day for a date, got serial 45292.5"
    );
    assert_eq!(
        system.days_from_serial(-1.0).unwrap_err().to_string(),
        "invalid record value at $: expected a serial date between 0 and 2958465 in the 1900 \
         date system, got -1"
    );
    assert_eq!(DateSystem::Year1904.days_from_serial(0.0).unwrap(), -24_107);
}

#[test]
fn scalar_from_serial_reads_the_value_each_number_format_spells() {
    let system = DateSystem::Year1900;
    let new_year = 19_723 * DAY;
    let read =
        |serial: f64, format: NumberFormat| system.scalar_from_serial(serial, format).unwrap();

    assert_eq!(
        read(45_292.5, NumberFormat::General),
        Scalar::from(45_292.5)
    );
    assert_eq!(read(-0.5, NumberFormat::General), Scalar::from(-0.5));
    assert_eq!(read(45_292.0, NumberFormat::Date), Scalar::date32(19_723));
    assert_eq!(
        read(45_292.5, NumberFormat::Date),
        naive_millis(new_year + 43_200_000)
    );
    assert_eq!(
        read(45_292.0, NumberFormat::DateTime),
        naive_millis(new_year)
    );
    assert_eq!(
        read(45_292.5, NumberFormat::DateTimeFraction),
        naive_millis(new_year + 43_200_000)
    );
    assert_eq!(
        read(0.5, NumberFormat::Time),
        Scalar::time32(43_200_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
    );
    assert_eq!(
        read(1.5, NumberFormat::Time).into_json().unwrap(),
        "\"1900-01-01T12:00:00\""
    );
    assert_eq!(
        read(1.5, NumberFormat::Duration),
        Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap()
    );
    assert_eq!(
        read(-0.5, NumberFormat::Duration),
        Scalar::duration64(-43_200_000, TimeUnit::Millisecond).unwrap()
    );
    assert_eq!(
        system
            .scalar_from_serial(-1.0, NumberFormat::Date)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a serial date between 0 and 2958465 in the 1900 \
         date system, got -1"
    );
}

#[test]
fn temporal_from_serial_reads_a_serial_in_the_family_the_declared_leaf_names() {
    let system = DateSystem::Year1900;
    let new_year = 19_723 * DAY;
    for (dtype, serial, millis) in [
        (DataType::Date32, 45_292.0, new_year),
        (DataType::Date64, 45_292.0, new_year),
        (DataType::time32(TimeUnit::Second).unwrap(), 0.5, 43_200_000),
        (
            DataType::time32(TimeUnit::Millisecond).unwrap(),
            0.25,
            21_600_000,
        ),
        (
            DataType::time64(TimeUnit::Microsecond).unwrap(),
            0.5,
            43_200_000,
        ),
        (
            DataType::time64(TimeUnit::Nanosecond).unwrap(),
            0.5,
            43_200_000,
        ),
        (
            DataType::datetime64(TimeUnit::Second, Timezone::NAIVE).unwrap(),
            45_292.5,
            new_year + 43_200_000,
        ),
        (
            DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            45_292.5,
            new_year + 43_200_000,
        ),
        (
            DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            45_292.0,
            new_year,
        ),
        (
            DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            45_292.5,
            new_year + 43_200_000,
        ),
        (
            DataType::duration64(TimeUnit::Millisecond).unwrap(),
            1.5,
            129_600_000,
        ),
        (
            DataType::duration64(TimeUnit::Nanosecond).unwrap(),
            1.5,
            129_600_000,
        ),
    ] {
        let value = system.temporal_from_serial(serial, &dtype).unwrap();
        assert_eq!(
            value.id().temporal_family(),
            dtype.id().temporal_family(),
            "{dtype}"
        );
        assert_eq!(
            value.temporal_count_at(TimeUnit::Millisecond),
            Some(millis),
            "{dtype}"
        );
    }
    assert_eq!(
        system
            .temporal_from_serial(45_292.5, &DataType::Date32)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a whole day for a date, got serial 45292.5"
    );
    let time = DataType::time64(TimeUnit::Microsecond).unwrap();
    assert_eq!(
        system
            .temporal_from_serial(1.5, &time)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a serial below one day for a time, got 1.5"
    );
    let datetime = DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE).unwrap();
    assert_eq!(
        system
            .temporal_from_serial(-1.0, &datetime)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a serial date between 0 and 2958465 in the 1900 \
         date system, got -1"
    );
}

#[test]
fn serial_of_writes_each_naive_temporal_with_the_format_that_reads_it_back() {
    let system = DateSystem::Year1900;
    let noon = 1_704_110_400;
    for (value, serial, format) in [
        (Scalar::date32(19_723), 45_292.0, NumberFormat::Date),
        (Scalar::date64(19_723 * DAY), 45_292.0, NumberFormat::Date),
        (
            Scalar::datetime64(noon, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            45_292.5,
            NumberFormat::DateTime,
        ),
        (
            naive_millis(noon * 1_000),
            45_292.5,
            NumberFormat::DateTimeFraction,
        ),
        (
            Scalar::datetime64(noon * 1_000_000, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            45_292.5,
            NumberFormat::DateTimeFraction,
        ),
        (
            Scalar::datetime64(noon * 1_000_000_000, TimeUnit::Nanosecond, Timezone::NAIVE)
                .unwrap(),
            45_292.5,
            NumberFormat::DateTimeFraction,
        ),
        (
            Scalar::time32(43_200, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            0.5,
            NumberFormat::Time,
        ),
        (
            Scalar::time64(43_200_000_000, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            0.5,
            NumberFormat::Time,
        ),
        (
            Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap(),
            1.5,
            NumberFormat::Duration,
        ),
        (
            Scalar::duration32(129_600, TimeUnit::Second).unwrap(),
            1.5,
            NumberFormat::Duration,
        ),
    ] {
        assert_eq!(
            system.serial_of(&value).unwrap(),
            Some((serial, format)),
            "{}",
            value.into_json().unwrap()
        );
    }
    for text in [
        Scalar::from("2024-01-01"),
        Scalar::from(45_292.5),
        Scalar::from(true),
        Scalar::Null,
        Scalar::datetime64(noon, TimeUnit::Second, Timezone::UTC).unwrap(),
    ] {
        assert_eq!(
            system.serial_of(&text).unwrap(),
            None,
            "{}",
            text.into_json().unwrap()
        );
    }
    assert_eq!(
        DateSystem::Year1904
            .serial_of(&Scalar::date32(19_723))
            .unwrap(),
        Some((43_830.0, NumberFormat::Date))
    );
    assert_eq!(
        system
            .serial_of(&Scalar::date32(-25_569))
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected an instant the 1900 date system spells, from its \
         first day to 9999-12-31, got 1899-12-30T00:00:00"
    );
}

#[test]
fn a_cell_from_a_value_states_the_kind_and_format_it_writes_as_and_keeps_the_value() {
    let noon = 1_704_110_400;
    for (value, kind, format, text) in [
        (
            Scalar::from("AAPL"),
            CellKind::SharedString,
            NumberFormat::General,
            "AAPL",
        ),
        (
            Scalar::from("a<&>b"),
            CellKind::SharedString,
            NumberFormat::General,
            "a<&>b",
        ),
        (
            Scalar::from(187.23),
            CellKind::Number,
            NumberFormat::General,
            "187.23",
        ),
        (
            Scalar::from(1.0),
            CellKind::Number,
            NumberFormat::General,
            "1",
        ),
        (
            Scalar::from(42_i64),
            CellKind::Number,
            NumberFormat::General,
            "42",
        ),
        (
            Scalar::from(true),
            CellKind::Boolean,
            NumberFormat::General,
            "true",
        ),
        (
            Scalar::from(false),
            CellKind::Boolean,
            NumberFormat::General,
            "false",
        ),
        (
            Scalar::date32(19_723),
            CellKind::Number,
            NumberFormat::Date,
            "2024-01-01",
        ),
        (
            Scalar::datetime64(noon, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            CellKind::Number,
            NumberFormat::DateTime,
            "2024-01-01T12:00:00",
        ),
        (
            naive_millis(noon * 1_000 + 123),
            CellKind::Number,
            NumberFormat::DateTimeFraction,
            "2024-01-01T12:00:00.123",
        ),
        (
            Scalar::time32(43_200, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            CellKind::Number,
            NumberFormat::Time,
            "12:00:00",
        ),
        (
            Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap(),
            CellKind::Number,
            NumberFormat::Duration,
            "PT129600S",
        ),
        (
            Scalar::datetime64(noon, TimeUnit::Second, Timezone::UTC).unwrap(),
            CellKind::SharedString,
            NumberFormat::General,
            "2024-01-01T12:00:00Z",
        ),
        (
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            CellKind::SharedString,
            NumberFormat::General,
            "[1,\"AAPL\"]",
        ),
        (Scalar::Null, CellKind::Number, NumberFormat::General, ""),
    ] {
        let cell = Cell::from_scalar(reference("B2"), value.clone(), DateSystem::Year1900).unwrap();
        assert_eq!(cell.kind(), kind, "{text}");
        assert_eq!(cell.format(), format, "{text}");
        assert_eq!(cell.value(), &value, "{text}");
        assert_eq!(cell.text(), text);
        assert_eq!(cell.is_null(), value == Scalar::Null, "{text}");
        assert_eq!((cell.formula(), cell.error()), (None, None), "{text}");
        assert_eq!(cell.reference(), reference("B2"));
    }
}

#[test]
fn a_float_that_is_not_finite_is_the_num_error_cell() {
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let cell =
            Cell::from_scalar(reference("C4"), Scalar::from(number), DateSystem::Year1900).unwrap();
        assert_eq!(cell.kind(), CellKind::Error, "{number}");
        assert_eq!(cell.format(), NumberFormat::General, "{number}");
        assert_eq!(cell.error(), Some("#NUM!"), "{number}");
        assert_eq!(cell.value(), &Scalar::Null, "{number}");
        assert!(cell.is_null(), "{number}");
        assert_eq!(cell.text(), "#NUM!", "{number}");
    }
}

#[test]
fn a_cell_refuses_text_past_the_character_limit_and_an_instant_the_system_does_not_spell() {
    assert_eq!(MAX_CELL_TEXT, 32_767);
    let widest = "é".repeat(MAX_CELL_TEXT);
    let cell = Cell::from_scalar(
        reference("B2"),
        Scalar::from(widest.as_str()),
        DateSystem::Year1900,
    )
    .unwrap();
    assert_eq!(cell.kind(), CellKind::SharedString);
    assert_eq!(cell.text().chars().count(), MAX_CELL_TEXT);

    let longer = "x".repeat(MAX_CELL_TEXT + 1);
    assert_eq!(
        Cell::from_scalar(
            reference("B2"),
            Scalar::from(longer.as_str()),
            DateSystem::Year1900
        )
        .unwrap_err()
        .to_string(),
        "invalid record value at B2: expected at most 32767 characters in a cell, got 32768"
    );
    assert_eq!(
        Cell::from_scalar(
            reference("B2"),
            Scalar::date32(-25_569),
            DateSystem::Year1900
        )
        .unwrap_err()
        .to_string(),
        "invalid record value at B2: expected an instant the 1900 date system spells, \
         from its first day to 9999-12-31, got 1899-12-30T00:00:00"
    );
    assert_eq!(
        Cell::from_scalar(
            reference("D7"),
            Scalar::date32(-24_108),
            DateSystem::Year1904
        )
        .unwrap_err()
        .to_string(),
        "invalid record value at D7: expected an instant the 1904 date system spells, \
         from its first day to 9999-12-31, got 1903-12-31T00:00:00"
    );
    let first = Cell::from_scalar(
        reference("D7"),
        Scalar::date32(-24_107),
        DateSystem::Year1904,
    )
    .unwrap();
    assert_eq!(
        (first.kind(), first.format()),
        (CellKind::Number, NumberFormat::Date)
    );
}

#[test]
fn a_cell_states_its_facts_and_each_with_restates_one_of_them() {
    let stated = Cell::new(
        reference("B2"),
        CellKind::InlineString,
        NumberFormat::General,
        Scalar::from("AAPL"),
    );
    assert_eq!((stated.row(), stated.column()), (1, 1));
    assert_eq!(stated.kind(), CellKind::InlineString);
    assert_eq!((stated.formula(), stated.error()), (None, None));
    assert!(!stated.is_null());
    assert_eq!(stated.text(), "AAPL");
    let empty = Cell::new(
        reference("B2"),
        CellKind::Number,
        NumberFormat::General,
        Scalar::Null,
    );
    assert!(empty.is_null());
    assert_eq!(empty.text(), "");

    let summed = Cell::from_scalar(reference("A3"), Scalar::from(3.0), DateSystem::Year1900)
        .unwrap()
        .with_formula("SUM(A1:A2)");
    assert_eq!(summed.formula(), Some("SUM(A1:A2)"));
    assert_eq!(
        (summed.kind(), summed.value()),
        (CellKind::Number, &Scalar::from(3.0))
    );
    assert_eq!(summed.text(), "3");

    let failed = summed.with_error("#DIV/0!");
    assert_eq!(failed.kind(), CellKind::Error);
    assert_eq!(failed.value(), &Scalar::Null);
    assert!(failed.is_null());
    assert_eq!(failed.error(), Some("#DIV/0!"));
    assert_eq!(failed.text(), "#DIV/0!");
    assert_eq!(failed.formula(), Some("SUM(A1:A2)"));
    assert_eq!(failed.format(), NumberFormat::General);

    let moved = stated.at(CellRef::new(4, 27));
    assert_eq!(moved.reference().to_string(), "AB5");
    assert_eq!((moved.row(), moved.column()), (4, 27));
    assert_eq!(moved.kind(), CellKind::InlineString);
    assert_eq!(moved.into_scalar(), Scalar::from("AAPL"));
}

//! `rust/src/excel/table.rs`: authoritative table identity, row bands and columns.

use crate::excel_package::{named_table_package, named_table_parts};
use yggdryl::excel::{Excel, ExcelOptions};
use yggdryl::holder::Buffer;
use yggdryl::{Error, IOMedia, MimeType};

#[test]
fn named_table_st_xstring_names_decode_before_lookup_and_duplicate_checks() {
    let mut parts = named_table_parts();
    let xml = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *xml = xml
        .replace("displayName=\"Names\"", "displayName=\"Na_x006d_es\"")
        .replace("name=\"id\"", "name=\"Region_x000a_Year\"")
        .replace("name=\"name\"", "name=\"Notes_x000d__x000a_Detail\"");
    let media = Excel::new(
        Buffer::from_bytes(named_table_package(&parts)).with_media_type(MimeType::XLSX.into()),
    )
    .with_options(ExcelOptions::new().with_table("Names"));
    let options = media.record_options().unwrap();
    let field = media.read_arrow_field(&options).unwrap();
    assert_eq!(
        field
            .fields()
            .iter()
            .map(yggdryl::Field::name)
            .collect::<Vec<_>>(),
        ["Region\nYear", "Notes\r\nDetail"]
    );

    let mut fallback_name = named_table_parts();
    let xml = &mut fallback_name
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *xml = xml
        .replace(" displayName=\"Names\"", "")
        .replace("name=\"Names\"", "name=\"Na_x006d_es\"");
    let media = Excel::new(
        Buffer::from_bytes(named_table_package(&fallback_name))
            .with_media_type(MimeType::XLSX.into()),
    )
    .with_options(ExcelOptions::new().with_table("Names"));
    assert_eq!(media.column_size().unwrap(), 2);

    let mut literal = named_table_parts();
    let xml = &mut literal
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *xml = xml.replace("name=\"id\"", "name=\"_x005F_x000A_\"");
    let media = Excel::new(
        Buffer::from_bytes(named_table_package(&literal)).with_media_type(MimeType::XLSX.into()),
    )
    .with_options(ExcelOptions::new().with_table("Names"));
    let options = media.record_options().unwrap();
    assert_eq!(
        media.read_arrow_field(&options).unwrap().fields()[0].name(),
        "_x000A_"
    );

    let mut duplicate_column = named_table_parts();
    let xml = &mut duplicate_column
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *xml = xml
        .replace("name=\"id\"", "name=\"A_x000a_B\"")
        .replace("name=\"name\"", "name=\"A&#10;B\"");
    let media = Excel::new(
        Buffer::from_bytes(named_table_package(&duplicate_column))
            .with_media_type(MimeType::XLSX.into()),
    )
    .with_options(ExcelOptions::new().with_table("Names"));
    match media
        .read_arrow_field(&media.record_options().unwrap())
        .unwrap_err()
    {
        Error::InvalidRecord { path, reason } => {
            assert!(path.contains("#tableColumn[2]@name"), "{path}");
            assert!(reason.contains("distinct"), "{reason}");
        }
        error => panic!("expected duplicate decoded column refusal, got {error:?}"),
    }

    let mut duplicate_table = named_table_parts();
    let first = &mut duplicate_table
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *first = first.replace("displayName=\"Names\"", "displayName=\"Na_x006d_es\"");
    let second = &mut duplicate_table
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table2.xml")
        .unwrap()
        .1;
    *second = second.replace("displayName=\"Quantities\"", "displayName=\"Names\"");
    let media = Excel::new(
        Buffer::from_bytes(named_table_package(&duplicate_table))
            .with_media_type(MimeType::XLSX.into()),
    )
    .with_options(ExcelOptions::new().with_table("Names"));
    match media
        .read_arrow_field(&media.record_options().unwrap())
        .unwrap_err()
    {
        Error::InvalidRecord { path, reason } => {
            assert_eq!(path, "$.table");
            assert!(reason.contains("unique table name"), "{reason}");
        }
        error => panic!("expected duplicate decoded table refusal, got {error:?}"),
    }

    let mut unnamed = named_table_parts();
    let xml = &mut unnamed
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *xml = xml.replace(" name=\"Names\" displayName=\"Names\"", "");
    let media = Excel::new(
        Buffer::from_bytes(named_table_package(&unnamed)).with_media_type(MimeType::XLSX.into()),
    )
    .with_options(ExcelOptions::new().with_table("Names"));
    match media.column_size().unwrap_err() {
        Error::InvalidRecord { path, reason } => {
            assert_eq!(path, "xl/tables/table1.xml");
            assert!(
                reason.contains("nonempty table displayName or name"),
                "{reason}"
            );
        }
        error => panic!("expected unnamed table refusal, got {error:?}"),
    }
}

#[test]
fn named_table_metadata_accepts_strict_encoded_and_prefixed_namespaces() {
    for namespace in [yggdryl::excel::NAMESPACE, yggdryl::excel::STRICT_NAMESPACE] {
        for prefixed in [false, true] {
            let mut parts = named_table_parts();
            let xml = &mut parts
                .iter_mut()
                .find(|(name, _)| *name == "xl/tables/table1.xml")
                .unwrap()
                .1;
            *xml = xml.replace(
                yggdryl::excel::NAMESPACE,
                &namespace.replace("http", "h&#116;tp"),
            );
            if prefixed {
                *xml = xml.replace("xmlns=", "xmlns:t=");
                for element in ["table", "tableColumns", "tableColumn"] {
                    *xml = xml
                        .replace(&format!("<{element} "), &format!("<t:{element} "))
                        .replace(&format!("</{element}>"), &format!("</t:{element}>"));
                }
            }
            *xml = xml.replace(" count=\"2\"", ""); // Optional OOXML count.
            let media = Excel::new(
                Buffer::from_bytes(named_table_package(&parts))
                    .with_media_type(MimeType::XLSX.into()),
            )
            .with_options(ExcelOptions::new().with_table("Names"));
            assert_eq!(media.column_size().unwrap(), 2);
            assert_eq!(media.row_size().unwrap(), 2);
        }
    }
}

#[test]
fn named_table_metadata_refuses_inconsistent_bands_count_and_names() {
    for (old, new, location) in [
        (
            "ref=\"A1:B3\"",
            "ref=\"A1:B3\" headerRowCount=\"3\" totalsRowCount=\"1\"",
            "xl/tables/table1.xml",
        ),
        ("count=\"2\"", "count=\"1\"", "#tableColumns@count"),
        ("count=\"2\"", "count=\"bad\"", "#tableColumns@count"),
        ("name=\"name\"", "name=\"\"", "#tableColumn[2]@name"),
        ("name=\"name\"", "name=\"ID\"", "#tableColumn[2]@name"),
        ("<tableColumn id=\"2\" name=\"name\"/>", "", "#tableColumns"),
        (
            "<tableColumn id=\"2\" name=\"name\"/>",
            "<q:tableColumn xmlns:q=\"urn:other\" id=\"2\" name=\"name\"/>",
            "#tableColumns",
        ),
    ] {
        let mut parts = named_table_parts();
        let xml = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/tables/table1.xml")
            .unwrap()
            .1;
        assert!(xml.contains(old));
        *xml = xml.replace(old, new);
        let media = Excel::new(
            Buffer::from_bytes(named_table_package(&parts)).with_media_type(MimeType::XLSX.into()),
        )
        .with_options(ExcelOptions::new().with_table("Names"));
        match media.column_size().unwrap_err() {
            Error::InvalidRecord { path, .. } => assert!(path.contains(location), "{old}: {path}"),
            error => panic!("expected a located table refusal, got {error:?}"),
        }
    }
}

#[test]
fn named_table_metadata_refuses_duplicate_authoritative_attributes() {
    for (old, new, location) in [
        ("ref=\"A1:B3\"", "ref=\"A1:B3\" ref=\"D1:E4\"", "#ref"),
        (
            "count=\"2\"",
            "count=\"2\" count=\"3\"",
            "#tableColumns@count",
        ),
        (
            "name=\"name\"",
            "name=\"name\" name=\"other\"",
            "#tableColumn[2]@name",
        ),
    ] {
        let mut parts = named_table_parts();
        let xml = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/tables/table1.xml")
            .unwrap()
            .1;
        *xml = xml.replace(old, new);
        let media = Excel::new(
            Buffer::from_bytes(named_table_package(&parts)).with_media_type(MimeType::XLSX.into()),
        )
        .with_options(ExcelOptions::new().with_table("Names"));
        match media.column_size().unwrap_err() {
            Error::InvalidRecord { path, reason } => {
                assert!(path.contains(location), "{path}");
                assert!(reason.contains("duplicate"), "{reason}");
            }
            error => panic!("expected an ambiguous-attribute refusal, got {error:?}"),
        }
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::excel_table::resized_part;

    // CellRange::Display shortens full-width A1:XFD3 to 1:3, while table
    // ref and filter ref must retain both A1 cell corners on the wire.
    #[test]
    fn whole_width_resize_writes_full_cell_corners() {
        let old = br#"<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Wide" displayName="Wide" ref="A1:XFD2"><autoFilter ref="A1:XFD2"/><tableColumns count="1"><tableColumn id="1" name="Any"/></tableColumns></table>"#;
        let output = resized_part(old, 3).unwrap();
        let xml = std::str::from_utf8(&output).unwrap();
        assert_eq!(xml.matches("ref=\"A1:XFD3\"").count(), 2, "{xml}");
        assert!(!xml.contains("ref=\"1:3\""), "{xml}");
    }
}

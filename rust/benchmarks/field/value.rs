use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::{DataType, Field, MediaType, Metadata, MimeType, Scheme, Url};

use super::nested_field;

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("value");
    let field = nested_field();

    group.bench_function("nested_field_clone", |bencher| {
        bencher.iter(|| black_box(&field).clone());
    });
    group.bench_function("field_stable_hash", |bencher| {
        bencher.iter(|| black_box(&field).stable_hash());
    });
    group.bench_function("metadata_hit", |bencher| {
        bencher.iter(|| black_box(&field).get_metadata(black_box("source")));
    });
    group.bench_function("metadata_miss", |bencher| {
        bencher.iter(|| black_box(&field).get_metadata(black_box("missing")));
    });
    let property_field = field
        .clone()
        .try_with_property(&Scheme::POSTGRES, "table", "trades")
        .expect("the static protocol property is valid")
        .with_parquet_field_id(17)
        .with_location(
            Url::from_str("https://example.com/catalog/trades")
                .expect("the static location is valid"),
        );
    group.bench_function("protocol_property_hit", |bencher| {
        bencher.iter(|| {
            black_box(&property_field)
                .get_property(black_box(&Scheme::POSTGRES), black_box("table"))
        });
    });
    let wide_property_field = Field::from_parts(
        "value",
        DataType::Utf8,
        true,
        (0..1_024)
            .map(|index| (format!("key-{index:04}"), index.to_string()))
            .chain(std::iter::once((
                "postgres:table".to_owned(),
                "trades".to_owned(),
            ))),
    )
    .expect("the generated metadata is valid");
    group.bench_function("protocol_property_hit_wide", |bencher| {
        bencher.iter(|| {
            black_box(&wide_property_field)
                .get_property(black_box(&Scheme::POSTGRES), black_box("table"))
        });
    });
    let protocol_properties = Field::from_parts(
        "value",
        DataType::Utf8,
        true,
        (0..1_024).map(|index| (format!("postgres:key-{index:04}"), index.to_string())),
    )
    .expect("the generated protocol metadata is valid");
    group.bench_function("protocol_property_iter_1024", |bencher| {
        bencher.iter(|| {
            black_box(&protocol_properties)
                .property_iter(black_box(&Scheme::POSTGRES))
                .for_each(|entry| {
                    black_box(entry);
                });
        });
    });
    group.bench_function("protocol_property_cursor_1024", |bencher| {
        bencher.iter(|| {
            let mut after = None;
            while let Some((name, value)) = black_box(&protocol_properties)
                .next_property_entry(black_box(&Scheme::POSTGRES), after)
            {
                black_box(value);
                after = Some(name);
            }
        });
    });
    group.bench_function("protocol_view_hit", |bencher| {
        bencher.iter(|| {
            black_box(&property_field)
                .as_postgres()
                .get(black_box("table"))
        });
    });
    group.bench_function("protocol_view_hit_wide", |bencher| {
        bencher.iter(|| {
            black_box(&wide_property_field)
                .as_postgres()
                .get(black_box("table"))
        });
    });
    group.bench_function("protocol_view_len_1024", |bencher| {
        bencher.iter(|| black_box(&protocol_properties).as_postgres().len());
    });
    // The view is built per call, so what a caller pays for one is the `Scheme`
    // clone plus the snapshot borrow, with no map walk behind either.
    group.bench_function("protocol_view_as_properties", |bencher| {
        bencher.iter(|| black_box(&property_field).as_postgres().as_properties());
    });
    let partitioned = Field::new(
        "row",
        DataType::from_fields((0..64).map(|index| {
            let column = DataType::Int64.required_field(format!("column-{index:02}"));
            if index % 8 == 0 {
                column.with_partition(true)
            } else {
                column
            }
        }))
        .expect("the generated columns are unique"),
        false,
    );
    group.bench_function("partition_field_names_64", |bencher| {
        bencher.iter(|| {
            black_box(&partitioned)
                .partition_field_names()
                .for_each(|name| {
                    black_box(name);
                });
        });
    });
    group.bench_function("without_partition_fields_64", |bencher| {
        bencher.iter(|| {
            black_box(&partitioned)
                .without_partition_fields()
                .expect("the generated root subtracts its marked columns")
        });
    });
    group.bench_function("typed_location", |bencher| {
        bencher.iter(|| {
            black_box(&property_field)
                .location()
                .expect("validated location metadata remains valid")
        });
    });
    group.bench_function("typed_field_id", |bencher| {
        bencher.iter(|| {
            black_box(&property_field)
                .parquet_field_id()
                .expect("validated field ID metadata remains valid")
        });
    });
    let media = MediaType::from_parts(
        MimeType::JSON,
        [MimeType::GZIP, MimeType::BROTLI, MimeType::ZSTD],
    )
    .expect("the static media type is valid");
    let mut http_field = Field::from_parts(
        "payload",
        DataType::Binary,
        false,
        [
            ("http:content-type", "application/json"),
            ("http:content-encoding", "gzip, br, zstd"),
            ("http:content-length", "18446744073709551615"),
        ],
    )
    .expect("the static HTTP metadata is valid");
    http_field
        .clone()
        .into_arrow_ref()
        .expect("the static HTTP field projects to Arrow");
    group.bench_function("http_content_type_exact", |bencher| {
        bencher.iter(|| black_box(&http_field).as_http().content_type());
    });
    group.bench_function("http_content_length_typed", |bencher| {
        bencher.iter(|| {
            black_box(&http_field)
                .as_http()
                .content_length()
                .expect("the static content length remains valid")
        });
    });
    group.bench_function("http_media_type_typed", |bencher| {
        bencher.iter(|| {
            black_box(&http_field)
                .as_http()
                .media_type()
                .expect("the static HTTP media headers remain valid")
        });
    });
    group.bench_function("http_metadata_hit_noncanonical_https", |bencher| {
        bencher.iter(|| black_box(&http_field).get_metadata(black_box("HTTPS:CONTENT-TYPE")));
    });
    group.bench_function("http_media_type_set_noop", |bencher| {
        bencher.iter(|| {
            http_field
                .as_http_mut()
                .set_media_type(black_box(media.clone()))
                .expect("the static media type remains valid");
        });
    });
    let changed_media = MediaType::from_parts(MimeType::CSV, [MimeType::COMPRESS])
        .expect("the changed static media type is valid");
    group.bench_function("http_media_type_set_changed", |bencher| {
        bencher.iter_batched(
            || http_field.clone(),
            |mut field| {
                field
                    .as_http_mut()
                    .set_media_type(black_box(changed_media.clone()))
                    .expect("the changed media type remains valid");
                black_box(field)
            },
            BatchSize::SmallInput,
        );
    });
    #[cfg(feature = "iceberg")]
    {
        use yggdryl::media::iceberg::Transform;

        let mut iceberg_field = DataType::Int64.required_field("id");
        let mut view = iceberg_field.as_iceberg_mut();
        view.set_schema_id(3)
            .expect("the static schema identifier is valid");
        view.set_identifier_field_ids(&[1, 2, 3])
            .expect("the static identifier columns are valid");
        view.set_doc("row identifier")
            .expect("the static doc string is valid");
        view.set_spec_id(7)
            .expect("the static spec identifier is valid");
        view.set_partition_source_id(11)
            .expect("the static source column is valid");
        view.set_transform(&Transform::Identity)
            .expect("the static transform is valid");
        iceberg_field
            .clone()
            .into_arrow_ref()
            .expect("the static Iceberg field projects to Arrow");

        group.bench_function("iceberg_doc_exact", |bencher| {
            bencher.iter(|| black_box(&iceberg_field).as_iceberg().doc());
        });
        group.bench_function("iceberg_schema_id_typed", |bencher| {
            bencher.iter(|| {
                black_box(&iceberg_field)
                    .as_iceberg()
                    .schema_id()
                    .expect("the static schema identifier remains valid")
            });
        });
        // The one read whose assembled key outgrows the inline key buffer, so
        // it is the pair that says what that boundary costs.
        group.bench_function("iceberg_partition_source_id_typed", |bencher| {
            bencher.iter(|| {
                black_box(&iceberg_field)
                    .as_iceberg()
                    .partition_source_id()
                    .expect("the static source column remains valid")
            });
        });
        group.bench_function("iceberg_spec_id_typed", |bencher| {
            bencher.iter(|| {
                black_box(&iceberg_field)
                    .as_iceberg()
                    .spec_id()
                    .expect("the static spec identifier remains valid")
            });
        });
        group.bench_function("iceberg_transform_typed", |bencher| {
            bencher.iter(|| {
                black_box(&iceberg_field)
                    .as_iceberg()
                    .transform()
                    .expect("the static transform remains valid")
            });
        });
        group.bench_function("iceberg_identifier_field_ids_typed", |bencher| {
            bencher.iter(|| {
                black_box(&iceberg_field)
                    .as_iceberg()
                    .identifier_field_ids()
                    .expect("the static identifier columns remain valid")
            });
        });
        group.bench_function("iceberg_doc_set_noop", |bencher| {
            bencher.iter(|| {
                iceberg_field
                    .as_iceberg_mut()
                    .set_doc(black_box("row identifier"))
                    .expect("the identical doc string remains valid");
            });
        });
        group.bench_function("iceberg_doc_set_changed", |bencher| {
            bencher.iter_batched(
                || iceberg_field.clone(),
                |mut field| {
                    field
                        .as_iceberg_mut()
                        .set_doc(black_box("the row identifier"))
                        .expect("the replacement doc string is valid");
                    black_box(field)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.bench_function("metadata_into_arrow_unique", |bencher| {
        bencher.iter_batched(
            || {
                Metadata::from_entries([("comment", "analytics"), ("postgres:table", "trades")])
                    .expect("the static metadata is valid")
            },
            |metadata| black_box(metadata.into_arrow()),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("metadata_overlay_32", |bencher| {
        bencher.iter_batched(
            || Field::new("value", DataType::Utf8, true),
            |mut field| {
                field
                    .update_metadata(
                        (0..32)
                            .map(|index| (format!("key-{index:02}"), format!("value-{index:02}"))),
                    )
                    .expect("generated metadata keys are valid");
                black_box(field)
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("metadata_overlay_noop_32", |bencher| {
        let pairs = (0..32)
            .map(|index| (format!("key-{index:02}"), format!("value-{index:02}")))
            .collect::<Vec<_>>();
        let mut field = Field::new("value", DataType::Utf8, true);
        field
            .update_metadata(
                pairs
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .expect("generated metadata keys are valid");
        bencher.iter_batched(
            || field.clone(),
            |mut field| {
                field
                    .update_metadata(
                        pairs
                            .iter()
                            .map(|(key, value)| (key.as_str(), value.as_str())),
                    )
                    .expect("the identical metadata overlay remains valid");
                black_box(field)
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

# Scalar row streams

`StreamSerie` is a native stream of `Scalar` record runs under one `Field`.
It pulls one row at a time, without first landing an Arrow batch. Python and
JavaScript iterate native `Scalar` values; record-mapping adapters remain at
`read_records` / `readRecords` and explicit Arrow exports.

`into_stream` / `intoStream` is available on every serie kind. A primitive column
is wrapped as the one child of a record. A schema-free run needs an explicit
field before entering a record stream. A failure follows its successful prefix
once, then the iterator is fused.

Python `StreamSerie.from_rows(field, rows)` retains a lazy Python producer.
`from_arrow_reader` / `fromArrowReader` adapts a declared Arrow stream. Python
`collect()` and JavaScript `collect()` consume the remaining native scalar rows.
Rust `collect_rows()` is the corresponding collecting door.

For adjacent windows and distinct-key partitions, see [Key series](key-serie.md).
For optional row and byte bounds, see [Chunk streams](stream-chunked-serie.md).
`into_arrow_reader` / `intoArrowReader` is the explicit Arrow boundary.

# Native chunk streams

`StreamChunkedSerie` yields generic `Serie` chunks under a non-null record field.
Native columnar sources keep their chunks and buffers. Row sources are landed
only when a chunk or an Arrow consumer asks for them.

## Bounds

Every serie kind answers `into_chunked_stream(row_size, byte_size)` in Rust,
`into_chunked_stream(row_size=None, byte_size=None)` in Python and
`intoChunkedStream(rowSize?, byteSize?)` in JavaScript. Omission and `None` / `null`
mean that axis was not given. With both absent, the byte bound is the smaller of
64 MiB and the process spill bound. One stated axis bounds alone; both stop at
whichever is reached first. A bound admits at least one row.

Pieces are joined through the native chunked implementation. A single incoming
chunk already larger than the bound passes through unchanged. A key kind never
joins across its items. There is no implicit 65,536-row prebatching ceiling.

## Arrow and iteration

Python exposes `schema`, `read_next_batch()`, `read_all()` and
`__arrow_c_stream__`, as well as native chunk iteration. JavaScript exposes
`schema`, `readNextBatch()`, `readAll()` and native iteration. Arrow exports use
the same source and field. Consuming conversions spend the stream once.

`from_arrow_reader` / `fromArrowReader` compiles the field cast before pulling a
batch; an identity transport hands its reader back unwrapped.
`from_serie` / `fromSerie` and `from_chunked` / `fromChunked` retain native input
buffers. Sorting, spilling and joins use the core's implementations.

Clustering returns [native key items](key-serie.md), with explicit key fields,
paths, payload fields and source positions.

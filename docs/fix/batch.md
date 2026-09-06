# Batch parsing

`FixBatchReader` turns FIX frames into the crate's ordinary Arrow `BatchReader`. `from_rows` reads byte rows; `from_column` reads one payload column from an existing Arrow stream and keeps the capture beside the parsed message.

## Contract

| Item | Rule |
| --- | --- |
| Schema | fixed from `FixOptions` and the `FixRegistry` before any row is pulled; parsed fields are named by numeric tag |
| Arrow input | `from_column(registry, source, column, options)` consumes one source batch at a time |
| Output | every source child field, value, nullability, and metadata first; then the native FIX fields, `entries`, and `unmapped` |
| Root metadata | the FIX/options root metadata; a source root's record-class metadata cannot describe the enlarged row |
| Collisions | a source name matching a native FIX name case-insensitively is refused from the schemas before the source is pulled |
| Row parameters | `fixbranch`, `beginstring`, `targetversion`, `sep`, and `direction`; each source column remains in the result |
| Provenance | `branch` is an ordinary source column; only `fixbranch` selects a per-row dialect |
| Bounds | `batch_row_size`, `batch_byte_size`, and `max_row_size` shape the output without collecting the stream |
| Bad frame | still one output row, with the FIX projection empty; only `dedup` deliberately changes row correspondence |

Python exposes the same reader over the Arrow C stream as [`yggdryl.fix.parse_arrow_reader`](../extensions/python.md#fix-registry-at-the-boundary). It accepts an explicit registry or uses the process default.


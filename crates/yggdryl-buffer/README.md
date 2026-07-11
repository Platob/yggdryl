# yggdryl-buffer

The **Buffer** layer of [yggdryl](https://github.com/Platob/yggdryl) — typed,
immutable, Apache-Arrow-backed contiguous buffers (`I8Buffer` … `F64Buffer` and the
bit-packed `BooleanBuffer`) for the native primitives.

Each buffer shares its allocation on clone, round-trips through little-endian bytes,
bridges to the [`yggdryl-core`](../yggdryl-core) positioned-IO cursors, and carries
optional [`yggdryl-field`](../yggdryl-field) metadata — `buffer.field(name, nullable)`
hands out the matching typed `Field` (`I64Buffer::field` → `I64Field`).

Depends on `yggdryl-field` and `yggdryl-core` (the top of the layer stack:
buffer → field → dtype → core).

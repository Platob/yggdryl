# yggdryl-scalar

Arrow-centric scalar **values** for **yggdryl**. [`Scalar`] is the trait every
value implements — it knows its `dtype` and round-trips through its raw byte form
(`to_bytes` / `from_bytes`). [`Binary`] is the byte-backed value carrying any
binary data type from `yggdryl-schema`.

> **Project reset.** A thin layer over the Arrow-centralized schema crate. See
> `CLAUDE.md` at the repository root for contributor rules.

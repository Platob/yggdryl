# yggdryl — design guide: the layered type hierarchy

> This is the **how-to-implement** companion to `CLAUDE.md` (which holds the hard
> cross-cutting rules). It describes the one repeated shape every data-model feature
> follows, so a new type or capability lands identically across the layers and the three
> languages. When in doubt, open the nearest existing type and mirror it — this document
> only names the pattern that code already embodies.

## 1. The crate topology (dependencies point strictly downward)

```
                        arrow-buffer
                             │
        ┌────────────────────┴─────────────────────┐
        │                                           │
   yggdryl-core            (native i96/i256 + Encoder/Decoder byte-codec base)
        │
   yggdryl-buffer          (positioned IO + typed buffers + IoPrimitive)   yggdryl-http
        │                                           │                      (Headers, standalone)
   ┌────┴─────┐                                     │
compression  converter     (codecs over the base; converter also owns PrimitiveType)
                  │
              yggdryl-dtype  → depends on {buffer, converter}
                  │
              yggdryl-scalar (→ dtype)      yggdryl-field (→ {dtype, buffer, http})
```

A lower layer never imports an upper one. Needing the reverse means the abstraction
belongs lower — move it down, don't add a back-edge. The `bindings/{python,node}/src`
trees **mirror this crate tree module-for-module** (`CLAUDE.md` rule 8): one binding
module per crate/module, same name, same path.

## 2. The three data-model layers share one shape

`dtype` → `field` → `scalar` are the Arrow data model, one concern per layer. Each layer
repeats the **same four-tier trait hierarchy**, so learning one teaches the others:

| Tier | dtype | field | scalar |
| --- | --- | --- | --- |
| **Base** (FFI-opaque, object-safe, no lifetimes/generics) | `DataType` | `Field` | `Scalar` |
| **Typed** (Rust-only extension; exposes the concrete type + native `T`) | `TypedDataType<T>` | `TypedField<DT, T>` | `TypedScalar<DT, T>` |
| **Category** (a family sharing an implementation) | `PrimitiveType` / `LogicalType` / `NestedType` | `PrimitiveField` / `LogicalField` / `NestedField` | `PrimitiveScalar` / … |
| **Concrete** (one file per type, stamped from a macro) | `I64Type`, `BooleanType`, … | `I64Field`, … | `I64Scalar`, … |

Invariants that hold in every tier:

- **Base traits are the FFI surface.** No lifetime parameters, no generics on the base
  trait, object-safe — the bindings must be able to hold every one. Temporary borrows
  appear only on `&self` accessors and never escape (`CLAUDE.md` rule 2).
- **Typed traits are Rust-only.** `TypedField<DT, T>` etc. carry generics/associated
  types, so they do **not** cross FFI; a binding exposes the *concrete* type's methods,
  not the generic trait. State such omissions in the binding module doc + docs site.
- **Category traits centralise the shared logic.** Add an intermediate category trait
  (replicated across dtype/field/scalar) rather than repeating code per concrete type.
- **Concrete types are macro-stamped, one file per type** (`i64_type.rs` holds `I64Type`
  only), re-exported from `mod.rs`; `lib.rs` is glue only (`CLAUDE.md` rule 1).
- **Names line up across layers and use the short primitive form:**
  `yggdryl_dtype::I64Type` / `yggdryl_field::I64Field` / `yggdryl_scalar::I64Scalar`
  (and the buffer `I64Buffer`). Use `I8`/`U8`/`F32`, never `Int8`/`UInt8`/`Float32`; the
  Arrow-canonical string (`"int8"`) stays on `name()`.

## 3. How to add a new concrete type (worked recipe)

Adding, say, a new primitive `X` (native `x`) is the **same edit in each layer**:

1. **buffer** — `crates/yggdryl-buffer/src/x_buffer.rs`: `primitive_buffer!(XBuffer, x);`
   (the io element codec `impl IoPrimitive for x` goes in `io/primitive.rs`).
2. **dtype** — `crates/yggdryl-dtype/src/x_type.rs`: stamp `XType` from `primitive_type!`,
   wiring `primitive_tag()` ↔ `yggdryl_converter::PrimitiveType::X`.
3. **field** — `crates/yggdryl-field/src/x_field.rs`: `primitive_field!(XField, XType, …)`;
   add `impl_to_field!(XBuffer, XField)` in `to_field.rs` (the buffer→field bridge).
4. **scalar** — `crates/yggdryl-scalar/src/x_scalar.rs`: stamp `XScalar`.
5. **bindings** — the matching `x_*` entry in each binding's `buffer` / `dtype` / `field`
   / `scalar` module, in **both** Python and Node, adapting only to idioms (Python
   dunders / keyword defaults; Node camelCase / `Option<T>` defaults). Same commit.
6. **docs + tests + bench** — update the mirrored `docs/*.md` page (synced
   `=== "Python"` / `=== "Node"` / `=== "Rust"` tabs, that order), extend the layer's
   integration tests, and — if it's on a throughput hot path — the three-language
   benchmark (`CLAUDE.md` **Benchmarks**).

A logical/nested type is the same recipe against the `LogicalType` / `NestedType`
category traits (scaffolding exists; no implementors yet).

## 4. Value-type contract (every serializable type)

Every value type round-trips through bytes and has value semantics, and the two agree:

- `serialize_bytes()` / `deserialize_bytes(bytes)` — exact inverses; `deserialize`
  validates fully (length/width) with a **guided** error (`CLAUDE.md` rules 5 & 12).
- `PartialEq + Eq + Hash` such that two values are equal **iff** their `serialize_bytes`
  are equal, and equal values hash equal (rule 7). Add `Ord` only where a total order is
  natural (widths/levels).
- Bindings mirror the pair: Python `__eq__` / `__hash__` / `__reduce__` (pickle), Node
  `equals()` / `hashCode()` (`i32`, Java-style), `serializeBytes()` / `deserializeBytes()`.
- Live/stream resources (IO handles, sessions) are the **only** exemption — they carry no
  serializable value.

## 5. Interpreted bindings infer the element type (rule 13)

Where the Rust core reaches a typed op through an explicit generic, the dynamically-typed
bindings **infer** the type from the runtime value (a Python `int` in `int64` range → the
matching width, `bytes`/`Buffer` → the byte buffer, `bool` → the bit buffer), so
`write(value)` / `buffer(values)` just work. Inference is a convenience **over**, never a
replacement for, the explicit API — every inferring call has an explicit counterpart
(`write_i64` / `writeI64`, `Int64Buffer(...)`), the two bindings infer **identically**,
and the mapping table is documented in each binding module doc + the docs site.

## 6. Before you commit

Run the full gate in `CLAUDE.md` (**Required checks before committing**) and finish with
the **coherence pass**: no redundancy, cross-language parity (the binding test suites are
the executable proof the three surfaces match method-for-method), one concern per file in
the right crate, every public item documented, docs + benchmarks in sync. A change is not
done until the Rust core, the Python binding, and the Node binding — and their tests, docs,
and benchmarks — all move together.

# yggdryl

!!! warning "Project status: rebuilding"
    yggdryl is being rebuilt around an **Apache Arrow-centralized** data model. This
    site documents the foundations that have landed in `yggdryl-core` and its Python
    and Node.js extensions; it grows as the workspace does.

A **Rust-core** library with **Python** and **Node.js** extensions. All logic lives
in the Rust crates; the bindings are thin wrappers, so the three languages behave
identically. Each Rust crate is exposed under its own namespace — `core` (the
foundations, mirroring `yggdryl-core`) and the three Arrow data-model layers
`dtype`, `field` and `scalar` (mirroring `yggdryl-dtype`, `yggdryl-field` and
`yggdryl-scalar`), whose concrete types share one naming convention across the
layers (`dtype.Int64Type` describes the type, `field.Int64Field` names a column of
it, `scalar.Int64Scalar` holds one value of it).

## Install

=== "Python"

    ```bash
    pip install yggdryl
    ```

=== "Node"

    ```bash
    npm install yggdryl
    ```

=== "Rust"

    ```bash
    cargo add yggdryl-core
    ```

## Hello

=== "Python"

    ```python
    from yggdryl import core

    print(core.version())
    core.hello()  # -> Hello from yggdryl 0.1.1!
    ```

=== "Node"

    ```js
    const { core } = require('yggdryl')

    console.log(core.version())
    core.hello() // -> Hello from yggdryl 0.1.1!
    ```

=== "Rust"

    ```rust
    fn main() {
        println!("{}", yggdryl_core::version());
        yggdryl_core::hello(); // -> Hello from yggdryl 0.1.1!
    }
    ```

## What's here

<div class="grid cards" markdown>

- :material-swap-horizontal: **[Positioned I/O](io.md)**

    `ByteBuffer` / `BitBuffer` and the `RawIOBase` / `IOBase<T>` traits — byte- and
    bit-level positioned reads and writes, in all three languages — plus the
    `RawIOCursor` / `IOCursor` (moving cursor) and `RawIOSlice` / `IOSlice` (byte
    window) adapters.

- :material-alphabetical: **[Charset](charset.md)**

    The `Charset` trait with the `Utf8` and `Latin1` text ↔ bytes codecs.

- :material-code-json: **[Serialization](base.md)**

    The `Base` trait — content JSON and an implementor-defined byte form.

- :material-shape-outline: **[Data types](dtype.md)**

    The `DataType` / `TypedDataType<T>` descriptors with the native byte codecs —
    every integer, `binary`, `null`, `union`, the logical `optional`, and the
    nested `list` / `map` / `struct`.

- :material-table-column: **[Fields](field.md)**

    The `Field` / `TypedField<DT, T>` layer — a name paired with a data type and a
    nullability flag, mirroring an Arrow `Field`.

- :material-numeric: **[Scalars](scalar.md)**

    The `Scalar` / `TypedScalar<DT, T>` layer — single, possibly-null values with
    exact-or-error `as_*` accessors, mirroring one-element Arrow arrays.

</div>

!!! note "Bindings vs. core"
    `ByteBuffer`, `BitBuffer`, `Whence` and the cursor/slice adapters
    (`ByteBufferCursor`, `ByteBufferSlice`, and the `BitBuffer` variants) are exposed
    in **all three** languages. `Charset`, `Base`, the typed `IOCursor` / `IOSlice`,
    and the two-resource streams (`pread_raw_io` / `pwrite_raw_io` and the typed
    `pread_typed_io` / `pwrite_typed_io`) are currently **Rust core only** — they gain
    Python/Node tabs when the bindings expose them.

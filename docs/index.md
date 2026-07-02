# yggdryl

!!! warning "Project status: rebuilding"
    yggdryl is being rebuilt around an **Apache Arrow-centralized** data model. This
    site documents the foundations that have landed in `yggdryl-core` and its Python
    and Node.js extensions; it grows as the workspace does.

A **Rust-core** library with **Python** and **Node.js** extensions. All logic lives
in the Rust crates; the bindings are thin wrappers, so the three languages behave
identically. Each Rust crate is exposed under its own namespace — currently just
`core` (the foundations), mirroring `yggdryl-core`.

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
    `RawIOCursor` / `IOCursor` adapters that add a moving cursor.

- :material-alphabetical: **[Charset](charset.md)**

    The `Charset` trait with the `Utf8` and `Latin1` text ↔ bytes codecs.

- :material-code-json: **[Serialization](base.md)**

    The `Base` trait — content JSON and an implementor-defined byte form.

</div>

!!! note "Bindings vs. core"
    `ByteBuffer`, `BitBuffer` and `Whence` are exposed in **all three** languages.
    `Charset` and `Base` are currently available in the **Rust core only** — they gain
    Python/Node tabs when the bindings expose them.

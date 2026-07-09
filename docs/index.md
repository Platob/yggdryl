# yggdryl

!!! warning "Project status: rebuilding"
    yggdryl is being rebuilt around an **Apache Arrow-centralized** data model. The
    previous implementation was removed; only the hello-world skeleton remains. This
    site grows as the workspace does.

A **Rust-core** library with **Python** and **Node.js** extensions. All logic lives
in the Rust crates; the bindings are thin wrappers, so the three languages behave
identically. Functionality is grouped into namespaces mirroring `yggdryl-core`:
`core` (the foundations), [`compression`](compression.md) (codecs like gzip),
[`io`](io.md) (positioned byte buffers), and [`buffer`](buffer.md) (typed
native-type buffers).

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
    core.hello()  # -> Hello, world!
    ```

=== "Node"

    ```js
    const { core } = require('yggdryl')

    console.log(core.version())
    core.hello() // -> Hello, world!
    ```

=== "Rust"

    ```rust
    fn main() {
        println!("{}", yggdryl_core::version());
        yggdryl_core::hello(); // -> Hello, world!
    }
    ```

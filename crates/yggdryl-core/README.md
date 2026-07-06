# yggdryl-core

The Rust core foundations of **yggdryl**.

> **Project reset.** The implementation was removed and is being rebuilt around an
> Apache Arrow-centralized design. Only the hello-world skeleton and the contributor
> rules (`CLAUDE.md` at the repository root) remain.

The crate currently exposes two entry points — the clean-slate example that
round-trips identically through the Python and Node bindings:

```rust
fn main() {
    println!("{}", yggdryl_core::version()); // the crate version
    yggdryl_core::hello(); // -> Hello, world!
}
```

Add further foundational types here as the design lands — one module per concern,
each re-exported at the crate root — following the rules in `CLAUDE.md`.

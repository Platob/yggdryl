# yggdryl (Python)

The Python extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed as a submodule of the `yggdryl` package, mirroring the
crate tree — currently just `yggdryl.core` (the foundations, `yggdryl-core`).

```python
import yggdryl
from yggdryl import core

print(core.version())  # the crate version
core.hello()           # -> Hello, world!
```

> **Project reset.** A thin hello-world scaffold over the Arrow-centralized Rust
> core, rebuilt from scratch. See `CLAUDE.md` at the repository root for contributor
> rules.

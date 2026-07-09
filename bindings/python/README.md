# yggdryl (Python)

The Python extension for **yggdryl**, backed by the Rust core.

The package mirrors the `yggdryl-core` module tree as submodules: `yggdryl.core`
(`version` / `hello`), `yggdryl.compression` (the gzip / zstd codecs), `yggdryl.io`
(positioned byte buffers), and `yggdryl.buffer` (typed native-type buffers).

```python
import yggdryl
from yggdryl import core

print(core.version())  # the crate version
core.hello()           # -> Hello, world!
```

See `CLAUDE.md` at the repository root for contributor rules.

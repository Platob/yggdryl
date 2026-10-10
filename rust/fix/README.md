# yggdryl-fix

FIX for yggdryl: the FIX dictionary registry and its store, the codec, its messages, their lifecycle and their Arrow rows over the market crate's graph.

Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the project's site, and `install()` claims what this crate registers on the core. A Rust caller runs `yggdryl_fix::install()?` once, which installs `yggdryl-market` first, before the core reads a FIX Latest datatype name; the Python and Node packages install it when they load.

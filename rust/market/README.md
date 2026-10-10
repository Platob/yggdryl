# yggdryl-market

Market data for yggdryl: the four market enum kinds, the identifiers, the instruments and the market-data graph, its books and their Arrow rows.

Part of [yggdryl](https://github.com/platob/yggdryl); the documentation is the project's site, and `install()` claims what this crate registers on the core. A Rust caller runs `yggdryl_market::install()?` once before the core reads a market kind - a parsed `side`, a serde tag, a `yggdryl.side` column; the Python and Node packages install it when they load.

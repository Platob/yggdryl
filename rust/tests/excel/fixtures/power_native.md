# Native Excel Power observations

The guarded active-Excel numeric observer opened one newly authored workbook, calculated its 15 independent `^` and `POWER` formulas, saved it, and compared `Value2` before/after with saved OOXML caches. Excel 16.0 build 20430.0, French locale; all 15 cases passed observer checks, no failures, `cleanup_completed=true`. Source report SHA-256: `fa918904e758008f2e0ce4907dfe6999998fd4de83d377bc4ed4e225ac614029`.

The fixture records exact numeric bits or COM error codes and cache type/text, plus source and saved workbook hashes. Native results include `0^0` → `#NUM!`, `0^-1` → `#DIV/0!`, `2^-1074` → cached zero, and the expression-sensitive `(-8)^(1/3)` → approximately negative two while `(-8)^0.5` → `#NUM!`. These observations establish only this bounded set; Rust equivalence remains a separate test.

The Rust replay passes 12 exact outcomes and explicitly holds the three negative-base fractional powers (`local-p5-power-green-isref-fill-stack-red.log`, Power step). Generic expression `pow`/`power` and the Excel adapter share `Scalar::checked_pow`; Excel owns its distinct error and finite-result rules. The shared bound-row kernel has zero allocations at 64 and 4096 rows.

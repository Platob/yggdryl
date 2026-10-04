# Native defined-name observations

Excel 16.0 build 20430.0, French UI 1036/country33. Both owned active-session runs completed with cleanup true and no failures; each recorded110 observations, both date systems. The A1/Data and D5/LocalB saved active contexts produced identical saved caches for all110 corresponding cases. No user workbook or application language was modified.

`defined_names_native.json` preserves all220 listed observations, raw COM before/after, typed XML caches, source cells,29 definitions, input/report/output hashes and the two saved active contexts. This is listed-case evidence, not whole-workbook validation. Original failed files remain in the recorded source locations.

Native Open isolation: the original31-name fixture and its last15 subset failed both epochs. First16, qualified8 and expression5 subsets opened and calculated their constant control. The isolated pair `GlobalOwnAbsolute=$A$1` / local `LocalOwnAbsolute=$A$1` failed Open1004 in both epochs. The amended full matrix removes only those two definitions and their three dependent formulas; neither individual definition was separately tested. No general name grammar rule is inferred.

Observed semantics:
- A cell's unqualified name selects its sheet-local definition before global; spelling lookup is case-insensitive. Explicit sheet qualification selects the local definition.
- A bare name inside a defined expression binds globally, including the observed local alias. `GlobalAlias=Rate` and local `LocalAlias=Rate` both yield global7 despite local11/17/23 shadows.
- Qualified relative A1 name references use the consuming cell coordinates; fixed axes remain fixed. Changing saved active sheet/cell had no effect. This tests wire-defined names, not COM Names.Add authoring conversion.
- Named ranges preserve reference origin: SUM ignores referenced text/Boolean but accepts name-defined text/Boolean constants. Scalar consumers intersect named rectangles/whole axes with the original consuming coordinate.
- Each named expression has its own arithmetic root: the cancellation definition remains0 inside arithmetic, ABS and an outer group; grouping inside the definition preserves the raw2^-50 result. Aliases preserve that boundary.

The Rust replay matches all 220 typed caches after integrating the shared Power kernel (`local-p5-fill-rounding-name-context-green.log`). Separate work-count tests prove bounded repeated-name evaluation and graph traversal; chains of 1 and 4096 names both pass on the same 128 KiB Windows debug thread stack. This stack budget is a measured fixed runtime floor: even one name overflowed at 64 KiB, while 4096 passed at 128 KiB. Warm name evaluation allocates nothing at 64 and 4096 consumers, including corpora with 64 and 4096 unused definitions (`local-p5-names-cost-comparisons-red.log`, the name allocation step passed before the intentionally red comparison step).

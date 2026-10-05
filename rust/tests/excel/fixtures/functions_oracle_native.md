# Full function oracle: native scope and Rust policy

Excel16.0 build20430.0, French UI1036, accepted both owned workbooks and all324
selected cases. The native report passed with cleanup complete. It records
before/after SaveAs in the same owned workbook; no reopen is claimed. Source and
native-save hashes, raw COM observations, native errors, exact XML caches, case
IDs and manifest amendment provenance are retained in functions_oracle_native.json.
The two input XLSX files are the original authored inputs. `functions_excel.xlsx`
and `functions_excel_1904.xlsx` are Excel-produced SaveAs outputs, copied byte-for-byte
under their verified native hashes; the saved-spelling replay opens those actual
packages before replacing selected caches with sentinels. Only selected cases are validated; this is not whole-package
formula certification.

Five TEXT cases have independent en-US proofs. Two date and two numeric/percent
cases use separate NumberFormatLocal companions; original currency uses its
unchanged authored cache directly. Every chosen value equals explicit LCID1033
evaluation of the original English formula. Original locale-dependent caches
remain recorded separately. The original unadapted324 failure and failed currency
translation run remain failures. NumberFormatLocal changed bare dollar symbols to
euros; no currency token replacement or second format parser was introduced.

The Rust replay consumes every case in authored and saved spelling:
312 exact policies, seven explicit numeric-policy holds and five injected-clock
behavior policies per replay. The seven holds are two nonzero-rate PMT calls,
one variance beyond the exact moment bound, and four hidden SUBTOTAL variance
calls whose mean is 8/3. Each hold preserves a non-native sentinel cache.

Literal-array, union and intersection cases now use their unchanged native
answers. The reference promotion follows 280 passing authored/saved comparisons
from 140 native observations, plus incremental-dependency and cycle controls.
The full replay expects 624 exact comparisons, 14 holds and ten clock controls;
its executed result is recorded in the phase check log.

RAND/RANDBETWEEN bounds and NOW/TODAY use a fixed Rust Clock. Native RNG and current
wall-clock caches are observations only; they are never compared to the injected
xorshift sequence. RANDBETWEEN(7,7) remains an exact deterministic case.

No mismatch may be relabeled as held just to make this fixture pass. A mask change
requires a documented source policy or a separately reviewed implementation,
then the narrow test reruns. P5 is not complete merely because159 registry names
have a bounded path.

Two follow-up proofs refine the replay without replacing any recorded native value.
The independent LCID1033 Boolean-text oracle returns `a12TRUE` for
`CONCATENATE("a",12,TRUE)`; the original French cache remains `a12VRAI`.
The en-US core compares to the explicit en-US observation.

Excel's native SaveAs changes the source `Errors!A8` from `#GETTING_DATA` to
`#N/A` while leaving its same-session `ERROR.TYPE` consumer cache at 8.
Opening that exact Excel-produced package and recalculating the selected formula
returns 7 in native Excel (cleanup complete). The authored replay therefore
expects 8, and the saved-package recalculation expects 7. Both cache observations,
the source transition, and follow-up report hashes are retained separately.

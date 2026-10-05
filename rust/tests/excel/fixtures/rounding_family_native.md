# Native rounding functions

Excel 16.0 build 20430.0 (French UI 1036, country 33) computed all 648
source/function cases in the 1900 and 1904 workbooks. All 1,296 comparisons
between native values before/after SaveAs and saved XML caches passed; owned
workbook cleanup completed. The source manifest SHA-256 is
`98341776e5a3dd193accc8cfb4b963f1d40a552fc369aee3ca6d83155df20aab`;
report SHA-256 is
`ad077efd8b05b9fc161f12695f63c749c0529b7bc071f3f43a1ecda90606e7b6`.
The accompanying JSON retains input hashes, formula spellings, typed values
and exact saved-cache bits.

The Rust replay failed before implementation, then matched all 648 cases.
Fourteen numeric boundary tests and thirteen aggregate tests using the same
fixture reader passed. The three exposed directional/parity/quotient kernels
allocate zero times for both 64 and 4,096 inputs; all ten functions have
cold/warm benchmark smoke rows. These are smoke results, not release timings.

ROUNDUP/ROUNDDOWN and EVEN/ODD use the shared fifteen-digit decimal owner.
QUOTIENT truncates raw binary division: the observed `QUOTIENT(0.3,0.1)` is 2.
CEILING/FLOOR normalize their quotient to fifteen digits before rounding to a
multiple. MROUND retains the raw half boundary: `MROUND(1.005,0.01)` is 1.
CEILING.MATH/FLOOR.MATH resolve significance and negative-number mode in the
Excel adapter. Generic floor/truncation and Arithmetic remain shared owners.

The corpus does not prove omitted arguments or arbitrary arrays. Subnormal
input/output and unrepresented overflow transitions retain the named numeric
refusal where the separate raw-value/cache contract is unresolved.

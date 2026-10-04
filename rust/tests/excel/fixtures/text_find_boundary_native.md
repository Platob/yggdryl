# Native text position boundaries

Excel16.0 build20430, UI1036:36 observations,72 exact before/after-SaveAs cache
comparisons, run passed and cleanup completed. The JSON retains the report and
manifest hashes. The workbook remained open between those observations; this
is not a reopen claim.

FIND with an empty needle accepts start=len+1, including empty source/start1.
SEARCH rejects start>len, including empty source/start1. Both accept a starting
UTF-16 position inside a surrogate pair; a nonempty UTF-8 needle can only begin
at the next scalar boundary, while an empty needle returns the requested native
position. These observations refine the general start bounds in Microsoft's
[FIND documentation](https://support.microsoft.com/en-gb/office/find-function-c7912941-af2a-4bdf-a553-d0d89b0a0628)
and [SEARCH documentation](https://support.microsoft.com/en-us/excel/functions/search-function).

The initial Rust fixture consumes every observation:18 FIND cases compute and
18 SEARCH cases preserve prior caches until the shared wildcard matcher slice.
Existing text_native.json adds26 FIND and16 REPLACE cases; unicode_native.json
adds18 source-version controls. Unpaired-UTF16 replacement results remain held
without replacement characters. Output size is checked in UTF16 units before
construction; searches retain no per-character position array.

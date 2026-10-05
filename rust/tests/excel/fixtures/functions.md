# Excel function registry fixture

`functions.json` is the 159-row UI signature, category, and description fixture
recorded for P5 in `handoff/ui-dev/fixtures/functions.json` (SHA-256
`0691f861c7df274e313c88ce0a9ee3e13a27bd087261983a9f9633a4c07462ba`).
It is copied here so the mirrored Rust registry test has a durable input.

P5 corrects two signatures from Excel 16.0 build 20430.0 observations:
IF needs two argument slots (an explicit blank is allowed), while INDEX's
reference form accepts its fourth area argument. The signature fixture and
the UI development fixture carry the same corrections; the hash above records
the original handoff input before these refinements.

The registry adds 11 prefix-only spellings carried by the existing formula
parser. This fixture does not state that a function has an evaluator or give
expected formula results. Native result fixtures are a separate oracle phase.

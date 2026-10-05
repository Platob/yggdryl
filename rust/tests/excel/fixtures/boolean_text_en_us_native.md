# Boolean text is explicitly en-US

Excel 16.0 build 20430.0, French UI 1036: twelve original expressions were evaluated by owned Worksheet Evaluate with LCID 1033. All observations and cleanup passed. The same physical cells retain French VRAI/FAUX caches; LEN(FALSE) is 4 there and 5 under the explicit en-US evaluation. These caches are not relabeled or overwritten.

The core en-US text boundary spells a Boolean TRUE/FALSE once, through Operand::text_argument. Concatenation, text indexing, casing and scalar text coercion consume this same owner. Numeric text and the separate locale-sensitive logical-text parser are unchanged. This is a formula contract, not a language guess from a file name or UI locale.

# Phase 6 — the source: FIX Latest into the committed dictionary

**Goal.** Generate and commit the dictionary the other six phases describe.

**Depends.** Phases 1, 3 and 4.

> **Read `00-contract.md` first.** It is short and binding: the never-list
> `N1`–`N7`, the landed facts `L1`–`L2`, the precedence rule, and the command
> block that says a phase is done. Nothing below repeats it.
>
> **Never, in short:** no public symbol or dependency this brief does not
> name; no compatibility shim or second path; no fact stored that is already
> derivable; no widening for the next phase; no `TODO`, `#[allow]` or ignored
> test; and never guess where a rule says refuse, or refuse where it says
> fall through.

---

**Surface.** A generator script beside the repository's existing Python
tooling, and a generated constants module beside the registry. It *writes*
the committed dictionary and a provenance manifest beside its trees. It
*edits* the FIX registry (the two constant accessors), the FIX module's
tests, and the FIX documentation page.

**Never.** Add an HTTP client, or any dependency, to the crate manifest (N2).
The generator is a script; the crate only ever reads the committed output
through `FixRegistry::from_handle`.

### Sources

Browsable rendering, for checking work by eye:
<https://orchimate.org/fixtrading/fix-latest> - FIX Latest as of EP309,
Orchestra v1.0 - with `/fields`, `/codeSets`, `/datatypes`, `/messages`,
`/components`, `/groups`, `/revisions`; field pages at `/fields/<Name>`,
code sets at `/codeSets/<Name>CodeSet`. Its "Orchimate MCP" is useful for
interactive lookups. **HTML is never scraped.**

Machine-readable source of record, under
`https://raw.githubusercontent.com/FIXTradingCommunity/orchestrations/<commit>/`
(percent-encode the space):

| version | file |
| --- | --- |
| FIX Latest | `FIX Standard/OrchestraFIXLatest.xml` |
| FIX 4.4 | `FIX Standard/OrchestraFIX44.xml` |
| FIX 4.2 | `FIX Standard/OrchestraFIX42.xml` |

Versions Orchestra does not publish - 4.0, 4.1, 4.3, 5.0, 5.0SP1, 5.0SP2 -
come from the QuickFIX data dictionaries at
<https://github.com/quickfix/quickfix/tree/master/spec> (`FIX40.xml` …
`FIX50SP2.xml`), which carry names, types and enum values per version:
enough for lineage, and the only public per-version set that is.

Vendor and community orchestrations come from Orchestra Hub,
`https://orchestrahub.org/api/v3/repos/<owner>/<repo>/revisions/<id>/download`
- how yggfin loads its `fixtrading-udf` and `clear-street` namespaces.

### What the Orchestra XML gives

Verbatim from `repositorytypes.xsd`:

```xml
<xs:attributeGroup name="entityAttribGrp">
  <xs:attribute name="added"        type="fixr:Version_t"/>
  <xs:attribute name="addedEP"      type="fixr:EP_t"/>
  <xs:attribute name="updated"      type="fixr:Version_t"/>
  <xs:attribute name="updatedEP"    type="fixr:EP_t"/>
  <xs:attribute name="deprecated"   type="fixr:Version_t"/>
  <xs:attribute name="deprecatedEP" type="fixr:EP_t"/>
  <xs:attribute name="replaced"     type="fixr:Version_t"/>
  <xs:attribute name="replacedEP"   type="fixr:EP_t"/>
</xs:attributeGroup>

<xs:complexType name="fieldType">
  <xs:attribute name="id" type="fixr:id_t"/>
  <xs:attribute name="name" type="fixr:Name_t"/>
  <xs:attribute name="type" type="fixr:Name_t"/>
  <xs:attribute name="codeSet" type="fixr:Name_t"/>
  <xs:attribute name="minInclusive" type="xs:string"/>
  <xs:attribute name="maxInclusive" type="xs:string"/>
  <xs:attribute name="presence" type="fixr:presence_t"/>
  <xs:attribute name="value" type="xs:string"/>
</xs:complexType>

<xs:complexType name="codeType">
  <xs:attribute name="value" type="xs:token" use="required"/>
  <xs:attribute name="sort" type="xs:nonNegativeInteger"/>
  <xs:attribute name="group" type="xs:string"/>
</xs:complexType>
```

`fixr:codeSet` carries `type` plus id, name and pedigree from the entity
base; documentation hangs off `fixr:annotation`/`fixr:documentation` on
fields, code sets and individual codes. `added="FIX.2.7"` and
`updatedEP="254"` are exactly P1's `Version` and P4's `ep`.
`replaced`/`replacedEP` is the rename axis, but FIX Latest records only
current names - the historical spelling lives in the per-version files,
which is why the script reads all of them.

### Rules

- **P6-R1. Every source URL is pinned to a commit, never `master`.** yggfin
  does exactly this - FIX Latest at
  `.../orchestrations/099914dd0edd49a699326f0441776d6e21cfaf93/…` and
  QuickFIX at `.../quickfix/3536699e830e65f875df4a50b647a6d3bad3b884/…` -
  because a dictionary regenerated from `master` is not reproducible and its
  diff is unreviewable.
- **P6-R2. `--source`** takes a local directory or a URL base so a run is
  reproducible offline; the default is the pinned set.
- **P6-R3. Output is exactly the shard layout `from_handle` reads:**
  a `primitive` and a `nested` tree, a folder per branch, one shard per
  `tag / 100` bucket, each a JSON array
  of core field documents ordered by canonical identifier, byte-identical to
  what `write_into` would produce.
- **P6-R4. Provenance in a provenance manifest beside the trees,** one record per source:
  `source_id`, `format` (`orchestra` | `quickfix`), pinned `url`, `sha256`
  of the bytes fetched, `sha256` of the definitions produced, `branch`,
  `version` label, `priority`, and **`license_url`**. The licence field is
  not bookkeeping: this commits a derived copy of FIX Trading Community and
  QuickFIX material into the repository and the attribution must travel with
  it. The two checksums give CI a drift test whose second half needs no
  network.
- **P6-R5. The provenance file is safe beside the trees.** `from_handle`
  descends only `primitive/` and `nested/` (it descends only those two trees), so a leaf
  beside them is never listed. It MUST NOT be named `records`, the store's
  retired-layout tripwire.
- **P6-R6. Pull one vendor dictionary into a non-standard `FixBranch` in the
  same run,** so P2's digest table and P7's branch inference have real data
  under them rather than a fixture.
- **P6-R7. Datatypes resolve through `DataType::LOGICAL_NAMES`,** already
  the FIX Latest datatype table : `Qty` is
  `decimal64(18,8)`, `SeqNum` is `int64`, no second mapping. A FIX datatype
  the table does not hold is a hard failure of the script, never a silent
  `utf8`.
- **P6-R8. The generator narrows nothing the standard did not.**
  `CheckSum(10)` is three digits with leading zeros and stays `String`;
  `BodyLength(9)` is `Length`, so a malformed one nulls a field rather than
  costing a batch (P7-R24); `MonthYear` stays `ascii(8)` because `202608` is
  a month and `202608w2` a week, neither a point in time; `SecureData(91)`,
  `XmlData(213)` and `Signature(89)` are `binary` though the wire carries
  them as text. Each is a case in yggfin's `test_fields.py` because each was
  got wrong once.
- **P6-R9. Repeating groups become the nested tree as they do today:** a
  List of a non-null `item` Struct whose `fix:tag` is the counter's.
- **P6-R10. Priority ordering.** Merge sources lowest priority first so the
  highest-priority source wins by P5-R1.

### The standard header and trailer

- **P6-R11. They are order, not nesting.** `FixMsg` lays a message out flat
  (P7), so both components are carried as generated `const` tag lists in
  a generated constants module beside the registry, read through
  `FixRegistry::standard_header_tags()` and `standard_trailer_tags()`:

  ```rust
  pub const STANDARD_HEADER_TAGS:  &[i32] = &[8, 9, 35, 1128, /* … */ 369];
  pub const STANDARD_TRAILER_TAGS: &[i32] = &[93, 89, 10];
  ```

- **P6-R12. They are not registry entries.** A component has no tag and
  `insert` admits only a field carrying one (L1); a synthetic tag would put
  a fiction in the identity space. The names `StandardHeader` and
  `StandardTrailer` and their ids are dropped, and the module docs name that
  loss. Every field *in* them is an ordinary registry entry by its own tag.
- **P6-R13. The constants are the union across every scraped version,** in
  canonical order, with `defined_at` deciding what a given version may
  carry. FIX Latest alone is not enough: yggfin's `versions.json` records
  `SecureDataLen(90)` and `SecureData(91)` in the 4.0–4.4 headers, which FIX
  Latest no longer lists, and the trailer is `CheckSum` alone in Latest but
  `SignatureLength(93)`, `Signature(89)`, `CheckSum(10)` in 4.2 and 4.4.
- **P6-R14. Requiredness rides the lineage, not a parallel table.** yggfin
  stores a `required` flag per session field per version; here presence is
  already `nullable` on the field a version resolves to, so the flag belongs
  in the P3 lineage entry.

FIX Latest's `StandardHeader`, in order: 8 BeginString, 9 BodyLength,
35 MsgType, 1128 ApplVerID, 1156 ApplExtID, 1129 CstmApplVerID,
49 SenderCompID, 56 TargetCompID, 115 OnBehalfOfCompID, 128 DeliverToCompID,
34 MsgSeqNum, 50 SenderSubID, 142 SenderLocationID, 57 TargetSubID,
143 TargetLocationID, 116 OnBehalfOfSubID, 144 OnBehalfOfLocationID,
129 DeliverToSubID, 145 DeliverToLocationID, 43 PossDupFlag, 97 PossResend,
52 SendingTime, 122 OrigSendingTime, 212 XmlDataLen, 213 XmlData,
347 MessageEncoding, 369 LastMsgSeqNumProcessed — plus the `HopGrp` group.
`StandardTrailer` is component id 1025.

### The worked case

Tag 32, end to end:

- FIX Latest: id 32, name `LastQty`, type `Qty`, added `FIX.2.7`.
- `OrchestraFIX42.xml` and `FIX42.xml`: tag 32 is `LastShares`, type `Qty`
  in 4.2 and `int` in 4.0/4.1.
- Generated field: name `LastQty`, datatype `decimal64(18,8)`, `fix:tag` 32,
  `fix:aliases` containing `LastShares`, and a `fix:lineage` of
  `{"since":"2.7","name":"LastShares","type":"int"}`,
  `{"since":"4.2","name":"LastShares","type":"Qty"}`,
  `{"since":"4.3","name":"LastQty","type":"Qty"}`.

### Tests

1. Load the committed dictionary and assert every line of the worked case.
2. Load the generated tree and write it back: byte-identical (P6-R3).
3. Every tag in both header constants resolves in the generated dictionary.
4. The generated header matches yggfin's `versions.json` ordering and
   required flags for 4.2 and 4.4 (P6-R13, P6-R14).
5. The two checksums in `sources.json` match a regeneration from the same
   pinned inputs.

**Docs.** Provenance, version coverage, the FIXT omission (P3-R2), the
dropped component names (P6-R12) and the regeneration command, on
the FIX documentation page.

---

## Handoff

Phase 7 needs the dictionary this phase commits:

- the dictionary root with lineages (`P3`) and code sets (`P4`) populated, so
  `from_pairs` has something to resolve against.
- `STANDARD_HEADER_TAGS` and `STANDARD_TRAILER_TAGS` (`P6-R11`), which are
  the message layout order (`P7-R21`).
- At least one non-standard branch (`P6-R6`), so Phase 7's branch inference
  (`P7-R16`) is tested against real data.
- `data`-typed tags (`P6-R8`), which `P7-R50` reads to find length-prefixed
  fields.

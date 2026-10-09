#!/usr/bin/env python3
"""Record D36, D37 and D38 in the handoff: DESIGN.md gains three design sections before
"## The ledger" and three ledger rows; MARKET_SPLIT_NEXT.md gains the rows, a rewritten
"## Next" and question 8. Idempotent: a second run changes nothing. Usage: record_designs.py <repo>."""
import re, sys, pathlib
S = pathlib.Path(__file__).resolve().parent
repo = pathlib.Path(sys.argv[1])
design = repo / ".handoff/split/DESIGN.md"
nxt = repo / ".handoff/next/MARKET_SPLIT_NEXT.md"

d38 = (S / "d38_design.md").read_text()
d37 = (S / "d37_design.md").read_text()
d36 = (S / "d36_design.md").read_text()
d39 = (S / "d39_design.md").read_text()

ROW36 = ("| D36 | `yggdryl-s3` through a storage-backend extension point: `StorageBackend` claimed per scheme on the register (`claim_backend`, `backend_for`, `backends`), asked by `Holder::from_url` after lowering, its answer described; `Holder::Registered(Box<dyn RegisteredHandle>)`; the verbs the wildcards specialized on S3 (`upload_from`, `discard`, `as_leaf`, `as_container`, `set_known_size`) as `IOBase` defaults and `into_byte_stream` over `owned_stream_bytes`; `Site::Opened` with an opener; `aws/` and `auth/` stay core under `aws`; `yggdryl-iceberg[s3tables]` depends on `yggdryl-s3`; CI leaf `s3` with the three exchanges | the backend map, `s3_backend_map/design_inputs.md` | P3 in place, S6d the move |\n")
ROW37 = ("| D37 | `MarketMessage` a concrete public struct in `graph/message.rs` - boxed facts, `StatedFacts`, the entries as an `Arc<Field>` root and a `Scalar` row, `children`, `Metadata`, `Vec<Anomaly>`, `InstrumentStatement` - the four traits implemented once on it; `MarketData::Message`, `MarketKind::Message` (`message`); `FixMsg` the codec's handle over a message (`into_message`, `from_message`), an idmap-mapped tag never an entry, a native message rendered by the inverse idmap else its crate tags; S3's trait, `as_message::<T>()` and `Box<dyn MarketMessage>` deleted | the message map, `message_map/design_inputs.md` | P5 |\n")
ROW39 = ("| D39 | a leaving medium keeps its rank: `media::codec::RESERVED_RANKS` (`parquet` 1, `avro` 2, `xmla` 4, `excel` 6) admitted by `claim` under the codec's own name, every other medium at or above `EXTERNAL_RANK`, so the `s2_pins` hashes and the order pins are byte-identical through S6; `implementer` grows once per move by S3's routes, Avro and Parquet carry their own hidden `implementer` for what Iceberg reaches; the Iceberg field view built by `protocol_field_types!` in `yggdryl-iceberg` (`IcebergField::new`), `as_iceberg` gone; core tests building a leaving crate's objects move to that crate's tests; `install()` at every init; order avro, parquet, excel, xmla, then iceberg after `yggdryl-s3` | the five media maps | S6 |\n")
ROW38 = ("| D38 | `currunix` -> `transunix` (the transaction instant, required, the identity and order axis), `recdunix` -> `sendunix` (the technical wire clock, optional, the merge reference), and the element's own `curruuid` -> `uuid`, `currhashcode` -> `hashcode` (`prevuuid`, `crossuuid`, `crosshashcode`, `srcuuids` keep their prefix); precedences, values, derivations, positions and tags unchanged - the carrier's clock first, else `SendingTime(52)`; crate fields 65_001, 65_004, 65_007 and 65_009 re-spelled, so the dump, the dictionary hash (once, with its sentence), the snapshot's keys and `fix.json` move and the census does not | the instants map | P4 |\n")

def upsert_row(text, dnum, row):
    pat = re.compile(r"^\| %s \|.*\n" % dnum, re.M)
    if pat.search(text):
        return pat.sub(lambda m: row, text, count=1)
    # insert after the last ledger row of the table that holds D33
    m = list(re.finditer(r"^\| D33 \|.*\n", text, re.M))[-1]
    return text[:m.end()] + row + text[m.end():]

# DESIGN.md
t = design.read_text()
block = "## P4: design\n\n" + d38.strip() + "\n\n## P5: design\n\n" + d37.strip() + "\n\n## P3: design\n\n" + d36.strip() + "\n\n## S6: design\n\n" + d39.strip() + "\n\n"
if "## P4: design" not in t:
    i = t.index("## The ledger")
    t = t[:i] + block + t[i:]
elif "## S6: design" not in t:
    i = t.index("## The ledger")
    t = t[:i] + "## S6: design\n\n" + d39.strip() + "\n\n" + t[i:]
for d, r in (("D36", ROW36), ("D37", ROW37), ("D38", ROW38), ("D39", ROW39)):
    t = upsert_row(t, d, r)
t = t.replace("65_001 `uuid`, 65_002 `hashcode`", "65_001 `uuid`, 65_004 `hashcode`").replace("65_001,\n65_002, 65_007 and 65_009", "65_001,\n65_004, 65_007 and 65_009").replace("65_001, 65_002, 65_007 and 65_009", "65_001, 65_004, 65_007 and 65_009")  # FIX_TAG: hashcode is 65_004
design.write_text(t)

# MARKET_SPLIT_NEXT.md
n = nxt.read_text()
for d, r in (("D36", ROW36), ("D37", ROW37), ("D38", ROW38), ("D39", ROW39)):
    n = upsert_row(n, d, r)
NEXT = """## Next

The slices, in the order they land, each one commit on the program branch:

1. **S3** (in flight: its worktree `wip/s3` merging and settling) - the
   remaining seams in place (D5, D6, D9, D10).
2. **P4** (D38) - `transunix` and `sendunix` at every door; the dump, the
   dictionary hash, the snapshot's keys and `fix.json` regenerated once each.
   Design: "## P4: design" in DESIGN.md; contract `scratchpad/p4_contract.md`;
   the sweep `scratchpad/p4_sweep.py` applied to a worktree cut after S3 lands.
3. **P5** (D37) - `MarketMessage` the concrete generic message, `FixMsg` the
   codec's handle over it, both doors (`into_message`, `from_message`, the
   render of a native message), `MarketData::Message`; after P4 so it speaks
   the new names. Design: "## P5: design"; contract `scratchpad/p5_contract.md`.
4. **P3** (D36) - the storage-backend extension point in place: the register,
   `Holder::Registered`, the `IOBase` capabilities, `Site::Opened`, the S3 trio
   claimed by the core itself; disjoint files from P4 and P5, so it runs
   beside them and lands when its chain is clean. Design: "## P3: design";
   contract `scratchpad/p3_contract.md`.
5. **S4** - `yggdryl-market` (the `graph/` vocabulary, the four enum kinds,
   the identifiers, `isin_registry.rs`) and `yggdryl-fix` move out, as the
   prompt's S4 section says, over the `implementer` door S3 opened.
6. S5, S6 (a-d: Avro, Parquet, Iceberg with `s3tables`, XMLA, the object
   stores), S7-S9 as the prompt states them.

The first command of every slice, from a fresh checkout of the program branch:

```bash
git fetch origin && git switch ccr-0fe6f9d0-ruymat && git merge origin/main
```

"""
n = re.sub(r"## Next\n.*?(?=## The decision ledger)", NEXT, n, flags=re.S)
Q8 = """8. D38: `sendunix` keeps `recdunix`'s precedence - the carrier's clock (the
   capture's write time) first, else the stated `SendingTime(52)` - because
   `SendingTime(52)` already has its own fixed-row column and the capture's
   clock has no other. If "the technical sending time" means the sender's
   clock first, say so: it is one precedence line (`fix/build.rs`'s carrier
   fill and `fix/msg.rs`'s `record_at_sending`) and its tests, and the
   snapshot's `sendunix` values would move with it.
"""
if "8. D38:" not in n:
    m = re.search(r"^7\. Branch protection:.*?(?=^\d+\. |\Z)", n, re.M | re.S)
    if m:
        n = n[:m.end()].rstrip("\n") + "\n" + Q8 + n[m.end():]
    else:
        n = n.rstrip("\n") + "\n" + Q8
nxt.write_text(n)
print("recorded")

# P16: design (D49) - the cross identity from the creation instant

The user's decisions 37, 38 and 39 (`$S/user_decisions.md:235-263`, the words in
`$S/p16/user_instruction.md`; `$S` is `.handoff/split/scratch`). Decision 38 is the user's own
restatement of 37 and replaces it. Decision 39 came with it. D49 amends decision 28 (the lineage,
`user_decisions.md:161-172`), which it leaves as it is, and P7's D40.5 (`$S/p7/d40_design.md:139-155`),
which it **supersedes whole**. It lands as **P7's phase 1**, in place of D40.5 (`$S/p7/p7_plan.md:51-73`),
so every cross identity moves once (decision 37: "lands with P7, which moves every cross identity
once").

Read on HEAD `2902c86fe`. P9b is uncommitted in the tree, and its edits touch `book.rs` and
`market.rs` but not `rust/src/graph` or `rust/src/txhash`. Every line number is from that tree,
except where a line is named as P12's (`/home/user/yg-p12`) or P14's (`/home/user/yg-core`).
Nothing was built to write this. The behaviour evidence is from `python/.venv/bin/python -I`
against the installed extension, which may predate P9b, P12 and P14.

## The ask, sentence by sentence

Decision 38, verbatim: "change the crossuuid rule to take creaunix and use txhash generally taking
creation unix if given else initialize the lifecycle with the transunix, else on lifecycle pass or
merge with different minimum creaunix reset it using the crosshashcode, also updated on crosscode
or crosshashcode changes".

Decision 39, verbatim: "leverage the optimized set unix of unix and ensure to update on diffs for
uuid txhash or hashes to not rehash always".

| Clause | Read as | Where |
| --- | --- | --- |
| "change the crossuuid rule to take creaunix" | An event's `crossuuid` reads its creation instant. Today it reads none: `Uuid::from_v8(crosshashcode)`, `element.rs:297-302`. | D49.1 |
| "and use txhash generally" | The rule is the crate's `TxHash` coupling, `coupled(unix, code)` (`element.rs:540-549`), projected by `TxHash::into_uuid` (`txhash/value.rs:335-339`). "Generally" means every event, every holder and every door, through one provided reading. | D49.1 |
| "taking creation unix if given" | The instant is `creaunix` where the event states one. | D49.1, D49.2 |
| "else initialize the lifecycle with the transunix" | With no `creaunix`, the lifecycle's creation is the event's own `transunix`. The identity reads `creaunix.unwrap_or(transunix)`. Every lifecycle pass writes that instant into the column: the walk on every element it walks (`iterator.rs:1489-1501`, as today), and `fold_lifecycle` on every follow, restatement and merge (D49.3). | D49.2, D49.3 |
| "else on lifecycle pass or merge with different minimum creaunix reset it" | When a follow, a restatement or a merge lowers the earliest creation (`fold_lifecycle`, `element.rs:1162-1170`, reading `creation_unix()` on both sides), `crossuuid` is derived again from the new minimum. | D49.3 |
| "using the crosshashcode" | The digest half of the `TxHash` is `crosshashcode` itself, packed and not hashed again. | D49.1 |
| "also updated on crosscode or crosshashcode changes" | `sync_cross`, the holders' cross setters, and the side and kind moves that re-prefix the stored code (`reprefix`) derive `crossuuid`, and on an event `uuid`, again whenever the code or its hash moves. | D49.4 |
| "leverage the optimized set unix of unix" | The setters that already project an identity on a write: `set_transunix`/`set_seqnum` calling `refresh_uuids` (`facts.rs:1508-1513,1623-1645`) or `derive_uuids` (`text/line.rs:1135-1140`). They carry the new input too, through `set_creaunix`. No second derivation door is added. | D49.4 |
| "ensure to update on diffs for uuid txhash or hashes" | A setter recomputes `uuid`, `crossuuid`, `crosshashcode` and `hashcode` only where an input they read moved, and every derivation writes through the `moved` helper (`element.rs:532-538`). `finalize` keeps deriving both UUIDs every call, so the Arrow validation stays closed. | D49.4 |
| "to not rehash always" | A value written unchanged derives nothing. A cross code written unchanged is not hashed again. `sync_cross` hashes the code only while it is out of step. Every such skip is pinned. | D49.4, Tests |

## What the tree holds today

| Fact | Where |
| --- | --- |
| `Element::cross_uuid`, provided: `Uuid::from_v8(u128::from(crosshashcode))`, else the element's own `get_uuid()` when the code hash is 0. It reads no instant. | `rust/src/graph/element.rs:291-302` |
| `Element::sync_cross`, provided: hashes the cross code on **every** call (`crosshash`, `:516-520`), then writes `crosshashcode` and `crossuuid` through `moved`. 16 call sites. | `element.rs:270-289`; callers `element.rs:369,391,509`; `facts.rs:450,1604,1894,2037,2190`; `operation.rs:303,801`; `trade.rs:144`; `book.rs:3433`; `msg.rs:2146,3300,7387`; `instrument.rs:749` |
| `Event::finalized`, provided: `set_hashcode`, then `uuid = time_uuid()` (kept on `Err`), then `crossuuid = cross_uuid()`. Unconditional. | `element.rs:1187-1200` |
| `Event::txhash` = `coupled(transunix, hashcode)` = `TxHash::new_in(unix, Nanosecond, Digest(Xxh3, code))`. `time_uuid` = `txhash()?.into_sequenced_uuid(seqnum, crosshashcode)`, which keeps milliseconds and a hashed 62-bit payload. | `element.rs:540-549,1228-1253`; `txhash/value.rs:355-366` |
| `TxHash::into_uuid`: the instant floored to microseconds, then `Uuid::from_v7(micros, digest)`. It keeps 48 bits of millisecond, the version 7, 10 bits of microsecond, and all 64 digest bits verbatim around the variant. No hash. It refuses an instant outside `0..=281474976710655999` µs. | `txhash/value.rs:325-339`; `uuid.rs:480-515` |
| `fold_lifecycle` keeps the earliest `creaunix`, the latest `exprunix` and the furthest state, through `moved`. It derives no identity. It runs in `follow_timed` (`:627`), `merge_timed` (`:657`) and `restating` (`:1104`). | `element.rs:1155-1179,600-679,1098-1107` |
| `creaunix` per door: the FIX parse writes `transunix` unless tag 65_008 is stated. `redate_by_transaction` moves it to TransactTime or an earlier OrigSendingTime(122). A `W` entry takes MDEntryDate/Time. A text read writes the earliest instant it has dated a line of the object by. A book folds the earliest of its members. `MarketEventFacts::at` starts at `None`. | `rust/fix/src/msg.rs:1773-1775,4450-4476`; `rust/fix/src/market.rs:2063`; `rust/src/text/arrow.rs:1041-1064`; `rust/market/src/graph/book.rs:1885`; `rust/market/src/graph/facts.rs:1495` |
| The walk's `created` writes the chain's `creaunix`, else the element's own `transunix`, into every element it walks, after `rekeyed`. Its doc says "the identity does not move", which D49 makes false. | `rust/market/src/graph/iterator.rs:1145-1154,1489-1501` |
| The walk keys a live chain by `(crossuuid, kind)`: `type Chain = (Uuid, MarketDataKind)`. `identity_of` looks up `(get_crossuuid(), kind)`. A new chain's key is the element's `crossuuid`. `rekeyed` forces every follower's `crossuuid` to the key. The snapshot grid sorts by the key and restores `crossuuid` by hand after `walked_restamp`. | `iterator.rs:659-663,1191-1196,1156-1160,1478-1486,980-988,1006-1014` |
| The book identifies a live entry by `LiveKey { identity: get_crossuuid(), symbol, scope }` and orders chains by a `HashMap<Uuid, _>` over `get_crossuuid()`. | `book.rs:218-232,3315-3345` |
| The market Arrow reader refuses a row whose stated `crossuuid` differs from the one `canonical.finalize()` derives ("expected the value derived from the row"). A FIX row is exempt (`MarketData::Fix(_) => Ok(())`). | `rust/market/src/graph/arrow.rs:1692,1695-1736,1833-1866` |
| Projecting setters with no diff guard: `MarketEventFacts::set_crosscode` hashes the code and refreshes, `set_hashcode`/`set_crosshashcode`/`set_transunix`/`set_seqnum`/`finalized` all refresh both UUIDs, `set_creaunix` refreshes nothing. `refresh_uuids` = `time_uuid()` (one TxHash plus one seeded XXH3) plus `cross_uuid()`. | `facts.rs:1504-1513,1561-1590,1620-1653,1699-1702` |
| The one setter that hashes only on a move: `MarketFacts::reprefix`. `MarketFacts::set_crosscode` stores the text and leaves the hash to `finalize`'s `sync_cross`. | `facts.rs:330-339,414-416,446-454` |
| `TextLine`: `set_transunix`/`set_seqnum`/`set_hashcode`/`set_crosshashcode` drop both identity cells (`derive_uuids`) even when the value is unchanged. `set_crosscode` also drops the hash cell. `set_creaunix` drops nothing. `get_hashcode` hashes the whole body on **every** unstated read. | `rust/src/text/line.rs:1123-1140,1348-1395,1436-1477,1361-1364` |
| FIX: `settle` = `settle_facts` (`sync_cross`) + `stamp_identity` (`self.hashcode()`, a digest of all content, then `finalized`). `settle_clock` re-runs `finalized` over the held code when only the clock moved. `settle_refiled` re-digests. | `rust/fix/src/msg.rs:2104-2155,3270-3302` |
| Elements with no instant: `MarketFacts`/`OperationFacts` finalize `uuid = from_v8(hashcode)`, `crossuuid = cross_uuid()`. `OperationElement::finalize` does the same. `Instrument::finalize` does `sync_cross`, then `uuid = crossuuid`. `cross_uuid_of` hand-spells `from_v8(crosshash(code))` for the aliases index, `underlying_uuid`/`leg_uuids` and `get_by_uuid`. The mint reads `xxh128(crosscode)` directly. | `facts.rs:446-454,1890-1898`; `operation.rs:296-316`; `rust/market/src/instrument.rs:747-752,1733-1738,968-989` |
| D40.5 has **not** landed. The commit titled P7 (`483644b1c`) recorded decision 28 only. | `git show --stat 483644b1c` |
| Medallion primary key `("transunix", "crosshashcode", "seqnum", "hashcode")`. It reads no `crossuuid`. | `python/tests/medallion.py:96` |
| The equivalence snapshot holds 365 `crossuuid` cells, every row with a non-null `creaunix`. 262 are coded and 103 code-less (`crossuuid == uuid`). In 28 coded cells `creaunix != transunix` (ulbridge 16, lifecycle 12). | `rust/fix/tests/root/equivalence.snapshot` (tally by the survey) |

Probe (installed extension): two parses of `11=ORD1` at 10:00:00 and 10:00:05 share
`crosscode 10:1:ORD1`, `crosshashcode 7574475182723473348` (`0x691df10c223e9fc4`) and today's
`crossuuid 00000000-0000-8000-a91d-f10c223e9fc4`. Their `creaunix` values are 1767261600000000000
and 1767261605000000000. `TxHash.from_parts(unix, Digest.from_int('xxh3', code), unit='ns').into_uuid()`
answers:

- `019b78ff-f900-7001-a91d-f10c223e9fc4` for the first instant;
- `019b7900-0c88-7001-a91d-f10c223e9fc4` for +5 s;
- the first value again for +999 ns;
- `019b78ff-f900-7005-a91d-f10c223e9fc4` for +1 µs.

The low 48 bits are today's crossuuid's low 48 bits. Bits 48-63 differ: the UUIDv7 value carries
`a91d` where the hash reads `691d`, the hash's top two bits sitting at 64-65 under the variant, and
today's `from_v8` value carries `a91d` because it overwrites those two bits with the variant.

## The decision

### D49.1 - the rule

```text
crossuuid(e) =
    e.get_uuid()                                            where crosshashcode == 0
    TxHash(cross_unix, Nanosecond, Xxh3(crosshashcode))
        .into_uuid()                                        where cross_unix is Some and fits UUIDv7
    Uuid::from_v8(u128::from(crosshashcode))                otherwise (no instant, or one before the epoch)
cross_unix(e) = Some(creaunix.unwrap_or(transunix))         for an event
              = None                                        for an element with no instant
```

- **Layout.** RFC 9562 UUIDv7: 48 bits of creation millisecond, version 7, 10 bits of the
  microsecond within it (0..999), the top 2 bits of `crosshashcode`, the RFC variant, and the low 62
  bits of `crosshashcode`. Nothing is hashed. `coupled(unix, crosshashcode)` (`element.rs:543-549`)
  is the existing coupler and `into_uuid` the existing projection, so one `TxHash` is built per
  derivation and no XXH3 runs. `into_sequenced_uuid` is **not** used: its seeded payload hides the
  code, and a cross identity carries no `seqnum`.
- **Why it sorts by creation.** The 58 high bits are the creation instant at microsecond
  resolution, so cross identities sort by creation millisecond, then microsecond, then the code
  hash. Two creations within one microsecond under one code are one identity, which is right for
  one chain. The whole `crosshashcode` is recoverable from the UUIDv7 value (its top two bits at
  64-65 and the 62 `rand_b` bits).
- **A per-creation-instant identity, not a chain key.** `crossuuid` names a code at one creation
  instant. It is not a stable chain key: two statements of one chain more than a microsecond apart
  that were never walked or merged hold two values, a chain whose minimum is lowered mid-life holds
  two (D49.3), and a value depends on what the reader had seen when it derived it. Chains join by
  `crosshashcode` or the stored `crosscode`, never by `crossuuid`. The docs and the skills say so
  in those words.
- **A code-less event keeps its own identity.** This is D40.5's interpretation 5
  (`d40_design.md:224`), kept. `TxHash(creaunix, 0)` would fold every code-less event created in one
  microsecond into one identity: the snapshot's 103 code-less cells would collapse onto 15 values.
- **An element with no instant keeps the code rule.** That covers `MarketFacts`, `OperationFacts`,
  `OperationElement<K>` and `Instrument`: `from_v8(crosshashcode)`, exactly as today, so their values
  **do not move**. D40.5's XXH3-128 is withdrawn for them too. The reason it existed (the mint reading
  `crossuuid`, D42's "What P7 changes" 2) is withdrawn in D49.5. The two readings do not carry the
  same bits: the UUIDv7 value carries all 64 bits of `crosshashcode`, while the code rule keeps only
  its low 62 (`from_v8` overwrites bits 62-63 with the variant, `uuid.rs:565-566`), so `from_v8` is
  not a bijection of the hash.
- **A refused instant** falls back to the code rule, not to the held value. `coupled` takes an
  `i64` nanosecond count, whose maximum is in 2262, so only an instant before the epoch is refused. The rule stays a pure function of the row's columns (`creaunix`, `transunix`,
  `crosshashcode`, `uuid`), so the Arrow reader's validation (`arrow.rs:1833-1866`) stays closed. A
  held value is not derivable from a row.
- **Where it lives.** `Element::cross_uuid` stays the one provided reading. `Element` gains one
  **required** method, `fn cross_unix(&self) -> Option<i64>`: "the instant this element's cross
  identity was created at; `None` for an element with no instant". It is required, not defaulted,
  so no event implementor can forget it and silently fall to the code rule. The 18 `impl Element`
  sites (`grep -rn "impl.*Element for"`: core `text/line.rs:1314`, the two rustdoc examples
  `element.rs:98,813`, `rust/tests/graph/element.rs:92`, `rust/market/tests/graph/element.rs:73`,
  `facts.rs:393,1540,1836,1979`, `operation.rs:245,742`, `trade.rs:160`, `book.rs:90,3470`,
  `market_data.rs:170`, `instrument.rs:693`, `fix/src/msg.rs:6677`, `fix/src/enrich.rs:519`) each
  answer it, delegators by delegating. `Event` gains the provided reading
  `fn creation_unix(&self) -> i64 { self.get_creaunix().unwrap_or(self.get_transunix()) }`, the one
  spelling of the "else", and every event's `cross_unix` answers `Some(self.creation_unix())`.
- **`code_uuid(crosshashcode)`** becomes a crate-private free function beside `crosshash` in
  `element.rs`, holding the code rule. `cross_uuid` calls it. The market crate's `cross_uuid_of`
  (`instrument.rs:1736-1738`) calls it through `yggdryl::implementer::code_uuid` (an `#[inline]`
  forwarder, AGENTS "Implementer"), and the hand copy goes.

### D49.2 - an element stating no `creaunix`

- **The identity reads it. Construction does not write it, and a lifecycle pass does.**
  `creation_unix()` reads `creaunix`, else `transunix`. That is decision 38's "initialize the
  lifecycle with the transunix" for an element no lifecycle has touched yet. Writing
  `creaunix = transunix` at construction (`MarketEventFacts::at`, `facts.rs:1489-1501`) is rejected:
  a later `set_transunix` would leave the creation stale, and the column would claim a fact nobody
  stated. A follow, a restatement or a merge is a lifecycle pass, so it writes the folded creation
  into the column, even where neither side stated one (D49.3).
- **The column is written where a lifecycle is.** These writers are kept as they are:
  - The FIX parse writes the message's `transunix` (`msg.rs:1773-1775`), so every parsed message
    states it.
  - The text read writes the earliest dated instant of the object (`text/arrow.rs:1059-1064`).
  - The walk's `created` (`iterator.rs:1489-1501`) writes, on every element it walks, the chain's
    earliest `creaunix` if it continues a chain, else its own `transunix` if it opens one: "the
    first statement that opened the chain". An element in no chain *is* a chain of one and takes
    its own `transunix`.
  - A book folds the earliest of its members (`book.rs:1885`). A book none of whose members states
    one reads its own `transunix` through `creation_unix()`.

  - Every follow, restatement and merge: `fold_lifecycle` (D49.3).

  A hand-built event that was never walked, followed or merged keeps `creaunix = None` in its column
  (the pin `rust/tests/text/line.rs:964`, `(None, None)`, stands). Its identity reads its
  `transunix`, and its Arrow row derives the same identity back from the null cell.
- **A text line's cross identity varies with how it was read.** The text read writes each line's
  `creaunix` as the running minimum of the instants it has dated so far, and a `creaunix` capture
  stands per line (`text/arrow.rs:1055-1063`). The lines of one object are documented as one chain
  (`text/arrow.rs:1039`). Under D49 they share one `crossuuid` only while that minimum holds. They
  get several in three cases:
  - a log not written in order, where the minimum drops mid-object;
  - per-line `creaunix` captures;
  - a read that starts past the object's first lines, which has another scope and another minimum.

  A line's `crossuuid` then depends on how it was read. Its `uuid` does not, because it reads
  `transunix`, `seqnum`, `hashcode` and `crosshashcode` only. That is decision 38 read literally.
  The alternative derives a text line's identity from one object-level creation that the read fixes
  once, and it is put to the user (#9). The text pins that state "one chain" are re-read
  (`python/tests/text/test_line.py:83`, `node/tests/text/line.test.js:64`,
  `python/tests/text/test_init.py:223`). They hold for a log written in order, which is what they
  read. A red test pins a log written out of order (Tests, 9).
- **An `Instrument`** is an `Element` with no `Event`, so it has no `creaunix`. `firstunix`
  (`instrument.rs:626`) is a fact, not a creation input. Its `cross_unix()` is `None`, and
  `crossuuid = uuid = code_uuid(crosshash(crosscode))`, unchanged.

### D49.3 - the lifecycle and the merge

- **One place: the fold reads the initialized creation.** Today `fold_lifecycle`
  (`element.rs:1162-1166`) folds `earliest(get_creaunix(), other.get_creaunix())`, and
  `earliest(None, None)` is `None`. Outside the walk, then, nothing initializes the creation from
  `transunix`: `Event::following`/`with_previous` (a hand-built chain, the bindings'
  `with_previous`), `restating`, `merging`, `book_arrow_reader`'s unwalked statements. A chain whose
  members state no `creaunix` would end with one `crossuuid` per member. The fold becomes
  `earliest(Some(own), Some(other.creation_unix()))`, written through `set_creaunix` only where it
  `moved`. `own` is `self.creation_unix()` read **before** any instant of the pass moved. In
  `merge_timed` (`element.rs:643-650`) the reference's `transunix` is written before the fold, so
  `merge_timed` reads `this.creation_unix()` on entry and hands it to the fold. A merge of two
  statements stating none then keeps the earlier microsecond, not the reference's `transunix`. A
  `None`/`Some(x)` merge whose `None` side has the earlier `transunix` keeps that `transunix`.
  Following, restating and merging therefore initialize the chain's earliest creation and keep it,
  as the walk does.
- **The reset is the derivation.** On a projecting holder (`MarketEventFacts`, so every market
  leaf and the FIX message; `TextLine`) `set_creaunix` projects `crossuuid` (D49.4). On any other
  implementor, the `finalize` that `following`/`merging`/`restating` already run
  (`element.rs:1066-1079,1098-1107,1135-1154`) reaches `sync_cross`, which reads `cross_uuid()`. A
  follow, a restatement and a merge therefore all reset `crossuuid` exactly when the minimum moved,
  through `cross_uuid` alone. No second derivation is added.
- **The walk's chain key moves off the element's `crossuuid`, onto today's value.** Under D49 a
  follower parsed at a later instant arrives with `crossuuid = TxHash(own creaunix, code)` before the
  walk. `identity_of`'s lookup `(get_crossuuid(), kind)` (`iterator.rs:1193`) would miss its live
  chain (the probe above: two `ORD1` statements, two `creaunix`). The key becomes exactly today's
  value, read by one crate-private function in `iterator.rs`:
  `chain_key(e) = code_uuid(e.get_crosshashcode())` where the code hash is non-zero, else
  `e.get_crossuuid()`. `type Chain = (Uuid, MarketDataKind)` stays.
  - For a coded element this is the `from_v8` value today's key holds: the low 62 bits of
    `crosshashcode` under the v8 version and variant. Every key value, every collision and every
    order is kept bit for bit.
  - For a code-less one it is today's key unchanged.
  - The order matters beyond the lookup. `expirations: BTreeSet<(i64, Chain)>` (`iterator.rs:627`)
    orders expirations at one deadline by the key. `expire` (`:1024-1046`) gives them their places
    from `InstantSequence` in that order, and their `seqnum`, and so their `uuid`, follow. The
    snapshot grid sorts `snapshot_identities` by it too (`:985-987`). A key ordered any other way
    (the raw hash, an enum with code and code-less keys apart) would move event `uuid`s and the
    snapshot rows' order at a grid tick. Keeping the `Uuid` keeps both. A guard test pins it
    (Tests, 3).
  - `chain_key` is used by `identity_of`, the new-chain key (`:1156-1160`), `settle`, `alive`,
    `expirations`, `named`/`names_of` and `snapshot_identities`. The `internals` doors
    `settle`/`retire`/`named_*`/`rekeyed` (`:1552-1599`) keep their `Uuid` signatures. The conflict
    detail (`Spelled(code, ..)` and `chain(..)`, `iterator.rs:1261-1270,1453`) prints the element's
    `crossuuid` and the chain key as `Uuid`s and needs no change.
- **`created` before `rekeyed`, in every arm.** `walk_source` runs `rekeyed`, then `created`
  (`iterator.rs:1144-1154`, and the out-of-order arm at `:1110-1111`). Under D49 `created` lowers
  `creaunix` through the projecting `set_creaunix`, whose `refresh_crossuuid` derives from the
  element's own code. In an `Element` chain (its first element code-less), a follower stating a code
  of its own would then lose the chain identity `rekeyed` stated and end under
  `TxHash(min, own code)`. `created` therefore runs first and `rekeyed` last, in both arms, so the
  `Element`-chain identity is stated last.
- **`rekeyed`** (`iterator.rs:1477-1486`) keeps `walked_follow_identity` (the chain's side and stored
  cross code) and its finalize. It forces `crossuuid` only for an `Element` chain, which is one whose
  live element's `crosshashcode` is 0. There it keeps stating the first element's identity, as today:
  that is the one cross identity a walk states rather than derives, documented as such
  (`follow_element`'s comment, `element.rs:503-506`), and FIX-only on write (`arrow.rs:1692`). For a
  `Code` chain it forces nothing. The follower holds the chain's code, `created` gives it the chain's
  minimum, and its derived `crossuuid` is the chain's.
- **`created`** (`iterator.rs:1494-1501`) gains the minimum. Today it fills only where none is
  stated. It becomes `walked_set_creaunix(earliest(Some(own creation_unix()), chain))` (`earliest`,
  `element.rs:552-557`), diff-guarded. A follower stating a later creation of its own (every FIX
  message does, at its parse) takes the chain's earlier one, and the setter derives its `crossuuid`
  once. Its doc's "the identity does not move" sentence becomes "the cross identity follows it".
- **A passed statement** (the `passed` branch, `iterator.rs:1096-1102`) is restated over the statement
  it repeats, with no `created` call today, so it folds only that statement's creation, not the
  chain's current and possibly lowered minimum. It gains `created(&mut element, ..)` with the live
  chain's current `creaunix`, where the chain is still alive under `chain`. Where it has retired,
  the statement's creation stands. It runs before its `rekeyed`.
- **An out-of-order arrival** (`iterator.rs:1104-1112`) takes `earliest(own, live)`. Where its own
  is earlier, the live element's `creaunix` is lowered to it in `alive`. This is the creation half
  of `fold_lifecycle` only: state, expiry and place are untouched, so "moves the live element not at
  all" still holds for every other fact. Statements after it then carry the chain's true minimum.
  The lowering needs `self.alive.get_mut(&identity)` outside the immutable `match` on
  `self.alive.get(&identity)`, so the arm is split: the match decides, and the mutable borrow
  lowers after it. This is put to the user (#3).
- **Already yielded members keep theirs.** The walk is lazy and rewrites no row it has already
  answered. So a chain whose minimum moves mid-life holds two cross identities in its output: the
  ones before the move and the ones after. That is decision 38's "reset" taken literally, and #3
  offers the reading that never lowers. The committed snapshot's `lifecycle` section goes from 19 to
  23 distinct coded identities by the survey's tally, which counted four such chains, e.g.
  `10:1:00079132557GLXC0` with members `[011]`/`[013]` against `[015]`/`[019]`. The passed-branch
  change can move that count, so it is re-counted at the regeneration, not taken from the survey.
  The stable chain identity is `crosshashcode` (or the stored `crosscode`), which is what joins
  (D49.5).
- **The snapshot restore** (`iterator.rs:1006-1014`) stays. For a coded chain it now writes the
  value `walked_restamp` already derived, because only `transunix`/`snapunix` moved and the walk
  stated `creaunix`. For a code-less chain it restores the chain's stated identity, as today.
- **A merge** (`merging`, `merge_timed`'s `fold_lifecycle`, `element.rs:657`) resets through the same
  setter, or through the closing `finalize`. `merge_event_element`'s `sync_cross` (`element.rs:391`)
  runs before `merge_timed` folds the creation, so it derives over the pre-merge creation. It is an
  extra derivation, and it hashes nothing in step (D49.4). The closing `finalize` resets the identity
  over the merged minimum.
- **The book** (`book.rs:218-232,3315-3345`): `LiveKey.identity`, `order_chains`' map key (`:3342`)
  and its scan path's comparison (`earlier.get_crossuuid() == later.get_crossuuid()`, `:3328`) read
  `chain_key(operation)` (the same function, reached in-crate). `book_arrow_reader` runs no lifecycle
  (`rust/fix/src/market.rs:438-439`), so two unwalked statements of one order differ in `crossuuid`
  under D49, and a `crossuuid` key would leave a ghost entry. A book's own `crossuuid` becomes
  `TxHash(earliest member creation, book crosshashcode)`, so it varies with the window a book was
  built over. Books of one instrument join by `crosscode` `3:0:{instcode}` or `crosshashcode`, never
  by `crossuuid`.
- **Moves from an event into an element with no instant.** `OperationEvent::into_element`
  (`operation.rs:678-684`, public Rust API) moves `self.data.into_entry()`, which carries the event's
  `crossuuid` into an `OperationElement` whose rule is the code rule. The `From<&E>` copies for
  `MarketFacts`/`OperationFacts` (`copy_element`, `facts.rs:2195-2202`, and `:2290-2291,2321-2322`)
  do the same. Python and Node re-finalize after the move (`python/src/graph/operation.rs:173-176`),
  but the core does not. So under D49 the output would hold the event's TxHash value, which its own
  `finalize` would reset. `into_entry`/`into_element` and the instant-less `From<&E>` impls re-derive
  `crossuuid` by the target's rule (`code_uuid(crosshashcode)`, else the element's own `uuid` for a
  code-less one) wherever the source's `cross_unix()` is `Some` and the target's is `None`. The value
  is then today's, and "every element's `crossuuid` does not move" holds. Tests, 2, pins it.

### D49.4 - `sync_cross`, and decision 39: derive only on a diff

Each identity and its inputs. A **setter** derives when, and only when, one of these moved.
`finalize`/`finalized` always derive `uuid` and `crossuuid` and write each only where it differs
(below: "`finalize` stays closed").

| Identity | Inputs | Cost of one derivation |
| --- | --- | --- |
| `crosshashcode` | `crosscode` text | one XXH3-64 of the code |
| `uuid` (event) | `transunix`, `seqnum`, `hashcode`, `crosshashcode` | one `TxHash` + one seeded XXH3 (`into_sequenced_uuid`) |
| `crossuuid` (event) | `creaunix` (else `transunix`), `crosshashcode`; `uuid` where the code hash is 0 | one `TxHash` pack, no hash |
| `hashcode` | the content the holder digests | the holder's content digest |

- **`sync_cross`** (`element.rs:270-289`) splits into two provided steps:
  - `sync_crosshashcode(&mut self) -> bool`. The default hashes the code and writes through
    `moved`, which is today's behaviour and correct for every implementor. The holders that keep the
    hash in step override it to skip the hash where the held hash is the code's (below).
  - `moved(get_crossuuid, cross_uuid())`.

  `cross_uuid()` is a pack and costs no hash, so `sync_cross` costs one `TxHash` and no XXH3 on a
  holder in step.
- **"Keep the hashed code", not "compare before hashing".** A holder cannot compare text it no
  longer holds, and a foreign `crosshashcode` stated on read-back must still be caught by
  `canonical.finalize()` (`arrow.rs:1699-1704`). So each holder keeps one bit, `crosshashed`, true
  where `crosshashcode == crosshash(crosscode)` is known. It is set by every door that hashes the
  current text and cleared by a stated `set_crosshashcode` of a value other than the held one.
  - The cross-code setters compare the text first. Equal text means no write, no hash, no
    projection. Moved text means one hash and the bit set.
  - `sync_crosshashcode` hashes only while the bit is clear.
  - `TextLine` already has the shape: `resolved.crosshashcode` is the derived cell and
    `stated.crosshashcode` the foreign one (`line.rs:1380-1395`). Its override answers "in step"
    while nothing is stated.
- **`finalize` stays closed.** The market Arrow validation (`arrow.rs:1695-1736,1868-1878`) clones
  the element, runs `canonical.finalize()` and compares the stated claims with what it left. That
  only catches a foreign `uuid` or `crossuuid` if `finalize` re-derives them unconditionally. If
  `finalized` derived nothing for an equal content code, and every setter skipped equal values, a
  stated `uuid` would survive whenever every input it met landed equal. Whether it was caught would
  then depend on the order the reader read its columns. So D39 applies to the **setters** and to
  `sync_crosshashcode` only. `finalize`/`finalized` run because facts moved, and they already digest
  the content. They compute `time_uuid()` and `cross_uuid()` every time and write each through
  `moved`. The extra cost is one `TxHash` plus one seeded XXH3 beside a content digest.

Every place that recomputes regardless of a diff today, and what it becomes:

| Site | Today | Becomes |
| --- | --- | --- |
| `Element::sync_cross` `element.rs:275-289` | hashes the code every call | `sync_crosshashcode()` (skipped in step) + `crossuuid` through `moved` |
| `Event::finalized` (provided) `element.rs:1193-1200` | `time_uuid` + `cross_uuid` every call | unchanged: it derives both every call, written through `moved` |
| `MarketEventFacts::refresh_uuids` `facts.rs:1508-1513` | both UUIDs on any input | split into `refresh_uuid()` (the `uuid` inputs) and `refresh_crossuuid()` (the `crossuuid` inputs), each writing through `moved` |
| `MarketEventFacts::set_transunix`/`set_seqnum` `facts.rs:1623-1626,1642-1645` | refresh both, always | equal value: nothing. Moved: `refresh_uuid()`, plus `refresh_crossuuid()` where `creaunix` is `None` (`set_transunix`) or the code hash is 0 (both) |
| `MarketEventFacts::set_hashcode` `:1575-1578` | refresh both | equal: nothing. Moved: `refresh_uuid()`, plus `refresh_crossuuid()` where the code hash is 0 |
| `MarketEventFacts::set_crosshashcode` `:1584-1587` | refresh both | equal: nothing. Moved: store, clear `crosshashed`, refresh both |
| `MarketEventFacts::set_crosscode` `:1561-1569` | hash + refresh both, always | text equal after `stored_crosscode`: nothing. Moved: one hash, `crosshashed` set, then the `set_crosshashcode` path (which itself skips when the hash came out equal) |
| `MarketEventFacts::set_creaunix` `:1651-1653` | nothing | equal: nothing. Moved: `refresh_crossuuid()` (no-op where the code hash is 0) |
| `MarketEventFacts::finalized` `:1699-1702` | refresh both | `set_hashcode` through `moved`, then `refresh_uuid()` and `refresh_crossuuid()` every call ("`finalize` stays closed") |
| `MarketEventFacts::finalize` `:1600-1605` | `sync_cross` (hash) + digest + `finalized` | `sync_cross` without a hash in step; the content digest stays (a finalize is called because facts moved); `finalized` as above |
| `MarketFacts`/`OperationFacts` finalize `facts.rs:446-454,1890-1898` | `sync_cross` (hash) | `sync_cross` skipped in step; `uuid = from_v8(hashcode)` is a pack and stays |
| `MarketFacts::set_crosscode` `:414-416` | stores the text | compare first, clear `crosshashed` on a move (the hash is `finalize`'s) |
| `MarketFacts::reprefix` `:333-339`, reached from `side_moved` (`:645,672`) and `set_marketdatakind` (`:345`) | hashes on a prefix move, and writes `crossuuid = self.cross_uuid()` by `MarketFacts`' own rule | returns whether the code moved and sets `crosshashed`. `MarketFacts` itself writes its code-rule `crossuuid`, which is right for an element with no instant. The event holders override the side and kind setters that reach it (`MarketEventFacts::set_side`, which today comes through `delegate_market!`, and `set_marketdatakind` `:1518`; `OperationEventFacts::set_marketdatakind` `:1809`): after a move they call `refresh_uuid()` and `refresh_crossuuid()`. An event's `crossuuid` and `uuid` then follow a side or kind move by the event's rule, with no finalize. Today the two rules agree, so the gap is invisible |
| `OperationElement::finalize` `operation.rs:303-316`; `OperationEvent` `:801`; `TradeEvent` `trade.rs:144`; `book.rs:3433` | `sync_cross` (hash) | the in-step skip via the delegated `sync_crosshashcode` |
| `Instrument::finalize` `instrument.rs:747-752` | `sync_cross` (hash) | keeps the code in step at its setter; `sync_cross` skips |
| `TextLine::derive_uuids` and its callers `line.rs:1134-1140,1376-1395,1436-1468`, `set_index` `:459-463` | drop both cells on any set | each setter compares with the value the getter answers; an equal value does nothing. A moved input drops both **stated** cells, because a stated identity restored from a row is no longer that row's: this is the restored-line contract `changing_identity_inputs_rederives_a_restored_lines_generic_identities` (`rust/tests/text/line.rs:1312-1328`) pins, and it stands. A moved input drops the **resolved** `uuid` cell, and the resolved `crossuuid` cell only where it read that input: `creaunix`/`transunix`/`crosshashcode`, or `uuid` where the cross hash is 0 |
| `TextLine::set_creaunix` `line.rs:1475-1477` | drops nothing (the cell would go stale under D49) | equal: nothing. Moved: drop the stated `crossuuid` and the resolved `crossuuid` cell |
| `TextLine::set_crosscode` `line.rs:1348-1356` | drops the hash and both identity cells | equal text: nothing |
| `TextLine::get_hashcode` `line.rs:1361-1364` | XXH3 of the whole body on **every** unstated read | a `resolved.hashcode` `OnceLock` beside `uuid`/`crossuuid`, reset by `Resolved::reset_identity` (`line.rs:304-310`, which `derive_identity` `:1126-1131` and `state_header` `:753-770` call) and by `set_body` (`:632-640`), which resets the whole `Resolved` |
| FIX `settle_clock` `msg.rs:3277-3283` | `finalized(held code)` re-derives both | the call goes: every clock write of the message reaches the projecting `set_transunix`/`set_seqnum` (audit below) |
| FIX `stamp_identity` `msg.rs:2152-2155`; `settle_refiled` `:3295-3302` | `self.hashcode()`, a digest of all content, at every settle | digests only where a digested fact moved since the last stamp. One bit on `FixMsg`, `digested`, is cleared in the **lowest** write path every digested fact reaches (the one field store every field set, the state, the chain words, the text and the metadata write through), never door by door. Otherwise the held code stands. `finalized` still derives the two identities. Put to the user (#6): it is the largest of the changes |
| The walk's `walked_restamp` `iterator.rs:214-217` | `finalized(held code)` | unchanged: `finalized` derives both, written through `moved` |
| The `From` copies `facts.rs:2283-2328`, `copy_element` `:2195-2202`; `OperationEvent::into_element` `operation.rs:678-684` | copy exact identities, re-derive `crossuuid` where `resync_copied` re-prefixed | re-derive `crossuuid` by the target's rule where the source's `cross_unix()` is `Some` and the target's `None` (D49.3) |

**Direct writes to an identity input.** The setter guards are only correct if every write to an
identity input goes through a projecting setter, or is followed by a derivation. These are all the
direct writes (`grep -nE "\.(transunix|seqnum|hashcode|crosshashcode|creaunix|uuid|crossuuid) = "`
over `rust/market/src/graph/*.rs`, `rust/fix/src/*.rs`, `rust/src/graph/*.rs`,
`rust/src/text/line.rs`):

| Site | Write | Route |
| --- | --- | --- |
| `facts.rs:336-337` `MarketFacts::reprefix` | `crosshashcode`, `crossuuid` | returns whether it moved; the event holders project (table above) |
| `facts.rs:399,407,423,431`; `:1842,1850,1866,1874` | `MarketFacts`/`OperationFacts` `set_uuid`/`set_crossuuid`/`set_hashcode`/`set_crosshashcode` | raw setters of elements with no instant; `finalize` derives |
| `facts.rs:451-453`; `:1895-1897` | element `finalize` | the derivation itself |
| `facts.rs:1510,1512` `refresh_uuids` | `uuid`, `crossuuid` | the derivation itself (split in two) |
| `facts.rs:1546,1554` `MarketEventFacts::set_uuid`/`set_crossuuid` | stated identity | a statement; the sentinel rows read it |
| `facts.rs:1563,1576,1585,1624,1643,1652,1700` | `set_crosscode`, `set_hashcode`, `set_crosshashcode`, `set_transunix`, `set_seqnum`, `set_creaunix`, `finalized` | the projecting setters themselves |
| `facts.rs:1818` `OperationFacts::at` | `event.transunix = unix` | a raw write; its doc says "finalized by the caller once the clocks are in", and `finalize` derives unconditionally. Pinned: `Order::at(unix)` then `finalize`, `uuid == time_uuid()` |
| `facts.rs:2290-2291,2321-2322` the `From` copies | `uuid`, `crossuuid` | exact copies, `crossuuid` re-derived by the target's rule (D49.3) |
| `line.rs:307-309` `reset_identity`; `:462` `set_index`; `:538` `state_source`; `:761-763` `state_header`; `:632-640` `set_body` | resolved cells | each drops the identity cells its input feeds (`set_index` through `derive_uuids`, `state_header` through `reset_identity`, `set_body` the whole `Resolved`) |
| `rust/fix/src/*.rs` | none by struct field | the message writes through `self.event`'s setters; the `digested` bit is the one FIX risk, cleared in the lowest write path |

**What D39 pins** (hashing is not an allocation, so the allocation counter cannot see a skipped
hash):

- **Sentinel rows**, one per skip: state a foreign identity, write an input unchanged, and assert the
  foreign value survives. For example, `set_crossuuid(S)`, then `set_transunix(held)`, then
  `get_crossuuid() == S`; or `set_crosshashcode(H)` foreign, then `set_crosscode(same text)`, then
  `get_crosshashcode() == H`. Then write it moved and assert the derived value. These go in
  `rust/market/tests/graph/facts.rs` (`MarketEventFacts`, through `OrderEvent`) and
  `rust/tests/text/line.rs` (`TextLine`, beside `state_generic_identities`, `:1278-1305`).
- **A side move on an event**, in `rust/market/tests/graph/facts.rs`: an `OrderEvent` with a code,
  then `set_side(Buy)` and no finalize. Then `get_crossuuid() == cross_uuid()`, which is
  `TxHash(creation, new crosshashcode)`, and `get_uuid() == time_uuid()`.
- **`finalize` stays closed**, in `rust/market/tests/graph/arrow.rs`: a row stating a foreign `uuid`
  whose inputs are all equal to the leaf's defaults is refused at `$.uuid`.
- **Allocation row** `rust/market/tests/allocations.rs:220-234`. It stays as it is for moved inputs
  and gains a second loop writing every input unchanged, plus `set_creaunix` moved: zero
  allocations, as `rust/fix/tests/allocations.rs:2150-2208` pins for the FIX message.
- **Benchmark rows**: a new `identity` group in `rust/market/benchmarks/graph/mod.rs` with
  `set_transunix`, `set_creaunix` and `set_crosscode`, each equal and moved, and `finalize` with
  nothing moved. A `TextLine::get_hashcode` repeated-read row goes in the core `text` bench. All are
  `--quick` direction only. The equal setter rows must read well below the moved ones.

### D49.5 - what may not key by `crossuuid` any more, and what each reads

| Consumer | Reads instead | Where |
| --- | --- | --- |
| The walk's live chains, expirations, names, snapshot grid | `chain_key`: `code_uuid(crosshashcode)`, else the code-less chain's stated identity. These are today's key values | `iterator.rs:663` and the sites in D49.3 |
| The book's `LiveKey` and `order_chains` (map key and scan comparison) | `chain_key` | `book.rs:218-232,3315-3345` (`:3328`, `:3342`) |
| `Instrument`'s `uuid`, `get_by_uuid`, the aliases index, `underlying_uuid`/`leg_uuids` | the code rule, `code_uuid(crosshash(code))`, through the core's one door. Values unchanged | `instrument.rs:747-752,1165-1180,1733-1738,3203-3213` |
| The mint (`mint_digest`, `is_own_mint`) | `xxh128(crosscode)` directly, as today. D42's "at P7 the argument becomes `crossuuid.as_u128()`" (`$S/p9/d42_design.md:56-72,866-873`) is **withdrawn**: a mint over an identity that moves with a creation instant would not be stable | `instrument.rs:968-989,1087-1089` |
| D45's tombstone (`$S/p12/d45_design.md:510-523`) | base cross code and `uuid`, as designed. Its live chains are the walk's, so they take `chain_key` | P12 |
| D44's `InstrumentEvent` (`$S/p10/d44_design.md:516-525`) | its `crossuuid` follows the event rule, so it is **not** the instrument's identity. The link to the instrument is `instcode`, and `Instruments::get(instcode)` | P10 |
| The medallion: primary key, joins, partitions | `crosshashcode` / `crosscode` / `instcode` (`python/tests/medallion.py:96`). Unchanged | - |
| Candles | the book's `crosscode` text (`candle.rs:145`). Unchanged | - |
| Decision 28's lineage | `crosscode` and `srcuuids`, which no rule here touches. Unchanged | `user_decisions.md:161-172` |
| D40.3's code digest of `instcode` | a code digest, never described as any row's `crossuuid` | `d40_design.md:103-109` |

### D49.6 - the fold into P7, and what other lanes must not rely on

- **One lane.** P7's phase 1 (`p7_plan.md:51-73`) is replaced by D49: the core `element.rs` (the
  rule, `cross_unix`, `creation_unix`, `code_uuid`, the `sync_cross` split), the market crate (the
  holders, `chain_key`, `created`-then-`rekeyed`, the book keys, the moves into elements,
  `cross_uuid_of`), FIX (`settle_clock`,
  the digest bit), and the text line. It lands **before** P7's phase 3 regenerates the equivalence
  snapshot (`p7_plan.md:112-131`), so the snapshot moves once, for D40.6 and D49 together. D40.5 is
  superseded whole: its rule, its "mint reads `crossuuid`", and the instruments' panel move all go.
  D40.5's "no event's `uuid` moves" still holds, because `time_uuid` reads `crosshashcode`
  (`element.rs:1250-1253`).
- **Order.** P16 opens after P12 (yg-p12: `iterator.rs`, `facts.rs`, `msg.rs`), P14 (yg-core:
  `text/line.rs`) and P9b (main tree: `book.rs`) commit. Each edits a file D49 edits. P10 (D44)
  follows P16.
- **P12** keys no ledger or tombstone by `crossuuid`. Its new walk code reads the live key through
  `Chain` and will take `chain_key` with the rest.
- **P10** does not pin "`InstrumentEvent.crossuuid` == the instrument's `uuid`", and D44's "must not
  move: every market element's `crossuuid`" (`d44_design.md:767`) is read against the post-P16
  values.
- **P14** leaves `TextLine::set_creaunix`/`derive_uuids` to P16, so the two lanes do not edit one
  setter twice.

## Tests, red first

Each fails on today's tree before the edit, except where a row is marked a guard. A guard is
green today, is written before the edit, and must stay green across it.

1. `rust/tests/graph/element.rs`: an event's `crossuuid` is
   `TxHash(creaunix, crosshashcode).into_uuid()`: its exact layout, version 7, all 64 code bits at
   bits 64-65 and 0-61, the low 48 bits the code's. With `creaunix` none it reads `transunix`. A
   code-less event is its own `uuid`. An event before the epoch takes the code rule. An element with
   no instant (`Report` fixture under `cross_unix() = None`) keeps `from_v8`.
2. Same file, the lifecycle fold:
   - `following`/`merging`/`restating` over two statements with creations 5 and 1 give both the
     identity of 1. A fold that keeps 1 against 3 moves nothing.
   - `following` over two events stating no `creaunix`, at t1 < t2, gives both `creaunix = t1` and
     `TxHash(t1)`.
   - A merge of two unstated statements in one millisecond and different microseconds keeps the
     earlier microsecond.
   - A `None`/`Some(x)` merge whose `None` side has the earlier `transunix` keeps that `transunix`.
   - In `rust/market/tests/graph/operation.rs`: `OrderEvent::at(..).into_element().get_crossuuid()`
     equals that of the `Order` built from the same facts, and so do the instant-less `From<&E>`
     copies. Also `Order::at(unix)` then `finalize` gives `uuid == time_uuid()`.
3. `rust/market/tests/graph/iterator.rs`:
   - **Guard:** two parsed-style statements of one order, each stating its own later `creaunix`,
     walk into **one** chain. It goes red the moment the rule moves while the key is still the
     element's `crossuuid`. The follower's `creaunix` and `crossuuid` are the first's.
   - **Guard:** two chains expiring at one deadline keep their `seqnum` and `uuid` across the edit
     (the key order, D49.3). Likewise, two snapshot views at one grid tick keep their row order.
   - A later statement stating an earlier creation resets every following member's `crossuuid`, the
     earlier members keeping theirs.
   - An out-of-order earlier creation lowers the live chain.
   - A passed statement restated after the chain's minimum was lowered takes the lowered minimum.
   - In an `Element` chain (code-less first element), a follower stating a code of its own and a
     later `creaunix` ends under the first element's identity (`created` before `rekeyed`).
4. **Guard**, `rust/market/tests/graph/book.rs`: two **unwalked** statements of one order with
   different `creaunix` rest as **one** entry (`LiveKey` on `chain_key`). Green today, because
   `LiveKey` is the element's `from_v8(code)`.
5. The D39 sentinels, the side-move row, the allocation row and the `finalize`-closed row (D49.4).
   In `rust/tests/text/line.rs`: `set_creaunix` moved drops a stale `crossuuid` (red today: the cell
   is not reset), and an equal set keeps a stated one.
6. `rust/market/tests/graph/arrow.rs`: a row with null `creaunix` reads back and validates. A row
   whose `crossuuid` is the pre-D49 `from_v8` value is refused at `$.crossuuid`, which names the
   rebuild.
7. `rust/fix/tests/root/msg.rs`: `settle_clock` leaves a stated `crossuuid` sentinel when only the
   clock moved to an equal value. The `digested` bit skips the content digest on a settle that
   moved no digested fact. This reads an `internals` count of digests through
   `yggdryl_fix::internals::msg`, a **new** `#[cfg(feature = "internals")] pub mod internals` in
   `rust/fix/src/msg.rs`. `python scripts/generate_internals.py` re-writes the crate-level
   re-export, and its `--check` runs in the FIX phase.
8. `rust/tests/text/line.rs` (or `rust/tests/text/arrow.rs`): a log written out of order, read
   whole, gives its later lines a lower `creaunix` and another `crossuuid`, while every `uuid` is
   unchanged (D49.2). This pins the reading #9 puts to the user.

## The pins that move, each with its sentence

Sentence for every row: "the cross identity is the TxHash of the creation instant and the cross hash
code, D49".

| Pin | Moves to |
| --- | --- |
| `rust/fix/tests/root/equivalence.snapshot`: 262 coded `crossuuid` cells | the UUIDv7 of each row's `creaunix` and `crosshashcode`. The 103 code-less cells **stand**. The `lifecycle` section's distinct coded identities go from 19 by the survey's tally to a count re-taken at the regeneration (D49.3). Regenerated once in P7 phase 3 (`YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test --locked -p yggdryl-fix --test root equivalence`, `codec.rs:3769`), with a new "It last moved when" sentence on `the_codec_answers_what_it_answered` (`codec.rs:4153-4215`) |
| `rust/tests/graph/element.rs:708,815,1034,1191,1218` (literal `from_v8` of an event's code) | the TxHash value; `:795-842` (the `sync_cross` rustdoc-shaped test) states `cross_unix` |
| `rust/tests/text/line.rs:1302` (`from_v8(crosshashcode)`) | the TxHash of the line's creation |
| `rust/tests/text/line.rs:1312-1328` (`changing_identity_inputs_rederives_a_restored_lines_generic_identities`) | **stands**: a moved input still drops both stated cells (D49.4) |
| `rust/market/tests/graph/iterator.rs:27`, `element.rs:900,1034`, `market.rs:797,2611` | the TxHash value |
| `rust/market/tests/allocations.rs:1596-1650` (`rekeyed` moves nothing for a follower) | stands for a `Code` chain (nothing forced). Re-read for the follower whose `creaunix` the walk lowers (`created` moves it once) |
| `rust/fix/tests/allocations.rs:2148-2208` (`a_rekey_of_a_fix_message_is_one_settle`: every follower's `crossuuid == identity` after `rekeyed`) | **re-read**. `rekeyed` alone no longer forces a coded chain's identity. The followers are parsed at 10:00:01, :02 and :03 with their own `creaunix`, so after `rekeyed` alone each holds `TxHash(own creation)`. The pin asserts that `rekeyed` states side and code, then runs `created` with the chain's creation and asserts the chain's `crossuuid`. Its `rekeyed(.., identity: Uuid)` call keeps its signature (the `internals` door) |
| `rust/fix/tests/root/codec.rs:4378-4380`, `fix/tests/graph/iterator.rs:34-35`, `root/msg.rs:488-489,620-621`, `root/enrich.rs:2420-2421` | the TxHash value |
| `node/tests/fix.test.js:1662` (`crossuuid` matches `-8xxx-`) | `-7xxx-`, the version nibble |
| `python/tests/graph/test_operation.py:369`, `node/tests/graph/index.test.js:222` ("one chain": `OrderEvent(CLOCK)` and `(CLOCK + 1)` share `crossuuid`; Node at 1 ns and 2 ns) | **stand**. Each pair falls inside one microsecond, so `into_uuid` gives one value. A separate case beside each, more than 1 µs apart, asserts equal `crosshashcode` and different `crossuuid` |
| Text "one chain" pins (`python/tests/text/test_line.py:83`, `node/tests/text/line.test.js:64`, `python/tests/text/test_init.py:223`) | **re-read**. They stand for a log written in order, read whole (D49.2) |
| Chain-equality pins (`rust/fix/tests/root/codec.rs:4397,4427,4563`, `enrich.rs:1764,2205-2206,2418`, `rust/market/tests/graph/iterator.rs:143,372,403,441-479,804,935,1349,1444,1836,2052-2053,2116`, `python/tests/test_fix.py:4440-4441`, `python/tests/graph/test_iterator.py:99,192`) | **stand**: the walk gives every member the chain's minimum. Each that crosses a moved minimum is re-read, not re-pinned blindly |
| Instrument pins (`python/tests/test_instrument.py:263`, `docs/graph/instrument.md:75,124`, `rust/market/tests/root/instrument.rs:111`) | **stand** (code rule unchanged) |
| The `CrossUuid` column description (`rust/src/graph/element_column.rs:119-121`) | "The TxHash of the creation instant and the code an element names; the element's own where it names none. A per-creation identity: chains join by `crosshashcode`." This moves the dump (`config/fix/fields/000000650.json:26`), the dictionary hash (`rust/fix/tests/root/store.rs:3554`, with its "It last moved when" sentence) and `docs/assets/fix.json`, regenerated in the AGENTS §2 order inside P7's own regeneration |

**Must not move:** every event `uuid` and `hashcode`, the `seqnum`s of expirations at one deadline
included; every `crosshashcode`; every element's and instrument's `crossuuid`/`uuid`, the moves out
of an event included (D49.3); every `QY` number; every code-less event's `crossuuid`; the walk's
chain key values and order; the medallion keys; every cost pin except the rows D39 adds.

**Stored tables:** a market table written before D49 is refused on read at `$.crossuuid`
(`arrow.rs:1856-1866`). The medallion's silver tables are rebuilt from bronze, as D40.5 already said
for P7.

## Cost

- One `TxHash` pack per `crossuuid` derivation and no XXH3. Today it is one `from_v8` and no hash,
  so the per-derivation cost is equal and the number of setter derivations falls (D39). `finalize`
  keeps deriving both identities every call (D49.4).
- No allocation per row: `TxHash` and `Uuid` are `Copy`. The chain key stays a 16-byte `Uuid`, and
  the `Chain` tuple is unchanged.
- One bit per holder (`crosshashed`). `MarketFacts`' size is re-pinned only if the bit does not fit
  the existing padding. The `MarketData` size pin (`rust/market/tests/graph/market_data.rs`) never
  rises, and a rise is a defect.
- One `OnceLock<u64>` on `TextLine`'s resolved cells, and one bit on `FixMsg`.

## Docs, skills, inventories

- **Docs (executed examples to rewrite):** `docs/graph/event.md:622`. `docs/graph/element.md:46` is
  an `Order`, which has no instant, so it keeps the code rule and stands.
- **Docs (executed examples to re-read):**
  - The "one chain" examples (`docs/graph/event.md:99,143,180`;
    `docs/fix/lifecycle.md:79,157,234,412,444,472,504,532`; `docs/fix/message.md:86`) stand where
    walked.
  - The chain-equality examples in `docs/fix/arrow.md:388-466` (the lifecycle stage's `crossuuid`
    chain column, in three languages) and `:819-823`.
- **Docs (prose to re-spell):**
  - `docs/hashing.md:32`; `docs/fix/capture.md:645`; `docs/fix/lifecycle.md:3,11,275,279`.
  - `docs/graph/event.md:308-320`: the Creation row's "no identity moves" becomes "the cross
    identity follows it".
  - `docs/graph/element.md:12,17,126-127`; `docs/graph/book.md:46-48`;
    `docs/types/enum/marketdatakind.md:350`; `docs/fix/message.md:16`.
  - `docs/graph/market.md:159-162`. Line 162's "the entry a book keys by `crossuuid` stay where they
    stood" becomes "the chain key": a book keys its entries by the chain key, the code's.
  - `docs/graph/market-data.md:300`: the canonical-row refusal now also refuses a table written
    before D49 at `$.crossuuid`. Say so beside the rebuild.

  Each gets one sentence: the cross identity is the creation instant's TxHash over the code hash, a
  per-creation identity rather than a chain key; joins go by `crosshashcode`/`crosscode`; and a
  chain whose minimum moved holds two.
- **Skills:** `skills/yggdryl-fix/SKILL.md:3,39,135,352` and its `references/`;
  `skills/yggdryl-market-data/SKILL.md:3,45,94,292,304,400` and its `references/` (lines from the
  survey). The rule sentence goes in each SKILL's identity row.
- **Inventories:**
  - `.api-inventory.txt:947` (`CROSSUUID_TAG_NAME`) and `:1486` (`Element::get_crossuuid`).
  - New rows: `Element::cross_unix`, `Event::creation_unix`, and `implementer::code_uuid`
    (`#[doc(hidden)]`, listed under the market section).
  - `.api-bindings.txt` prose rows `206,238,789,816`, and `python/yggdryl/_native.pyi`'s docstrings.
- **Code docs:** `rust/fix/src/crated.rs:217-225` (`CROSSUUID_TAG_NAME`), `node/src/fix.rs:1392-1397`,
  `python/yggdryl/fix.py:46,213`, and the text read's "one chain" doc (`rust/src/text/arrow.rs:1039`).
- **AGENTS.md:** rows 479, 480 and 484 (the instrument keeps the code rule; say so), and the text row
  at 1662.

## Order and checks

The phases sit inside P7's lane, in AGENTS order, with one commit for P7. Smoke per phase:

| Phase | Files | Smoke |
| --- | --- | --- |
| core | `rust/src/graph/element.rs`, `rust/src/text/line.rs`, `rust/src/text/arrow.rs` (doc), the two rustdoc `impl Element`, `rust/tests/graph/element.rs`, `rust/tests/text/line.rs` | `cargo check -p yggdryl --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl --test graph element`; `cargo test -p yggdryl --test text line`; `cargo test -p yggdryl --doc graph::element` |
| market | `facts.rs`, `iterator.rs`, `book.rs`, `operation.rs`, `trade.rs`, `market_data.rs`, `mod.rs` macros, `instrument.rs`, `implementer` use; tests above; `benchmarks/graph/mod.rs` | `cargo check -p yggdryl-market --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-market --test graph iterator`; `--test graph book`; `--test graph facts`; `--test graph operation`; `--test graph arrow`; then `--features internals` for the `internals`-gated walk pins: `cargo test -p yggdryl-market --features internals --test graph iterator` and `cargo test -p yggdryl-market --features internals --test allocations` (the `rekeyed` pins and `market_event_identity`); `cargo bench -p yggdryl-market --bench graph -- identity --quick` |
| FIX | `rust/fix/src/msg.rs` (with its new `internals` module), `enrich.rs`, `crated.rs`; FIX tests | `python scripts/generate_internals.py`, then `--check`; `cargo check -p yggdryl-fix --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-fix --features internals --test root msg`; `--test root codec`; `cargo test -p yggdryl-fix --features internals --test allocations` |
| bindings, docs | the pins and pages above | §3/§4 loops; `mkdocs build --strict`; the three docs runners in the chain |

## Put to the user (interpretations taken; say if another was meant)

1. **The identity reads `creaunix` else `transunix`. Construction writes no `creaunix`, and every
   lifecycle pass does.** A follow, a restatement and a merge fold `creation_unix()` on both sides
   and write it, so a chain stating no creation is initialized from its earliest `transunix`. A
   never-walked, never-followed hand-built event keeps a null `creaunix` column, and its row still
   derives. The alternative stamps `creaunix = transunix` at every construction, which goes stale on
   a later `set_transunix`.
2. **A code-less event keeps its own `uuid`**, and **an element with no instant (the instrument, the
   plain elements, and an event moved into one) keeps `from_v8(crosshashcode)`**. Their values do
   not move, and D40.5's XXH3-128 is withdrawn everywhere. The alternative moves the elements to
   XXH3-128 as D40.5 said.
3. **`crossuuid` is a per-creation-instant identity, not a chain key.** The walk keys chains by
   today's key value (`code_uuid(crosshashcode)`, else the code-less chain's identity) and gives
   every member the chain's current minimum. An out-of-order earlier creation lowers the live
   chain's `creaunix` and nothing else. Members yielded before the lower minimum arrived keep the
   identity they were yielded with, so a walked chain can hold two cross identities, and an unwalked
   pair of statements more than a microsecond apart holds two. Chains join by
   `crosshashcode`/`crosscode`. The alternative is a chain identity that never lowers mid-chain: a
   live chain's creation is fixed by its first statement, so an earlier creation stated later moves
   nothing, and every member of a walked chain shares one `crossuuid`. That is decision 38's "reset"
   read less literally: a reset only where a pass meets the chain for the first time.
4. **An instant before the epoch falls back to the code rule**, not to the held value. No later
   instant is refused: an `i64` nanosecond count ends in 2262.
5. **`InstrumentEvent`'s `crossuuid` follows the event rule** and is not the instrument's identity.
   P10 joins by `instcode`. The alternative overrides its `cross_unix` to `None`.
6. **`FixMsg` digests its content only where a digested fact moved.** One `digested` bit is cleared
   in the lowest write path every digested fact reaches. This is the widest D39 change, and its risk
   is a write that bypasses that path. The alternative limits D39 to the identity setters and
   `sync_crosshashcode`, and leaves the content digest per settle.
7. **The `CrossUuid` column description is rewritten**, which moves the dump, the dictionary hash
   and `docs/assets/fix.json` inside P7's regeneration. The alternative leaves a false description.
8. **"The optimized set unix of unix"** is read as the existing projecting setters
   (`set_transunix`/`set_seqnum` -> `refresh_uuids`/`derive_uuids`) carrying `set_creaunix` too. They
   derive only on a moved value, while `finalize` keeps deriving both identities every call so the
   Arrow validation stays closed. If a different door was meant, name it.
9. **A text line's `crossuuid` follows the read's running minimum.** It can therefore differ
   between lines of one object (a log not written in order, per-line `creaunix` captures) and
   between reads of one object (another scope, another minimum), while every line's `uuid` does
   not. The alternative derives a text line's identity from one object-level creation that the read
   fixes once. That keeps one `crossuuid` per object and read, but it still varies with the read's
   scope.

## Taken (decision 40, 2026-10-10)

The user answered #3 "Reset to new minimum", #1 "Only lifecycle writes it" and #9 "Read's running
minimum"; every other question here is taken as recommended (`user_decisions.md` 40). Implement
these, not the alternatives.

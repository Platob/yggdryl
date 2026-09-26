# Element

`Element` is what every graph node answers about itself: identity, cross name, digest, provenance.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Element` (`graph::element`); Rust-only - [leaves](index.md#leaves) answer it in Python/JavaScript |
| `curruuid` | `get_curruuid`/`set_curruuid`: [`Uuid`](../types/uuid.md) - UUIDv8 over the content code (undated) or [event identity](event.md#identity) (dated) |
| `crosscode` | `get_crosscode`/`set_crosscode`: name in another graph, shared by every incarnation; empty if unstated |
| `crosshashcode`, `crossuuid` | `get_crosshashcode`/`set_crosshashcode`: XXH3-64 of the cross code, zero if none; `get_crossuuid`/`set_crossuuid`: the cross element, never absent - UUIDv8 over the cross hash, else the element's own identity, so every element stands in one chain |
| `currhashcode` | `get_currhashcode`/`set_currhashcode`: XXH3-64 digest of content |
| `srcuuids` | `get_srcuuids`/`set_srcuuids`: sorted unique identities this one was read from; provenance, never chain, never digested |
| Order | `is_after()`/`is_before()` (its mirror): one strict weak order |
| `finalize` | implementor's: sync cross codes, digest content, record it, reset identity if content-derived |
| `sync_cross`, `cross_uuid` | provided: cross hash = digest of the cross code, cross element = `cross_uuid()`; call when copying a cross code; `finalize` calls it first |
| `digest` | provided, `-> Xxh3`: only the cross code (no identity, instant, source); [`Event`](event.md#identity), [`Market`](market.md#contract), [`Operation`](operation.md#contract) continue it; `finalize` appends content, then `as_u64` |
| Composite | feeds only nested `curruuid` bytes per occurrence, never `currhashcode`/content, framed by the layout's kind/count |
| `with_previous` | implementor's: this element after another; events delegate to [`Event::following`](event.md#following) |
| `merge_with` | provided: folds another same-`curruuid` statement (missing cross code, unioned sources), else no-op; events use [`Event::merging`](event.md#merging) |
| Not here | operation names: [`Operation::get_altids`](operation.md#identifier-maps); book placement: [book control](order.md#book-control) |

## Example

An undated order read from two lines: one element, two sources.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Order};
    use yggdryl::Uuid;

    let mut order = Order::new();
    order.set_crosscode("O-1001".to_owned());
    order.set_srcuuids(vec![Uuid::from_v8(2), Uuid::from_v8(1), Uuid::from_v8(2)]);
    order.finalize();

    // An undated identity is its content: UUIDv8 over its code.
    assert_eq!(order.get_curruuid(), Uuid::from_v8(u128::from(order.get_currhashcode())));
    // The sources are sorted and unique at the setter.
    assert_eq!(order.get_srcuuids(), [Uuid::from_v8(1), Uuid::from_v8(2)]);
    // The cross code names the chain: its digest and the cross element follow it.
    assert_eq!(order.get_crossuuid(), Uuid::from_v8(u128::from(order.get_crosshashcode())));
    assert_eq!(order.get_crossuuid(), order.cross_uuid());

    // The same order read from a third line: sources are provenance, not
    // content, so it is the same element, and merging unions them.
    let mut again = order.clone();
    again.set_srcuuids(vec![Uuid::from_v8(3)]);
    again.finalize();
    assert_eq!(again.get_curruuid(), order.get_curruuid());
    let merged = order.clone().merge_with(&again).expect("the same order");
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(1), Uuid::from_v8(2), Uuid::from_v8(3)]);
    assert!(merged.clone().merge_with(&merged).is_none(), "nothing moved");

    // Unsaying the cross code puts the cross element back on the element.
    let mut alone = order;
    alone.set_crosscode(String::new());
    alone.finalize();
    assert_eq!((alone.get_crosshashcode(), alone.get_crossuuid()), (0, alone.get_curruuid()));
    // An undated element states no order.
    assert!(!alone.is_after(&merged) && !merged.is_after(&alone));
    ```

=== "Python"

    ```python
    from yggdryl import graph

    line_1 = "018bcfe5-6800-7000-8000-000000000001"
    line_2 = "018bcfe5-6800-7000-8000-000000000002"
    order = graph.Order(crosscode="O-1001", srcuuids=[line_2])
    again = graph.Order(crosscode="O-1001", srcuuids=[line_1])

    # Sources are provenance, not content: the same element.
    assert again.curruuid == order.curruuid
    assert order.crossuuid != order.curruuid, "a cross code names a chain of its own"
    assert order.crosshashcode != 0

    merged = order.merge_with(again)
    assert merged is not None
    assert [source.as_py() for source in merged.srcuuids] == [line_1, line_2]
    assert merged.merge_with(merged) is None, "nothing moved"

    # With no cross code, the cross element is the element itself.
    alone = graph.Order()
    assert (alone.crosshashcode, alone.crossuuid) == (0, alone.curruuid)
    # An undated element states no order.
    assert not alone.is_after(order) and not order.is_after(alone)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const line1 = '018bcfe5-6800-7000-8000-000000000001'
    const line2 = '018bcfe5-6800-7000-8000-000000000002'
    const order = new graph.Order({ crosscode: 'O-1001', srcuuids: [line2] })
    const again = new graph.Order({ crosscode: 'O-1001', srcuuids: [line1] })

    // Sources are provenance, not content: the same element.
    assert.equal(again.curruuid, order.curruuid)
    assert.notEqual(order.crossuuid, order.curruuid, 'a cross code names a chain of its own')
    assert.notEqual(order.crosshashcode, 0n)

    const merged = order.mergeWith(again)
    assert.deepEqual(merged.srcuuids, [line1, line2])
    assert.equal(merged.mergeWith(merged), null, 'nothing moved')

    // With no cross code, the cross element is the element itself.
    const alone = new graph.Order()
    assert.equal(alone.crosshashcode, 0n)
    assert.equal(alone.crossuuid, alone.curruuid)
    // An undated element states no order.
    assert.ok(!alone.isAfter(order) && !order.isAfter(alone))
    ```

## Edges

- `set_srcuuids(Vec::new())` unsays the sources; a nonempty list sorts/dedups at the setter. Following/restating keep an observation's own; merging alone unions them.
- `set_crosscode(String::new())` unsays the cross code, reverting the cross element to this identity.
- A dated leaf's setters recompute UUID/cross element on any change to instant, sequence, content/cross code or hash, keeping identity if the instant lacks UUIDv7 (a book side: a private store, content-derived); a foreign event may keep an assigned identity.
- `is_after`/`is_before`: never after itself; equal when neither holds, so a stable sort keeps arrival order. Undated elements/book sides state no order; [`MarketData`](market-data.md) only between dated values.
- The traits validate nothing else: an unanswerable predecessor or source is the holder's to refuse.

# Paths

`FieldPath` is the one path into a nested schema or value. Every surface that
addresses a child by path resolves it here, once, and carries the resolved value
afterwards.

## Grammar

| step | spelling | reaches |
| --- | --- | --- |
| child | `.name`, or a bare `name` at the start | a struct child, resolved ASCII case-insensitively |
| position | `[0]`, `[-1]` | one list element, 0-based, a negative index counting back from the end |
| key | `['k']` | one map entry by text key |
| quoted child | `"a.b"` | a child whose name carries a dot, a bracket or a space |
| alias | `... as name`, `... as "a name"` | what to call what the path reached |

A leading dot is optional, so `price` and `.price` are the same path. The empty
path is the root and selects the value it is applied to. Rendering is the exact
inverse of parsing: parse, render, parse is the identity, and the rendered form
is canonical.

Quoting is what removes the ambiguity the plain splitters could not:
`"a.b"` is one child named `a.b`, and `a.b` is two levels. A whole number in
brackets is a position; `['7']` is a key that happens to look like one.

## Aliases

A path says which value to take. An alias says what to call the thing holding
it, and it is the one question a selector cannot answer by itself. It is spelled
the way SQL spells it, and the keyword is read in any case:

```
order.line[0].price as price
"55" as symbol
tags['k'] as "the key"
```

`alias` is what was written; `column_name` is the name the path gives what it
reaches, which is the alias where there is one and the last segment's own name
otherwise. A text read's `lift_names` takes `column_name`, so `"55" as symbol`
names its column in the same breath that selects it.

The keyword is only looked for once a path has something to alias, which leaves
`as` usable as an ordinary segment name: `order.as` is a child called `as`, and
`assets` is one name rather than `as` followed by `sets`. An alias on the root
path is refused, because the root reaches the value it is applied to and there
is nothing there for a name to be about. Walking with `parent` or `join` drops
the alias: it named what the whole path reached, and a different path reaches
something else.

Both quote styles are accepted for an alias, and the canonical rendering uses
double quotes.

## Resolve once, apply many

A path is parsed at the boundary that accepts it and never again. A caller
looking one up in a loop hoists it; every compiled plan in the crate holds its
paths already resolved. Applying a resolved path allocates nothing.

=== "Rust"

    ```rust
    use yggdryl::{FieldPath, FieldSegment};

    let path = FieldPath::from_str("order.line[0].price")?;
    assert_eq!(path.len(), 4);
    assert_eq!(path.to_string(), "order.line[0].price");
    assert_eq!(path.parent().map(|held| held.to_string()).as_deref(), Some("order.line[0]"));

    // Built from parts, with nothing rendered and nothing re-parsed.
    let built = FieldPath::new([FieldSegment::field("order"), FieldSegment::index(1)]);
    assert_eq!(built.to_string(), "order[1]");

    // An alias names what the path reached, and survives a round trip.
    let aliased = FieldPath::from_str("order.line[0].price as price")?;
    assert_eq!(aliased.alias(), Some("price"));
    assert_eq!(aliased.column_name(), Some("price"));
    assert_eq!(aliased.len(), 4, "the alias is not a segment");

    // One child named `a.b`, not two levels.
    assert_eq!(FieldPath::from_str("\"a.b\"")?.len(), 1);
    assert_eq!(FieldPath::from_str("a.b")?.len(), 2);
    ```

=== "Python"

    ```python
    from yggdryl import FieldPath

    path = FieldPath("order.line[0].price")
    assert len(path) == 4
    assert str(path) == "order.line[0].price"
    assert str(path.parent()) == "order.line[0]"

    built = FieldPath("order").join(1)
    assert str(built) == "order[1]"

    aliased = FieldPath("order.line[0].price as price")
    assert aliased.alias == "price"
    assert aliased.column_name == "price"
    assert len(aliased) == 4

    assert len(FieldPath('"a.b"')) == 1
    assert len(FieldPath("a.b")) == 2
    ```

=== "JavaScript"

    ```javascript
    const { FieldPath } = require('yggdryl')

    const path = new FieldPath('order.line[0].price')
    console.assert(path.length === 4)
    console.assert(path.toString() === 'order.line[0].price')
    console.assert(path.parent().toString() === 'order.line[0]')

    const built = new FieldPath('order').join(1)
    console.assert(built.toString() === 'order[1]')

    const aliased = new FieldPath('order.line[0].price as price')
    console.assert(aliased.alias === 'price')
    console.assert(aliased.columnName === 'price')
    console.assert(aliased.length === 4)

    console.assert(new FieldPath('"a.b"').length === 1)
    console.assert(new FieldPath('a.b').length === 2)
    ```

## Where it is used

- The [expression grammar](../expression/index.md) writes the same steps and
  shares the one segment type, which is why the path value lives beside it.
- [Text lines](../media/text.md#entries-and-paths) address one entry of a
  decoded line, and `lift_names` names the entry paths that become columns,
  each taking its alias where it writes one.

## What it is not

`FieldPath` is a *selector*: it says which child a caller wants. It is not the
walker state a recursive operation carries to report where a failure happened —
that is internal, rendered only when an error is actually produced, and the two
never merge.

The paths a URI carries, the dotted parts of a file name, a host name and a
Python qualified name are not field paths either. They split different things at
the same character, and this grammar does not reach them.

## Edges

- an empty path, or one of only whitespace -> the root.
- `order.` -> refused, naming the byte it stopped at.
- `order[1.5]`, `order[]`, `order['k` -> refused the same way.
- a position wider than 64 bits -> refused, saying so.
- `price as`, `price as one two`, `as name` -> refused; an alias is one name and
  it needs a path in front of it.
- `order.as` -> a child named `as`; `assets` -> one name.
- a quote doubled inside a quoted name -> that quote, so `"say ""hi"""` is
  `say "hi"`.
- two paths built differently but equal -> equal, and they hash alike.

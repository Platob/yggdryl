# Enums

A closed vocabulary stored as the `int32` code of its member: the member is the value, the code is what a column holds, and the codes are laid out so the stored integers sort the way the vocabulary is read.

An enum is not a [code](../codes/index.md). A code is an identity over a published registry that stores as the text it is; an enum's members are the crate's own, each already a fact a reader asks about - is it live, did it end, how - so it stores as the number that answers those questions by order alone. The family is the `enum` range of identifier bytes, `0xc0`-`0xcf`, and [State](state.md) is its one member today.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | Each enum's `DataType` variant, `Field` leaf and `Scalar` variant; the family is the `enum` range of `DataTypeId` bytes, not a type |
| Validates | At the value door, once: a member, the code of one, or a spelling one of its vocabularies names; anything else is refused naming the enum |
| Lazy | Nothing - the member table is static |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field |
| Refuses | An integer that is the code of no member, a spelling that names none; `code_width`, `string_parameters` and `bytes_parameters`, which an enum has none of |
| Errors | Rust `Error::InvalidDataType { kind, reason }` where `kind` is the enum's own name; Python `ValueError`; JavaScript throws |
| Storage | Arrow `Int32` under the enum's own extension name, so a column is one value buffer and a row group's min and max are its first and last member |
| Identity | The extension *name*: `yggdryl.state` over `int32` is a state, the same `int32` under no name is the integer it is |
| Crossing | Rust holds the member; Python the member of an `enum.IntEnum` built from the core's table; JavaScript the member's name, beside a frozen object mapping every name to its code |

## Pages

| Page | Vocabulary | Members | Arrow extension |
| --- | --- | ---: | --- |
| [State](state.md) | What state one thing is in, from asked for to ended, over FIX and a scheduler | 60 | `yggdryl.state` |

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- state:: datatype_id::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_state.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/state.test.js
    ```

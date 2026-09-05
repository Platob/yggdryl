# Query parameters

`Parameters` reads a query as the `key=value` pairs it spells, and writes the edited pairs back as one component.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `Parameters`, the pair view over a query |
| Built by | `Uri::parameters(decode)` / `Url::parameters(decode)`; Python `parameters(decode=False)` |
| Written by | `set_parameters`, or `set_query` for the component itself |
| Order | The query's own; a key may repeat |
| Duplicates | `get` answers with the first, `get_all` with every one, `insert` replaces the first and drops the rest, `append` adds another |
| Pair with no `=` | Empty value; the `&&` in `a=1&&b=2` is not a pair |
| `decode = true` | Text in and out; writes encode what the query cannot carry |
| `decode = false` | The query's own bytes; a write that would not parse back is refused |
| `+` | A literal plus, encoded as `%2B` on write, never a space |
| Zero copy | Reads borrow the query; only a changed or decoded pair owns its text |
| Rust borrow | A view borrows its URI, so `into_owned` is what frees it for write-back |
| Python | A live view of the value it came from, with the mapping dunders and methods a `dict` has |
| Bindings | Rust and Python; JavaScript reads the query component only |

## Use

=== "Rust"

    ```rust
    use yggdryl::Url;

    let mut url = Url::from_str("https://example.com/trades?symbol=AAPL&venue=XNAS&symbol=MSFT")?;

    let parameters = url.parameters(false)?;
    assert_eq!(parameters.len(), 3);
    assert_eq!(parameters.get("symbol"), Some("AAPL"));
    assert_eq!(parameters.get_all("symbol").collect::<Vec<_>>(), ["AAPL", "MSFT"]);

    // The view borrows the URL, so owning its pairs is what lets the edit be
    // written back to that same URL.
    let mut parameters = parameters.into_owned();
    parameters.insert("symbol", "TSLA")?;
    parameters.remove("venue");
    url.set_parameters(&parameters)?;
    assert_eq!(url.to_string(), "https://example.com/trades?symbol=TSLA");
    ```

=== "Python"

    ```python
    from yggdryl import Url

    url = Url("https://example.com/trades?symbol=AAPL&venue=XNAS&symbol=MSFT")
    parameters = url.parameters()

    assert len(parameters) == 3
    assert parameters["symbol"] == "AAPL"
    assert parameters.get_all("symbol") == ("AAPL", "MSFT")
    assert list(parameters.items()) == [
        ("symbol", "AAPL"),
        ("venue", "XNAS"),
        ("symbol", "MSFT"),
    ]

    # The view is live: an edit is a write to the URL it came from.
    parameters["symbol"] = "TSLA"
    del parameters["venue"]
    assert str(url) == "https://example.com/trades?symbol=TSLA"
    ```

## Decoded text

A decoding view answers with the text the escapes stand for, and encodes what it is given. A raw view answers with the query's own bytes and refuses text that would not parse back.

=== "Rust"

    ```rust
    use yggdryl::Url;

    let mut url = Url::from_str("https://example.com/t?as%20of=2026-01-02&note=a%26b")?;

    let raw = url.parameters(false)?;
    assert_eq!(raw.get("as%20of"), Some("2026-01-02"));
    assert_eq!(raw.get("note"), Some("a%26b"));

    let decoded = url.parameters(true)?;
    assert_eq!(decoded.get("as of"), Some("2026-01-02"));
    assert_eq!(decoded.get("note"), Some("a&b"));

    let mut decoded = decoded.into_owned();
    decoded.insert("side", "buy & sell")?;
    url.set_parameters(&decoded)?;
    assert!(url.to_string().ends_with("&side=buy%20%26%20sell"));
    assert_eq!(url.parameters(true)?.get("side"), Some("buy & sell"));
    ```

=== "Python"

    ```python
    from yggdryl import Url

    url = Url("https://example.com/t?as%20of=2026-01-02&note=a%26b")

    raw = url.parameters()
    assert raw["as%20of"] == "2026-01-02"
    assert raw["note"] == "a%26b"

    decoded = url.parameters(decode=True)
    assert decoded["as of"] == "2026-01-02"
    assert decoded["note"] == "a&b"
    assert decoded == {"as of": "2026-01-02", "note": "a&b"}

    decoded["side"] = "buy & sell"
    assert str(url).endswith("&side=buy%20%26%20sell")

    # A raw view refuses what it cannot write back unchanged.
    try:
        raw["side"] = "buy & sell"
    except ValueError as error:
        assert "uri query" in str(error)
    ```

## Edges

- No query at all -> an empty view, not an error; writing an empty view clears the component.
- `?flag` -> one pair with an empty value; `parameters["flag"] == ""`.
- `?a=1&&b=2` -> two pairs; an empty pair is not one.
- `%2F` in a key or value -> a literal `/`; the query has no structure a separator could change.
- `%FF` -> a raw view reads it; a decoding view refuses it, because it stands for no UTF-8 text.
- Python: a hashed `Url` is frozen, so a write through its view raises `TypeError`.
- Python: `pop(key)` raises without a default, as `dict.pop` does; `pop(key, None)` answers `None`.
- Rust: a view borrows its URI, so `url.set_parameters(&url.parameters(true)?)` cannot compile; `into_owned` is the answer.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib uri::tests::parameters
    cargo test --features "parquet iceberg" -p yggdryl --lib uri::tests::decoding
    cargo bench -p yggdryl --bench uri -- "resource_value/(parameter_pairs_raw|parameter_pairs_decoded|parameter_write_back|component_decoding)"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/uri/test_uri.py -k "parameters or decoded"
    ```

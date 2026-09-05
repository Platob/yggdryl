# Streaming

Resumable hash states that answer without consuming, plus the seed and secret rules of the XXH3 pair.

## Contract

| item | rule |
| --- | --- |
| Owns | Resumable `Xxh32`, `Xxh64`, `Xxh3`, `Xxh128` states |
| Chunk invariance | Any split of the same bytes answers one digest |
| Answering | Reading the digest leaves the state running |
| `clear()` | Back to the constructed seed and secret |
| Rust traits | Each state is a `std::hash::Hasher` and its own `BuildHasher` |
| Seeds | XXH32 and XXH64 take a seed and never a secret |
| Secretable | XXH3 pair only; ask `DigestAlgorithm::is_secretable` |
| Secret cutoff | Consulted only for inputs longer than 240 bytes |
| Secret length | At least `SECRET_MINIMUM_LENGTH`, which is 136 bytes |

## Use

Feed bytes with `write_bytes`, read the digest at any commit boundary.

=== "Rust"

    ```rust
    use yggdryl::xxhash::{Xxh3, xxh3};

    let payload = b"symbol,price\nAAPL,187.23\n";
    for split in [1, 7, payload.len()] {
        let mut state = Xxh3::new();
        for chunk in payload.chunks(split) {
            state.write_bytes(chunk);
        }
        // The split never changes the answer.
        assert_eq!(state.as_u64(), xxh3(payload));
    }

    // Answering does not consume the state, so a running digest can be read at
    // every commit boundary rather than only at the end.
    let mut state = Xxh3::new();
    state.write_bytes(b"AAPL");
    assert_eq!(state.as_u64(), state.as_u64());
    state.write_bytes(b",187.23");
    assert_eq!(state.as_u64(), xxh3(b"AAPL,187.23"));
    ```

=== "Python"

    ```python
    from yggdryl import xxhash

    payload = b"symbol,price\nAAPL,187.23\n"
    for split in (1, 7, len(payload)):
        state = xxhash.Xxh3()
        for index in range(0, len(payload), split):
            state.write_bytes(payload[index : index + split])
        assert int(state.as_digest()) == xxhash.xxh3(payload)

    state = xxhash.Xxh3()
    state.write_bytes(b"AAPL")
    assert state.as_digest() == state.as_digest()
    state.write_bytes(b",187.23")
    assert int(state.as_digest()) == xxhash.xxh3(b"AAPL,187.23")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xxhash } = require('yggdryl')

    const payload = Buffer.from('symbol,price\nAAPL,187.23\n')
    for (const split of [1, 7, payload.length]) {
      const state = new xxhash.Xxh3()
      for (let index = 0; index < payload.length; index += split) {
        state.writeBytes(payload.subarray(index, index + split))
      }
      assert.equal(state.asDigest().value(), xxhash.xxh3(payload))
    }

    const state = new xxhash.Xxh3()
    state.writeBytes(Buffer.from('AAPL'))
    assert.ok(state.asDigest().equals(state.asDigest()))
    state.writeBytes(Buffer.from(',187.23'))
    assert.equal(state.asDigest().value(), xxhash.xxh3(Buffer.from('AAPL,187.23')))
    ```

## Seeds and secrets

The examples hash 241 bytes, past the cutoff where a custom secret is consulted. One-shot calls take the same arguments; see [xxHash](index.md).

=== "Rust"

    ```rust
    use yggdryl::xxhash::{self, SECRET_MINIMUM_LENGTH, Xxh3};
    use yggdryl::{DigestAlgorithm, Error};

    assert!(!DigestAlgorithm::Xxh64.is_secretable());
    assert!(DigestAlgorithm::Xxh3.is_secretable());

    let secret = vec![0x5a_u8; SECRET_MINIMUM_LENGTH];
    let payload = vec![0x11_u8; 241];
    assert_ne!(
        xxhash::xxh3_with_secret(&payload, &secret)?,
        xxhash::xxh3(&payload),
    );
    // At or below the cutoff the secret is not consulted at all.
    assert_eq!(xxhash::xxh3_with_secret(b"AAPL", &secret)?, xxhash::xxh3(b"AAPL"));

    // A short secret is refused by length, whatever the payload: the reference
    // only consults a secret past its 240-byte cutoff, and a secret that is
    // sometimes used is worse than one that is refused.
    let short = vec![0x5a_u8; SECRET_MINIMUM_LENGTH - 1];
    let error = Xxh3::from_secret(&short).unwrap_err();
    assert!(matches!(error, Error::InvalidSecret { actual: 135, .. }));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import xxhash

    secret = bytes(xxhash.SECRET_MINIMUM_LENGTH)
    payload = bytes(241)
    assert xxhash.xxh3(payload, secret=secret) != xxhash.xxh3(payload)
    # At or below the cutoff the secret is not consulted at all.
    assert xxhash.xxh3(b"AAPL", secret=secret) == xxhash.xxh3(b"AAPL")

    with pytest.raises(ValueError, match="at least 136 bytes, got 135"):
        xxhash.Xxh3(secret=bytes(xxhash.SECRET_MINIMUM_LENGTH - 1))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xxhash } = require('yggdryl')

    const payload = Buffer.alloc(241)
    const secret = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH)
    assert.notEqual(xxhash.xxh3(payload, { secret }), xxhash.xxh3(payload))
    // At or below the cutoff the secret is not consulted at all.
    const brief = Buffer.from('AAPL')
    assert.equal(xxhash.xxh3(brief, { secret }), xxhash.xxh3(brief))

    const truncated = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH - 1)
    assert.throws(
      () => xxhash.xxh3(payload, { secret: truncated }),
      /at least 136 bytes, got 135/,
    )
    ```

## Edges

- An empty chunk -> contributes nothing, wherever it sits.
- A secret with a payload of 240 bytes or fewer -> never consulted; the derived secret answers.
- A secret below `SECRET_MINIMUM_LENGTH` -> refused whatever the payload: `Error::InvalidSecret { actual: 135, .. }`, `ValueError`, or `at least 136 bytes, got 135`.
- `clear()` -> keeps the constructed seed and secret; it never returns an unseeded state.
- `Hasher::finish` on `Xxh128` -> the low 64 bits; `as_u128` carries the full value.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib xxhash::tests -- --skip xxhash::tests::handles --skip xxhash::tests::hashed --skip xxhash::tests::values --skip published_vectors --skip xxh128_carries --skip the_module_digest_helper
    cargo bench -p yggdryl --bench xxhash -- xxhash_streaming
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/xxhash -k TestStates
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="seed changes|custom secret|streaming agrees|clear returns|state clone" node/tests/xxhash/xxhash.test.js
    ```

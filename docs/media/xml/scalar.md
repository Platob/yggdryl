# XML scalars

One document as one [`Scalar`](../../types/scalar.md), with no Arrow anywhere. The record surface is on the [Arrow page](arrow.md).

## Read a document

=== "Rust"

    ```rust
    use yggdryl::media::xml;
    use yggdryl::Scalar;
    // Attributes and child elements are one namespace of column names.
    let value = xml::from_utf8(r#"<Order id="7"><symbol>AAPL</symbol></Order>"#)?;
    assert_eq!(value.get_key_str("id").and_then(Scalar::as_str), Some("7"));
    assert_eq!(value.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));
    ```

=== "Python"

    ```python
    from yggdryl.media import xml

    # Attributes and child elements are one namespace of column names.
    value = xml.loads('<Order id="7"><symbol>AAPL</symbol></Order>')

    assert value == {"id": "7", "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    // Attributes and child elements are one namespace of column names.
    const value = xml.loads('<Order id="7"><symbol>AAPL</symbol></Order>')

    assert.deepEqual(value, { id: '7', symbol: 'AAPL' })
    ```

Every leaf is text, because text is all a document proves. What turns `9.50` into a decimal is a declared [`Field`](../../types/field.md), on the [Arrow page](arrow.md).

## An element that has both

`<Amt Ccy="EUR">9.50</Amt>` is the commonest shape a real document has - an amount with its currency, a measurement with its unit. The characters are the element's own value, and they take the name the crate already gives the payload of a root that is not a record.

=== "Rust"

    ```rust
    use yggdryl::media::xml;
    use yggdryl::Scalar;
    let value = xml::from_utf8(r#"<Amt Ccy="EUR">9.50</Amt>"#)?;
    assert_eq!(value.get_key_str("Ccy").and_then(Scalar::as_str), Some("EUR"));
    assert_eq!(value.get_key_str("value").and_then(Scalar::as_str), Some("9.50"));
    ```

=== "Python"

    ```python
    from yggdryl.media import xml

    assert xml.loads('<Amt Ccy="EUR">9.50</Amt>') == {"Ccy": "EUR", "value": "9.50"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    assert.deepEqual(xml.loads('<Amt Ccy="EUR">9.50</Amt>'), {
      Ccy: 'EUR',
      value: '9.50',
    })
    ```

Characters beside *child elements* are mixed content, which a row has no cell for, and that is refused by name.

## Repeats are sequences

=== "Rust"

    ```rust
    use yggdryl::media::xml;
    use yggdryl::Scalar;
    let value = xml::from_utf8("<legs><leg>1</leg><leg>2</leg></legs>")?;
    let legs = value.get_key_str("leg").and_then(Scalar::as_sequence).unwrap_or_default();
    assert_eq!(legs.len(), 2);
    ```

=== "Python"

    ```python
    from yggdryl.media import xml

    assert xml.loads("<legs><leg>1</leg><leg>2</leg></legs>") == {"leg": ["1", "2"]}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    assert.deepEqual(xml.loads('<legs><leg>1</leg><leg>2</leg></legs>'), {
      leg: ['1', '2'],
    })
    ```

A record names each field once, so two children with one name are one column holding both, in document order.

## Write a document

XML has no anonymous document: a value needs an element to be written as, so the root's name is an argument rather than a default nobody chose.

=== "Rust"

    ```rust
    use yggdryl::media::xml;
    use yggdryl::Scalar;
    let value = Scalar::from_record([("symbol", Scalar::from("AAPL"))])?;
    let document = xml::into_utf8("Order", &value)?;
    assert!(document.contains("<symbol>AAPL</symbol>"), "{document}");

    // And it reads back to what was written.
    assert_eq!(xml::from_utf8(&document)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.media import xml

    document = xml.dumps({"symbol": "AAPL"}, "Order")

    assert b"<symbol>AAPL</symbol>" in document
    # And it reads back to what was written.
    assert xml.loads(document) == {"symbol": "AAPL"}

    # The layout every codec takes: omitted is XML's own readable two-space
    # indent, None writes one line, an int is that many spaces, "\t" is tabs.
    assert xml.dumps({"symbol": "AAPL"}, "Order", indent=None) == (
        b'<?xml version="1.0" encoding="UTF-8"?>'
        b"<Order><symbol>AAPL</symbol></Order>"
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    const document = xml.dumps({ symbol: 'AAPL' }, 'Order')

    assert.ok(document.includes('<symbol>AAPL</symbol>'))
    // And it reads back to what was written.
    assert.deepEqual(xml.loads(document), { symbol: 'AAPL' })

    // The layout every codec takes: omitted is XML's own readable two-space
    // indent, null writes one line, a number is spaces, '\t' is tabs.
    const flat = xml.dumps({ symbol: 'AAPL' }, 'Order', { indent: null })
    assert.equal(
      flat.toString('utf8'),
      '<?xml version="1.0" encoding="UTF-8"?>' +
        '<Order><symbol>AAPL</symbol></Order>',
    )
    ```

A written leaf is always a child element: a document spells the same fact two ways and a writer picks one. Which spelling a *declared* column takes is on the [Arrow page](arrow.md), under `xml:kind`.

## Escaping

A parser rewrites some characters before a reader ever sees them, so writing them literally loses them:

- **Line-ending normalization** turns a carriage return into a newline everywhere, text content included. So a carriage return is written as `&#xD;`.
- **Attribute-value normalization** replaces every tab, newline and carriage return in an attribute value with a space. So those are written as `&#x9;`, `&#xA;` and `&#xD;`.

Reading applies the same two rules, which is why two documents differing only in line ending read as one document.

A character XML cannot carry at all - `U+0000`, most of the C0 controls, `U+FFFE` - is refused rather than escaped, because no reference can spell one either.

## Limits

=== "Rust"

    ```rust
    use yggdryl::media::xml;
    use yggdryl::Limits;
    // max_depth, max_input_bytes, max_nodes, max_documents.
    let bounded = Limits::new(2, 4096, 1_000, 1);
    assert!(xml::from_utf8_with_limits("<a><b><c/></b></a>", bounded).is_err());
    ```

=== "Python"

    ```python
    from yggdryl.media import xml

    try:
        xml.loads("<a><b><c/></b></a>", max_depth=2)
    except ValueError as refusal:
        assert "byte 6" in str(refusal), refusal
    else:
        raise AssertionError("the depth bound must refuse")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    assert.throws(
      () => xml.loads('<a><b><c/></b></a>', { maxDepth: 2 }),
      /byte 6/,
    )
    ```

Every refusal names its byte position. `MAX_PARSER_DEPTH` is a hard ceiling above any caller's bound, checked first and with its own message.

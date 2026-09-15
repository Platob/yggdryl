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

Every refusal names its byte position. `MAX_PARSER_DEPTH` is a hard ceiling above any caller's bound, checked first and with its own message.

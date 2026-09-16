# XML

`yggdryl::media::xml` reads and writes XML documents as rows, as streamed Arrow batches or as native values, and reads an XML Schema as the `Field` that types them.

## Contract

| | |
| --- | --- |
| Owns | `xml::Xml` (stateful handle form), `xml::XmlOptions`, `from_bytes`/`into_bytes` over `Scalar`, `field_from_xsd`/`field_into_xsd`, the `xml:` field vocabulary |
| Bindings | `yggdryl.media.xml` in Python and `xml` in JavaScript carry the document and schema pair; a handle composes to `yggdryl._native.Xml` and `Xml`, and `document`, `row_element` and the decode budget are record options in every language |
| Selects | A name whose media type says `application/xml` or `text/xml`, on any handle, with no format argument |
| Rows | The document element's element children, which must agree on one name unless `row_element` names one |
| Columns | An element's attributes and its child elements, in one namespace of local names |
| Types | Structure is inferred; type never is. Every inferred leaf is `utf8`; a declared `Field` types each leaf through the one value contract |
| Content coding | Accepted, because XML does not compress internally - `trades.xml.gz` reads and writes through the coding |
| Cached | Nothing. A document states no schema and carries no index, so there is no header to hold |
| Limits | `XmlOptions::limits` bounds bytes, depth and nodes, on the value surface and the record surface alike |

## Surfaces

| Page | Owns |
| --- | --- |
| [Scalars](scalar.md) | `from_bytes`/`from_utf8`, `into_bytes`/`into_utf8`, the document-to-value mapping |
| [Arrow](arrow.md) | the record surface: batch readers, the write intents, XSD, and what a read costs |

## The mapping

XML says the same fact more than one way, and a row has one cell for it. So a read accepts every spelling and a write puts back the one the document used.

| Document | Value |
| --- | --- |
| `<a>text</a>` | `"text"` - a leaf is an element with no attributes and no element children |
| `<a x="1"><b>2</b></a>` | `{x: "1", b: "2"}` - attributes and child elements share one namespace of names |
| `<Amt Ccy="EUR">9.50</Amt>` | `{Ccy: "EUR", value: "9.50"}` - characters beside attributes are the element's own value |
| `<a><b>1</b><b>2</b></a>` | `{b: ["1", "2"]}` - a repeated child is one sequence, in document order |
| `<a/>` and `<a></a>` | `""` |
| `<a xsi:nil="true"/>` | `null`; an absent attribute or element is `null` too |

A namespace prefix names a document's own vocabulary and never a field, so it is dropped and the local name is the column's. `xmlns` bindings are never columns.

## Refusals

Each names what it found, and where.

| Document | Refusal |
| --- | --- |
| Characters beside child elements | Mixed content is the one shape a row has no cell for |
| One name claimed by an attribute *and* a child element | Two spellings competing for one cell |
| One local name in two namespaces, inside one element or across rows | Two vocabularies, not one column |
| A namespace prefix nothing declared | Two undeclared prefixes would fold into one column |
| Content after the document element | One document is one document |
| `&#0;`, a raw NUL, the other C0 controls | XML cannot carry them and no reference can spell them |
| An internal DTD subset | The entities it declares are ones nothing here will resolve |
| A prolog naming a charset other than the decoded one | The bytes crossed the charset boundary already |

## Performance

Measured on 20,000 rows of four columns, one of them an attribute, release build.

| Path | Time | Throughput |
| --- | --- | --- |
| `read_arrow_reader`, nothing declared | 83.5 ms | 239 Kelem/s |
| `read_arrow_reader`, field declared | 88.3 ms | 227 Kelem/s |
| `read_arrow_reader`, one column of four | 61.7 ms | 324 Kelem/s |
| `read_arrow_field` | 57.0 ms | 351 Kelem/s |
| `from_utf8` | 50.4 ms | 28.3 MiB/s |

A document states no schema and carries no index, so unlike Avro and Parquet every one of those reads the document. What a caller declares changes how much of it is decoded, not whether it is read.

What it does not change is the Arrow side, and that is built lazily: a reader answers its schema before building anything and then builds one batch at a time, so a schema read pays the parse and nothing else, and a caller that stops after one batch stops paying. Typing a leaf answers the column's canonical value already, so the batch build takes the rows as they are rather than walking every one of them a second time.

```bash
cargo bench -p yggdryl --bench media -- media/xml
```

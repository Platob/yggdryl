# Functions

The closed function set, and its one door: a user-defined function is registered with a signature, spelled `namespace.name(...)`, and typed and called exactly as a grammar function is.

## Contract

| Key | Value |
| --- | --- |
| Owns | `FunctionSignature`, `UserFunction`, `UserRef`, the registry (`register_function`, `unregister_function`, `lookup_function`, `registered_functions`), `Function::User` |
| Spelling | `namespace.name(arguments...)`; both parts identifiers, ASCII case-insensitive, held lowercase |
| Signature | a struct `Field` named `namespace.name`: one child per parameter in position order, a parameter carrying `FUNCTION:default` optional, the return as the `FUNCTION:returns` property; `as_field` and `from_field` are lossless |
| Call | arguments bound by position, defaults filled, each cast to its parameter through `DataType::cast_scalar`; a null meeting a parameter declared `not null` answers null without a call; the answer is cast to the declared return |
| Tiers | the scalar tier calls `call`; the vectorized tier calls `call_arrow`, whose default runs `call` once per row through the one array crossing; the statistics tier never learns a user function, so a filter over one reads the rows |
| Stored | a call over plain columns is a column's `TRANSFORM:function` and `TRANSFORM:sources` ([Selectors](selectors.md#a-selector-declares-a-schema)) |
| Registry | process-wide, one implementation per qualified name, the latest registration wins; an unregistered name is refused where it is typed or bound, never silently null |
| Bindings | Python `@user_defined_function` and `@user_defined_filter` in `yggdryl.expression`; JavaScript parses and prints the spelling and refuses it at bind |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, RecordBatch};
    use yggdryl::expression::{
        FunctionSignature, UserFunction, UserRef, register_function, unregister_function,
    };
    use yggdryl::{DataType, Field, Filter, Result, Scalar, Selector, StructType};

    struct Double(FunctionSignature);

    impl UserFunction for Double {
        fn signature(&self) -> &FunctionSignature {
            &self.0
        }

        fn call(&self, arguments: &[Scalar]) -> Result<Scalar> {
            Ok(Scalar::from(arguments[0].as_i64().unwrap_or_default() * 2))
        }
    }

    let signature = FunctionSignature::new(
        UserRef::new("docs", "double")?,
        [DataType::Int64.required_field("value")],
        DataType::Int64.nullable_field("returns"),
    )?;
    register_function(Arc::new(Double(signature)))?;

    let rows = DataType::from(StructType::from_fields([DataType::Int64.nullable_field("size")])?).required_field("rows");
    let batch = RecordBatch::try_from_iter([(
        "size",
        Arc::new(Int64Array::from(vec![Some(1), None, Some(3)])) as ArrayRef,
    )])?;

    // Typed by the registered signature: a nullable argument meeting a
    // `not null` parameter makes the answer nullable.
    let selector: Selector = "docs.double(size) as doubled".parse()?;
    assert!(selector.apply_field(&rows)?.fields()[0].is_nullable());
    let doubled = selector.apply_arrow_batch(&batch)?;
    assert_eq!(doubled.column(0).as_ref(), &Int64Array::from(vec![Some(2), None, Some(6)]) as &dyn arrow_array::Array);
    assert_eq!("docs.double(size) > 2".parse::<Filter>()?.apply_arrow_batch(&batch)?.num_rows(), 1);

    // The signature is a field, and the stored column knows its function.
    let stored = selector.into_field(&rows)?;
    assert_eq!(stored.fields()[0].get_metadata("TRANSFORM:function"), Some("docs.double"));
    let field = FunctionSignature::from_field(&Field::from_str(&stored.fields()[0].to_string()).unwrap_or(stored.fields()[0].clone()));
    assert!(field.is_err() || field.is_ok());

    assert!(unregister_function(&UserRef::parse("docs.double")?));
    assert!("docs.double(size)".parse::<yggdryl::expression::Term>()?.field(&rows).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field, Filter, Selector, Term
    from yggdryl.expression import user_defined_filter, user_defined_function

    @user_defined_function(namespace="docs")
    def double(value: int) -> int:
        return value * 2

    @user_defined_function(namespace="docs", vectorized=True, returns="utf8")
    def shout(text: str) -> str:
        import pyarrow.compute as pc

        return pc.utf8_upper(text)

    @user_defined_filter(namespace="docs")
    def big(size: int | None) -> bool | None:
        return None if size is None else size > 1

    rows = Field("rows", "struct<size: int64, ccy: utf8>", nullable=False)
    batch = pa.record_batch({"size": pa.array([1, None, 3], pa.int64()), "ccy": ["a", "b", "c"]})

    # The decorated function stays a Python function, and is a term as well.
    assert double(4) == 8
    assert str(double.term("size")) == "docs.double(size)"
    assert double.signature.name == "docs.double"
    assert double.signature.metadata["FUNCTION:returns"] == "int64 not null"

    projected = Selector("docs.double(size) as doubled, docs.shout(ccy) as loud").apply_arrow_batch(batch)
    assert projected.column("doubled").to_pylist() == [2, None, 6]
    assert projected.column("loud").to_pylist() == ["A", "B", "C"]
    assert big.where("size").apply_arrow_batch(batch).column("ccy").to_pylist() == ["c"]
    assert Filter("docs.big(size)").apply_records([{"size": 5, "ccy": "x"}], rows).collect() == [{"size": 5, "ccy": "x"}]

    # A stored column derives by function and sources.
    stored = Selector("docs.double(size) as doubled").into_field(rows)
    assert stored.dtype["doubled"].transform["function"] == "docs.double"
    assert stored.dtype["doubled"].transform["sources"] == '["size"]'

    for function in (double, shout, big):
        assert function.unregister()
    try:
        Term("docs.double(size)").field(rows)
    except ValueError as error:
        assert "docs.double" in str(error)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Term } = require('yggdryl')

    // The spelling parses and prints in every language; JavaScript registers
    // nothing, so a bind refuses the function by name.
    const term = new Term('Docs.Double(size)')
    assert.equal(term.toString(), 'docs.double(size)')
    assert.equal(Term.call('docs.double', [Term.column('size')]).toString(), 'docs.double(size)')
    const rows = Field.from('rows: struct<size: int64> not null')
    assert.throws(() => term.bind(rows), /docs\.double/)
    ```

## The signature is a field

A parameter with a Python default, or a Rust field carrying `FUNCTION:default`, is optional, and a call may leave it out; every parameter after a defaulted one has to carry a default too. The default is stored as the literal the grammar spells - `1`, `'EUR'`, `date32 '2024-01-01'` - and read back cast to the parameter's datatype. `FunctionSignature::as_field` writes `namespace.name` as a struct of the parameters with `FUNCTION:returns = "<dtype> null|not null"`, and `from_field` reads it back, so a signature travels like any schema.

## Calling from Python

| Registration | Call |
| --- | --- |
| row-wise (the default) | one interpreter attachment per batch; each argument column crosses once as native values, each row is filled to the signature and handed to the function as the Python values its parameter fields project to, and the answers form one array cast to the return |
| `vectorized=True` | one call per batch; each argument column crosses as a `pyarrow` array cast to its parameter's datatype, nulls included, and the array the function answers is cast to the return |

`python benchmarks/udf.py` times `size * 2` beside both registrations of the same function.

## Edges

- `nobody.knows(x)` -> parses, and typing or binding refuses it: `expected a registered function for nobody.knows, got none; register it before binding`.
- `9lives.f(x)` -> a parse error, because `9lives` is not an identifier; `UserRef::new("9lives", "f")` names the identifier it expected.
- A defaulted parameter before a required one -> refused when the signature is built.
- An argument that does not cast to its parameter -> refused naming `namespace.name.parameter`.
- A `None` argument meeting a parameter that is not nullable -> `None` without a call; a nullable parameter receives the `None`.
- A function registered twice -> the second registration; `unregister` answers whether one was registered.
- A `where` over a user function -> reads every row: the statistics tier answers unknown for it, so nothing is pruned by it and nothing is wrongly skipped.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::user::tests
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/expression/test_user_functions.py
    python/.venv/bin/python python/benchmarks/udf.py --rows 100000 --repeat 3
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="user function" node/tests/expression
    ```

# Functions

The closed function set, and its one door: a user-defined function is registered with a signature, spelled `namespace.name(...)`, and typed and called exactly as a grammar function is.

## Contract

| Key | Value |
| --- | --- |
| Owns | `FunctionSignature`, `UserFunction`, `UserRef`, the registry (`register_function`, `unregister_function`, `lookup_function`, `registered_functions`), `Function::User` |
| Spelling | `namespace.name(arguments...)`; both parts identifiers, ASCII case-insensitive, held lowercase |
| Signature | a struct `Field` named `namespace.name`: one child per parameter in position order, a parameter carrying `FUNCTION:default` optional, the return as the `FUNCTION:returns` property; `as_field` and `from_field` are lossless |
| Call | arguments bound by position, defaults filled, each cast to its parameter through `DataType::cast_scalar`; a null meeting a parameter declared `not null` answers null without a call; the answer is cast to the declared return |
| Tiers | the scalar tier calls `call`; the vectorized tier calls `call_arrow(arguments, rows, output)`, each argument one landed [`Serie`](../types/serie.md) and the answer the `Serie` of `output`, whose default reads each row off the argument columns, runs `call` on it and lays the answers out once; the statistics tier never learns a user function, so a filter over one reads the rows |
| Stored | a call over plain columns is a column's `TRANSFORM:function` and `TRANSFORM:by` ([Selectors](selectors.md#a-selector-declares-a-schema)) |
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
    use yggdryl::{DataType, Filter, Result, Scalar, Selector, StructType};

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
    register_function(Arc::new(Double(signature.clone())))?;

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

    // The signature is a field and reads back from it; the stored column
    // knows its function.
    let declared = signature.as_field()?;
    assert_eq!(declared.name(), "docs.double");
    assert_eq!(declared.get_metadata("FUNCTION:returns"), Some("int64 null"));
    assert_eq!(FunctionSignature::from_field(&declared)?, signature);
    let stored = selector.into_field(&rows)?;
    assert_eq!(stored.fields()[0].get_metadata("TRANSFORM:function"), Some("docs.double"));

    assert!(unregister_function(&UserRef::parse("docs.double")?));
    assert!("docs.double(size)".parse::<yggdryl::expression::Term>()?.field(&rows).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field, Filter, Selector, Term, Serie
    from yggdryl.expression import user_defined_filter, user_defined_function, user_function_signature

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
    assert user_function_signature("docs.double") == double.signature

    projected = Selector("docs.double(size) as doubled, docs.shout(ccy) as loud").apply_arrow_batch(batch)
    assert projected.column("doubled").to_pylist() == [2, None, 6]
    assert projected.column("loud").to_pylist() == ["A", "B", "C"]
    assert big.where("size").apply_arrow_batch(batch).column("ccy").to_pylist() == ["c"]
    assert Serie.from_(Filter("docs.big(size)").apply_records([{"size": 5, "ccy": "x"}], rows)).as_py() == [{"size": 5, "ccy": "x"}]

    # A stored column derives by function and the terms it reads.
    stored = Selector("docs.double(size) as doubled").into_field(rows)
    assert stored.dtype["doubled"].transform["function"] == "docs.double"
    assert stored.dtype["doubled"].transform["by"] == '["size"]'

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

## Calendar parts and epoch periods

Two families read a temporal, and they answer different questions. The four *calendar parts* - `year(x)`, `month(x)`, `day(x)`, `hour(x)` - read a field off the date: 2024, 1 through 12, 1 through 31, 0 through 23. The seven *epoch periods* - `years(x)`, `quarters(x)`, `months(x)`, `weeks(x)`, `days(x)`, `hours(x)`, `minutes(x, n)` - count the whole periods from the Unix epoch to the value, floored, so an instant before 1970 is in a negative period rather than the one after it: `years('1969-12-31')` is `-1`. They are spelled in the plural as Spark's Iceberg DDL spells them, each is one [Iceberg partition transform](../media/iceberg.md#partition-transforms) (`minutes(ts, 15)` is the `minutes[15]` transform of `ts`), and each is monotone over its argument, so a range on `x` prunes a filter on `years(x)` by the same statistics.

`minutes(x, n)` always states its step `n`, a whole-number literal from 1 to 4294967295 - `minutes(ts, 1)` the minute, `minutes(ts, 15)` the quarter hour, `minutes(ts, 30)` the half hour, `minutes(ts, 60)` the hour `hours(ts)` answers. A missing step is refused by the parser, which reads `minutes` as a call of exactly two arguments; `minutes(ts, 0)`, `minutes(ts, 'x')` and a step a column holds parse, and are refused where the call is typed; a parameter supplied as a whole number is a literal there.

| Function | Argument | Answers |
| --- | --- | --- |
| `years(x)` | date or timestamp | `int32` years since 1970 |
| `quarters(x)` | date or timestamp | `int32` quarters since 1970-Q1 |
| `months(x)` | date or timestamp | `int32` months since 1970-01 |
| `weeks(x)` | date or timestamp | `int32` weeks since Monday 1969-12-29; every week starts on a Monday as an ISO 8601 week does |
| `days(x)` | date or timestamp | `date32`, the UTC day |
| `hours(x)` | timestamp | `int32` hours since the epoch |
| `minutes(x, n)` | timestamp, and a whole-number literal `n` | `int32` periods of `n` minutes since the epoch |
| `time_bucket(width, x)` | a constant fixed-length width, and a date or timestamp | `x` floored to a multiple of the width, in `x`'s own datatype, unit and zone kept |

`time_bucket` is DuckDB's, name and argument order: where an epoch period counts, it floors, so the answer is still an instant - a column a table can partition and sort by as it is. The width is text - `'15 minutes'`, `'1.5h'`, `'900s'`, ISO 8601 `'PT15M'`, a clock `'00:15:00'` - or a duration literal, read once where the call is typed; units are `ns`, `us`, `ms`, `s`, `min`, `h`, `d` and `w` with their long forms, and `'15m'` is refused because a minute and a month share the letter. Buckets start from DuckDB's origin, Monday 2000-01-03 00:00:00 - UTC for a zoned value, the wall clock for a naive one - so every width dividing a day lines up with the Unix epoch and a week starts on a Monday. A calendar width, a zero or negative one, a width finer than `x`'s unit or not a whole multiple of it, a clock width under a date and a width a column holds are refused naming the argument. The function is monotone, so a range on `x` prunes through it.

A date has no clock, so a sub-day period over one is refused where it is typed; a null answers null, and so does a period past `int32` - never a number wrapped back into range - so the column is nullable wherever its source's count reaches one, even over a required source: every period over seconds, `months` and finer over milliseconds or a `date64`, `hours` and `minutes(x, 1)` over microseconds, and none over a `date32` or nanoseconds. A calendar unit is not a fixed length, so `truncate(x, 'month')` stays refused and `months(x)` is how a month is read.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Selector, StructType, TimeUnit, Timezone};

    let root = Field::new(
        "rows",
        DataType::from(StructType::from_fields([DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::NAIVE,
        }
        .required_field("ts")])?),
        false,
    );
    let selector: Selector = "year(ts) as calendar, years(ts) as y, weeks(ts) as w, minutes(ts, 15) as q, days(ts) as d".parse()?;
    let published = selector.apply_field(&root)?;
    assert_eq!(published.fields()[1].dtype(), &DataType::Int32);
    assert_eq!(published.fields()[4].dtype(), &DataType::date32());

    // 2017-11-16T22:31:08: the calendar year is 2017, the 47th year since 1970.
    let row = Scalar::from_sequence([Scalar::datetime64(1_510_871_468_000_000, TimeUnit::Microsecond, Timezone::NAIVE)?]);
    let answered = selector.apply_scalar(&root, &row)?;
    let cells = answered.as_sequence().expect("a row");
    assert_eq!(cells[0], Scalar::from(2017));
    assert_eq!(cells[1], Scalar::from(47));
    assert_eq!(cells[2], Scalar::from(2498));
    assert_eq!(cells[3], Scalar::from(1_678_746));
    assert_eq!(cells[4], Scalar::date32(17_486));

    // The half hour is the step 30, and the step is always written.
    let half_hours: Selector = "minutes(ts, 30) as h".parse()?;
    let cells = half_hours.apply_scalar(&root, &row)?;
    assert_eq!(cells.as_sequence().expect("a row")[0], Scalar::from(839_373));
    assert!("minutes(ts)".parse::<Selector>().is_err());

    // Before the epoch, a period is negative: the last day of 1969 is year -1.
    let before = Scalar::from_sequence([Scalar::datetime64(-1, TimeUnit::Microsecond, Timezone::NAIVE)?]);
    let cells = selector.apply_scalar(&root, &before)?;
    assert_eq!(cells.as_sequence().expect("a row")[1], Scalar::from(-1));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, Selector, Serie

    root = Field("rows", "struct<ts:timestamp(us)>", False)
    selector = Selector("year(ts) as calendar, years(ts) as y, minutes(ts, 15) as q, days(ts) as d")
    assert str(selector) == "year(ts) as calendar, years(ts) as y, minutes(ts, 15) as q, days(ts) as d"
    assert selector.names == ["calendar", "y", "q", "d"]

    published = selector.apply_field(root)
    assert published.dtype["y"].dtype == DataType("int32")
    assert published.dtype["q"].dtype == DataType("int32")
    assert published.dtype["d"].dtype == DataType("date32")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Selector } = require('yggdryl')

    const root = new Field('rows', 'struct<ts:timestamp(us)>', false)
    const selector = new Selector('year(ts) as calendar, years(ts) as y, minutes(ts, 15) as q, days(ts) as d')
    assert.equal(selector.toString(), 'year(ts) as calendar, years(ts) as y, minutes(ts, 15) as q, days(ts) as d')
    assert.deepEqual(selector.names, ['calendar', 'y', 'q', 'd'])

    const published = selector.applyField(root)
    assert.equal(String(published.dtype.getFieldAt(1).dtype), 'int32')
    assert.equal(String(published.dtype.getFieldAt(2).dtype), 'int32')
    assert.equal(String(published.dtype.getFieldAt(3).dtype), 'date32')
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
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- user
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_expression.py
    python/.venv/bin/python python/benchmarks/udf.py --rows 100000 --repeat 3
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="user function" node/tests/expression.test.js
    ```

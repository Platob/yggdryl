# MarketDataType

What type of its kind a market element is - how an order is priced, what a quote commits to, what kind of trade was reported, what a book entry is, what a trade report, a quote request, a mass cancel or a market data request asks for - as an enum of one hundred and eighteen members, stored as the `uint16` code of its member. The hundreds of the code name the FIX code set it types: `1xx` `OrdType(40)`, `2xx` `QuoteType(537)`, `3xx` `TrdType(828)`, `4xx` `MDEntryType(269)`, `5xx` `TradeReportType(856)`, `6xx` `QuoteRequestType(303)`, `7xx` `MassCancelRequestType(530)`, `8xx` `SubscriptionRequestType(263)`, each closing with an `OTHER` catch-all at `x99`, and `UNKN` at `0`. It is the second column of every [market data row](../../graph/market-data.md), after the [`marketdatakind`](marketdatakind.md) it types within.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `marketdatatype`, `MarketDataTypeType`/`MarketDataTypeField`, the `MarketDataType` enum and `Scalar::MarketDataType`; `DataType::marketdatatype()`; `MARKETDATATYPE_FIX_TAGS` and `MARKETDATATYPE_MSGTYPE_RULES` |
| Validates | A member, the code of one, or a spelling - the stored name in any case, or the FIX specification's own name for the value, folded - reaches one member; anything else is refused rather than stored |
| Lazy | Nothing - the member table is static |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | An integer that is the code of no member, naming the code; a spelling that names no type, naming the spelling |
| Stores | `uint16` under `yggdryl.marketdatatype`: the member's code |
| Reads FIX | `from_fix(tag, wire)` reads one wire value of any of the eight typing fields, a value no member names reading as its set's catch-all; `fix_code()` answers it back. A [registry](../../fix/registry.md#a-field-maps-its-values-onto-a-market-data-type) may map any field's values onto members of its own choosing, read before the crate's table |

The kind says what an element is - an order, a quote, a trade, a book; the type says which of its kind: a limit order `ORDLIMIT`, a tradeable quote `QUOTRAD`, a block trade `TRDBLOCK`, a bid entry `BOOKBID`, an accepted trade report `TRPTACCEPT`, a mass cancel of every order `MCXALL`.

## DataType

`marketdatatype` is the one spelling, `DataType::marketdatatype()` the constructor; kind `enum`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::marketdatatype(), DataType::MarketDataType);
    assert_eq!(DataType::from_str("marketdatatype")?, DataType::MarketDataType);
    assert_eq!(DataType::MarketDataType.to_string(), "marketdatatype");
    assert_eq!(DataType::MarketDataType.kind(), DataTypeKind::Enum);
    assert_eq!(DataType::MarketDataType.id().as_u8(), 0xc4);
    assert!(DataType::MarketDataType.is_enum() && !DataType::MarketDataType.is_code());
    assert_eq!(DataType::MarketDataType.code_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    kind = DataType("marketdatatype")
    assert (kind.id, kind.kind, kind.code_width) == ("marketdatatype", "enum", None)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const kind = new DataType('marketdatatype')
    assert.equal(kind.id, 'marketdatatype')
    assert.equal(kind.kind, 'enum')
    assert.equal(kind.codeWidth, null)
    ```

## Field

`MarketDataTypeField` is the typed marker; Python and JavaScript name the factory `marketdatatype`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MarketDataTypeField};

    let kind = MarketDataTypeField::unit("marketdatatype", false);
    assert_eq!(kind.dtype(), &DataType::MarketDataType);
    assert_eq!(
        kind.to_field(),
        Field::new("marketdatatype", DataType::MarketDataType, false)
    );
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field

    kind = yggdryl.marketdatatype("marketdatatype", nullable=False)
    assert isinstance(kind, Field)
    assert kind.dtype == DataType("marketdatatype")
    assert not kind.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const kind = fields.marketdatatype('marketdatatype', { nullable: false })
    assert.equal(kind.dtype.toString(), 'marketdatatype')
    assert.equal(kind.nullable, false)
    ```

## Scalar

The value is the member, whichever spelling named it: `ORDLIMIT` for `ORDLIMIT`, `ordlimit`, FIX's `Limit` or the code `102`. Rust holds the `MarketDataType` member; Python the member of the `yggdryl.MarketDataType` `IntEnum`, which is the integer it stores and renders as its stored name; JavaScript the member's name, with `MarketDataType` mapping every name to its code ([Enums](index.md#enum-facts-in-the-bindings)).

=== "Rust"

    ```rust
    use yggdryl::{DataType, MarketDataType, Scalar};

    let limit = DataType::MarketDataType.scalar("ORDLIMIT")?;
    assert_eq!(limit, Scalar::MarketDataType(MarketDataType::OrdLimit));
    assert_eq!(limit.kind(), "marketdatatype");
    assert_eq!(MarketDataType::OrdLimit.code(), 102);

    // The stored name in any case, FIX's own name and the code reach one member.
    assert_eq!(DataType::MarketDataType.scalar("ordlimit")?, limit);
    assert_eq!(DataType::MarketDataType.scalar("Limit")?, limit);
    assert_eq!(DataType::MarketDataType.scalar(102_i32)?, limit);
    assert_eq!(
        DataType::MarketDataType.scalar("block trade")?,
        Scalar::MarketDataType(MarketDataType::TrdBlock)
    );

    // A stored code is an integer, never text; a wire value is no spelling;
    // the code of no member answers nothing.
    assert!(DataType::MarketDataType.scalar("102").is_err());
    assert!(DataType::MarketDataType.scalar("2").is_err());
    assert!(DataType::MarketDataType.scalar(198_i32).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, MarketDataType

    kind = DataType("marketdatatype")
    assert kind.scalar("ORDLIMIT").as_py() is MarketDataType.ORDLIMIT
    assert kind.scalar("Limit").as_py() is MarketDataType.ORDLIMIT
    assert kind.scalar(301).as_py() is MarketDataType.TRDBLOCK
    assert kind.scalar("ORDLIMIT").kind == "marketdatatype"

    # A member is the integer it stores and reads as its name.
    assert MarketDataType.BOOKBID == 400 and str(MarketDataType.BOOKBID) == "BOOKBID"

    with pytest.raises(ValueError, match="marketdatatype"):
        kind.scalar("102")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, MarketDataType } = require('yggdryl')

    const kind = new DataType('marketdatatype')
    assert.equal(kind.scalar('ordlimit').asJs(), 'ORDLIMIT')
    assert.equal(kind.scalar('Limit').asJs(), 'ORDLIMIT')
    assert.equal(MarketDataType.TRDBLOCK, 301)
    assert.throws(() => kind.scalar('102'), /marketdatatype/)
    ```

## Arrow storage

`UInt16` under `yggdryl.marketdatatype`: one value buffer of codes, like every [enum](index.md). Text entering the column is read as a spelling and an integer of any width, signed or unsigned, as a code, each refused - or null under `safe` in a nullable column - where it names no member; the column cast to text answers each member's stored name, and cast to an integer its code.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray, UInt16Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let kind = Field::new("marketdatatype", DataType::MarketDataType, false);
    let arrow = kind.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.marketdatatype");
    assert_eq!(Field::from_arrow_field(&arrow)?, kind);

    // Spellings land as the codes their members store.
    let spelled: ArrayRef = Arc::new(StringArray::from(vec!["ORDLIMIT", "block trade", "bookbid"]));
    let stored = Serie::from_arrow_array(Some(&kind), spelled, ArrowCastOptions::new())?
        .into_arrow_array()
        .expect("a column, not a run");
    let codes = stored.as_any().downcast_ref::<UInt16Array>().expect("uint16 codes");
    assert_eq!(codes.values().to_vec(), [102, 301, 400]);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, MarketDataType, Serie

    kind = Field("marketdatatype", "marketdatatype")
    arrow_field = kind.into_arrow()
    assert arrow_field.type.storage_type == pa.uint16()
    assert arrow_field.type.extension_name == "yggdryl.marketdatatype"
    assert Field.from_arrow(arrow_field) == kind

    stored = Serie.from_arrow_array(pa.array(["ORDLIMIT", "block trade"]), kind, safe=False)
    assert stored.as_py() == [MarketDataType.ORDLIMIT, MarketDataType.TRDBLOCK]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { MarketDataType, Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['ORDLIMIT', 'block trade']), fields.marketdatatype('type'))
    assert.deepEqual([...stored.intoArrowArray()], [MarketDataType.ORDLIMIT, MarketDataType.TRDBLOCK])
    ```

## The members

The code groups by the FIX set it types; the stored name is what a column renders and every text format writes; the FIX value is the field and wire value `from_fix` reads and `fix_code` answers; the FIX name is the specification's own name for the value, folded, which `from_spelling` reads beside the stored name - a name two sets share, such as `Counter`, reaches neither and is read by its stored name alone. Every member states what it means (`description`).

| Code | Stored name | FIX value | FIX name | Description |
| ---: | --- | --- | --- | --- |
| `0` | `UNKN` | - | - | No type stated |
| `101` | `ORDMKT` | `OrdType(40)` = `1` | `market` | Market order |
| `102` | `ORDLIMIT` | `OrdType(40)` = `2` | `limit` | Limit order |
| `103` | `ORDSTOP` | `OrdType(40)` = `3` | `stop` | Stop, or stop loss, order |
| `104` | `ORDSTOPLIMIT` | `OrdType(40)` = `4` | `stoplimit` | Stop limit order |
| `105` | `ORDMOC` | `OrdType(40)` = `5` | `marketonclose` | Market on close order |
| `106` | `ORDWOW` | `OrdType(40)` = `6` | `withorwithout` | With or without order |
| `107` | `ORDLOB` | `OrdType(40)` = `7` | `limitorbetter` | Limit or better order |
| `108` | `ORDLWOW` | `OrdType(40)` = `8` | `limitwithorwithout` | Limit with or without order |
| `109` | `ORDBASIS` | `OrdType(40)` = `9` | `onbasis` | On basis order |
| `110` | `ORDONCLOSE` | `OrdType(40)` = `A` | `onclose` | On close order |
| `111` | `ORDLOC` | `OrdType(40)` = `B` | `limitonclose` | Limit on close order |
| `112` | `ORDFXMKT` | `OrdType(40)` = `C` | `forexmarket` | Forex market order |
| `113` | `ORDPREVQUOTED` | `OrdType(40)` = `D` | `previouslyquoted` | Previously quoted order |
| `114` | `ORDPREVINDIC` | `OrdType(40)` = `E` | `previouslyindicated` | Previously indicated order |
| `115` | `ORDFXLIMIT` | `OrdType(40)` = `F` | `forexlimit` | Forex limit order |
| `116` | `ORDFXSWAP` | `OrdType(40)` = `G` | `forexswap` | Forex swap order |
| `117` | `ORDFXPREVQUOTED` | `OrdType(40)` = `H` | `forexpreviouslyquoted` | Forex previously quoted order |
| `118` | `ORDFUNARI` | `OrdType(40)` = `I` | `funari` | Funari order: a limit day order whose unexecuted part becomes a market on close order |
| `119` | `ORDMIT` | `OrdType(40)` = `J` | `marketiftouched` | Market if touched order |
| `120` | `ORDMKTLIMIT` | `OrdType(40)` = `K` | `marketwithleftoveraslimit` | Market order whose unexecuted part stands as a limit order |
| `121` | `ORDPREVFUND` | `OrdType(40)` = `L` | `previousfundvaluationpoint` | Order at the previous fund valuation point |
| `122` | `ORDNEXTFUND` | `OrdType(40)` = `M` | `nextfundvaluationpoint` | Order at the next fund valuation point |
| `123` | `ORDPEGGED` | `OrdType(40)` = `P` | `pegged` | Pegged order |
| `124` | `ORDCOUNTER` | `OrdType(40)` = `Q` | `counterorderselection` | Counter-order selection |
| `125` | `ORDSTOPBO` | `OrdType(40)` = `R` | `stoponbidoroffer` | Stop on bid or offer order |
| `126` | `ORDSTOPLIMITBO` | `OrdType(40)` = `S` | `stoplimitonbidoroffer` | Stop limit on bid or offer order |
| `127` | `ORDMKTBAND` | `OrdType(40)` = `T` | `marketwithinpriceband` | Market order within a price band |
| `199` | `ORDOTHER` | any other `OrdType(40)` | - | An order type no member names |
| `200` | `QUOINDIC` | `QuoteType(537)` = `0` | `indicative` | Indicative quote |
| `201` | `QUOTRAD` | `QuoteType(537)` = `1` | `tradeable` | Tradeable quote |
| `202` | `QUORESTR` | `QuoteType(537)` = `2` | `restrictedtradeable` | Restricted tradeable quote |
| `203` | `QUOCOUNTER` | `QuoteType(537)` = `3` | - | Counter quote |
| `204` | `QUOINIT` | `QuoteType(537)` = `4` | `initiallytradeable` | Initially tradeable quote |
| `299` | `QUOOTHER` | any other `QuoteType(537)` | - | A quote type no member names |
| `300` | `TRDREG` | `TrdType(828)` = `0` | `regulartrade` | Regular trade |
| `301` | `TRDBLOCK` | `TrdType(828)` = `1` | `blocktrade` | Block trade |
| `302` | `TRDEFP` | `TrdType(828)` = `2` | `efp` | Exchange for physical |
| `303` | `TRDTRANSFER` | `TrdType(828)` = `3` | `transfer` | Transfer |
| `304` | `TRDLATE` | `TrdType(828)` = `4` | `latetrade` | Late trade |
| `305` | `TRDT` | `TrdType(828)` = `5` | `ttrade` | T trade |
| `306` | `TRDWAP` | `TrdType(828)` = `6` | `weightedaveragepricetrade` | Weighted average price trade |
| `307` | `TRDBUNCHED` | `TrdType(828)` = `7` | `bunchedtrade` | Bunched trade |
| `308` | `TRDLATEBUNCHED` | `TrdType(828)` = `8` | `latebunchedtrade` | Late bunched trade |
| `309` | `TRDPRIORREF` | `TrdType(828)` = `9` | `priorreferencepricetrade` | Prior reference price trade |
| `310` | `TRDAFTERHOURS` | `TrdType(828)` = `10` | `afterhourstrade` | After hours trade |
| `311` | `TRDEFR` | `TrdType(828)` = `11` | `exchangeforrisk` | Exchange for risk |
| `312` | `TRDEFS` | `TrdType(828)` = `12` | `exchangeforswap` | Exchange for swap |
| `315` | `TRDTAS` | `TrdType(828)` = `15` | `tradingatsettlement` | Trading at settlement |
| `316` | `TRDAON` | `TrdType(828)` = `16` | `allornone` | All or none trade |
| `324` | `TRDERROR` | `TrdType(828)` = `24` | `errortrade` | Error trade |
| `338` | `TRDLARGE` | `TrdType(828)` = `38` | `largetrade` | Large trade |
| `345` | `TRDEXERCISE` | `TrdType(828)` = `45` | `optionexercise` | Option exercise |
| `350` | `TRDPORTFOLIO` | `TrdType(828)` = `50` | `portfoliotrade` | Portfolio trade |
| `351` | `TRDVWAP` | `TrdType(828)` = `51` | `volumeweightedaveragetrade` | Volume weighted average trade |
| `354` | `TRDOTC` | `TrdType(828)` = `54` | `otc` | Over the counter trade |
| `356` | `TRDOPENING` | `TrdType(828)` = `56` | `openingtrade` | Opening trade |
| `357` | `TRDNETTED` | `TrdType(828)` = `57` | `nettedtrade` | Netted trade |
| `362` | `TRDDARK` | `TrdType(828)` = `62` | `darktrade` | Dark trade |
| `363` | `TRDTECHNICAL` | `TrdType(828)` = `63` | `technicaltrade` | Technical trade |
| `364` | `TRDBENCHMARK` | `TrdType(828)` = `64` | `benchmark` | Benchmark trade |
| `365` | `TRDPACKAGE` | `TrdType(828)` = `65` | `packagetrade` | Package trade |
| `366` | `TRDROLL` | `TrdType(828)` = `66` | `rolltrade` | Roll trade |
| `367` | `TRDCLOSING` | `TrdType(828)` = `67` | `closingpricetrade` | Closing price trade |
| `399` | `TRDOTHER` | any other `TrdType(828)` | - | A trade type no member names |
| `400` | `BOOKBID` | `MDEntryType(269)` = `0` | `bid` | Book bid entry |
| `401` | `BOOKOFFER` | `MDEntryType(269)` = `1` | `offer` | Book offer entry |
| `402` | `BOOKTRADE` | `MDEntryType(269)` = `2` | `trade` | Book trade entry |
| `403` | `BOOKINDEX` | `MDEntryType(269)` = `3` | `indexvalue` | Index value entry |
| `404` | `BOOKOPEN` | `MDEntryType(269)` = `4` | `openingprice` | Opening price entry |
| `405` | `BOOKCLOSE` | `MDEntryType(269)` = `5` | `closingprice` | Closing price entry |
| `406` | `BOOKSETTLE` | `MDEntryType(269)` = `6` | `settlementprice` | Settlement price entry |
| `407` | `BOOKHIGH` | `MDEntryType(269)` = `7` | `tradingsessionhighprice` | Session high price entry |
| `408` | `BOOKLOW` | `MDEntryType(269)` = `8` | `tradingsessionlowprice` | Session low price entry |
| `409` | `BOOKVWAP` | `MDEntryType(269)` = `9` | `vwap` | Volume weighted average price entry |
| `410` | `BOOKIMBALANCE` | `MDEntryType(269)` = `A` | `imbalance` | Imbalance entry |
| `411` | `BOOKVOLUME` | `MDEntryType(269)` = `B` | `tradevolume` | Trade volume entry |
| `412` | `BOOKOI` | `MDEntryType(269)` = `C` | `openinterest` | Open interest entry |
| `417` | `BOOKMID` | `MDEntryType(269)` = `H` | `midprice` | Mid price entry |
| `418` | `BOOKEMPTY` | `MDEntryType(269)` = `J` | `emptybook` | Empty book entry |
| `499` | `BOOKOTHER` | any other `MDEntryType(269)` | - | A book entry type no member names |
| `500` | `TRPTSUBMIT` | `TradeReportType(856)` = `0` | `submit` | Trade report submitted |
| `501` | `TRPTALLEGED` | `TradeReportType(856)` = `1` | `alleged` | Trade report alleged by the counterparty |
| `502` | `TRPTACCEPT` | `TradeReportType(856)` = `2` | `accept` | Trade report accepted |
| `503` | `TRPTDECLINE` | `TradeReportType(856)` = `3` | `decline` | Trade report declined |
| `504` | `TRPTADDENDUM` | `TradeReportType(856)` = `4` | `addendum` | Trade report addendum |
| `505` | `TRPTNOWAS` | `TradeReportType(856)` = `5` | `nowas` | Trade report replacing an earlier one: no, was |
| `506` | `TRPTCANCEL` | `TradeReportType(856)` = `6` | `tradereportcancel` | Trade report canceled |
| `507` | `TRPTBREAK` | `TradeReportType(856)` = `7` | `lockedintradebreak` | Locked-in trade broken |
| `508` | `TRPTDEFAULTED` | `TradeReportType(856)` = `8` | `defaulted` | Trade report defaulted |
| `509` | `TRPTINVALIDCMTA` | `TradeReportType(856)` = `9` | `invalidcmta` | Trade report with an invalid give-up agreement |
| `510` | `TRPTPENDED` | `TradeReportType(856)` = `10` | `pended` | Trade report pended |
| `511` | `TRPTALLEGEDNEW` | `TradeReportType(856)` = `11` | `allegednew` | New trade report alleged |
| `512` | `TRPTALLEGEDADD` | `TradeReportType(856)` = `12` | `allegedaddendum` | Trade report addendum alleged |
| `513` | `TRPTALLEGEDNOWAS` | `TradeReportType(856)` = `13` | `allegednowas` | Replacing trade report alleged |
| `514` | `TRPTALLEGEDCANCEL` | `TradeReportType(856)` = `14` | `allegedtradereportcancel` | Trade report cancel alleged |
| `515` | `TRPTALLEGEDBREAK` | `TradeReportType(856)` = `15` | `allegedtradebreak` | Trade break alleged |
| `599` | `TRPTOTHER` | any other `TradeReportType(856)` | - | A trade report type no member names |
| `601` | `QRQMANUAL` | `QuoteRequestType(303)` = `1` | `manual` | Quote requested by hand |
| `602` | `QRQAUTO` | `QuoteRequestType(303)` = `2` | `automatic` | Quote requested automatically |
| `699` | `QRQOTHER` | any other `QuoteRequestType(303)` | - | A quote request type no member names |
| `701` | `MCXSECURITY` | `MassCancelRequestType(530)` = `1` | `cancelordersforasecurity` | Cancel the orders for a security |
| `702` | `MCXUNDERLYING` | `MassCancelRequestType(530)` = `2` | `cancelordersforanunderlyingsecurity` | Cancel the orders for an underlying security |
| `703` | `MCXPRODUCT` | `MassCancelRequestType(530)` = `3` | `cancelordersforaproduct` | Cancel the orders for a product |
| `704` | `MCXCFI` | `MassCancelRequestType(530)` = `4` | `cancelordersforacficode` | Cancel the orders for a CFI code |
| `705` | `MCXSECTYPE` | `MassCancelRequestType(530)` = `5` | `cancelordersforasecuritytype` | Cancel the orders for a security type |
| `706` | `MCXSESSION` | `MassCancelRequestType(530)` = `6` | `cancelordersforatradingsession` | Cancel the orders for a trading session |
| `707` | `MCXALL` | `MassCancelRequestType(530)` = `7` | `cancelallorders` | Cancel all orders |
| `708` | `MCXMARKET` | `MassCancelRequestType(530)` = `8` | `cancelordersforamarket` | Cancel the orders for a market |
| `709` | `MCXSEGMENT` | `MassCancelRequestType(530)` = `9` | `cancelordersforamarketsegment` | Cancel the orders for a market segment |
| `710` | `MCXGROUP` | `MassCancelRequestType(530)` = `A` | `cancelordersforasecuritygroup` | Cancel the orders for a security group |
| `711` | `MCXISSUER` | `MassCancelRequestType(530)` = `B` | `cancelordersforsecuritiesissuer` | Cancel the orders for a securities issuer |
| `712` | `MCXUNDISSUER` | `MassCancelRequestType(530)` = `C` | `cancelordersforissuerofunderlyingsecurity` | Cancel the orders for the issuer of an underlying security |
| `799` | `MCXOTHER` | any other `MassCancelRequestType(530)` | - | A mass cancel request type no member names |
| `800` | `MDRSNAPSHOT` | `SubscriptionRequestType(263)` = `0` | `snapshot` | Market data requested as one snapshot |
| `801` | `MDRSUBSCRIBE` | `SubscriptionRequestType(263)` = `1` | `snapshotandupdates` | Market data requested as a snapshot and its updates |
| `802` | `MDRUNSUBSCRIBE` | `SubscriptionRequestType(263)` = `2` | `disablepreviousrequest` | A market data subscription withdrawn |
| `899` | `MDROTHER` | any other `SubscriptionRequestType(263)` | - | A market data request type no member names |

=== "Rust"

    ```rust
    use yggdryl::MarketDataType;

    assert_eq!(MarketDataType::ALL.len(), 118);
    assert!(MarketDataType::ALL.windows(2).all(|pair| pair[0].code() < pair[1].code()));
    assert_eq!(MarketDataType::from_code(400), Some(MarketDataType::BookBid));
    assert_eq!(MarketDataType::from_name("TRDBLOCK"), Some(MarketDataType::TrdBlock));
    assert_eq!(MarketDataType::from_spelling("market_if_touched"), Some(MarketDataType::OrdMarketIfTouched));
    // `Counter` names a quote type and an order selection alike: neither.
    assert_eq!(MarketDataType::from_spelling("Counter"), None);
    assert_eq!(MarketDataType::from_spelling("QUOCOUNTER"), Some(MarketDataType::QuoCounter));
    assert_eq!(MarketDataType::OrdMarket.as_str(), "ORDMKT");
    assert!(MarketDataType::QuoTradeable.description().starts_with("Tradeable"));
    ```

=== "Python"

    ```python
    from yggdryl import MarketDataType

    assert len(MarketDataType) == 118
    codes = [int(member) for member in MarketDataType]
    assert codes == sorted(codes)
    assert MarketDataType(400) is MarketDataType.BOOKBID
    assert MarketDataType.from_spelling("market_if_touched") is MarketDataType.ORDMIT
    assert MarketDataType.from_spelling("Counter") is None
    assert MarketDataType.from_spelling("102") is None
    assert MarketDataType.QUOTRAD.description.startswith("Tradeable")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MarketDataType } = require('yggdryl')

    const codes = Object.values(MarketDataType)
    assert.equal(codes.length, 118)
    assert.deepEqual(codes, [...codes].sort((a, b) => a - b))
    assert.equal(MarketDataType.UNKN, 0)
    assert.equal(MarketDataType.BOOKBID, 400)
    assert.equal(MarketDataType.BOOKOTHER, 499)
    assert.equal(MarketDataType.MDROTHER, 899)
    assert.ok(Object.isFrozen(MarketDataType))
    ```

## FIX

A member stands for at most one FIX value: `from_fix(tag, wire)` reads it, `fix_code()` answers it back, and `UNKN` and the eight catch-alls answer none. A wire value no member names reads as the catch-all of the set its field draws from - `ORDOTHER` for an `OrdType(40)` - and a field that types nothing reads as none. `fix_tags(kind)` names the fields that type an element of one [`MarketDataKind`](marketdatakind.md), its own first: an order's `OrdType(40)`; a quote's `QuoteType(537)`, then `OrdType(40)`; an execution's or a trade's `TrdType(828)`, then `OrdType(40)`; every other element's `MDEntryType(269)`, `TrdType(828)`, `QuoteType(537)`, `OrdType(40)`. `MARKETDATATYPE_FIX_TAGS` is the eight, `[40, 537, 828, 269, 856, 303, 530, 263]`.

A message type may name its own fields before its kind's: `fix_tags_of(msgtype, kind)` answers the message type's rule where `MARKETDATATYPE_MSGTYPE_RULES` states one, else `fix_tags(kind)`.

| Message type | Typed by, first stated first |
| --- | --- |
| `AE` trade capture report, `AR` its acknowledgement | `TradeReportType(856)`, `TrdType(828)`, `OrdType(40)` |
| `R` quote request | `QuoteRequestType(303)`, `QuoteType(537)`, `OrdType(40)` |
| `AG` quote request reject | `QuoteRequestType(303)`, `QuoteType(537)` |
| `q` order mass cancel request, `r` its report | `MassCancelRequestType(530)` |
| `V` market data request | `SubscriptionRequestType(263)` |

A [FIX message](../../fix/message.md) states its type as it is parsed, reading the first of its `fix_tags_of(msgtype, kind)` it states through its registry's [`marketdatatype_of`](../../fix/registry.md#a-field-maps-its-values-onto-a-market-data-type): a field's own `FIX:marketdatatype` mapping first, then `from_fix`. The registry's intrinsic `marketdatatypecodeset` renders the members beside `msgcatcodeset` and `statecodeset`.

=== "Rust"

    ```rust
    use yggdryl::{MarketDataKind, MarketDataType, MARKETDATATYPE_FIX_TAGS};

    assert_eq!(MarketDataType::from_fix(40, "2"), Some(MarketDataType::OrdLimit));
    assert_eq!(MarketDataType::from_fix(537, "1"), Some(MarketDataType::QuoTradeable));
    assert_eq!(MarketDataType::from_fix(269, "0"), Some(MarketDataType::BookBid));
    assert_eq!(MarketDataType::OrdLimit.fix_code(), Some((40, "2")));
    // A value the set does not name reads as its catch-all, which stands for none.
    assert_eq!(MarketDataType::from_fix(828, "999"), Some(MarketDataType::TrdOther));
    assert_eq!(MarketDataType::TrdOther.fix_code(), None);
    // A field that types nothing reads as none.
    assert_eq!(MarketDataType::from_fix(54, "1"), None);

    assert_eq!(MarketDataType::fix_tags(MarketDataKind::Order), [40]);
    assert_eq!(MarketDataType::fix_tags(MarketDataKind::Quotation), [537, 40]);
    assert_eq!(MarketDataType::fix_tags(MarketDataKind::Book), [269, 828, 537, 40]);
    assert_eq!(MARKETDATATYPE_FIX_TAGS, [40, 537, 828, 269, 856, 303, 530, 263]);

    // A message type's own rule first: a trade capture report by what the report is.
    assert_eq!(MarketDataType::fix_tags_of("AE", MarketDataKind::Trade), [856, 828, 40]);
    assert_eq!(MarketDataType::fix_tags_of("D", MarketDataKind::Order), [40]);
    assert_eq!(MarketDataType::from_fix(856, "2"), Some(MarketDataType::TrptAccept));
    assert_eq!(MarketDataType::from_fix(530, "7"), Some(MarketDataType::McxAll));
    assert_eq!(MarketDataType::from_fix(263, "9"), Some(MarketDataType::MdrOther));
    ```

=== "Python"

    ```python
    from yggdryl import MarketDataKind, MarketDataType

    assert MarketDataType.from_fix(40, "2") is MarketDataType.ORDLIMIT
    assert MarketDataType.ORDLIMIT.fix_code == (40, "2")
    assert MarketDataType.from_fix(828, "999") is MarketDataType.TRDOTHER
    assert MarketDataType.TRDOTHER.fix_code is None
    assert MarketDataType.from_fix(54, "1") is None
    assert MarketDataType.fix_tags(MarketDataKind.QUOT) == (537, 40)
    assert MarketDataType.fix_tags_of("AE", MarketDataKind.TRAD) == (856, 828, 40)
    assert MarketDataType.from_fix(856, "2") is MarketDataType.TRPTACCEPT
    assert MarketDataType.from_fix(263, "9") is MarketDataType.MDROTHER
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { marketDataTypeFixCode, marketDataTypeFromFix } = require('yggdryl')

    assert.equal(marketDataTypeFromFix(40, '2'), 'ORDLIMIT')
    assert.deepEqual(marketDataTypeFixCode('ORDLIMIT'), { tag: 40, wire: '2' })
    assert.equal(marketDataTypeFromFix(828, '999'), 'TRDOTHER')
    assert.equal(marketDataTypeFixCode('TRDOTHER'), null)
    assert.equal(marketDataTypeFromFix(54, '1'), null)
    assert.equal(marketDataTypeFromFix(856, '2'), 'TRPTACCEPT')
    assert.equal(marketDataTypeFromFix(263, '9'), 'MDROTHER')
    ```

`fix_tags` and `fix_tags_of` are Rust and Python; JavaScript exposes `marketDataTypeFromFix` and `marketDataTypeFixCode`.

## Edges

- A spelling that names no type, or an integer that is the code of none -> refused naming `marketdatatype`, never stored; a value that names none leaves a nullable column null under `safe`.
- A stored code is an integer, never text: `"102"` is no spelling, `102` is `ORDLIMIT`. A wire value is no spelling either: `2` is a limit order only under `OrdType(40)`, which `from_fix` reads.
- The stored name folds case only; the FIX name folds the way every name in this crate folds, ASCII case insensitive with `_`, `-` and spaces ignored. A FIX name two sets share - `Counter` - reaches neither.
- The default value is `UNKN`, code `0`: a stated value, not an absence - what an element no typing field names is. An empty text cell entering the column is null ([Cast](../cast.md#empty-text)), and a required column refuses it.
- The codes leave gaps where FIX's own values do - `TrdType(828)` `13` has no member and reads as `TRDOTHER` - because a code is the set's value plus its hundreds, never renumbered.
- JSON, TOML, YAML and XML write a type as its stored name, and the [value stream](../value-stream.md) and a digest feed its four-byte little-endian code under the type's own identifier, so a type, a [kind](marketdatakind.md) and an integer of one code are three values.
- The type is not the kind: `ORDLIMIT` may be stated on an execution that reports a limit order's fill, and `fix_tags_of` is what says which field a message type or its kind reads first.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- marketdatatype::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_marketdatatype.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/marketdatatype.test.js
    ```

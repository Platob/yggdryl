"""The shared log corpus specification, stated once for both Python targets.

Anonymized production-shaped OMS log lines::

    2026-08-14 00:05:01.167_250 [250-e7256676:9ffe:72503] [OrderFlow_Enrichment] (DEBUG) ...

The Rust ``benchmarks/text/line.rs`` and Node ``benchmarks/lines.js`` targets
re-implement exactly this arithmetic, so the three languages' corpora stay
byte-for-byte identical and their numbers describe the same work. Do not
"improve" one implementation without the other two.
"""

from __future__ import annotations

# The shared corpus pattern, byte-identical in Rust, JavaScript, and here.
# `(?P<name>...)` groups are the spelling CPython's `re` and the binding's
# engine both read, so a baseline compiles exactly this string. `port` is the
# one capture whose whole body is `\d+`, which the closed inference table
# types `int64`.
PATTERN = (
    r"^(?P<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}_\d{3})"
    r" \[(?P<thread>\d+-[^\]]*:(?P<port>\d+))\]"
    r" \[(?P<logger>[^\]]+)\]"
    r" \((?P<level>[A-Z]+)\)"
)

LOGGERS = (
    "OrderFlow_Enrichment",
    "Regulatory_Timestamps",
    "GatewayBridge",
    "OrderFlow",
    "RiskManager",
    "MarketDataManager",
    "TagWrapper",
    "RouteCheck",
)
LEVELS = ("DEBUG", "INFO", "WARNING")


def message(index: int) -> str:
    """One of the eight anonymized message shapes, chosen by ``index``."""
    shape = index % 8
    if shape == 0:
        return (
            "-> [S] (trade || cancel || tradecancel || replace || new)"
            f" - ExecType=required, cumQty={index % 100}, CompositeID=null"
        )
    if shape == 1:
        return f"CLIENTID set to ROUTE{index % 50:02}"
    if shape == 2:
        return (
            "After Enrichment -> #ROUTINGINDICATOR=yes #CFICODE=ESXXXX"
            f" #GROUP=GRP{index % 9} #ISINCODE=XX{index % 10_000:010}"
        )
    if shape == 3:
        return (
            "Message received: Message type [executionreport] from"
            f" [gateway as FU{index % 1_000_000:06}] forwarded to"
            " [(null) as (null)] [Direct reject]"
        )
    if shape == 4:
        return "Message rejected because : Ignoring expiry message from fully filled orders"
    if shape == 5:
        return f"Setting last event id for order , 1 to 20260814-2206{index % 100:02}-906-02-1"
    if shape == 6:
        return (
            'Expression from TCRPRICE=xpath("/event/action/trade/capturereport/@price")'
            " gives no result, no mapping is done"
        )
    return (
        f"Found code(db: XX{index % 10_000:010}_XNAS_USD)"
        f" from instrument(db: XX{index % 10_000:010} XNAS USD)"
    )


def line(index: int, *, continuations: bool = False) -> str:
    """Render record ``index`` of the shared corpus specification.

    With ``continuations``, every fiftieth record carries two continuation
    lines that belong to the same row: the multi-line shape a naive
    ``splitlines`` loop miscounts and the projection folds.
    """
    minute, second = index // 3_600 % 60, index // 60 % 60
    micro = index % 1_000_000
    ms, us = micro // 1_000, micro % 1_000
    pool = 250 + index % 4
    hex_a = index * 2654435761 % 4294967296
    hex_b = index * 40503 % 65536
    port = 72_500 + index % 8
    rendered = (
        f"2026-08-14 00:{minute:02}:{second:02}.{ms:03}_{us:03}"
        f" [{pool}-{hex_a:08x}:{hex_b:04x}:{port}]"
        f" [{LOGGERS[index % 8]}] ({LEVELS[index % 3]}) {message(index)}\n"
    )
    if continuations and index % 50 == 49:
        rendered += "    at core::enrich(order.rs:118)\n"
        rendered += "    at core::route(order.rs:64)\n"
    return rendered

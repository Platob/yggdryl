"""Expressions: terms, clauses, plans, and the pushdown they drive.

:class:`Term` is the tree a predicate or a projection is built from, from
text or composed method by method; :class:`Bound` is that tree compiled
against one struct root, which is where the vectorized Arrow answers and the
statistics pushdown live. :class:`Filter` is a ``where`` clause and
:class:`Selector` a ``select`` clause; :class:`Plan` is the sections of one
read or write - ``create``, a write verb, ``select``, ``from``, ``where``,
``order by``, ``limit``, ``offset`` - and :class:`Expression` is whichever of
those one piece of text turns out to be, a ``;``-separated sequence included.
:class:`Records` streams native rows through any of them, and :class:`Bounds`
carries one container's per-column statistics, so a caller can skip a file
without opening it.

The vocabularies the grammar closes over cross as their canonical spellings:
:data:`COMPARISONS`, :data:`FUNCTIONS`, :data:`HOLDER_ATTRIBUTES`, and
:data:`VERBS`.
"""

from __future__ import annotations

from ._native import (
    Bound,
    Bounds,
    BoundSelector,
    Expression,
    Filter,
    Plan,
    Records,
    Selector,
    Term,
    expression_needs_quoting,
    expression_vocabularies,
)

_VOCABULARIES = expression_vocabularies()

#: Every comparison the grammar knows, e.g. ``"="``, ``"is distinct from"``.
COMPARISONS: tuple[str, ...] = tuple(_VOCABULARIES["comparisons"])

#: Every function the closed scalar set knows, e.g. ``"year"``, ``"truncate"``.
FUNCTIONS: tuple[str, ...] = tuple(_VOCABULARIES["functions"])

#: Every holder attribute ``&holder.<name>`` can name, e.g. ``"size"``.
HOLDER_ATTRIBUTES: tuple[str, ...] = tuple(_VOCABULARIES["holder_attributes"])

#: Every write verb a plan spells canonically, e.g. ``"upsert into"``.
VERBS: tuple[str, ...] = tuple(_VOCABULARIES["verbs"])

#: Whether an identifier has to be quoted to survive the grammar's round trip.
needs_quoting = expression_needs_quoting

__all__ = [
    "COMPARISONS",
    "FUNCTIONS",
    "HOLDER_ATTRIBUTES",
    "VERBS",
    "Bound",
    "Bounds",
    "BoundSelector",
    "Expression",
    "Filter",
    "Plan",
    "Records",
    "Selector",
    "Term",
    "needs_quoting",
]

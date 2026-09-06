"""Expressions, bound expressions, statements, and the pushdown they drive.

:class:`Expression` is the tree, built from text or composed method by method;
:class:`Bound` is that tree compiled against one struct root, which is where
the vectorized Arrow answers and the statistics pushdown live.
:class:`Bounds` carries one container's per-column statistics, so a caller can
skip a file without opening it.

The three vocabularies the grammar closes over cross as their canonical
spellings: :data:`COMPARISONS`, :data:`FUNCTIONS`, and
:data:`HOLDER_ATTRIBUTES`.
"""

from __future__ import annotations

from ._native import (
    Bound,
    Bounds,
    BoundStatement,
    Expression,
    Statement,
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

#: Whether an identifier has to be quoted to survive the grammar's round trip.
needs_quoting = expression_needs_quoting

__all__ = [
    "COMPARISONS",
    "FUNCTIONS",
    "HOLDER_ATTRIBUTES",
    "Bound",
    "Bounds",
    "BoundStatement",
    "Expression",
    "Statement",
    "needs_quoting",
]

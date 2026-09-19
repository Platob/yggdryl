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

import functools
import inspect
import typing
from collections.abc import Callable, Mapping
from typing import Any

from ._native import (
    Bound,
    Bounds,
    BoundSelector,
    DataType,
    Expression,
    Field,
    Filter,
    Plan,
    Records,
    Selector,
    Term,
    expression_needs_quoting,
    expression_vocabularies,
    register_user_function,
    unregister_user_function,
    user_function_signature,
    user_functions,
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


class UserFunction:
    """A Python callable registered as the expression function ``namespace.name``.

    The wrapper stays the callable it decorates - calling it runs the Python
    function directly - and adds the expression side: :meth:`term` spells a
    call over terms or values, :meth:`where` the same call as a ``where``
    clause, and :attr:`signature` is the struct :class:`Field` the function
    is typed by, one child per parameter and the return as its
    ``FUNCTION:returns`` property. A stored column declaring
    ``TRANSFORM:function = "namespace.name"`` derives through the same
    registration.
    """

    def __init__(self, function: Callable[..., Any], namespace: str, name: str, signature: Field) -> None:
        functools.update_wrapper(self, function)
        self.function = function
        self.namespace = namespace
        self.name = name
        self.qualified = f"{namespace}.{name}"
        self.signature = signature

    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        return self.function(*args, **kwargs)

    def __repr__(self) -> str:
        return f"UserFunction({self.qualified!r})"

    def term(self, *arguments: Any) -> Term:
        """The call ``namespace.name(arguments...)`` as a :class:`Term`.

        Each argument is a :class:`Term`, the text of one, or a value read as
        the literal it is.
        """
        return Term.call(self.qualified, list(arguments))

    def where(self, *arguments: Any) -> Filter:
        """The call as a ``where`` clause: the rows it answers true for."""
        return Filter(self.term(*arguments))

    def unregister(self) -> bool:
        """Remove the registration; the Python function stays callable."""
        return unregister_user_function(self.qualified)


def _parameter_field(name: str, hint: object) -> Field:
    if isinstance(hint, Field):
        return hint if hint.name == name else Field(name, hint.dtype, hint.nullable)
    if isinstance(hint, (DataType, str)):
        return Field(name, hint, True)
    return Field.from_pyhint(name, hint)


def _returns_field(hint: object) -> Field:
    if isinstance(hint, Field):
        return hint
    if isinstance(hint, (DataType, str)):
        return Field("returns", hint, True)
    return Field.from_pyhint("returns", hint)


def user_defined_function(
    function: Callable[..., Any] | None = None,
    /,
    *,
    namespace: str = "py",
    name: str | None = None,
    parameters: Mapping[str, object] | None = None,
    returns: object = None,
    vectorized: bool = False,
) -> Any:
    """Register a Python function as the expression function ``namespace.name``.

    The signature is read off the function: each parameter is typed by its
    annotation - or by the ``parameters`` entry that overrides it, a
    :class:`Field`, a :class:`DataType` or the text of one - and a parameter
    with a Python default is optional, its default cast to the parameter's
    datatype. The return is ``returns`` or the return annotation. A
    ``vectorized`` function takes and answers ``pyarrow`` arrays, one call
    per batch; every other function is called once per row, its arguments
    cast to the declared datatypes and a ``None`` meeting a parameter that
    is not ``Optional`` answering ``None`` without a call.

    Registration is process-wide and the latest one wins, so a decorator run
    twice keeps the second definition. The function is refused by name, not
    silently null, wherever it is typed or bound before it is registered.
    """

    def decorate(function: Callable[..., Any]) -> UserFunction:
        signature = inspect.signature(function)
        hints = typing.get_type_hints(function)
        fields: list[Field] = []
        defaults: dict[str, object] = {}
        for parameter in signature.parameters.values():
            if parameter.kind in (parameter.VAR_POSITIONAL, parameter.VAR_KEYWORD):
                raise TypeError(
                    f"expected positional parameters for {function.__name__}, got *{parameter.name}"
                )
            hint = (parameters or {}).get(parameter.name, hints.get(parameter.name))
            if hint is None:
                raise TypeError(
                    f"expected a type hint or a parameters entry for {parameter.name!r} of "
                    f"{function.__name__}"
                )
            fields.append(_parameter_field(parameter.name, hint))
            if parameter.default is not parameter.empty:
                defaults[parameter.name] = parameter.default
        answer = returns if returns is not None else hints.get("return")
        if answer is None:
            raise TypeError(
                f"expected a return annotation or `returns` for {function.__name__}"
            )
        qualified_name = name or function.__name__
        stored = register_user_function(
            namespace,
            qualified_name,
            fields,
            _returns_field(answer),
            function,
            defaults=defaults,
            vectorized=vectorized,
        )
        return UserFunction(function, namespace, qualified_name, stored)

    return decorate if function is None else decorate(function)


def user_defined_filter(
    function: Callable[..., Any] | None = None,
    /,
    *,
    namespace: str = "py",
    name: str | None = None,
    parameters: Mapping[str, object] | None = None,
    vectorized: bool = False,
) -> Any:
    """Register a Python predicate as an expression function answering a boolean.

    :func:`user_defined_function` with the return fixed to ``bool``: the
    decorated function is usable as a ``where`` clause through
    :meth:`UserFunction.where`, and a ``None`` answer keeps no row.
    """
    return user_defined_function(
        function,
        namespace=namespace,
        name=name,
        parameters=parameters,
        returns=Field("returns", "bool", True),
        vectorized=vectorized,
    )

__all__ = [
    "UserFunction",
    "user_defined_filter",
    "user_defined_function",
    "user_function_signature",
    "user_functions",
    "unregister_user_function",
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

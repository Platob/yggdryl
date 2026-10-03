"""Catalogs, namespaces and tables: one abstraction for every place that
answers "which tables are there, and how do I read one".

Every object is an ``IOBase`` handle - ``Catalog``, ``Namespace`` and
``Table`` are the kinds, and ``type(object)`` is the implementation doing the
work: ``MemoryCatalog`` and ``MemoryNamespace`` keep registered objects in
order, ``FolderCatalog`` and ``FolderNamespace`` read a container as
namespaces and tables, ``MediaTable`` is a table over any location a record
medium reads. ``Namespaces`` and ``Tables`` are the lazy mapping views one
level of the hierarchy answers with. A ``Warehouse`` is the registry a dotted
path - ``lake.eu.trades`` - resolves against, and ``SystemWarehouse`` is the
process's one, which a plan's ``from catalog.namespace.table`` reads.
"""

from .._native import (
    Catalog,
    FolderCatalog,
    FolderNamespace,
    MediaTable,
    MemoryCatalog,
    MemoryNamespace,
    Namespace,
    Namespaces,
    SystemWarehouse,
    Table,
    Tables,
    Warehouse,
)

__all__ = [
    "Catalog",
    "FolderCatalog",
    "FolderNamespace",
    "MediaTable",
    "MemoryCatalog",
    "MemoryNamespace",
    "Namespace",
    "Namespaces",
    "SystemWarehouse",
    "Table",
    "Tables",
    "Warehouse",
]

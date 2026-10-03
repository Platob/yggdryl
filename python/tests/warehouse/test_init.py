"""The ``yggdryl.warehouse`` package: one name per native class, each a root
export too, and the class hierarchy a described object lands in."""

from __future__ import annotations

import yggdryl
from yggdryl import IOBase, warehouse
from yggdryl.warehouse import (
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


def test_every_name_is_listed_once_and_re_exported_at_the_root() -> None:
    listed = set(warehouse.__all__)
    assert len(warehouse.__all__) == len(listed)
    assert listed == {
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
    }
    for name in listed:
        assert getattr(yggdryl, name) is getattr(warehouse, name), name
        assert name in yggdryl.__all__, name
    assert "warehouse" in yggdryl.__all__
    assert yggdryl.warehouse is warehouse


def test_the_kinds_are_handles_and_the_implementations_their_subclasses() -> None:
    # An object is a handle, so every byte and record verb of `IOBase` is on
    # it; the kind is the class a caller tests for, the implementation the
    # class a described handle is.
    for kind in (Catalog, Namespace, Table):
        assert issubclass(kind, IOBase)
    assert issubclass(MemoryCatalog, Catalog)
    assert issubclass(FolderCatalog, Catalog)
    assert issubclass(MemoryNamespace, Namespace)
    assert issubclass(FolderNamespace, Namespace)
    assert issubclass(MediaTable, Table)
    # The views and the registries stand beside the handles, not under them.
    for other in (Namespaces, Tables, Warehouse, SystemWarehouse):
        assert not issubclass(other, IOBase)


def test_the_iceberg_names_keep_their_public_spelling() -> None:
    # The native classes moved to `Iceberg*` so the generic kinds could take
    # the plain names; `yggdryl.iceberg` still answers the names it always did,
    # and they are not the warehouse's.
    from yggdryl import iceberg

    assert iceberg.Catalog.__name__ == "IcebergCatalog"
    assert iceberg.Namespace.__name__ == "IcebergNamespace"
    assert iceberg.Namespaces.__name__ == "IcebergNamespaces"
    assert iceberg.Table.__name__ == "IcebergTable"
    assert iceberg.Tables.__name__ == "IcebergTables"
    assert iceberg.Catalog is not Catalog
    assert iceberg.Table is not Table

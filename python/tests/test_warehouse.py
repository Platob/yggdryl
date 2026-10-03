"""The warehouse through Python: objects as handles, the views over them, and
the registries a dotted path resolves against.

Every object is the handle the core holds, so `type(object)` is the
implementation, the record verbs are `IOBase`'s, and what a view answers is
what the store holds at the moment it is asked.
"""

from __future__ import annotations

import pathlib
import uuid

import pyarrow as pa
import pytest

from yggdryl import IOBase, Url
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

TRADES = pa.table({"symbol": ["AAPL", "MSFT", "GOOG"], "price": [187.5, 410.25, 141.0]})


@pytest.fixture
def root(tmp_path: pathlib.Path) -> pathlib.Path:
    """A folder laid out as a catalog: a root table, a schema of two tables,
    an empty schema, and leaves no record medium reads."""
    (tmp_path / "eu").mkdir()
    (tmp_path / "empty").mkdir()
    IOBase(tmp_path / "trades.csv").overwrite_arrow_table(TRADES)
    IOBase(tmp_path / "eu" / "fills.arrows").overwrite_arrow_table(TRADES)
    IOBase(tmp_path / "eu" / "quotes.csv").overwrite_arrow_table(TRADES.slice(0, 1))
    (tmp_path / "README.md").write_text("# not a table\n", encoding="utf-8")
    (tmp_path / ".hidden.csv").write_text("a\n1\n", encoding="utf-8")
    return tmp_path


@pytest.fixture
def market(root: pathlib.Path) -> FolderCatalog:
    return FolderCatalog("market", root, description="the market")


def _unique(prefix: str) -> str:
    """A catalog name no other test registered on the process's warehouse."""
    return f"{prefix}_{uuid.uuid4().hex[:8]}"


class TestDescribeAnswersTheImplementation:
    """A described handle is the kind, then the implementation below it."""

    def test_a_folder_catalog_and_what_it_lists(self, market: FolderCatalog, root: pathlib.Path) -> None:
        assert type(market) is FolderCatalog
        assert isinstance(market, Catalog)
        assert isinstance(market, IOBase)
        assert type(market.namespaces["eu"]) is FolderNamespace
        assert type(market.tables["trades"]) is MediaTable
        assert isinstance(market.tables["trades"], Table)
        assert {type(child).__name__ for child in market.children()} == {
            "FolderNamespace",
            "MediaTable",
        }
        # The handle vocabulary answers as the core does.
        assert market.kind() == "catalog"
        assert market.namespaces["eu"].kind() == "namespace"
        assert market.url == Url.from_path(root)
        assert repr(market) == "FolderCatalog('market')"
        assert str(market) == "market"

    def test_a_memory_catalog_and_its_registered_objects(self, root: pathlib.Path) -> None:
        table = MediaTable("lake.raw.trades", root / "trades.csv")
        namespace = MemoryNamespace("lake.raw", objects=[table])
        lake = MemoryCatalog("lake", description="the lake", objects=[namespace])

        assert type(lake) is MemoryCatalog
        assert type(lake.namespaces["raw"]) is MemoryNamespace
        assert type(lake.tables["raw.trades"]) is MediaTable
        assert lake.description == "the lake"
        assert lake.namespace_levels is None
        assert repr(lake) == "MemoryCatalog('lake')"
        assert repr(lake.get("raw")) == "MemoryNamespace('lake.raw')"
        assert repr(lake.resolve(["raw", "trades"])) == "MediaTable('lake.raw.trades')"

    def test_from_url_picks_the_implementation_the_properties_name(self, root: pathlib.Path) -> None:
        folder = Catalog.from_url(root)
        assert type(folder) is FolderCatalog
        assert folder.name == root.name
        memory = Catalog.from_url(root, type="memory", name="m")
        assert type(memory) is MemoryCatalog
        assert memory.name == "m"
        with pytest.raises(ValueError, match="expected `memory` or `folder`"):
            Catalog.from_url(root, {"type": "rest"})

    def test_the_kinds_are_never_built_directly(self) -> None:
        with pytest.raises(TypeError):
            Catalog("x")  # type: ignore[call-arg]
        with pytest.raises(TypeError):
            Table("x")  # type: ignore[call-arg]


class TestConstructors:
    """Every fact an object states is stated at construction."""

    def test_a_folder_catalog_reads_its_levels_and_properties(self, root: pathlib.Path) -> None:
        catalog = FolderCatalog("market", root, levels=0, region="eu-west-1")
        assert catalog.namespace_levels == 0
        assert catalog.properties == {"region": "eu-west-1"}
        # With no level under it a folder is a table read as the rows beneath it.
        assert type(catalog.tables["eu"]) is MediaTable
        assert catalog.tables["eu"].storage == "directory"

    def test_a_location_is_a_handle_a_url_a_string_or_a_path(self, root: pathlib.Path) -> None:
        by_handle = FolderCatalog("market", IOBase(root))
        by_url = FolderCatalog("market", Url.from_path(root))
        by_text = FolderCatalog("market", str(root))
        by_path = FolderCatalog("market", root)
        for catalog in (by_handle, by_url, by_text, by_path):
            assert list(catalog.namespaces) == ["empty", "eu"]
        # A handle binds, a location names: the description is the same.
        assert by_url == by_path == by_text
        assert by_handle.url == by_path.url

    def test_a_media_table_declares_its_field_and_layout(self, root: pathlib.Path) -> None:
        table = MediaTable(
            "lake.trades",
            root / "trades.csv",
            dtype="struct<symbol: utf8, price: float64>",
            description="prices",
            tier="hot",
        )
        assert type(table) is MediaTable
        assert table.path == ("lake", "trades")
        assert table.name == "trades"
        assert table.description == "prices"
        assert table.storage == "text/csv"
        assert table.properties == {"tier": "hot"}
        # The declared field is renamed after the table and read before any row.
        assert table.field().name == "trades"
        assert [child.name for child in table.field().dtype] == ["symbol", "price"]
        assert table.read_arrow_reader().read_all().num_rows == 3

        folder = MediaTable("lake.eu", root / "eu", layout="folder")
        assert folder.storage == "directory"
        with pytest.raises(ValueError, match="leaf, folder or format"):
            MediaTable("lake.eu", root / "eu", layout="heap")

    def test_a_namespace_path_names_its_catalog_first(self, root: pathlib.Path) -> None:
        namespace = MemoryNamespace("lake.eu", description="europe")
        assert namespace.path == ("lake", "eu")
        assert namespace.description == "europe"
        assert FolderNamespace(["lake", "eu"], root / "eu").path == ("lake", "eu")
        with pytest.raises(ValueError, match="at least two parts"):
            MemoryNamespace("lake")
        with pytest.raises(ValueError, match="at least two parts"):
            FolderNamespace("lake", root / "eu")

    def test_a_path_is_dotted_text_or_parts(self, root: pathlib.Path) -> None:
        quoted = MediaTable('lake."eu west".trades', root / "trades.csv")
        assert quoted.path == ("lake", "eu west", "trades")
        assert str(quoted) == 'lake."eu west".trades'
        assert MediaTable(["lake", "eu west", "trades"], root / "trades.csv") == quoted
        with pytest.raises(ValueError, match="expected a path"):
            MediaTable("", root / "trades.csv")


class TestProperties:
    """Stated properties propagate to every child, and are read as a dict."""

    def test_as_a_mapping_and_as_keywords(self, root: pathlib.Path) -> None:
        lake = MemoryCatalog(
            "lake",
            properties={"owner": "ops", "region": "us-east-1"},
            region="eu-west-1",
            archived=False,
            retries=3,
            objects=[MediaTable("lake.trades", root / "trades.csv", tier="hot")],
        )
        # The mapping first, the keywords after it replacing by name, in the
        # order of first writing; every value its text.
        assert lake.properties == {
            "owner": "ops",
            "region": "eu-west-1",
            "archived": "false",
            "retries": "3",
        }
        assert list(lake.properties) == ["owner", "region", "archived", "retries"]
        # A child carries its parent's, then its own.
        assert lake.tables["trades"].properties == {
            "owner": "ops",
            "region": "eu-west-1",
            "archived": "false",
            "retries": "3",
            "tier": "hot",
        }
        assert lake.get("trades").properties["tier"] == "hot"
        assert next(iter(lake.children())).properties["owner"] == "ops"

    def test_none_clears_and_ellipsis_is_not_given(self) -> None:
        catalog = MemoryCatalog(
            "lake", properties={"owner": "ops", "tier": "hot"}, tier=None, region=...
        )
        assert catalog.properties == {"owner": "ops"}

    def test_folder_properties_reach_the_tables_listed(self, root: pathlib.Path) -> None:
        catalog = FolderCatalog("market", root, region="eu-west-1")
        assert catalog.namespaces["eu"].properties == {"region": "eu-west-1"}
        assert catalog.tables["eu.fills"].properties == {"region": "eu-west-1"}

    def test_a_memory_object_keeps_nothing_to_update(self) -> None:
        catalog = MemoryCatalog("lake")
        catalog.update_properties()
        with pytest.raises(ValueError, match="does not support updating the properties"):
            catalog.update_properties({"owner": "ops"})
        with pytest.raises(ValueError, match="does not support updating the properties"):
            catalog.update_properties(removes=["owner"])


class TestTheViews:
    """`Namespaces` and `Tables` are lazy mappings over one level."""

    def test_the_mapping_protocol(self, market: FolderCatalog) -> None:
        namespaces = market.namespaces
        tables = market.tables
        assert isinstance(namespaces, Namespaces)
        assert isinstance(tables, Tables)
        assert repr(namespaces) == "Namespaces('market')"
        assert repr(market.namespaces["eu"].tables) == "Tables('market.eu')"

        assert list(namespaces) == ["empty", "eu"]
        assert list(namespaces.keys()) == ["empty", "eu"]
        assert len(namespaces) == 2
        assert "eu" in namespaces
        assert "asia" not in namespaces
        assert [type(value).__name__ for value in namespaces.values()] == [
            "FolderNamespace",
            "FolderNamespace",
        ]
        assert [(name, str(value)) for name, value in namespaces.items()] == [
            ("empty", "market.empty"),
            ("eu", "market.eu"),
        ]

        # The root's tables are the tabular leaves alone: the markdown and the
        # hidden leaf are not listed.
        assert list(tables) == ["trades"]
        assert len(tables) == 1
        assert "trades" in tables
        assert "README" not in tables
        assert [str(table) for table in tables.values()] == ["market.trades"]
        # A dotted name descends.
        assert str(tables["eu.fills"]) == "market.eu.fills"
        assert sorted(market.namespaces["eu"].tables) == ["fills", "quotes"]

    def test_a_missing_name_is_a_key_error_carrying_the_core_message(
        self, market: FolderCatalog
    ) -> None:
        with pytest.raises(KeyError, match='expected a table at "market.nowhere", got nothing'):
            market.tables["nowhere"]
        # A folder lists one level and names what it looked for as a table,
        # whichever view asked; the memory levels name an object.
        with pytest.raises(KeyError, match='expected a table at "market.asia", got nothing'):
            market.namespaces["asia"]
        with pytest.raises(KeyError, match='expected a child at "lake.asia", got nothing'):
            MemoryCatalog("lake").namespaces["asia"]
        # A namespace where a table is asked for is the absence of a table.
        with pytest.raises(KeyError, match='expected a table at "market.eu", got nothing'):
            market.tables["eu"]
        assert market.tables.get("nowhere") is None
        assert market.tables.get("nowhere", 0) == 0
        assert str(market.tables.get("trades")) == "market.trades"
        assert market.namespaces.get("asia", "none") == "none"

    def test_a_folder_catalog_creates_nothing(self, market: FolderCatalog) -> None:
        with pytest.raises(ValueError, match='"FolderCatalog" does not support creating a namespace'):
            market.namespaces.create("asia")
        with pytest.raises(ValueError, match='"FolderCatalog" does not support creating a table'):
            market.tables.create("orders", "orders: struct<id: int64>")
        with pytest.raises(ValueError, match="does not support creating a table"):
            market.create_table("orders", "orders: struct<id: int64>", owner="ops")
        with pytest.raises(ValueError, match="does not support creating a namespace"):
            market.create_namespace("asia")
        # Open-or-create opens what is there and creates only what is not.
        assert str(market.namespaces.open_or_create("eu")) == "market.eu"
        assert str(market.tables.open_or_create("trades", "trades: struct<id: int64>")) == "market.trades"
        with pytest.raises(ValueError, match="does not support creating a table"):
            market.tables.open_or_create("orders", "orders: struct<id: int64>")

    def test_writes_through_the_view_reach_the_table(self, tmp_path: pathlib.Path) -> None:
        namespace = MemoryNamespace(
            "lake.out",
            objects=[
                MediaTable("lake.out.rows", tmp_path / "rows.arrows"),
                MediaTable("lake.out.narrow", tmp_path / "narrow.arrows"),
            ],
        )
        written = namespace.tables.append("rows", pa.table({"id": [1, 2]}))
        assert type(written) is MediaTable
        assert str(written) == "lake.out.rows"
        assert written.read_arrow_reader().read_all().to_pydict() == {"id": [1, 2]}
        # Any shape the record surface takes, under the table's own options.
        namespace.tables.append("rows", [{"id": 3}])
        assert namespace.tables["rows"].row_size() == 3
        replaced = namespace.tables.overwrite("rows", pa.table({"id": [7]}))
        assert replaced.read_arrow_reader().read_all().to_pydict() == {"id": [7]}
        # An overwrite replaces rows under the stored field, so a declared
        # field shapes a table on its first write: the properties beside
        # `options` are the record settings this write runs under.
        narrowed = namespace.tables.overwrite(
            "narrow", pa.table({"id": [8]}), field="narrow: struct<id: int32>"
        )
        assert narrowed.read_arrow_reader().read_all().schema.field("id").type == pa.int32()
        assert str(namespace.tables["narrow"].field().dtype[0].dtype) == "int32"
        # A memory namespace creates no table on first write.
        with pytest.raises(ValueError, match='"MemoryNamespace" does not support creating a table'):
            namespace.tables.append("absent", pa.table({"id": [1]}))


class TestTheHandle:
    """A catalog or a namespace is a container handle; a table is its rows."""

    def test_a_namespace_lists_children_and_refuses_bytes(self, market: FolderCatalog) -> None:
        eu = market.namespaces["eu"]
        assert isinstance(eu, IOBase)
        assert eu.is_dir()
        assert sorted(type(entry).__name__ for entry in eu.iterdir()) == ["MediaTable", "MediaTable"]
        assert sorted(entry.name for entry in eu) == ["fills", "quotes"]
        assert sorted(str(entry) for entry in market.ls(recursive=True)) == [
            "market.empty",
            "market.eu",
            "market.eu.fills",
            "market.eu.quotes",
            "market.trades",
        ]
        assert str(eu / "fills") == "market.eu.fills"
        assert str(market.joinpath("eu/quotes")) == "market.eu.quotes"
        with pytest.raises(ValueError, match="got a namespace"):
            eu.read_bytes()
        with pytest.raises(ValueError, match="got a namespace"):
            eu.write_bytes(b"x")
        with pytest.raises(ValueError, match="name a table under it"):
            eu.read_arrow_reader()
        with pytest.raises(ValueError, match="name a table under it"):
            eu.row_size()

    def test_a_table_reads_its_rows_through_the_record_verbs(self, market: FolderCatalog) -> None:
        fills = market.tables["eu.fills"]
        assert fills.storage == "application/vnd.apache.arrow.stream"
        assert fills.field().name == "fills"
        assert fills.row_size() == 3
        assert fills.read_arrow_reader().read_all() == TRADES
        assert [row["symbol"] for row in fills.read_records()] == ["AAPL", "MSFT", "GOOG"]
        assert fills.read_arrow_field().name == "fills"
        quotes = market.tables["eu.quotes"]
        assert quotes.storage == "text/csv"
        assert quotes.read_arrow_reader().read_all().num_rows == 1
        # A table appends through the same handle verb every leaf has.
        quotes.append_arrow_table(TRADES.slice(1, 2))
        assert market.tables["eu.quotes"].row_size() == 3

    def test_equality_and_hash_are_the_descriptions(self, market: FolderCatalog, root: pathlib.Path) -> None:
        twin = FolderCatalog("market", root, description="the market")
        assert market == twin
        assert hash(market) == hash(twin)
        assert market != FolderCatalog("market", root)
        assert market != FolderCatalog("other", root, description="the market")
        assert market != market.namespaces["eu"]
        assert market.tables["trades"] == market.tables["trades"]
        assert len({market, twin, market.tables["trades"]}) == 2
        assert MemoryCatalog("lake") == MemoryCatalog("lake")
        assert MemoryCatalog("lake") != MemoryCatalog("lake", owner="ops")
        assert market.modified is not None
        assert MemoryCatalog("lake").modified is None


class TestTheWarehouse:
    """A registry of catalogs, and the path a registration builds."""

    def test_registering_a_table_builds_the_levels_along_its_path(self, root: pathlib.Path) -> None:
        warehouse = Warehouse()
        assert warehouse.catalogs == []
        assert warehouse == Warehouse()
        table = MediaTable("lake.eu.trades", root / "trades.csv", tier="hot")
        warehouse.register(table)
        assert [str(catalog) for catalog in warehouse.catalogs] == ["lake"]
        assert type(warehouse.catalog("lake")) is MemoryCatalog
        assert type(warehouse.get("lake.eu")) is MemoryNamespace
        assert type(warehouse.namespace(["lake", "eu"])) is MemoryNamespace
        resolved = warehouse.table("lake.eu.trades")
        assert resolved == table
        assert resolved.properties == {"tier": "hot"}
        assert resolved.read_arrow_reader().read_all() == TRADES
        assert repr(warehouse) == 'Warehouse(["lake"])'
        assert warehouse != Warehouse()

        # A folder catalog registers by name and lists its own store.
        warehouse.register(FolderCatalog("market", root))
        assert str(warehouse.table("market.eu.fills")) == "market.eu.fills"
        with pytest.raises(ValueError, match="lists its own store"):
            warehouse.register(MediaTable("market.eu.orders", root / "trades.csv"))

    def test_absence_conflict_and_replacement(self, root: pathlib.Path) -> None:
        warehouse = Warehouse()
        warehouse.register(MediaTable("lake.eu.trades", root / "trades.csv"))
        # A memory level names an object where a folder names a table.
        with pytest.raises(ValueError, match='expected a child at "lake.eu.fills", got nothing'):
            warehouse.table("lake.eu.fills")
        with pytest.raises(ValueError, match='expected a catalog at "nowhere", got nothing'):
            warehouse.get("nowhere")
        with pytest.raises(ValueError, match='expected a table at "lake.eu", got nothing'):
            warehouse.table("lake.eu")
        with pytest.raises(
            ValueError, match='expected to create a table at "lake.eu.trades", got an existing table'
        ):
            warehouse.register(MediaTable("lake.eu.trades", root / "eu" / "quotes.csv"))
        replaced = warehouse.replace(MediaTable("lake.eu.trades", root / "eu" / "quotes.csv"))
        assert str(replaced) == "lake.eu.trades"
        assert replaced.url == Url.from_path(root / "trades.csv")
        assert warehouse.replace(MediaTable("lake.eu.fills", root / "trades.csv")) is None
        assert sorted(warehouse.namespace("lake.eu").tables) == ["fills", "trades"]
        unregistered = warehouse.unregister("lake.eu.fills")
        assert str(unregistered) == "lake.eu.fills"
        assert list(warehouse.namespace("lake.eu").tables) == ["trades"]
        with pytest.raises(TypeError, match="expected a Catalog, a Namespace or a Table"):
            warehouse.register("lake")  # type: ignore[arg-type]
        with pytest.raises(TypeError, match="expected a Catalog, a Namespace or a Table"):
            warehouse.register(IOBase(root))  # type: ignore[arg-type]

    def test_properties_for_a_location_are_the_deepest_holders(self, root: pathlib.Path) -> None:
        warehouse = Warehouse()
        warehouse.register(FolderCatalog("market", root, region="eu-west-1"))
        warehouse.register(MediaTable("lake.eu.trades", root / "eu" / "fills.arrows", tier="hot"))
        assert warehouse.properties_for(root / "eu" / "fills.arrows") == {"tier": "hot"}
        assert warehouse.properties_for(root / "eu" / "quotes.csv") == {"region": "eu-west-1"}
        assert warehouse.properties_for(Url.from_path(root)) == {"region": "eu-west-1"}
        assert warehouse.properties_for("file:///elsewhere/x.csv") == {}


class TestTheSystemWarehouse:
    """The process's one registry, reached through static methods."""

    def test_it_starts_with_the_local_catalog(self) -> None:
        local = SystemWarehouse.catalog("local")
        assert type(local) is MemoryCatalog
        assert "local" in [catalog.name for catalog in SystemWarehouse.catalogs()]
        temporary = SystemWarehouse.namespace("local.temporary")
        assert type(temporary) is FolderNamespace
        assert type(SystemWarehouse.get(["local", "temporary"])) is FolderNamespace
        assert str(temporary) == "local.temporary"

    def test_a_registered_table_resolves_and_unregisters(self, root: pathlib.Path) -> None:
        catalog = _unique("lake")
        path = f"{catalog}.eu.trades"
        SystemWarehouse.register(MediaTable(path, root / "trades.csv", tier="hot"))
        try:
            resolved = SystemWarehouse.table(path)
            assert type(resolved) is MediaTable
            assert str(resolved) == path
            assert resolved.properties == {"tier": "hot"}
            assert resolved.read_arrow_reader().read_all() == TRADES
            assert SystemWarehouse.properties_for(root / "trades.csv") == {"tier": "hot"}
            with pytest.raises(ValueError, match="got an existing table"):
                SystemWarehouse.register(MediaTable(path, root / "trades.csv"))
            assert str(SystemWarehouse.replace(MediaTable(path, root / "eu" / "quotes.csv"))) == path
            assert SystemWarehouse.table(path).row_size() == 1
        finally:
            assert str(SystemWarehouse.unregister(catalog)) == catalog
        with pytest.raises(ValueError, match="got nothing"):
            SystemWarehouse.get(catalog)

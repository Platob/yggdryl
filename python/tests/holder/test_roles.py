"""``type(handle)`` names the implementation doing the work.

A handle is described, not opened: the name says what the bytes are, and the
constructor answers with the composition that reads them - the record
implementation over the content coding over the storage role. What is asserted
here is that the class is that answer, that descending it reaches each layer,
and that a name declaring nothing still lands on the local role that resolves
itself.
"""

from __future__ import annotations

import gzip as stdlib_gzip
import pathlib
import re

import pyarrow.fs as pafs
import pytest

from yggdryl import IOBase
from yggdryl.coding import Coded, Gzip, Identity, Zlib, Zstd
from yggdryl.holder import (
    Buffer,
    Buffered,
    File,
    S3File,
    S3Folder,
    S3Path,
    Folder,
    FsFile,
    FsFolder,
    FsPath,
    Path,
)
from yggdryl.media import Avro, Ipc, Media, Parquet, Text

PLAIN = b"symbol,price\nAAPL,1\n"


@pytest.fixture
def log(tmp_path: pathlib.Path) -> pathlib.Path:
    """A real gzip-compressed text file on disk."""
    location = tmp_path / "data.txt.gz"
    location.write_bytes(stdlib_gzip.compress(PLAIN))
    return location


class TestTheNameComposesTheHandle:
    """The composition is the class, and it costs no read to arrive at."""

    def test_a_compressed_text_name_is_text_over_a_coding_over_a_location(
        self, log: pathlib.Path
    ) -> None:
        handle = IOBase(log)

        # Outermost is the record implementation, because the retained text
        # configuration describes the decoded rows; the coding underneath is
        # what decodes them, and the location is what holds the coded bytes.
        assert isinstance(handle, Text)
        assert repr(handle) == f'Text(Gzip(Path("{handle.url}")))'
        assert str(handle.media_type) == "text/plain"
        assert handle.codec == "gzip"
        assert handle.read_bytes() == PLAIN
        assert [row["body"] for row in handle.read_records()] == [
            b"symbol,price",
            b"AAPL,1",
        ]

    def test_a_file_url_composes_exactly_as_the_path_does(
        self, log: pathlib.Path
    ) -> None:
        assert repr(IOBase(log.as_uri())) == repr(IOBase(log))

    @pytest.mark.parametrize(
        ("name", "expected"),
        [
            ("trades.txt.gz", "Text(Gzip(Path))"),
            ("trades.txt.zz", "Text(Zlib(Path))"),
            ("trades.txt.zst", "Text(Zstd(Path))"),
            ("trades.log", "Text(Path)"),
            ("archive.bin.gz", "Gzip(Path)"),
            ("trades.parquet", "Parquet(Path)"),
            ("trades.arrows", "Ipc(Path)"),
            ("trades.avro", "Avro(Path)"),
            # Parquet compresses internally, so a coded name is left for the
            # writer to refuse rather than composed behind a decoded view.
            ("trades.parquet.gz", "Path"),
            # Nothing this build reads as rows, and no coding declared.
            ("trades.json", "Path"),
            ("trades", "Path"),
        ],
    )
    def test_each_name_composes_the_layers_it_declares(
        self, tmp_path: pathlib.Path, name: str, expected: str
    ) -> None:
        handle = IOBase(tmp_path / name)
        shape = re.sub(r'\("file:[^"]*"\)', "", repr(handle)).rstrip(")")
        assert shape == expected.rstrip(")")

    def test_composing_reads_nothing_and_needs_nothing_to_be_there(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "absent" / "trades.txt.gz")

        # Construction touches no store, so a location that does not exist yet
        # still arrives composed - and reads as empty, per the laziness
        # contract.
        assert isinstance(handle, Text)
        assert not handle.exists()
        assert handle.read_bytes() == b""


class TestDescendingTheComposition:
    """``into_handle`` walks down one layer, spending the handle it took."""

    def test_each_layer_answers_the_one_beneath_it(self, log: pathlib.Path) -> None:
        rows = IOBase(log)
        coding = rows.into_handle()
        assert isinstance(coding, Gzip)
        assert isinstance(coding, Coded)
        # The coding presents the decoded bytes; the location holds the coded
        # form, which is what the role underneath reads.
        assert coding.read_bytes() == PLAIN

        location = coding.into_handle()
        assert isinstance(location, Path)
        assert location.read_bytes()[:2] == b"\x1f\x8b"

        # A storage role stands on nothing, so it has no layer to descend to.
        assert not hasattr(location, "into_handle")

    def test_a_spent_handle_raises_rather_than_reading_nothing(
        self, log: pathlib.Path
    ) -> None:
        handle = IOBase(log)
        handle.into_handle()

        with pytest.raises(ValueError, match="consumed"):
            handle.read_bytes()
        with pytest.raises(ValueError, match="consumed"):
            handle.media_type


class TestTheExplicitRoles:
    """Naming a role is how a caller asks for the bytes rather than the value."""

    def test_a_foreign_filesystem_has_the_same_three_roles(
        self, log: pathlib.Path
    ) -> None:
        local = pafs.LocalFileSystem()
        stored = FsPath(local, str(log))

        # The role-only spelling on a bucket, for the same reason the local one
        # exists: the stored bytes, not the value they encode.
        assert isinstance(stored, FsPath)
        assert stored.read_bytes()[:2] == b"\x1f\x8b"
        assert isinstance(FsFile(local, str(log)), FsFile)
        assert isinstance(FsFolder(local, str(log.parent)), FsFolder)

    def test_the_stored_byte_role_skips_the_composition(
        self, log: pathlib.Path
    ) -> None:
        stored = Path(log)

        assert isinstance(stored, Path)
        assert stored.codec == "gzip"
        assert stored.read_bytes()[:2] == b"\x1f\x8b"
        assert str(stored.media_type) == "text/plain;encodings=application/gzip"

    def test_a_role_can_be_named_before_anything_is_there(
        self, tmp_path: pathlib.Path
    ) -> None:
        leaf = File(tmp_path / "trades.bin")
        container = Folder(tmp_path / "lake")

        assert isinstance(leaf, File)
        assert isinstance(container, Folder)
        assert not leaf.exists()
        assert not (tmp_path / "lake").exists()

        container.mkdir()
        assert container.is_dir()
        assert (tmp_path / "lake").is_dir()

    def test_the_well_known_roots_are_containers_nothing_created(self) -> None:
        for root in (Folder.temporary(), Folder.home(), Folder.config()):
            assert isinstance(root, Folder)
            assert root.url is not None

    def test_an_in_memory_handle_is_the_buffer_role(self) -> None:
        assert isinstance(IOBase.from_bytes(b"AAPL"), Buffer)
        assert isinstance(IOBase.from_bytes(), Buffer)


class TestRolesReachEveryHandleACallerGets:
    """Descent, listing, and the hierarchy answer roles too."""

    def test_children_and_listings_carry_their_own_roles(
        self, tmp_path: pathlib.Path
    ) -> None:
        (tmp_path / "lake").mkdir()
        (tmp_path / "lake" / "part-0.parquet").write_bytes(b"parquet")
        (tmp_path / "lake" / "notes.txt").write_text("notes", encoding="utf-8")

        root = Folder(tmp_path / "lake")
        listed = {entry.name: type(entry).__name__ for entry in root.iterdir()}
        assert listed == {"part-0.parquet": "Parquet", "notes.txt": "Text"}

        assert isinstance(root / "part-0.parquet", Parquet)
        assert isinstance(root.joinpath("notes.txt"), Text)
        assert isinstance(IOBase(tmp_path / "lake" / "notes.txt").parent, Path)

    def test_a_foreign_filesystem_composes_over_its_own_role(
        self, log: pathlib.Path
    ) -> None:
        handle = IOBase.from_fs(pafs.LocalFileSystem(), str(log))

        assert isinstance(handle, Text)
        assert handle.read_bytes() == PLAIN
        # The location a wrapper stands on is still the bound one, so every
        # filesystem accessor keeps answering.
        assert handle.path == str(log)
        assert handle.filesystem is not None
        assert isinstance(handle.into_handle().into_handle(), FsPath)

    def test_creating_a_container_answers_the_container(
        self, tmp_path: pathlib.Path
    ) -> None:
        leaf = Folder(tmp_path) / "sub"
        created = leaf.mkdir()

        # A byte write here would have made this location a file, so the
        # container is a different role - and a role is what a class says.
        assert isinstance(created, Folder)
        assert created.is_dir()

    def test_opening_never_changes_what_a_handle_is(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = IOBase(tmp_path / "trades.arrows")
        assert isinstance(handle, Ipc)

        handle.open()
        assert isinstance(handle, Ipc)
        handle.close()
        assert isinstance(handle, Ipc)

    def test_every_role_is_an_iobase_and_the_wrappers_share_a_base(
        self, tmp_path: pathlib.Path
    ) -> None:
        for role in (Buffer, Buffered, File, Folder, FsPath, Path, Text, Coded, Media):
            assert issubclass(role, IOBase)
        for coding in (Gzip, Zlib, Zstd, Identity):
            assert issubclass(coding, Coded)
        for encoding in (Ipc, Parquet, Avro):
            assert issubclass(encoding, Media)


class TestWhatTheRolesRefuse:
    """A role that cannot answer says so instead of answering wrongly."""

    def test_the_coding_transfers_want_stored_bytes(
        self, tmp_path: pathlib.Path
    ) -> None:
        plain = IOBase(tmp_path / "rows.json")
        plain.write_bytes(b"{}")

        # A coded handle codes what passes through it, so writing the coded
        # form through it as well would code the value twice.
        with pytest.raises(ValueError, match="copy_into already stores"):
            plain.compress_into(IOBase(tmp_path / "rows.json.gz"))

        # Writing plain bytes through it is what stores the coded form.
        coded = IOBase(tmp_path / "rows.json.gz")
        plain.copy_into(coded)
        assert stdlib_gzip.decompress(
            (tmp_path / "rows.json.gz").read_bytes()
        ) == b"{}"
        assert coded.read_bytes() == b"{}"

    def test_a_python_subclass_of_iobase_still_answers_a_native_role(
        self, tmp_path: pathlib.Path
    ) -> None:
        class Traced(IOBase):
            pass

        # Construction answers the composition the name declares, so the class
        # a caller wrote is not what comes back. Subclassing `IOBase` to add
        # behaviour to construction is not supported; wrap a handle instead.
        assert isinstance(Traced(tmp_path / "trades.arrows"), Ipc)


class TestTheCacheSurface:
    """A page cache is a role, and it answers what it is holding."""

    def test_the_cache_reports_and_drops_its_pages(self) -> None:
        cached = IOBase.from_bytes(bytes(range(256)) * 8).buffered(page_size=64)
        assert isinstance(cached, Buffered)
        assert cached.cached_pages == 0

        assert cached.read_range_bytes(0, 128) == (bytes(range(256)) * 8)[:128]
        assert cached.cached_pages > 0
        assert cached.cached_bytes > 0
        assert cached.has_cached_page(0)

        cached.clear_cache()
        assert cached.cached_pages == 0
        assert not cached.has_cached_page(0)


class TestTheS3Roles:
    """A bucket has the same three roles a disk does, and naming one is free.

    None of this contacts a store: an S3 handle is described exactly as a
    local one is, so every assertion here is about what the name alone says.
    """

    def test_each_role_commits_to_what_it_is(self) -> None:
        assert isinstance(S3File("s3://trades/lake/part.parquet"), S3File)
        assert isinstance(S3Folder("s3://trades/lake/"), S3Folder)
        assert isinstance(S3Path("s3://trades/lake/part.parquet"), S3Path)

        # A prefix always ends in the delimiter, whether or not one was written.
        assert S3Folder("s3://trades/lake").name == "lake"
        assert S3Folder("s3://trades/lake").is_dir()
        # A leaf reports what its name says, without asking the store.
        leaf = S3File("s3://trades/lake/part.parquet")
        assert str(leaf.media_type) == "application/vnd.apache.parquet"
        assert leaf.url is not None
        assert leaf.url.bucket == "trades"
        assert leaf.url.key == "lake/part.parquet"

    def test_a_bucket_and_a_raw_key_name_an_object_a_url_cannot_spell(self) -> None:
        # `a b/c.txt` is an ordinary key and not a URL, so the second argument
        # takes the name a store uses and the escaping belongs to the handle.
        handle = S3File("trades", "lake/a b/part.parquet")
        assert handle.url is not None
        assert str(handle.url) == "s3://trades/lake/a%20b/part.parquet"
        assert handle.url.key == "lake/a%20b/part.parquet"

        # The same name reaches the same object through the generic role.
        assert str(S3Path("trades", "lake/a b/part.parquet").url) == str(handle.url)
        assert str(S3Folder("trades", "lake/a b").url) == "s3://trades/lake/a%20b"

    def test_an_s3_location_is_a_handle_like_any_other(self) -> None:
        # The scheme is what selects the backend, so the ordinary constructor
        # reaches the same storage; the record implementation the name
        # declares still wraps it.
        composed = IOBase("s3://trades/lake/part.parquet")
        assert composed.url is not None
        assert composed.url.scheme == "s3"
        assert composed.partitions == ()

        partitioned = IOBase("s3://trades/lake/year=2026/part.parquet")
        assert partitioned.partitions == (("year", "2026"),)

    def test_a_location_naming_no_bucket_is_refused(self) -> None:
        with pytest.raises(ValueError, match="naming a bucket"):
            S3File("file:///tmp/part.parquet")

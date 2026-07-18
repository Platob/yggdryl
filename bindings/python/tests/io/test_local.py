"""Tests for the ``yggdryl.local`` ``LocalIO`` access point and raw ``Mmap`` mapping.

Mirrors ``crates/yggdryl-core/tests/io_local_io.rs`` on the Python surface: the lazy handle
(constructing/probing touches nothing, reads on a missing node are empty), auto-creating
self-optimizing writes (``is_mapped``), the ``close()`` release-but-stay-usable lifecycle,
fresh-lazy copies, the ``is_file`` / ``is_dir`` / ``exists`` predicates, graph navigation
(``name`` / ``parent`` / ``join`` and the ``/`` operator), streamed ``ls`` / collected
``children`` discovery, ``mkdir`` plus the empty-directory write refusal, the
directory-as-**memory-tree** byte surface (the lazy streamed ``byte_size``, reads stitched
across name-sorted child blocks, writes routed into them), and the shape-checked ``rm`` /
``rmfile`` / ``rmdir``. The ``Mmap`` sections (moved here from ``tests/io/test_memory.py``
with the core's ``io::local`` family) drive the byte surface over a real file: open/create
dispatch (str path or ``Uri``), persistence with exact truncation, read-only mappings,
capacity over a file, the ``close()`` / context-manager lifecycle, and the graph leaf
surface (``name`` / ``parent`` / the empty ``ls`` stream / the really-unlinking ``rmfile``).

Every handle is closed before the temp tree is removed — Windows cannot delete a mapped
file.
"""

import copy
import gc
import os
import pickle
import tempfile

import pytest

import yggdryl.local
from yggdryl.compression import Gzip
from yggdryl.headers import Headers
from yggdryl.io import IOKind, IOMode
from yggdryl.local import LocalEntries, LocalIO, Mmap
from yggdryl.mediatype import MediaType
from yggdryl.mimetype import MimeType
from yggdryl.memory import Heap, NoChildren, Whence
from yggdryl.uri import Uri


@pytest.fixture()
def root():
    """A ``LocalIO`` root over a fresh temp directory, removed after the test."""
    tmp = tempfile.TemporaryDirectory()
    try:
        yield LocalIO(tmp.name)
    finally:
        gc.collect()  # drop any leaked handles so their mappings release (Windows)
        tmp.cleanup()


def test_module_surface():
    for cls in (LocalIO, LocalEntries, Mmap):
        assert cls.__module__ == "yggdryl.local"
        assert hasattr(yggdryl.local, cls.__name__)


# -------------------------------------------------------------------------------------
# LocalIO: constructor + generic dispatch
# -------------------------------------------------------------------------------------


def test_localio_generic_dispatch_str_and_uri(root):
    target = root / "x.bin"
    by_str = LocalIO(target.path)  # str -> from_path
    assert by_str == target
    by_uri = LocalIO(Uri.from_path(target.path))  # Uri -> from_uri
    assert by_uri == target
    assert not by_uri.exists()  # constructing is lazy — nothing touched

    with pytest.raises(TypeError, match="expected a str filesystem path"):
        LocalIO(123)
    with pytest.raises(ValueError, match="unsupported scheme"):
        LocalIO(Uri.parse("mem://heap"))  # only file:// or plain-path URIs


# -------------------------------------------------------------------------------------
# LocalIO: lazy handle + auto-creating, self-optimizing writes
# -------------------------------------------------------------------------------------


def test_localio_lazy_probe_and_read_touch_nothing(root):
    note = root / "deep/nested/note.txt"
    assert not note.exists()
    assert not note.is_mapped
    assert note.kind == IOKind.Missing
    assert note.byte_size() == 0
    assert len(note) == 0
    assert note.is_empty()
    assert not note  # __bool__
    assert note.pread_byte_array(0, 16) == b""  # reads on a missing node are empty
    assert note.read_to_end() == b""
    with pytest.raises(ValueError, match="unexpected end of data"):
        note.pread_byte(0)  # a hard-length read still reports EOF
    assert not (root / "deep").exists()  # probing + reading created nothing


def test_localio_first_write_auto_creates_parents_and_maps(root):
    note = root / "deep/nested/dirs/note.txt"
    assert note.pwrite_utf8(0, "hello") == 5  # brings the ancestry + file into being
    assert note.is_file()
    assert note.is_mapped  # self-optimized: the mapping is kept
    assert (root / "deep/nested/dirs").is_dir()
    assert note.pread_utf8(0, 5) == "hello"  # now served from the mapping
    assert note.kind == IOKind.File
    assert note.capacity() >= 4096  # page-backed capacity while mapped
    note.flush()  # persists without closing
    note.close()


def test_localio_empty_write_does_not_create_the_file(root):
    # A zero-length write is a no-op — never a reason to auto-create the file (like touch).
    note = root / "empty/never/created.txt"
    assert note.pwrite_byte_array(0, b"") == 0  # the primitive writes nothing...
    assert not note.exists()  # ...and creates nothing on disk
    assert not note.is_mapped
    assert note.write(b"") == 0  # the cursor write is a no-op too
    assert note.pwrite_utf8(0, "") == 0
    assert not note.exists()
    assert not (root / "empty").exists()  # no parent folders materialized either


def test_localio_close_releases_mapping_handle_stays_usable(root):
    note = root / "note.bin"
    note.pwrite_utf8(0, "hello")
    assert note.is_mapped
    note.close()
    assert not note.is_mapped  # back to lazy...
    assert note.pread_utf8(0, 5) == "hello"  # ...and still usable (ad-hoc read)
    assert not note.is_mapped  # a read does not re-map
    note.close()  # idempotent
    note.pwrite_utf8(5, "!")  # the next write re-maps
    assert note.is_mapped
    assert note.pread_utf8(0, 6) == "hello!"
    note.close()
    # close truncated the capacity padding: a fresh probe sees the logical length.
    assert (root / "note.bin").byte_size() == 6


def test_localio_context_manager_closes_mapping_handle_stays_usable(root):
    target = (root / "ctx.bin").path
    with LocalIO(target) as node:
        assert node.pwrite_utf8(0, "managed") == 7
        assert node.is_mapped
        assert node.capacity() >= 4096  # padded while mapped...
    assert not node.is_mapped  # __exit__ closed the mapped backing
    assert os.path.getsize(target) == 7  # ...but on disk only the logical length remains
    assert node.pread_utf8(0, 7) == "managed"  # the handle stays usable (lazy again)


def test_localio_context_manager_closes_on_exception(root):
    target = (root / "ctx_err.bin").path
    with pytest.raises(RuntimeError, match="boom"):
        with LocalIO(target) as node:
            node.write(b"partial")
            raise RuntimeError("boom")  # __exit__ must close and re-raise
    assert not node.is_mapped
    assert os.path.getsize(target) == 7


def test_localio_copy_is_a_fresh_lazy_handle(root):
    a = root / "x.bin"
    a.pwrite_byte(0, 7)
    assert a.is_mapped
    b = a.copy()
    assert a == b  # same path...
    assert not b.is_mapped  # ...but its own lazy state (the mapping is not shared)
    assert copy.copy(a) == a
    assert copy.deepcopy(a) == a
    a.close()
    assert b.pread_byte(0) == 7
    assert a != root / "y.bin"


def test_localio_persistence_across_handles_exact_length(root):
    w = root / "keep.bin"
    w.pwrite_i64(0, 1 << 40)
    assert w.capacity() >= 4096  # padded while mapped
    w.close()  # releases the mapping, truncating to the logical length

    fresh = root / "keep.bin"
    assert fresh.byte_size() == 8
    assert fresh.pread_i64(0) == 1 << 40
    assert not fresh.is_mapped  # a never-written handle reads ad hoc


# -------------------------------------------------------------------------------------
# LocalIO: predicates (is_file / is_dir / exists)
# -------------------------------------------------------------------------------------


def test_localio_predicates_on_file_dir_and_missing(root):
    missing = root / "nothing.bin"
    assert missing.kind == IOKind.Missing
    assert not missing.is_file() and not missing.is_dir() and not missing.exists()

    f = root / "a.bin"
    f.pwrite_byte(0, 1)
    assert f.is_file() and not f.is_dir() and f.exists()
    f.close()

    d = root / "d"
    d.mkdir()
    assert d.is_dir() and not d.is_file() and d.exists()


# -------------------------------------------------------------------------------------
# LocalIO: navigation (name / parent / join / the / operator) + uri
# -------------------------------------------------------------------------------------


def test_localio_navigation_name_parent_join(root):
    node = root / "a" / "b" / "c.txt"
    assert node.name == "c.txt"
    assert node == root.join("a/b/c.txt")  # multi-segment join, same node
    parent = node.parent()
    assert isinstance(parent, LocalIO)
    assert parent.name == "b"
    assert parent.parent().name == "a"
    assert parent.parent().parent() == root
    assert parent.join("d/e.bin").name == "e.bin"
    assert LocalIO("/").parent() is None  # a root has no parent
    assert str(node.uri).endswith("c.txt")


def test_localio_parents_walk_the_ancestor_chain(root):
    node = root / "a/b/c.bin"
    parents = node.parents()
    assert isinstance(parents, list)  # a bounded ancestor walk collected as a list
    assert all(isinstance(p, LocalIO) for p in parents)
    # Nearest first: the chain walks up through the temp tree, all lazy (nothing touched).
    assert parents[0] == root / "a/b"
    assert parents[1] == root / "a"
    assert parents[2] == root
    # ...and continues up to the filesystem root, whose own parent() is None.
    assert parents[-1].parent() is None
    assert not node.exists()  # parents() is pure addressing — nothing created


def test_localio_uri_and_path(root):
    node = root / "meta.bin"
    uri = node.uri
    assert isinstance(uri, Uri)
    # The uri is the file:// URL of the path (rooted, back-slashes rewritten to slashes).
    assert uri.scheme == "file"
    normalized = node.path.replace("\\", "/")
    assert uri.path == (normalized if normalized.startswith("/") else "/" + normalized)
    assert node.path.endswith("meta.bin")


def test_localio_join_composes_addresses_and_reads_writes_the_child(root):
    # join composes the child's address over the parent's URI (Uri.joinpath) and is lazy —
    # nothing exists on disk until the child is written.
    child = root.join("logs/day1.bin")
    assert isinstance(child, LocalIO)
    assert str(child.uri).endswith("logs/day1.bin")
    assert child.name == "day1.bin"
    assert not child.exists()

    # Writing the joined child auto-creates its parents + file and reads back exactly.
    assert child.pwrite_utf8(0, "hello join") == 10
    assert child.is_file()
    assert child.pread_utf8(0, 10) == "hello join"
    child.close()

    # The child's parent() addresses the joined directory again — the inverse of join.
    logs = child.parent()
    assert logs == root.join("logs")
    assert logs.is_dir()

    # The "/" operator matches join, and a nested multi-segment join reads back through a
    # fresh handle (no shared state).
    deep = root / "a/b/c/note.txt"
    assert deep.pwrite_utf8(0, "deep") == 4
    deep.close()
    reread = root.join("a").join("b/c/note.txt")
    assert reread.pread_utf8(0, 4) == "deep"
    reread.close()


def test_localio_uri_percent_round_trips_a_path_with_a_space(root):
    node = root / "with space.bin"
    node.pwrite_utf8(0, "spaced")
    node.close()

    uri = node.uri
    assert "%20" in str(uri)  # the uri stores the path percent-encoded
    back = LocalIO(uri)  # from_uri percent-decodes it again
    assert back == node
    assert back.pread_utf8(0, 6) == "spaced"


# -------------------------------------------------------------------------------------
# LocalIO: streamed discovery (ls), collected convenience (children)
# -------------------------------------------------------------------------------------


def test_localio_ls_children_and_recursive(root):
    for rel, text in (
        ("one.txt", "1"),
        ("sub/two.txt", "2"),
        ("sub/deeper/three.txt", "3"),
    ):
        w = root / rel
        w.pwrite_utf8(0, text)
        w.close()

    direct = list(root.ls())
    assert all(isinstance(n, LocalIO) for n in direct)
    assert sorted(n.name for n in direct) == ["one.txt", "sub"]
    assert len(root.children()) == 2

    everything = sorted(n.name for n in root.ls(recursive=True))
    assert everything == ["deeper", "one.txt", "sub", "three.txt", "two.txt"]

    # A file (and a missing node) streams/lists nothing.
    assert list((root / "one.txt").ls()) == []
    assert (root / "ghost").children() == []


def test_localio_ls_streams_an_iterator_not_a_list(root):
    for rel in ("s1.txt", "sub/s2.txt"):
        w = root / rel
        w.pwrite_utf8(0, "x")
        w.close()

    # ls is a stream: an iterator over lazy handles, never a pre-collected tree.
    entries = root.ls()
    assert isinstance(entries, LocalEntries)
    assert not isinstance(entries, list)
    assert iter(entries) is entries  # the Python iterator protocol
    assert isinstance(next(entries), LocalIO)

    walk = root.ls(recursive=True)
    assert iter(walk) is walk
    assert sorted(n.name for n in walk) == ["s1.txt", "s2.txt", "sub"]
    with pytest.raises(StopIteration):
        next(walk)  # exhausted — pulling again keeps raising StopIteration


# -------------------------------------------------------------------------------------
# LocalIO: mkdir + the directory-write refusal
# -------------------------------------------------------------------------------------


def test_localio_mkdir_and_directory_write_guided(root):
    d = root / "a/b/c"
    d.mkdir()  # mkdir -p
    assert d.is_dir()
    assert (root / "a/b").is_dir()

    # A directory refuses a byte stream with a guided fix.
    with pytest.raises(ValueError, match="join a file name onto this directory and write there"):
        d.pwrite_i32(0, 1)
    assert d.pwrite_byte_array(0, b"x") == 0  # the primitive writes nothing
    assert d.pread_byte_array(0, 8) == b""  # reads on a directory are empty


# -------------------------------------------------------------------------------------
# LocalIO: a directory is a memory tree
# -------------------------------------------------------------------------------------


def test_localio_directory_reads_as_one_memory_tree(root):
    for rel, text in (("a.bin", "AAAA"), ("b.bin", "BB"), ("sub/c.bin", "CCC")):
        w = root / rel
        w.pwrite_utf8(0, text)
        w.close()  # release each mapping — the tree reads live from disk

    # byte_size is the lazy streamed sum of the subtree — recomputed live, nothing cached.
    assert root.byte_size() == 9
    assert len(root) == 9
    # Blocks are name-sorted (a.bin | b.bin | sub) — one contiguous region.
    assert root.pread_utf8(0, 9) == "AAAABBCCC"
    # A read across block boundaries stitches transparently.
    assert root.pread_utf8(3, 4) == "ABBC"
    assert root.pread_byte_array(2, 5) == b"AABBC"
    # Growth is visible immediately (full laziness): add a file, the tree grows.
    d = root / "d.bin"
    d.pwrite_utf8(0, "!")
    d.close()
    assert root.byte_size() == 10
    assert root.pread_utf8(0, 10) == "AAAABB!CCC"  # d.bin sorts before sub


def test_localio_directory_writes_route_across_blocks(root):
    for rel, text in (("a.bin", "AAAA"), ("b.bin", "BB")):
        w = root / rel
        w.pwrite_utf8(0, text)
        w.close()

    # A write inside one block stays inside it.
    assert root.pwrite_utf8(1, "XX") == 2
    assert (root / "a.bin").pread_utf8(0, 4) == "AXXA"
    # A write across the boundary splits between blocks — the middle block never grows.
    assert root.pwrite_utf8(3, "12") == 2
    assert (root / "a.bin").pread_utf8(0, 4) == "AXX1"
    assert (root / "b.bin").pread_utf8(0, 2) == "2B"
    # Bytes past the end grow the LAST block.
    assert root.pwrite_utf8(6, "ZZ") == 2
    assert (root / "b.bin").pread_utf8(0, 4) == "2BZZ"
    assert root.byte_size() == 8


def test_localio_empty_directory_full_write_is_guided(root):
    hollow = root / "hollow"
    hollow.mkdir()
    # An empty tree has no blocks: a full (pwrite_all-backed) write names the fix...
    with pytest.raises(ValueError, match="join a file name onto this directory and write there"):
        hollow.pwrite_byte(0, 1)
    # ...and the primitive writes nothing.
    assert hollow.pwrite_byte_array(0, b"x") == 0


# -------------------------------------------------------------------------------------
# LocalIO: CRUD (rm / rmfile / rmdir)
# -------------------------------------------------------------------------------------


def test_localio_rm_family_guided_mismatch_and_idempotent(root):
    f = root / "f.txt"
    f.pwrite_utf8(0, "x")
    f.close()  # release the mapping so Windows can delete
    d = root / "d"
    d.mkdir()

    with pytest.raises(ValueError, match="use rmdir"):
        d.rmfile()
    with pytest.raises(ValueError, match="use rmfile"):
        f.rmdir()

    f.rmfile()
    assert not f.exists()
    f.rmfile()  # idempotent on missing
    d.rmdir()
    assert not d.exists()
    d.rmdir()  # idempotent on missing

    # rm removes whatever exists (a file or a whole tree) and is a no-op on missing.
    for rel in ("g.txt", "h/i.txt"):
        w = root / rel
        w.pwrite_utf8(0, "z")
        w.close()
    (root / "g.txt").rm()
    (root / "h").rm()
    (root / "ghost").rm()
    assert root.children() == []


# -------------------------------------------------------------------------------------
# LocalIO: byte surface (typed + bulk + cursor stream + capacity) over one handle
# -------------------------------------------------------------------------------------


def test_localio_typed_bulk_and_repeat(root):
    node = root / "bulk.bin"
    node.pwrite_i32_array(0, [1, -2, 3])
    assert node.pread_i32_array(0, 3) == [1, -2, 3]
    node.pwrite_i64_array(12, [1 << 40])
    assert node.pread_i64_array(12, 1) == [1 << 40]
    node.pwrite_byte_repeat(20, 0xAB, 4)
    assert node.pread_byte_array(20, 4) == b"\xab" * 4
    node.pwrite_bit(200, True)  # bit 0 of byte 25
    assert node.pread_bit(200)
    with pytest.raises(ValueError, match="unexpected end of data"):
        node.pread_i32_array(0, 2_000_000_000)  # fail-fast bounds check
    node.close()


def test_localio_cursor_stream_and_capacity(root):
    node = root / "stream.bin"
    assert node.position == 0
    assert node.write(b"hello") == 5
    assert node.write_utf8(" wörld") == 7
    node.write_byte(0x21)
    node.write_i32(-7)
    node.write_i64(1 << 40)
    node.rewind()
    assert node.read(5) == b"hello"
    assert node.read_utf8(7) == " wörld"
    assert node.read_byte() == 0x21
    assert node.read_i32() == -7
    assert node.read_i64() == 1 << 40
    assert node.seek(Whence.End, -4) == node.byte_size() - 4
    node.set_position(0)
    assert node.read_to_end()[:5] == b"hello"
    with pytest.raises(ValueError, match="invalid seek"):
        node.seek(Whence.Start, -1)

    node.try_reserve(4096)
    assert node.capacity() >= 4096
    assert node.spare_capacity() == node.capacity() - node.byte_size()
    node.try_reserve_exact(1)
    node.ensure_capacity(8192)
    node.try_ensure_capacity(8192)
    assert node.capacity() >= 8192
    node.shrink_to(64)
    node.shrink_to_fit()
    assert node.capacity() == node.byte_size()
    node.close()


def test_localio_reserve_exact_materializes_real_capacity(root):
    node = root / "exact.bin"
    node.reserve_exact(4096)  # a fresh writable handle materializes the mapped backing
    assert node.is_mapped
    assert node.capacity() >= 4096
    node.close()


# -------------------------------------------------------------------------------------
# LocalIO: metadata (headers / mode) + read-only refusal
# -------------------------------------------------------------------------------------


def test_localio_headers_getter_returns_a_copy(root):
    node = root / "meta.bin"
    assert isinstance(node.headers, Headers)
    grabbed = node.headers
    grabbed.insert("a", "1")  # mutating the returned copy...
    assert len(node.headers) == 0  # ...does not touch the handle until written back
    node.set_headers(grabbed)
    assert node.headers.get("a") == "1"


def test_localio_read_only_mode_guided(root):
    node = root / "ro.bin"
    node.pwrite_utf8(0, "x")
    node.close()
    assert node.mode == IOMode.ReadWrite
    node.set_mode(IOMode.Read)
    assert node.mode == IOMode.Read
    assert node.pwrite_byte_array(0, b"Z") == 0  # the write primitives write nothing
    with pytest.raises(ValueError, match="set_mode"):
        node.pwrite_i32(0, 1)  # the full/typed writes name the fix
    assert node.pread_utf8(0, 1) == "x"
    node.set_mode(IOMode.ReadWrite)


def test_localio_read_only_try_reserve_refuses_and_creates_nothing(root):
    node = root / "ro_reserve.bin"
    node.set_mode(IOMode.Read)
    with pytest.raises(ValueError, match="read-only"):
        node.try_reserve(64)  # the guided refusal names the state...
    with pytest.raises(ValueError, match="set_mode"):
        node.try_reserve_exact(64)  # ...and the fix
    assert not node.exists()  # the refusal touched nothing on disk
    assert not node.is_mapped


# -------------------------------------------------------------------------------------
# LocalIO: live-handle dunders (eq by path, unhashable, no pickle)
# -------------------------------------------------------------------------------------


def test_localio_is_a_live_handle_not_a_value(root):
    node = root / "live.bin"
    with pytest.raises(TypeError):
        hash(node)  # defining equality leaves the mutable handle unhashable
    # A live handle pickles by its PORTABLE ADDRESS (not a byte codec): it reconstructs a lazy
    # handle addressing the same path, relocated against the receiving environment's roots.
    assert pickle.loads(pickle.dumps(node)) == node
    for absent in (
        "serialize_bytes",
        "deserialize_bytes",
        "with_headers",
        "with_mode",
        "cursor",
        "window",
        "slice",
    ):
        assert not hasattr(node, absent)
    # A LocalIO is io-file-like: `closed` exists but a handle is never truly closed (close()
    # only releases the mapping and the handle stays usable), so it is always False.
    assert node.closed is False
    assert repr(node) == f"LocalIO({node.path}, <0 bytes>)"


# -------------------------------------------------------------------------------------
# LocalIO: temp-dir builders (tmpfile / tmpfolder)
# -------------------------------------------------------------------------------------


def test_localio_tmpfile_is_lazy_created_on_write_and_unique():
    scratch = LocalIO.tmpfile()  # a unique handle in the system temp dir, nothing on disk
    try:
        assert isinstance(scratch, LocalIO)
        assert not scratch.exists()  # lazy — only the path is picked
        assert not scratch.is_mapped
        assert scratch.name.endswith(".tmp")  # the default unnamed file ends in .tmp
        assert os.path.dirname(scratch.path) == tempfile.gettempdir()

        assert scratch.pwrite_utf8(0, "temp data") == 9  # first write creates + maps it
        assert scratch.is_file()
        assert scratch.pread_utf8(0, 9) == "temp data"
        scratch.close()

        # Two unnamed tmpfiles pick distinct unique paths (never touched on disk here).
        assert LocalIO.tmpfile().path != LocalIO.tmpfile().path
    finally:
        scratch.close()
        gc.collect()
        scratch.rmfile()


def test_localio_tmpfolder_named_auto_creates_on_child_write():
    work = LocalIO.tmpfolder("yggdryl-py-tmpfolder-test")
    child = work / "out.bin"
    try:
        assert isinstance(work, LocalIO)
        assert not work.exists()  # lazy — nothing created yet
        assert os.path.dirname(work.path) == tempfile.gettempdir()

        assert child.pwrite_byte_array(0, b"x") == 1  # writing the child auto-creates work
        assert work.is_dir()
        assert child.is_file()
        assert child.pread_byte_array(0, 1) == b"x"
        child.close()
    finally:
        child.close()
        gc.collect()
        work.rmdir()  # removes the folder + its subtree (idempotent when missing)


# -------------------------------------------------------------------------------------
# Mmap: constructors + generic open dispatch
# -------------------------------------------------------------------------------------


def test_mmap_has_no_plain_constructor():
    # The explicit lifecycle verbs open/open_readonly/create are the only entry points.
    with pytest.raises(TypeError):
        Mmap()


def test_mmap_generic_open_dispatch_str_and_uri(tmp_path):
    p = tmp_path / "dispatch.bin"
    m = Mmap.create(str(p))  # str -> create_path
    assert m.pwrite_utf8(0, "hi") == 2
    m.close()

    by_str = Mmap.open(str(p))  # str -> open_path
    assert by_str.pread_utf8(0, 2) == "hi"
    by_str.close()

    uri = Uri.from_path(str(p))
    by_uri = Mmap.open(uri)  # Uri -> open_uri
    assert by_uri.pread_utf8(0, 2) == "hi"
    by_uri.close()

    ro = Mmap.open_readonly(uri)  # Uri -> open_uri_readonly
    assert ro.mode == IOMode.Read
    assert ro.pread_utf8(0, 2) == "hi"
    ro.close()

    made = Mmap.create(Uri.from_path(str(tmp_path / "made.bin")))  # Uri -> create_uri
    assert made.is_empty()
    made.close()


def test_mmap_open_dispatch_rejects_other_types_guided(tmp_path):
    with pytest.raises(TypeError, match="expected a str filesystem path"):
        Mmap.open(123)
    with pytest.raises(TypeError, match="str\\(path\\)"):
        Mmap.create(tmp_path / "x.bin")  # a pathlib.Path must be passed as str(path)


def test_mmap_open_missing_path_is_guided(tmp_path):
    missing = str(tmp_path / "missing.bin")
    with pytest.raises(ValueError, match="check that the path exists"):
        Mmap.open(missing)
    with pytest.raises(ValueError, match="check that the path exists"):
        Mmap.open_readonly(missing)


# -------------------------------------------------------------------------------------
# Mmap: write/read round-trips (typed + bulk + utf8 + cursor stream)
# -------------------------------------------------------------------------------------


def test_mmap_typed_positioned_round_trip(tmp_path):
    m = Mmap.create(str(tmp_path / "typed.bin"))
    m.pwrite_byte(0, 0xAB)
    m.pwrite_i32(1, -42)
    m.pwrite_i64(5, 1 << 40)
    assert m.pread_byte(0) == 0xAB
    assert m.pread_i32(1) == -42
    assert m.pread_i64(5) == 1 << 40
    assert m.byte_size() == 13
    m.pwrite_bit(104, True)  # bit 0 of byte 13 -- grows the file by one byte
    assert m.pread_bit(104)
    assert not m.pread_bit(105)
    assert m.byte_size() == 14
    with pytest.raises(ValueError, match="unexpected end of data"):
        m.pread_i64(m.byte_size())
    m.close()


def test_mmap_pread_pwrite_byte_array_and_gap_zero_fill(tmp_path):
    m = Mmap.create(str(tmp_path / "bytes.bin"))
    assert m.pwrite_byte_array(0, b"abc") == 3
    assert m.pwrite_byte_array(5, b"Z") == 1  # past the end: the gap is zero-filled
    assert m.pread_byte_array(0, 99) == b"abc\x00\x00Z"  # clamped to what remains
    assert m.pread_byte_array(6, 4) == b""  # at the end
    m.close()


def test_mmap_bulk_arrays_repeats_and_utf8(tmp_path):
    m = Mmap.create(str(tmp_path / "bulk.bin"))
    values = list(range(-500, 500))  # 1000 elements crosses the 256-element chunk
    m.pwrite_i32_array(0, values)
    assert m.pread_i32_array(0, 1000) == values
    wide = [(1 << 40) + i for i in range(300)]
    m.pwrite_i64_array(4000, wide)
    assert m.pread_i64_array(4000, 300) == wide

    m.pwrite_byte_repeat(6400, 0xAB, 5)
    assert m.pread_byte_array(6400, 5) == b"\xab" * 5
    m.pwrite_i32_repeat(6405, -1, 10)
    assert m.pread_i32_array(6405, 10) == [-1] * 10
    m.pwrite_i64_repeat(6445, 7, 4)
    assert m.pread_i64_array(6445, 4) == [7] * 4

    assert m.pwrite_utf8(0, "héllo") == 6  # é is 2 bytes
    assert m.pread_utf8(0, 6) == "héllo"
    with pytest.raises(ValueError, match="invalid UTF-8"):
        m.pread_utf8(0, 2)  # cuts the 2-byte é in half

    # The bulk-read bounds are checked before the result list is allocated.
    with pytest.raises(ValueError, match="unexpected end of data"):
        m.pread_i32_array(0, 2_000_000_000)
    with pytest.raises(ValueError, match="unexpected end of data"):
        m.pread_i64_array(0, 2_000_000_000)
    m.close()


def test_mmap_cursor_stream(tmp_path):
    m = Mmap.create(str(tmp_path / "stream.bin"))
    assert m.position == 0
    assert m.write(b"hello") == 5
    assert m.write_utf8(" wörld") == 7
    m.write_byte(0x21)
    m.write_i32(-7)
    m.write_i64(1 << 40)
    assert m.position == 5 + 7 + 1 + 4 + 8

    m.rewind()
    assert m.position == 0
    assert m.read(5) == b"hello"
    assert m.read_utf8(7) == " wörld"
    assert m.read_byte() == 0x21
    assert m.read_i32() == -7
    assert m.read_i64() == 1 << 40

    assert m.seek(Whence.Start, 6) == 6
    assert m.seek(Whence.Current, -1) == 5
    assert m.seek(Whence.End, -5) == m.byte_size() - 5
    with pytest.raises(ValueError, match="invalid seek"):
        m.seek(Whence.Start, -1)

    payload = (
        "hello wörld".encode()
        + b"\x21"
        + (-7).to_bytes(4, "little", signed=True)
        + (1 << 40).to_bytes(8, "little")
    )
    m.rewind()
    assert m.read_to_end() == payload
    assert m.position == m.byte_size()
    assert m.read(5) == b""  # at the end

    m.set_position(6)
    assert m.read(5) == payload[6:11]
    pos = m.position
    m.set_position(m.byte_size())
    with pytest.raises(ValueError):
        m.read_i32()  # past the end
    assert m.position == m.byte_size()  # a failed read must not advance
    m.set_position(pos)
    m.close()


def test_mmap_auto_grow_appends_and_size_dunders(tmp_path):
    m = Mmap.create(str(tmp_path / "grow.bin"))
    assert m.byte_size() == 0
    assert m.is_empty()
    assert len(m) == 0
    assert not m  # __bool__ over an empty file
    for i in range(100):
        m.pwrite_i64(i * 8, i)  # every append past the end auto-grows the file
    assert m.byte_size() == 800
    assert len(m) == 800
    assert m.bit_size() == 6400
    assert not m.is_empty()
    assert m
    assert m.pread_i64_array(0, 100) == list(range(100))
    m.pwrite_byte(1000, 0xFF)  # far past the end: the gap arrives zero-filled
    assert m.byte_size() == 1001
    assert m.pread_byte(900) == 0
    m.close()


# -------------------------------------------------------------------------------------
# Mmap: persistence (truncate-on-close) + reopen
# -------------------------------------------------------------------------------------


def test_mmap_persists_with_exact_logical_length(tmp_path):
    p = tmp_path / "persist.bin"
    m = Mmap.create(str(p))
    m.pwrite_utf8(0, "hello mapped world")
    assert m.byte_size() == 18
    assert m.capacity() >= 4096  # page-backed capacity padding while open
    del m  # drop unmaps and truncates the padding away
    gc.collect()

    assert p.stat().st_size == 18  # the on-disk file keeps only the logical length
    back = Mmap.open(str(p))
    assert back.byte_size() == 18
    assert back.pread_utf8(0, 5) == "hello"
    assert back.pread_utf8(6, 6) == "mapped"
    back.close()


def test_mmap_create_keeps_existing_contents(tmp_path):
    p = tmp_path / "keep.bin"
    first = Mmap.create(str(p))
    first.pwrite_utf8(0, "keep me")
    first.close()

    again = Mmap.create(str(p))  # create never truncates an existing file on open
    assert again.pread_utf8(0, 7) == "keep me"
    again.close()


# -------------------------------------------------------------------------------------
# Mmap: read-only mappings
# -------------------------------------------------------------------------------------


def test_mmap_open_readonly_reads_work_writes_are_guided(tmp_path):
    p = tmp_path / "ro.bin"
    m = Mmap.create(str(p))
    m.pwrite_i32(0, 99)
    m.close()

    ro = Mmap.open_readonly(str(p))
    assert ro.mode == IOMode.Read
    assert ro.pread_i32(0) == 99
    assert ro.pread_byte_array(0, 4) == (99).to_bytes(4, "little")
    assert ro.pwrite_byte_array(0, b"XX") == 0  # the write primitives write nothing
    assert ro.pread_i32(0) == 99
    with pytest.raises(ValueError, match="read-only"):
        ro.pwrite_i32(0, 1)  # the full/typed writes name the fix
    with pytest.raises(ValueError, match="read-only"):
        ro.try_reserve(64)
    with pytest.raises(ValueError, match="read-only"):
        ro.try_reserve_exact(64)
    ro.close()


# -------------------------------------------------------------------------------------
# Mmap: capacity family over a file
# -------------------------------------------------------------------------------------


def test_mmap_capacity_family_over_a_file(tmp_path):
    m = Mmap.create(str(tmp_path / "cap.bin"))
    m.pwrite_byte_array(0, b"\x01" * 16)
    assert m.capacity() >= 4096  # never below one page while mapped
    m.try_reserve(8192)
    assert m.capacity() >= 16 + 8192
    assert m.spare_capacity() == m.capacity() - 16
    m.try_reserve_exact(32)  # already satisfied
    m.reserve(1)
    m.reserve_exact(1)
    m.ensure_capacity(10_000)
    m.try_ensure_capacity(10_000)
    assert m.capacity() >= 10_000

    # An overflowing request raises the guided capacity error before touching the mapping.
    with pytest.raises(ValueError, match="reserve less"):
        m.try_reserve(2**64 - 1)
    assert m.pread_byte_array(0, 4) == b"\x01" * 4  # still fully usable

    m.shrink_to(64)
    m.shrink_to_fit()  # remaps to exactly the logical length
    assert m.capacity() == 16
    assert m.byte_size() == 16
    assert m.spare_capacity() == 0
    assert m.pread_byte_array(0, 16) == b"\x01" * 16  # contents survive the remaps
    m.close()


# -------------------------------------------------------------------------------------
# Mmap: metadata (path / uri / kind / mode / headers) + predicates + flush + repr
# -------------------------------------------------------------------------------------


def test_mmap_metadata_and_flush(tmp_path):
    p = tmp_path / "meta.bin"
    m = Mmap.create(str(p))
    assert m.kind == IOKind.File
    assert m.is_file() and not m.is_dir() and m.exists()  # a live mapping is a live file
    assert m.mode == IOMode.ReadWrite
    m.set_mode(IOMode.Read)  # a label only; the physical protection is fixed at open
    assert m.mode == IOMode.Read
    m.set_mode(IOMode.ReadWrite)

    assert m.path == str(p)
    uri = m.uri
    assert isinstance(uri, Uri)
    # The uri is the file:// URL of the mapped file path (rooted, back-slashes to slashes).
    assert uri.scheme == "file"
    normalized = str(p).replace("\\", "/")
    assert uri.path == (normalized if normalized.startswith("/") else "/" + normalized)

    assert isinstance(m.headers, Headers)
    assert len(m.headers) == 0
    grabbed = m.headers
    grabbed.insert("a", "1")  # mutating the returned copy...
    assert len(m.headers) == 0  # ...does not touch the mapping until written back
    m.set_headers(grabbed)
    assert m.headers.get("a") == "1"
    # There is deliberately no with_headers/with_mode: they would need a copy, and a
    # live mapping cannot be copied.
    assert not hasattr(m, "with_headers")
    assert not hasattr(m, "with_mode")

    m.pwrite_utf8(0, "flushed")
    m.flush()  # persists the mapped bytes without closing
    assert m.pread_utf8(0, 7) == "flushed"

    assert repr(m) == f"Mmap({p}, <7 bytes>)"
    m.close()


def test_mmap_is_a_live_resource_not_a_value(tmp_path):
    m = Mmap.create(str(tmp_path / "resource.bin"))
    # A live OS resource is deliberately not a value: no equality/copy/codec/pickle and
    # no monomorphic Cursor/Slice builders (they are Heap-only in the binding).
    for absent in (
        "copy",
        "__copy__",
        "__deepcopy__",
        "serialize_bytes",
        "deserialize_bytes",
        "with_capacity",
        "cursor",
        "window",
        "slice",
    ):
        assert not hasattr(m, absent)
    assert m == m  # default identity equality only
    other = Mmap.open(str(tmp_path / "resource.bin"))
    assert m != other
    with pytest.raises(TypeError):
        pickle.dumps(m)  # a mapping does not pickle
    other.close()
    m.close()


# -------------------------------------------------------------------------------------
# Mmap: the graph surface (a mapping is a leaf whose rm / rmfile really unlink)
# -------------------------------------------------------------------------------------


def test_mmap_graph_leaf_name_parent_ls_children(tmp_path):
    m = Mmap.create(str(tmp_path / "leaf.bin"))
    assert m.name == "leaf.bin"  # the node's name is its file name
    assert m.parent() is None  # a raw mapping is a leaf of the IO graph
    entries = m.ls()
    assert isinstance(entries, NoChildren)  # the shared always-empty leaf iterator
    assert not isinstance(entries, list)
    assert iter(entries) is entries  # the iterator protocol, like LocalIO.ls
    assert list(entries) == []  # ...but a leaf streams nothing
    with pytest.raises(StopIteration):
        next(entries)
    assert list(m.ls(recursive=True)) == []
    assert repr(m.ls()) == "NoChildren(<empty>)"
    assert m.children() == []
    m.close()
    with pytest.raises(ValueError, match="closed"):
        m.name  # noqa: B018 - the graph goes through the closed-guard too
    with pytest.raises(ValueError, match="closed"):
        m.rmfile()


def test_mmap_rmdir_is_the_guided_file_error(tmp_path):
    m = Mmap.create(str(tmp_path / "file.bin"))
    with pytest.raises(ValueError, match="use rmfile instead of rmdir"):
        m.rmdir()  # a mapping is never a directory
    m.close()


def test_mmap_rmfile_really_unlinks_after_the_writer_closed(tmp_path):
    p = tmp_path / "unlink.bin"
    w = Mmap.create(str(p))
    w.pwrite_utf8(0, "data")
    w.close()  # release the writing mapping first — portable removal (Windows)
    assert p.exists()

    m = Mmap.open(str(p))
    m.rmfile()  # really unlinks the on-disk file
    assert not p.exists()
    m.rmfile()  # idempotent when already missing
    m.rm()  # rm is rmfile for a mapping — idempotent too
    m.close()
    assert not p.exists()


# -------------------------------------------------------------------------------------
# Mmap: close() + context manager
# -------------------------------------------------------------------------------------


def test_mmap_close_is_idempotent_and_guards_every_access(tmp_path):
    p = tmp_path / "close.bin"
    m = Mmap.create(str(p))
    m.pwrite_utf8(0, "abc")
    assert not m.closed
    m.close()
    assert m.closed
    m.close()  # double-close is a no-op
    assert m.closed

    with pytest.raises(
        ValueError,
        match="the mapping is closed; reopen it with Mmap.open / Mmap.open_readonly / Mmap.create",
    ):
        m.byte_size()  # the guided error names all three reopen verbs
    with pytest.raises(ValueError, match="closed"):
        m.pread_byte(0)
    with pytest.raises(ValueError, match="closed"):
        m.pwrite_utf8(0, "x")
    with pytest.raises(ValueError, match="closed"):
        m.read_to_end()
    with pytest.raises(ValueError, match="closed"):
        m.flush()
    with pytest.raises(ValueError, match="closed"):
        m.path
    with pytest.raises(ValueError, match="closed"):
        m.exists()
    assert repr(m) == "Mmap(<closed>)"  # repr never raises

    assert p.stat().st_size == 3  # close truncated the file to its logical length
    # The guided error names the fix: reopen.
    reopened = Mmap.open(str(p))
    assert reopened.pread_utf8(0, 3) == "abc"
    reopened.close()


def test_mmap_context_manager_closes_and_persists(tmp_path):
    p = tmp_path / "ctx.bin"
    with Mmap.create(str(p)) as m:
        assert m.write(b"managed") == 7
        assert m.capacity() >= 4096  # padded while open...
    assert m.closed  # __exit__ closed it
    assert p.stat().st_size == 7  # ...but on disk only the logical length remains

    with Mmap.open(str(p)) as back:
        assert back.read_to_end() == b"managed"
    assert back.closed


def test_mmap_context_manager_closes_on_exception(tmp_path):
    p = tmp_path / "ctx_err.bin"
    with pytest.raises(RuntimeError, match="boom"):
        with Mmap.create(str(p)) as m:
            m.write(b"partial")
            raise RuntimeError("boom")  # __exit__ must close and re-raise
    assert m.closed
    assert p.stat().st_size == 7


# -------------------------------------------------------------------------------------
# Address: file:// URI + media type inferred from the file address
# -------------------------------------------------------------------------------------


def test_localio_uri_is_file_scheme(root):
    node = root.join("report.pdf")
    uri = node.uri
    assert isinstance(uri, Uri)
    assert uri.scheme == "file"
    assert str(uri).startswith("file:///")
    assert str(uri).endswith("/report.pdf")


def test_localio_mime_and_media_type_from_address(root):
    # Lazy handle (nothing written yet) still infers from the file name.
    pdf = root.join("report.pdf")
    assert isinstance(pdf.mime_type(), MimeType)
    assert pdf.mime_type().essence == "application/pdf"

    tgz = root.join("archive.tar.gz")
    media = tgz.media_type()
    assert isinstance(media, MediaType)
    assert media.essences() == ["application/x-tar", "application/gzip"]

    # An extension-less node falls back to octet-stream (never raises).
    assert root.join("mystery").mime_type().is_octet_stream()


def test_localio_headers_win_over_address(root):
    node = root.join("report.pdf")
    node.set_headers(Headers().with_("Content-Type", "application/json"))
    assert node.mime_type().essence == "application/json"  # declared header wins


def test_localio_ensure_content_type_memoizes(root):
    node = root.join("data.json")
    assert node.headers.content_type() is None
    resolved = node.ensure_content_type()
    assert resolved.essence == "application/json"  # inferred from the name, and stored
    assert node.headers.content_type() == "application/json"


def test_mmap_mime_type_from_file_name(root):
    path = root.join("photo.png")
    m = Mmap.create(path.path)
    try:
        assert m.mime_type().essence == "image/png"
        assert m.media_type().essences() == ["image/png"]
        assert m.ensure_content_type().essence == "image/png"
    finally:
        m.close()


# -------------------------------------------------------------------------------------
# LocalIO: truncate + content_length
# -------------------------------------------------------------------------------------


def test_localio_truncate_shrinks_and_extends(root):
    node = root / "t.bin"
    node.pwrite_utf8(0, "hello world")
    node.truncate(5)
    assert node.pread_utf8(0, 5) == "hello"
    assert node.byte_size() == 5
    node.truncate(8)  # extend, zero-filled
    assert node.byte_size() == 8
    assert node.pread_byte_array(5, 3) == b"\x00\x00\x00"
    node.close()


def test_localio_content_length_falls_back_to_byte_size(root):
    node = root / "cl.bin"
    node.pwrite_utf8(0, "hello")
    assert node.content_length() == 5  # no header — falls back to byte_size
    node.close()


def test_localio_memory_info_reports_the_disk_volume(root):
    from yggdryl.io import MemoryInfo

    info = root.memory_info()  # the capacity of the disk volume backing the temp tree
    assert isinstance(info, MemoryInfo)
    assert info.total() >= info.available()  # total >= free within a volume
    assert info.used() == info.total() - info.available()
    # A not-yet-created child resolves the same volume (walks up to an existing ancestor).
    child_info = (root / "deep/not/created.bin").memory_info()
    assert isinstance(child_info, MemoryInfo)
    assert child_info.total() >= child_info.available()


# -------------------------------------------------------------------------------------
# LocalIO: bulk unsigned + float arrays / repeats + cross-source copy
# -------------------------------------------------------------------------------------


def test_localio_bulk_unsigned_and_float(root):
    node = root / "ubulk.bin"
    node.pwrite_u16_array(0, [1, 2, 65535])
    assert node.pread_u16_array(0, 3) == [1, 2, 65535]
    node.pwrite_u32_array(8, [1, 2**31, 2**32 - 1])
    assert node.pread_u32_array(8, 3) == [1, 2**31, 2**32 - 1]
    node.pwrite_u64_array(24, [2**63])
    assert node.pread_u64_array(24, 1) == [2**63]
    node.pwrite_f32_array(40, [1.5, -2.25])
    assert node.pread_f32_array(40, 2) == [1.5, -2.25]
    node.pwrite_f64_array(52, [1.5, 1e300])
    assert node.pread_f64_array(52, 2) == [1.5, 1e300]
    node.pwrite_u16_repeat(80, 7, 4)
    assert node.pread_u16_array(80, 4) == [7] * 4
    with pytest.raises(ValueError, match="unexpected end of data"):
        node.pread_u32_array(0, 2_000_000_000)  # fail-fast bounds check
    node.close()


def test_localio_copy_from_and_pwrite_from_a_heap(root):
    node = root / "copy.bin"
    node.pwrite_utf8(0, "original longer content")
    assert node.copy_from(Heap(b"short")) == 5  # overwrite + truncate to the source length
    assert node.pread_byte_array(0, 99) == b"short"
    node.close()

    pf = root / "pfrom.bin"
    pf.pwrite_byte_array(0, b"..........")
    assert pf.pwrite_from(2, Heap(b"ABCDEF"), 1, 3) == 3  # src[1:4] = "BCD" at offset 2
    assert pf.pread_byte_array(0, 10) == b"..BCD....."
    pf.close()


# -------------------------------------------------------------------------------------
# LocalIO: readline / readlines
# -------------------------------------------------------------------------------------


def test_localio_readline_and_readlines(root):
    node = root / "lines.txt"
    node.pwrite_utf8(0, "line1\nline2\n")
    node.rewind()
    assert node.readline() == "line1\n"
    assert node.readlines() == ["line2\n"]
    node.close()


# -------------------------------------------------------------------------------------
# LocalIO: mmap() front door + tmpdir alias
# -------------------------------------------------------------------------------------


def test_localio_mmap_builds_a_standalone_mapping(root):
    node = root / "mapped.bin"
    m = node.mmap()  # creates + maps the file read-write, reusing the handle's params
    assert isinstance(m, Mmap)
    try:
        assert m.pwrite_utf8(0, "via mmap") == 8
        assert m.pread_utf8(0, 8) == "via mmap"
    finally:
        m.close()
    # The bytes persisted to the file the lazy handle addresses.
    assert (root / "mapped.bin").pread_utf8(0, 8) == "via mmap"


def test_localio_tmpdir_is_an_alias_of_tmpfolder():
    work = LocalIO.tmpdir("yggdryl-py-tmpdir-test")
    child = work / "out.bin"
    try:
        assert isinstance(work, LocalIO)
        assert not work.exists()  # lazy alias of tmpfolder
        assert os.path.dirname(work.path) == tempfile.gettempdir()
        assert child.pwrite_byte_array(0, b"x") == 1  # writing a child auto-creates work
        assert work.is_dir()
        child.close()
    finally:
        child.close()
        gc.collect()
        work.rmdir()


# -------------------------------------------------------------------------------------
# LocalIO: in-place (de)compression
# -------------------------------------------------------------------------------------


def test_localio_compress_and_decompress_in_place(root):
    node = root / "comp.bin"
    payload = b"compress this file in place " * 30
    node.pwrite_byte_array(0, payload)
    node.compress_in_place(Gzip())  # explicit codec
    assert node.byte_size() < len(payload)
    assert node.headers.content_type() == "application/gzip"
    node.decompress_in_place()  # codec inferred from the media type
    assert node.pread_byte_array(0, len(payload)) == payload
    node.close()


# -------------------------------------------------------------------------------------
# LocalIO: rm family exist_ok argument
# -------------------------------------------------------------------------------------


def test_localio_rm_family_exist_ok(root):
    missing = root / "ghost.bin"
    missing.rm(exist_ok=True)  # default — a missing node is a no-op
    with pytest.raises(ValueError, match="nothing exists here to remove"):
        missing.rm(exist_ok=False)  # ...but exist_ok=False raises on a missing node
    with pytest.raises(ValueError, match="exist_ok=true"):
        missing.rmfile(exist_ok=False)
    with pytest.raises(ValueError, match="exist_ok=true"):
        missing.rmdir(exist_ok=False)


# -------------------------------------------------------------------------------------
# Mmap: truncate + content_length + bulk widths + cross-source copy + in-place codecs
# -------------------------------------------------------------------------------------


def test_mmap_truncate_and_content_length(tmp_path):
    m = Mmap.create(str(tmp_path / "trunc.bin"))
    m.pwrite_utf8(0, "hello world")
    assert m.content_length() == 11  # falls back to byte_size
    m.truncate(5)
    assert m.pread_utf8(0, 5) == "hello"
    assert m.byte_size() == 5
    m.truncate(8)
    assert m.byte_size() == 8
    m.close()


def test_mmap_bulk_unsigned_and_float(tmp_path):
    m = Mmap.create(str(tmp_path / "ubulk.bin"))
    m.pwrite_u16_array(0, [1, 2, 65535])
    assert m.pread_u16_array(0, 3) == [1, 2, 65535]
    m.pwrite_u32_array(8, [1, 2**32 - 1])
    assert m.pread_u32_array(8, 2) == [1, 2**32 - 1]
    m.pwrite_u64_array(16, [2**63])
    assert m.pread_u64_array(16, 1) == [2**63]
    m.pwrite_f32_array(24, [1.5, -2.25])
    assert m.pread_f32_array(24, 2) == [1.5, -2.25]
    m.pwrite_f64_array(32, [1.5, 1e300])
    assert m.pread_f64_array(32, 2) == [1.5, 1e300]
    m.pwrite_u64_repeat(48, 9, 4)
    assert m.pread_u64_array(48, 4) == [9] * 4
    with pytest.raises(ValueError, match="unexpected end of data"):
        m.pread_u16_array(0, 2_000_000_000)
    m.close()


def test_mmap_copy_from_and_pwrite_from_a_heap(tmp_path):
    m = Mmap.create(str(tmp_path / "cp.bin"))
    m.pwrite_utf8(0, "original longer content")
    assert m.copy_from(Heap(b"short")) == 5
    assert m.pread_byte_array(0, 99) == b"short"
    assert m.pwrite_from(0, Heap(b"XYZ"), 0, 2) == 2  # "XY" over the head
    assert m.pread_byte_array(0, 5) == b"XYort"
    m.close()


def test_mmap_compress_and_decompress_in_place(tmp_path):
    p = tmp_path / "comp.bin"
    m = Mmap.create(str(p))
    payload = b"compress the mapping in place " * 30
    m.pwrite_byte_array(0, payload)
    m.compress_in_place(Gzip())
    assert m.byte_size() < len(payload)
    assert m.headers.content_type() == "application/gzip"
    m.decompress_in_place()
    assert m.pread_byte_array(0, len(payload)) == payload
    m.close()


def test_mmap_rmfile_accepts_exist_ok(tmp_path):
    p = tmp_path / "rmk.bin"
    w = Mmap.create(str(p))
    w.pwrite_utf8(0, "d")
    w.close()  # release the writer first (portable removal on Windows)
    m = Mmap.open(str(p))
    m.rmfile(exist_ok=False)  # the file exists — removes it fine
    assert not p.exists()
    m.rmfile(exist_ok=True)  # idempotent when already missing
    m.close()


# -------------------------------------------------------------------------------------
# LocalIO: os.PathLike + io-file-like duck typing
# -------------------------------------------------------------------------------------


def test_localio_is_os_pathlike(root):
    node = root / "fspath.bin"
    assert node.__fspath__() == node.path
    assert os.fspath(node) == node.path  # accepted by the os.PathLike protocol
    node.pwrite_utf8(0, "hi")
    node.close()
    assert os.path.getsize(node) == 2  # a stdlib call takes the handle straight


def test_localio_io_duck_methods(root):
    node = root / "duck.bin"
    node.write(b"line1\nline2\n")
    assert node.tell() == 12  # == position, under the io method name
    assert node.tell() == node.position
    assert node.seekable() is True
    assert node.readable() is True and node.writable() is True
    node.set_mode(IOMode.Read)
    assert node.readable() is True and node.writable() is False
    node.set_mode(IOMode.ReadWrite)
    assert node.closed is False  # a handle is never truly closed
    node.close()
    assert node.closed is False  # ...even after close() (it stays usable)


def test_localio_line_iteration(root):
    node = root / "lines2.txt"
    node.pwrite_utf8(0, "alpha\nbeta\ngamma")
    node.rewind()
    assert iter(node) is node
    assert list(node) == ["alpha\n", "beta\n", "gamma"]  # lines, not children
    node.close()


def test_localio_pickles_by_portable_address(root):
    # A LocalIO under the temp root pickles to a portable form and reconstructs a lazy handle
    # addressing the same file — reopened against THIS environment's temp root.
    node = root / "pickled.bin"
    node.pwrite_utf8(0, "persist me")
    node.close()

    revived = pickle.loads(pickle.dumps(node))
    assert isinstance(revived, LocalIO)
    assert revived == node  # same path identity
    assert revived.pread_utf8(0, 10) == "persist me"

    # to_portable_str folds the temp/home path to a $TMP / ~ token; from_portable expands it.
    portable = node.to_portable_str()
    assert portable.startswith("$TMP") or portable.startswith("~")
    assert LocalIO.from_portable(portable) == node


def test_localio_pickle_round_trips_a_path_outside_the_roots():
    # A handle outside the home/temp roots keeps a full file:// portable form (lossless
    # fallback) and still pickles to an equal handle.
    node = LocalIO(os.path.join(tempfile.gettempdir(), "ygg_pickle_probe.bin"))
    revived = pickle.loads(pickle.dumps(node))
    assert revived == node
    # from_portable is the exact inverse of to_portable_str in this environment.
    assert LocalIO.from_portable(node.to_portable_str()) == node


# -------------------------------------------------------------------------------------
# All native numeric widths on LocalIO + Mmap, LocalIO.load / move_into, module open
# -------------------------------------------------------------------------------------


def test_localio_scalar_all_widths(root):
    node = root.join("scalars.bin")
    node.pwrite_i8(0, -100)
    node.pwrite_u8(1, 250)
    node.pwrite_i16(2, -30000)
    node.pwrite_u16(4, 65000)
    node.pwrite_u32(6, 4_000_000_000)
    node.pwrite_u64(10, 10**19)
    node.pwrite_i128(18, -(2**126))
    node.pwrite_u128(34, 2**127)
    node.pwrite_f32(50, 1.5)
    node.pwrite_f64(54, 2.25)
    assert node.pread_i8(0) == -100
    assert node.pread_u8(1) == 250
    assert node.pread_i16(2) == -30000
    assert node.pread_u16(4) == 65000
    assert node.pread_u32(6) == 4_000_000_000
    assert node.pread_u64(10) == 10**19
    assert node.pread_i128(18) == -(2**126)
    assert node.pread_u128(34) == 2**127
    assert node.pread_f32(50) == pytest.approx(1.5)
    assert node.pread_f64(54) == 2.25
    node.close()


def test_localio_bulk_arrays_and_cursor_typed(root):
    node = root.join("bulk.bin")
    node.pwrite_i8_array(0, [-1, 2, -3])
    node.pwrite_i16_array(3, [1000, -2000])
    node.pwrite_i128_array(7, [2**100])
    node.pwrite_u128_array(23, [2**120])
    assert node.pread_i8_array(0, 3) == [-1, 2, -3]
    assert node.pread_i16_array(3, 2) == [1000, -2000]
    assert node.pread_i128_array(7, 1) == [2**100]
    assert node.pread_u128_array(23, 1) == [2**120]
    node.pwrite_i8_repeat(0, 5, 4)
    assert node.pread_i8_array(0, 4) == [5, 5, 5, 5]
    # cursor typed read/write
    cnode = root.join("cursor.bin")
    cnode.write_i128(-(2**100))
    cnode.write_u16(40000)
    cnode.rewind()
    assert cnode.read_i128() == -(2**100)
    assert cnode.read_u16() == 40000
    node.close()
    cnode.close()


def test_localio_load_eager_maps(root):
    node = root.join("loadme.bin")
    node.pwrite_utf8(0, "eager")
    node.close()  # release the write-mapping
    reopened = root.join("loadme.bin")
    assert not reopened.is_mapped  # lazy until loaded
    reopened.load()
    assert reopened.is_mapped
    assert reopened.pread_utf8(0, 5) == "eager"
    reopened.load()  # idempotent
    assert reopened.is_mapped
    reopened.close()


def test_localio_load_missing_is_noop(root):
    missing = root.join("nope.bin")
    missing.load()  # nothing to map yet — no error
    assert not missing.is_mapped


def test_localio_move_into_removes_source(root):
    src = root.join("src.bin")
    src.pwrite_utf8(0, "moved data")
    src.close()
    dst = root.join("dst.bin")
    n = src.move_into(dst)
    src.close()
    assert n == 10
    assert not src.exists()  # the source file is removed
    assert dst.pread_utf8(0, 10) == "moved data"
    dst.close()


def test_mmap_scalar_bulk_cursor_all_widths(root):
    path = root.join("m.bin").path
    with Mmap.create(path) as m:
        m.pwrite_i128(0, -(2**120))
        m.pwrite_u128(16, 2**127)
        m.pwrite_f32(32, 3.5)
        assert m.pread_i128(0) == -(2**120)
        assert m.pread_u128(16) == 2**127
        assert m.pread_f32(32) == pytest.approx(3.5)
        m.pwrite_i8_array(36, [-4, -5])
        assert m.pread_i8_array(36, 2) == [-4, -5]
        m.pwrite_u128_repeat(40, 2**100, 2)
        assert m.pread_u128_array(40, 2) == [2**100, 2**100]
        m.rewind()
        assert m.read_i128() == -(2**120)


def test_mmap_move_into_removes_source_file(root):
    src_path = root.join("ms.bin").path
    dst_path = root.join("md.bin").path
    src = Mmap.create(src_path)
    src.pwrite_utf8(0, "xyz")
    dst = Mmap.create(dst_path)
    n = src.move_into(dst)
    assert n == 3
    assert dst.pread_utf8(0, 3) == "xyz"
    dst.close()
    src.close()


def test_open_path_and_file_uri_and_pathlike_wrap_localio(root):
    import pathlib

    import yggdryl

    path = root.join("opened.bin").path
    assert isinstance(yggdryl.open(path), LocalIO)
    assert isinstance(yggdryl.open(pathlib.Path(path)), LocalIO)
    assert isinstance(yggdryl.open(Uri.from_path(path)), LocalIO)
    opened = yggdryl.open(path)
    opened.pwrite_utf8(0, "via open")
    opened.close()
    assert LocalIO(path).pread_utf8(0, 8) == "via open"

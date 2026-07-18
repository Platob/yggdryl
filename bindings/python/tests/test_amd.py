"""Tests for the ``yggdryl.amd`` AMD Radeon device-memory family and the ``yggdryl.io``
``MemoryInfo``.

Mirrors ``crates/yggdryl-core/src/io/amd`` and ``io/meminfo.rs`` on the Python surface: the
``detect`` probe (``None`` off a real Radeon adapter, else an ``AmdDevice``), the ``AmdDevice``
value descriptor (name, total memory, ``is_present``, ``memory_info``), the ``AmdHeap``
device-memory heap (``upload`` / ``download`` transfer plus the ``IOBase`` byte + bulk surface and
the auto-dispatched compute ops), and the ``MemoryInfo`` capacity snapshot (total / available /
used / usage_ratio, the ``system`` host-RAM route, the ``unknown`` sentinel, and its value
dunders).

The CPU device-memory type is ``yggdryl.memory.Heap`` (a ``Heap`` is simply the CPU heap), so no
separate CPU buffer class is exposed on ``yggdryl.amd``.
"""

import pickle
import statistics
import struct

import pytest

import yggdryl.amd
import yggdryl.io
from yggdryl.amd import AmdDevice, AmdHeap, detect
from yggdryl.io import MemoryInfo


# -------------------------------------------------------------------------------------
# Module surface
# -------------------------------------------------------------------------------------


def test_module_surface():
    for cls in (AmdDevice, AmdHeap):
        assert cls.__module__ == "yggdryl.amd"
        assert hasattr(yggdryl.amd, cls.__name__)
    assert callable(detect)
    assert MemoryInfo.__module__ == "yggdryl.io"
    assert hasattr(yggdryl.io, "MemoryInfo")
    # The removed by-architecture API is gone; the CPU device-memory type is yggdryl.memory.Heap.
    assert not hasattr(yggdryl.amd, "CpuHeap")
    assert not hasattr(yggdryl.amd, "GpuDevice")
    assert not hasattr(yggdryl.amd, "AmdBuffer")
    assert not hasattr(yggdryl.amd, "available_devices")
    assert not hasattr(yggdryl.amd, "default_device")


# -------------------------------------------------------------------------------------
# MemoryInfo — capacity snapshot value type
# -------------------------------------------------------------------------------------


def test_memory_info_fields_and_derived():
    info = MemoryInfo(1000, 250)
    assert info.total() == 1000
    assert info.available() == 250
    assert info.used() == 750  # total - available
    assert info.usage_ratio() == pytest.approx(0.75)
    assert not info.is_unknown()


def test_memory_info_available_clamped_to_total():
    # available is clamped to total (a backend can never report more free than it has).
    info = MemoryInfo(100, 500)
    assert info.available() == 100
    assert info.used() == 0


def test_memory_info_unknown_sentinel():
    unknown = MemoryInfo.unknown()
    assert unknown.is_unknown()
    assert unknown.total() == 0
    assert unknown.available() == 0
    assert unknown.usage_ratio() == 0.0  # a zero total reports 0.0, never divides
    assert unknown == MemoryInfo(0, 0)


def test_memory_info_system_reports_sane_totals():
    sys = MemoryInfo.system()
    assert isinstance(sys, MemoryInfo)
    # The API is total on every platform; on a real host RAM is reported.
    assert sys.total() >= sys.available()


def test_memory_info_value_dunders():
    a = MemoryInfo(1000, 250)
    b = MemoryInfo(1000, 250)
    c = MemoryInfo(1000, 300)
    assert a == b and a != c
    assert hash(a) == hash(b)  # equal values hash equal (immutable, hashable)
    assert {a, b, c} == {a, c}  # keys a set
    assert repr(a) == "MemoryInfo(total=1000, available=250)"
    assert pickle.loads(pickle.dumps(a)) == a  # pickles through (total, available)


# -------------------------------------------------------------------------------------
# detect / AmdDevice — the hardware probe + device descriptor
# -------------------------------------------------------------------------------------


def test_detect_is_none_or_an_amd_device():
    adapter = detect()
    # None on a machine with no AMD Radeon adapter; a present AmdDevice otherwise.
    assert adapter is None or isinstance(adapter, AmdDevice)
    if adapter is not None:
        assert adapter.is_present()
        assert isinstance(adapter.name(), str) and adapter.name() != ""
        assert adapter.total_memory() >= 0


def test_amd_device_descriptor_and_memory_info():
    dev = AmdHeap().device()  # the detected adapter, else the host fallback
    assert isinstance(dev, AmdDevice)
    assert isinstance(dev.name(), str) and dev.name() != ""
    assert isinstance(dev.is_present(), bool)
    # detect() agreeing with the heap's device present-ness.
    assert dev.is_present() == (detect() is not None)
    info = dev.memory_info()
    assert isinstance(info, MemoryInfo)
    assert info.total() >= info.available()  # total >= available within a device


def test_amd_device_value_dunders():
    a = AmdHeap().device()
    b = AmdHeap().device()
    assert a == b
    assert hash(a) == hash(b)  # equal values hash equal
    assert {a, b} == {a}  # keys a set
    r = repr(a)
    assert r.startswith("AmdDevice(")
    assert "name=" in r and "total_memory=" in r and "present=" in r


# -------------------------------------------------------------------------------------
# AmdHeap — device memory that speaks the IOBase byte contract
# -------------------------------------------------------------------------------------


def test_amd_heap_upload_download_round_trip():
    buf = AmdHeap()
    assert buf.is_empty()
    assert len(buf) == 0
    assert not buf  # __bool__ over an empty heap
    buf.upload(b"radeon payload")
    assert buf.byte_size() == 14
    assert len(buf) == 14
    assert buf.download_vec() == b"radeon payload"
    assert bytes(buf) == b"radeon payload"
    assert buf.to_bytes() == b"radeon payload"
    assert buf.download(6) == b"radeon"  # up to length, from the start
    assert buf.download(1000) == b"radeon payload"  # clamped to what remains


def test_amd_heap_from_host_and_with_capacity():
    buf = AmdHeap.from_host(b"seed")
    assert buf.download_vec() == b"seed"
    empty = AmdHeap.with_capacity(4096)
    assert empty.is_empty()
    empty.upload(b"x")
    assert empty.download_vec() == b"x"


def test_amd_heap_accepts_bytearray_upload():
    buf = AmdHeap()
    buf.upload(bytearray(b"mutable"))  # a bytearray borrows just like bytes
    assert buf.download_vec() == b"mutable"


def test_amd_heap_positioned_byte_surface():
    buf = AmdHeap()
    assert buf.pwrite_byte_array(0, b"abc") == 3
    assert buf.pwrite_byte_array(5, b"Z") == 1  # past the end zero-fills the gap
    assert buf.pread_byte_array(0, 99) == b"abc\x00\x00Z"
    assert buf.pread_byte_array(6, 4) == b""  # at the end


def test_amd_heap_bulk_vectorized_ops():
    buf = AmdHeap()
    buf.upload(b"radeon payload")
    buf.pwrite_i32_array(16, [1, -2, 3])  # a vectorized bulk op on device memory
    assert buf.pread_i32_array(16, 3) == [1, -2, 3]
    buf.pwrite_i64_array(32, [1 << 40])
    assert buf.pread_i64_array(32, 1) == [1 << 40]
    # The head content is untouched by the positioned bulk writes past it.
    assert buf.download_vec()[:14] == b"radeon payload"
    # The bulk-read bounds are checked before the result list is allocated.
    with pytest.raises(ValueError, match="unexpected end of data"):
        buf.pread_i32_array(0, 2_000_000_000)


def test_amd_heap_device_present_and_memory_info():
    buf = AmdHeap()
    dev = buf.device()
    assert isinstance(dev, AmdDevice)
    assert isinstance(dev.is_present(), bool)  # True when detected, else the host fallback
    # memory_info is the convenience for device().memory_info().
    assert buf.memory_info() == dev.memory_info()
    info = buf.memory_info()
    assert isinstance(info, MemoryInfo)
    assert info.total() >= info.available()


def test_amd_heap_context_manager_and_repr():
    with AmdHeap.from_host(b"ctx") as buf:
        assert buf.download_vec() == b"ctx"
    assert repr(AmdHeap.from_host(b"abc")).startswith("AmdHeap(<3 bytes on ")


# -------------------------------------------------------------------------------------
# Compute — auto-dispatched aggregations, threshold filter, device-aware copy
# -------------------------------------------------------------------------------------


def test_amd_heap_compute_i32_aggregations_and_filter():
    buf = AmdHeap()
    buf.pwrite_i32_array(0, [4, 8, 15, 16, 23, 42])
    assert buf.sum_i32(0, 6) == 108
    assert buf.min_i32(0, 6) == 4
    assert buf.max_i32(0, 6) == 42
    assert buf.mean_i32(0, 6) == pytest.approx(18.0)
    assert buf.first_i32(0, 6) == 4
    assert buf.last_i32(0, 6) == 42
    # Population standard deviation (matches the core's std_*).
    assert buf.std_i32(0, 6) == pytest.approx(statistics.pstdev([4, 8, 15, 16, 23, 42]))
    # count_ge is a threshold filter: how many values are >= threshold.
    assert buf.count_ge_i32(0, 6, 16) == 3
    assert buf.count_ge_i32(0, 6, 100) == 0
    # An empty span reduces to the None / 0 identities.
    assert buf.min_i32(0, 0) is None
    assert buf.max_i32(0, 0) is None
    assert buf.mean_i32(0, 0) is None
    assert buf.first_i32(0, 0) is None
    assert buf.last_i32(0, 0) is None
    assert buf.sum_i32(0, 0) == 0


def test_amd_heap_compute_i64_aggregations():
    buf = AmdHeap()
    buf.pwrite_i64_array(0, [1 << 40, 2 << 40, 3 << 40])
    # The i64 accumulator is a 128-bit int on the core side; Python's int carries it.
    assert buf.sum_i64(0, 3) == (1 << 40) + (2 << 40) + (3 << 40)
    assert buf.min_i64(0, 3) == 1 << 40
    assert buf.max_i64(0, 3) == 3 << 40
    assert buf.mean_i64(0, 3) == pytest.approx(2 << 40)
    assert buf.count_ge_i64(0, 3, 2 << 40) == 2


def test_amd_heap_compute_f64_large_array_streams():
    # More than one compute stack chunk (1024 elements) exercises the streaming reduction.
    values = [float(i) for i in range(1025)]  # 0.0 .. 1024.0
    buf = AmdHeap()
    buf.pwrite_byte_array(0, struct.pack(f"<{len(values)}d", *values))
    n = len(values)
    assert buf.sum_f64(0, n) == pytest.approx(sum(values))
    assert buf.min_f64(0, n) == pytest.approx(0.0)
    assert buf.max_f64(0, n) == pytest.approx(1024.0)
    assert buf.mean_f64(0, n) == pytest.approx(sum(values) / n)
    assert buf.count_ge_f64(0, n, 512.0) == sum(1 for v in values if v >= 512.0)


def test_amd_heap_compute_f32_aggregations():
    values = [1.5, 2.5, 3.5]
    buf = AmdHeap()
    buf.pwrite_byte_array(0, struct.pack(f"<{len(values)}f", *values))
    assert buf.sum_f32(0, 3) == pytest.approx(7.5)
    assert buf.min_f32(0, 3) == pytest.approx(1.5)
    assert buf.max_f32(0, 3) == pytest.approx(3.5)
    assert buf.mean_f32(0, 3) == pytest.approx(2.5)
    assert buf.count_ge_f32(0, 3, 2.5) == 2


def test_amd_heap_min_max_ignore_nan_order():
    # min/max are order-independent around a NaN: the numeric extreme wins wherever the NaN sits.
    for values in ([float("nan"), 2.0, 5.0, 1.0], [2.0, 5.0, 1.0, float("nan")]):
        buf = AmdHeap()
        buf.pwrite_byte_array(0, struct.pack(f"<{len(values)}d", *values))
        assert buf.min_f64(0, len(values)) == pytest.approx(1.0)
        assert buf.max_f64(0, len(values)) == pytest.approx(5.0)


def test_amd_heap_compute_copy_into_round_trip():
    src = AmdHeap.from_host(b"device-to-device payload")
    dst = AmdHeap()
    written = src.compute_copy_into(dst)
    assert written == src.byte_size() == 24
    assert dst.download_vec() == b"device-to-device payload"


def test_amd_heap_compute_backend_token_is_cpu_for_small_work():
    buf = AmdHeap.from_host(b"small")
    # A tiny workload never amortizes a host<->device transfer, so it runs on the CPU arm.
    assert buf.compute_backend(8) == "cpu"

"""Tests for the ``yggdryl.gpu`` device-memory layer and the ``yggdryl.io`` ``MemoryInfo``.

Mirrors ``crates/yggdryl-core/src/io/gpu`` and ``io/meminfo.rs`` on the Python surface: the
``available_devices`` / ``default_device`` probe (adapting to the hardware present, always
including a CPU device), the ``GpuDevice`` value descriptor (backend token, name, total
memory, ``memory_info``), the ``AmdBuffer`` device-memory buffer (``upload`` / ``download``
transfer plus the ``IOBase`` byte + bulk surface), and the ``MemoryInfo`` capacity snapshot
(total / available / used / usage_ratio, the ``system`` host-RAM route, the ``unknown``
sentinel, and its value dunders).

The CPU device-memory type is ``yggdryl.memory.Heap`` (the core aliases ``CpuHeap = Heap``),
so no separate CPU buffer class is exposed on ``yggdryl.gpu``.
"""

import pickle
import struct

import pytest

import yggdryl.gpu
import yggdryl.io
from yggdryl.gpu import AmdBuffer, GpuDevice, available_devices, default_device
from yggdryl.io import MemoryInfo


# -------------------------------------------------------------------------------------
# Module surface
# -------------------------------------------------------------------------------------


def test_module_surface():
    for cls in (GpuDevice, AmdBuffer):
        assert cls.__module__ == "yggdryl.gpu"
        assert hasattr(yggdryl.gpu, cls.__name__)
    assert MemoryInfo.__module__ == "yggdryl.io"
    assert hasattr(yggdryl.io, "MemoryInfo")
    # The CPU device-memory type is yggdryl.memory.Heap — there is no CpuHeap on yggdryl.gpu.
    assert not hasattr(yggdryl.gpu, "CpuHeap")


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
# available_devices / default_device — the by-architecture probe
# -------------------------------------------------------------------------------------


def test_available_devices_non_empty_and_has_cpu():
    devices = available_devices()
    assert isinstance(devices, list)
    assert len(devices) >= 1  # never empty — the CPU device is always appended
    assert all(isinstance(d, GpuDevice) for d in devices)
    # A CPU device is always present (the always-available fallback).
    cpus = [d for d in devices if d.is_cpu()]
    assert len(cpus) >= 1
    assert cpus[0].backend() == "cpu"


def test_default_device_is_a_gpu_or_the_cpu_fallback():
    dev = default_device()
    assert isinstance(dev, GpuDevice)
    assert dev.backend() in ("cpu", "amd", "cuda")


def test_gpu_device_descriptor_and_memory_info():
    cpu = next(d for d in available_devices() if d.is_cpu())
    assert cpu.backend() == "cpu"
    assert isinstance(cpu.name(), str) and cpu.name() != ""
    assert cpu.total_memory() >= cpu.memory_info().available()
    info = cpu.memory_info()
    assert isinstance(info, MemoryInfo)
    assert info.total() >= info.available()  # total >= available within a device


def test_gpu_device_value_dunders():
    cpu_a = next(d for d in available_devices() if d.is_cpu())
    cpu_b = next(d for d in available_devices() if d.is_cpu())
    assert cpu_a == cpu_b
    assert hash(cpu_a) == hash(cpu_b)
    assert "cpu" in repr(cpu_a)


# -------------------------------------------------------------------------------------
# AmdBuffer — device memory that speaks the IOBase byte contract
# -------------------------------------------------------------------------------------


def test_amd_buffer_upload_download_round_trip():
    buf = AmdBuffer()
    assert buf.is_empty()
    assert len(buf) == 0
    assert not buf  # __bool__ over an empty buffer
    buf.upload(b"radeon payload")
    assert buf.byte_size() == 14
    assert len(buf) == 14
    assert buf.download_vec() == b"radeon payload"
    assert bytes(buf) == b"radeon payload"
    assert buf.to_bytes() == b"radeon payload"
    assert buf.download(6) == b"radeon"  # up to length, from the start
    assert buf.download(1000) == b"radeon payload"  # clamped to what remains


def test_amd_buffer_from_host_and_with_capacity():
    buf = AmdBuffer.from_host(b"seed")
    assert buf.download_vec() == b"seed"
    empty = AmdBuffer.with_capacity(4096)
    assert empty.is_empty()
    empty.upload(b"x")
    assert empty.download_vec() == b"x"


def test_amd_buffer_positioned_byte_surface():
    buf = AmdBuffer()
    assert buf.pwrite_byte_array(0, b"abc") == 3
    assert buf.pwrite_byte_array(5, b"Z") == 1  # past the end zero-fills the gap
    assert buf.pread_byte_array(0, 99) == b"abc\x00\x00Z"
    assert buf.pread_byte_array(6, 4) == b""  # at the end


def test_amd_buffer_bulk_vectorized_ops():
    buf = AmdBuffer()
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


def test_amd_buffer_device_backend_is_amd_or_cpu():
    buf = AmdBuffer()
    dev = buf.device()
    assert isinstance(dev, GpuDevice)
    assert dev.backend() in ("amd", "cpu")  # amd when detected, else the cpu fallback
    # memory_info is the convenience for device().memory_info().
    assert buf.memory_info() == dev.memory_info()
    info = buf.memory_info()
    assert isinstance(info, MemoryInfo)
    assert info.total() >= info.available()


def test_amd_buffer_context_manager_and_repr():
    with AmdBuffer.from_host(b"ctx") as buf:
        assert buf.download_vec() == b"ctx"
    assert repr(AmdBuffer.from_host(b"abc")).startswith("AmdBuffer(<3 bytes on ")


# -------------------------------------------------------------------------------------
# Compute — auto-dispatched aggregations, threshold filter, device-aware copy
# -------------------------------------------------------------------------------------


def test_amd_buffer_compute_i32_aggregations_and_filter():
    buf = AmdBuffer()
    buf.pwrite_i32_array(0, [4, 8, 15, 16, 23, 42])
    assert buf.sum_i32(0, 6) == 108
    assert buf.min_i32(0, 6) == 4
    assert buf.max_i32(0, 6) == 42
    assert buf.mean_i32(0, 6) == pytest.approx(18.0)
    # count_ge is a threshold filter: how many values are >= threshold.
    assert buf.count_ge_i32(0, 6, 16) == 3
    assert buf.count_ge_i32(0, 6, 100) == 0
    # An empty span reduces to the None / 0 identities.
    assert buf.min_i32(0, 0) is None
    assert buf.max_i32(0, 0) is None
    assert buf.mean_i32(0, 0) is None
    assert buf.sum_i32(0, 0) == 0


def test_amd_buffer_compute_i64_aggregations():
    buf = AmdBuffer()
    buf.pwrite_i64_array(0, [1 << 40, 2 << 40, 3 << 40])
    # The i64 accumulator is a 128-bit int on the core side; Python's int carries it.
    assert buf.sum_i64(0, 3) == (1 << 40) + (2 << 40) + (3 << 40)
    assert buf.min_i64(0, 3) == 1 << 40
    assert buf.max_i64(0, 3) == 3 << 40
    assert buf.count_ge_i64(0, 3, 2 << 40) == 2


def test_amd_buffer_compute_f64_large_array_streams():
    # More than one compute stack chunk (1024 elements) exercises the streaming reduction.
    values = [float(i) for i in range(1025)]  # 0.0 .. 1024.0
    buf = AmdBuffer()
    buf.pwrite_byte_array(0, struct.pack(f"<{len(values)}d", *values))
    n = len(values)
    assert buf.sum_f64(0, n) == pytest.approx(sum(values))
    assert buf.min_f64(0, n) == pytest.approx(0.0)
    assert buf.max_f64(0, n) == pytest.approx(1024.0)
    assert buf.mean_f64(0, n) == pytest.approx(sum(values) / n)
    assert buf.count_ge_f64(0, n, 512.0) == sum(1 for v in values if v >= 512.0)


def test_amd_buffer_compute_f32_aggregations():
    values = [1.5, 2.5, 3.5]
    buf = AmdBuffer()
    buf.pwrite_byte_array(0, struct.pack(f"<{len(values)}f", *values))
    assert buf.sum_f32(0, 3) == pytest.approx(7.5)
    assert buf.min_f32(0, 3) == pytest.approx(1.5)
    assert buf.max_f32(0, 3) == pytest.approx(3.5)
    assert buf.mean_f32(0, 3) == pytest.approx(2.5)
    assert buf.count_ge_f32(0, 3, 2.5) == 2


def test_amd_buffer_compute_copy_into_round_trip():
    src = AmdBuffer.from_host(b"device-to-device payload")
    dst = AmdBuffer()
    written = src.compute_copy_into(dst)
    assert written == src.byte_size()
    assert dst.download_vec() == b"device-to-device payload"


def test_amd_buffer_compute_backend_token_is_cpu_for_small_work():
    buf = AmdBuffer.from_host(b"small")
    # A tiny workload never amortizes a host<->device transfer, so it runs on the CPU arm.
    assert buf.compute_backend(8) == "cpu"

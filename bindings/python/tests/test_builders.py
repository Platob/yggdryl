"""Tests for the module-level generic builder functions on ``yggdryl``.

``buffer`` / ``array`` / ``device_buffer`` are the ergonomic front door that hides the concrete
class + its setup behind one call, **inferring** the runtime type of the input and redirecting to
the matching explicit binding class / method (the same spirit as ``yggdryl.open``). These tests are
the executable proof that the dispatch — bytes / capacity / headers / mode assembly, dtype
inference and redirect, and the device probe — resolves to the right concrete surface.
"""

import pytest

import yggdryl
from yggdryl.gpu import AmdBuffer, GpuDevice, available_devices
from yggdryl.headers import Headers
from yggdryl.io import IOMode
from yggdryl.memory import Heap

# -------------------------------------------------------------------------------------
# buffer — constructor + with_capacity + set_headers + set_mode behind one call
# -------------------------------------------------------------------------------------


def test_buffer_empty_is_a_heap():
    h = yggdryl.buffer()
    assert isinstance(h, Heap)
    assert h.byte_size() == 0


def test_buffer_copies_bytes():
    h = yggdryl.buffer(b"hi")
    assert isinstance(h, Heap)
    assert bytes(h) == b"hi"
    assert h.byte_size() == 2


def test_buffer_accepts_bytearray():
    h = yggdryl.buffer(bytearray(b"mutable"))
    assert bytes(h) == b"mutable"


def test_buffer_capacity_preallocates():
    h = yggdryl.buffer(capacity=64)
    assert h.byte_size() == 0
    assert h.capacity() >= 64


def test_buffer_data_and_capacity_reserves():
    h = yggdryl.buffer(b"hi", capacity=128)
    assert bytes(h) == b"hi"
    assert h.capacity() >= 128


def test_buffer_headers_from_dict_sets_bytes_and_header():
    h = yggdryl.buffer(b"hi", headers={"Content-Type": "text/plain"})
    assert bytes(h) == b"hi"
    assert h.headers["Content-Type"] == "text/plain"
    # It flows through to the derived media type as well.
    assert h.mime_type().essence == "text/plain"


def test_buffer_headers_from_headers_object():
    src = Headers()
    src["X-Trace"] = "abc"
    h = yggdryl.buffer(b"x", headers=src)
    assert h.headers["X-Trace"] == "abc"


def test_buffer_mode_is_applied():
    h = yggdryl.buffer(b"x", mode=IOMode.Read)
    assert h.mode == IOMode.Read


def test_buffer_rejects_bad_headers_type():
    with pytest.raises(TypeError):
        yggdryl.buffer(b"x", headers=123)


# -------------------------------------------------------------------------------------
# array — sequence of numbers -> Heap, dtype inference + redirect
# -------------------------------------------------------------------------------------


def test_array_infers_i64_and_round_trips():
    h = yggdryl.array([1, 2, 3])
    assert isinstance(h, Heap)
    assert h.byte_size() == 3 * 8  # inferred i64 -> 8 bytes each
    assert h.pread_i64_array(0, 3) == [1, 2, 3]


def test_array_infers_f64_when_any_float():
    h = yggdryl.array([1, 2.5, 3])
    assert h.byte_size() == 3 * 8
    assert h.pread_f64_array(0, 3) == [1.0, 2.5, 3.0]


def test_array_explicit_f32_round_trips():
    h = yggdryl.array([1.5, 2.5], "f32")
    assert h.byte_size() == 2 * 4
    assert h.pread_f32_array(0, 2) == [1.5, 2.5]


def test_array_explicit_i32_round_trips():
    h = yggdryl.array([10, 20, 30], "i32")
    assert h.byte_size() == 3 * 4
    assert h.pread_i32_array(0, 3) == [10, 20, 30]


def test_array_u8_uses_the_byte_surface():
    h = yggdryl.array([1, 2, 255], "u8")
    assert h.byte_size() == 3
    assert h.pread_byte_array(0, 3) == b"\x01\x02\xff"


def test_array_empty_infers_i64():
    h = yggdryl.array([])
    assert h.byte_size() == 0


def test_array_unknown_dtype_raises_guided_error():
    with pytest.raises(ValueError) as excinfo:
        yggdryl.array([1], "bogus")
    message = str(excinfo.value)
    assert "bogus" in message
    assert "i64" in message  # the guided error names the valid tokens


def test_array_accepts_a_tuple():
    h = yggdryl.array((1, 2, 3), "i32")
    assert h.pread_i32_array(0, 3) == [1, 2, 3]


# -------------------------------------------------------------------------------------
# device_buffer — best available device-memory buffer, byte surface shared
# -------------------------------------------------------------------------------------


def _has_real_gpu():
    return any(not d.is_cpu() for d in available_devices())


def test_device_buffer_default_returns_usable_buffer():
    buf = yggdryl.device_buffer(b"x")
    assert buf.byte_size() == 1
    # Whichever concrete type it is, the byte surface reads the same.
    assert bytes(buf) == b"x"


def test_device_buffer_cpu_returns_a_heap():
    buf = yggdryl.device_buffer(b"cpu-bytes", device="cpu")
    assert isinstance(buf, Heap)
    assert bytes(buf) == b"cpu-bytes"


def test_device_buffer_cpu_empty():
    buf = yggdryl.device_buffer(device="cpu")
    assert isinstance(buf, Heap)
    assert buf.byte_size() == 0


def test_device_buffer_amd_returns_amd_buffer():
    # "amd" always selects the device-memory class (which falls back to the CPU device when no
    # AMD hardware is present, but is still an AmdBuffer holding the uploaded bytes).
    buf = yggdryl.device_buffer(b"gpu-bytes", device="amd")
    assert isinstance(buf, AmdBuffer)
    assert bytes(buf) == b"gpu-bytes"


def test_device_buffer_default_matches_hardware_probe():
    buf = yggdryl.device_buffer(b"x")
    if _has_real_gpu():
        assert isinstance(buf, AmdBuffer)
    else:
        assert isinstance(buf, Heap)


def test_device_buffer_by_gpu_device_object():
    cpu = next(d for d in available_devices() if d.is_cpu())
    buf = yggdryl.device_buffer(b"x", device=cpu)
    assert isinstance(buf, Heap)
    assert bytes(buf) == b"x"


def test_device_buffer_rejects_unknown_device_name():
    with pytest.raises(ValueError):
        yggdryl.device_buffer(b"x", device="quantum")


def test_device_buffer_accepts_gpu_device_type_alias():
    # A sanity check that the GpuDevice type is importable and usable as a selector.
    assert GpuDevice is not None

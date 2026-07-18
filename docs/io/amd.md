# The AMD device-memory family

`amd` is yggdryl's **AMD Radeon device-memory family** — a concrete infrastructure family, a sibling
of [local](local.md) and [memory](memory.md). Where `memory` *is* the CPU byte layer (a `Heap` is
simply the CPU heap), `amd` adds device memory over a detected AMD Radeon adapter that **is an
`IOBase`**: `AmdHeap`, `AmdCursor`, and `AmdSlice` read, write, and run the same vectorized bulk
numeric kernels a `Heap` does, plus a host↔device `upload` / `download` transfer. It **adapts to the
hardware present** — `detect()` finds a real Radeon adapter and reports its VRAM, and an `AmdHeap`
falls back to host memory when none is installed, so the family is usable on every OS. It sits behind
the `amd` cargo feature; the Python and Node extensions build with it, so `yggdryl.amd` is always
available there.

Because a device heap is just another source, everything about the byte contract carries over —
positioned typed access, the auto-vectorized bulk arrays, capacity — and the only new surface is the
host↔device transfer and the device probe. A quick end-to-end: allocate a heap, upload host bytes, run
a vectorized bulk op on the device, and download the result.

=== "Python"

    ```python
    from yggdryl.amd import AmdHeap, detect

    # Adapt to the hardware: detect() is not None only on a real Radeon adapter.
    adapter = detect()                              # AmdDevice | None

    # Device memory that IS an IOBase — allocate, upload, run a bulk op, download.
    buf = AmdHeap.from_host(b"device bytes")
    buf.pwrite_i32_array(16, [1, -2, 3])            # vectorized bulk op, on device memory
    assert buf.pread_i32_array(16, 3) == [1, -2, 3]
    assert buf.download_vec()[:12] == b"device bytes"
    assert buf.device().is_present() == (adapter is not None)
    ```

=== "Node"

    ```js
    const { AmdHeap, detect } = require('yggdryl').amd

    // Adapt to the hardware: detect() is null unless a real Radeon adapter is present.
    const adapter = detect()                        // AmdDevice | null

    // Device memory that IS an IOBase — allocate, upload, run a bulk op, download.
    const buf = AmdHeap.fromHost(Buffer.from('device bytes'))
    buf.pwriteI32Array(16, [1, -2, 3])              // vectorized bulk op, on device memory
    console.assert(buf.preadI32Array(16, 3).join() === '1,-2,3')
    console.assert(buf.downloadVec().slice(0, 12).toString() === 'device bytes')
    console.assert(buf.device().isPresent() === (adapter !== null))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::amd::{detect, AmdHeap, AmdMemory};
    use yggdryl_core::io::memory::IOBase;

    // Adapt to the hardware: detect() is Some only on a real Radeon adapter.
    let adapter = detect();

    // Device memory that IS an IOBase — allocate, upload, run a bulk op, download.
    let mut buf = AmdHeap::from_host(b"device bytes");
    buf.pwrite_i32_array(16, &[1, -2, 3]).unwrap(); // vectorized bulk op, on device memory
    let mut back = [0i32; 3];
    buf.pread_i32_array(16, &mut back).unwrap();
    assert_eq!(back, [1, -2, 3]);
    assert_eq!(&buf.download_vec()[..12], b"device bytes");
    assert_eq!(buf.device().is_present(), adapter.is_some());
    ```

## Detecting the device — `detect` / `AmdDevice`

`detect()` probes the OS for an AMD Radeon adapter (on Windows, the display-adapter registry class),
returning an `AmdDevice` with its name and VRAM, or nothing when none is present. An `AmdHeap` always
reports a `device()`: a detected adapter (`is_present()` true) or the host-memory fallback
(`is_present()` false, named `"no AMD device (host memory)"`). An `AmdDevice` is a plain **value
description** — `name()`, `total_memory()`, `is_present()`, and a live `memory_info()` capacity
snapshot (queried fresh, not baked in) — equal, hashable, and a map key.

=== "Python"

    ```python
    from yggdryl.amd import AmdHeap, detect

    adapter = detect()
    if adapter is not None:                         # only on a machine with a Radeon adapter
        assert adapter.is_present()
        assert isinstance(adapter.name(), str)
        assert adapter.total_memory() >= 0

    dev = AmdHeap().device()                        # the detected adapter, else the host fallback
    info = dev.memory_info()                        # a live capacity snapshot for the device
    assert info.total() >= info.available()
    ```

=== "Node"

    ```js
    const { AmdHeap, detect } = require('yggdryl').amd

    const adapter = detect()
    if (adapter !== null) {                         // only on a machine with a Radeon adapter
      console.assert(adapter.isPresent())
      console.assert(typeof adapter.name() === 'string')
      console.assert(adapter.totalMemory() >= 0)
    }

    const dev = new AmdHeap().device()              // the detected adapter, else the host fallback
    const info = dev.memoryInfo()                   // a live capacity snapshot for the device
    console.assert(info.total() >= info.available())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::amd::{detect, AmdHeap, AmdMemory};

    if let Some(adapter) = detect() {               // only on a machine with a Radeon adapter
        assert!(adapter.is_present());
        assert!(!adapter.name().is_empty());
    }

    let heap = AmdHeap::new();
    let dev = heap.device();                         // the detected adapter, else the host fallback
    let info = dev.memory_info();                    // a live capacity snapshot for the device
    assert!(info.total() >= info.available());
    ```

## `AmdHeap` — the device-memory heap

`AmdHeap` is device memory over the detected AMD Radeon adapter (or the host fallback when none is
present) that **is a full `IOBase`**: the positioned byte primitives and the vectorized bulk typed
array kernels run on it exactly as on a `Heap`. Construct it empty (`AmdHeap()`), pre-sized
(`with_capacity(n)`), or from host bytes (`from_host(bytes)`), then move data with `upload(bytes)`
(host → device, replacing the content) and `download(len)` / `download_vec()` (device → host);
`device()` reports which `AmdDevice` the memory lives on and `memory_info()` its capacity. The buffer
is correct and usable everywhere the feature builds; the resident store stages through host memory
today, with the device-side VRAM queue as the next increment behind a stable API.

=== "Python"

    ```python
    from yggdryl.amd import AmdHeap

    buf = AmdHeap()                                # empty; or AmdHeap.with_capacity(1024)
    buf.upload(b"radeon payload")                  # host -> device (replaces the content)
    assert buf.download(6) == b"radeon"            # download the first 6 bytes
    assert buf.download_vec() == b"radeon payload" # the whole buffer
    assert len(buf) == 14

    # It runs the byte + vectorized bulk numeric kernels, exactly like a Heap.
    wide = AmdHeap.from_host(b"")
    wide.pwrite_i64_array(0, [10, 20, 30])
    assert wide.pread_i64_array(0, 3) == [10, 20, 30]
    ```

=== "Node"

    ```js
    const { AmdHeap } = require('yggdryl').amd

    const buf = new AmdHeap()                      // empty; or AmdHeap.withCapacity(1024)
    buf.upload(Buffer.from('radeon payload'))      // host -> device (replaces the content)
    console.assert(buf.download(6).toString() === 'radeon')           // the first 6 bytes
    console.assert(buf.downloadVec().toString() === 'radeon payload')  // the whole buffer
    console.assert(buf.byteSize() === 14)

    // It runs the byte + vectorized bulk numeric kernels, exactly like a Heap.
    const wide = AmdHeap.fromHost(Buffer.alloc(0))
    wide.pwriteI64Array(0, [10, 20, 30])
    console.assert(wide.preadI64Array(0, 3).join() === '10,20,30')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::amd::{AmdHeap, AmdMemory};
    use yggdryl_core::io::memory::IOBase;

    let mut buf = AmdHeap::new();                  // empty; or AmdHeap::with_capacity(1024)
    buf.upload(b"radeon payload").unwrap();        // host -> device (replaces the content)
    assert_eq!(buf.download_vec(), b"radeon payload");
    assert_eq!(buf.byte_size(), 14);

    // It runs the byte + vectorized bulk numeric kernels, exactly like a Heap.
    let mut wide = AmdHeap::from_host(b"");
    wide.pwrite_i64_array(0, &[10, 20, 30]).unwrap();
    let mut back = [0i64; 3];
    wide.pread_i64_array(0, &mut back).unwrap();
    assert_eq!(back, [10, 20, 30]);
    ```

### `AmdCursor` / `AmdSlice` — the shared cursor & window

The AMD family reuses the crate's **one** cursor and window, instantiated over `AmdHeap`:
`AmdCursor` is a moving position (`read` / `write` / `seek`), `AmdSlice` a bounded window addressed
from its own `0`. Because `AmdHeap` forwards `as_bytes` to its contiguous store, both stay on the same
**zero-copy** fast path a CPU `Heap` cursor uses — one shared optimization across every memory type,
not a per-family reimplementation.

=== "Rust"

    ```rust
    use yggdryl_core::io::amd::{AmdCursor, AmdHeap, AmdSlice};
    use yggdryl_core::io::memory::IOBase;

    let mut cur = AmdCursor::new(AmdHeap::from_host(b"radeon payload"));
    let mut head = [0u8; 6];
    assert_eq!(cur.read(&mut head), 6);
    assert_eq!(&head, b"radeon");

    let win = AmdSlice::new(AmdHeap::from_host(b"radeon payload"), 7, 7).unwrap();
    assert_eq!(win.pread_vec(0, 7), b"payload");   // the window addressed from its own 0
    ```

## Auto-dispatched compute — aggregations, filters, copy

Beyond raw byte I/O, an `AmdHeap` runs **compute** operations — math **aggregations** (`sum` / `min` /
`max` / `mean` / `std` / `first` / `last`), a threshold **filter** (`count_ge` — how many values are
`>= threshold`), and a device-aware **copy** — that **auto-select GPU vs CPU**. The aggregations are
shared with every source (they live on the `Aggregate` trait, so an `AmdHeap` runs them exactly like a
`Heap`); what is AMD-specific is `compute_backend(n)`, which picks the **GPU** when the heap is on a
real adapter *and* the workload is large enough to amortize the host↔device transfer, else the **CPU**
(the dense, LLVM-vectorized reduction, streamed through a stack chunk with no heap allocation). Today
both arms run the CPU kernel — the GPU arm is the optimization **seam** a device kernel drops into, so
code written against these ops accelerates transparently when the kernels land.

=== "Python"

    ```python
    from yggdryl.amd import AmdHeap

    buf = AmdHeap()
    buf.pwrite_i32_array(0, [4, 8, 15, 16, 23, 42])
    assert buf.sum_i32(0, 6) == 108
    assert buf.min_i32(0, 6) == 4 and buf.max_i32(0, 6) == 42
    assert buf.mean_i32(0, 6) == 18.0
    assert buf.count_ge_i32(0, 6, 16) == 3      # a filter: how many >= 16
    assert buf.compute_backend(8) == "cpu"      # small workload stays on the CPU

    dst = AmdHeap()
    assert buf.compute_copy_into(dst) == 24     # device-aware copy, returns bytes moved
    ```

=== "Node"

    ```js
    const { AmdHeap } = require('yggdryl').amd

    const buf = new AmdHeap()
    buf.pwriteI32Array(0, [4, 8, 15, 16, 23, 42])
    console.assert(buf.sumI32(0, 6) === 108)
    console.assert(buf.minI32(0, 6) === 4 && buf.maxI32(0, 6) === 42)
    console.assert(buf.meanI32(0, 6) === 18)
    console.assert(buf.countGeI32(0, 6, 16) === 3)   // filter: how many >= 16
    console.assert(buf.computeBackend(8) === 'cpu')

    const dst = new AmdHeap()
    console.assert(buf.computeCopyInto(dst) === 24)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::amd::{AmdHeap, AmdMemory, ComputeBackend};
    use yggdryl_core::io::memory::{Aggregate, IOBase};

    let mut buf = AmdHeap::new();
    buf.pwrite_i32_array(0, &[4, 8, 15, 16, 23, 42]).unwrap();
    assert_eq!(buf.sum_i32(0, 6).unwrap(), 108);
    assert_eq!(buf.max_i32(0, 6).unwrap(), Some(42));
    assert_eq!(buf.mean_i32(0, 6).unwrap(), Some(18.0));
    assert_eq!(buf.count_ge_i32(0, 6, 16).unwrap(), 3); // filter: how many >= 16
    assert_eq!(buf.compute_backend(8), ComputeBackend::Cpu);

    let mut dst = AmdHeap::new();
    assert_eq!(buf.compute_copy_into(&mut dst).unwrap(), 24); // device-aware copy
    ```

## Capacity — `MemoryInfo`

`MemoryInfo` is the **one capacity type** across the whole library — the single answer to "how much
room is there?" whether the backend is host RAM, disk, or device VRAM. `MemoryInfo.system()` snapshots
physical RAM, a [`LocalIO`](local.md) reports its volume's free space through `memory_info()`, and an
`AmdDevice` reports its VRAM through `memory_info()` — all the same value. It carries a backend's
`total()` and `available()` (free) bytes, with `used()` and `usage_ratio()` derived from the pair, and
an `is_unknown()` sentinel (`0` / `0`) for a platform that cannot answer. It is an immutable value —
equal, hashable, and (in Python) picklable through its `(total, available)` pair.

=== "Python"

    ```python
    from yggdryl.io import MemoryInfo
    from yggdryl.amd import AmdHeap

    # One capacity type across host RAM, disk, and device VRAM.
    ram = MemoryInfo.system()                        # host RAM
    assert ram.total() >= ram.available()
    assert ram.used() == ram.total() - ram.available()
    assert 0.0 <= ram.usage_ratio() <= 1.0

    vram = AmdHeap().device().memory_info()          # the device's capacity snapshot
    assert vram.total() >= vram.available()

    # A plain immutable value — equal, hashable, and picklable through (total, available).
    assert MemoryInfo(1000, 400) == MemoryInfo(1000, 400)
    assert MemoryInfo(1000, 400).used() == 600
    assert MemoryInfo.unknown().is_unknown()
    ```

=== "Node"

    ```js
    const { MemoryInfo } = require('yggdryl').io
    const { AmdHeap } = require('yggdryl').amd

    // One capacity type across host RAM, disk, and device VRAM.
    const ram = MemoryInfo.system()                  // host RAM
    console.assert(ram.total() >= ram.available())
    console.assert(ram.used() === ram.total() - ram.available())
    console.assert(ram.usageRatio() >= 0 && ram.usageRatio() <= 1)

    const vram = new AmdHeap().device().memoryInfo() // the device's capacity snapshot
    console.assert(vram.total() >= vram.available())

    // A plain value — equatable, with used / usageRatio derived from the pair.
    console.assert(new MemoryInfo(1000, 400).equals(new MemoryInfo(1000, 400)))
    console.assert(new MemoryInfo(1000, 400).used() === 600)
    console.assert(MemoryInfo.unknown().isUnknown())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::amd::{AmdHeap, AmdMemory};
    use yggdryl_core::io::MemoryInfo;

    // One capacity type across host RAM, disk, and device VRAM.
    let ram = MemoryInfo::system();                  // host RAM
    assert!(ram.total() >= ram.available());
    assert_eq!(ram.used(), ram.total() - ram.available());
    assert!((0.0..=1.0).contains(&ram.usage_ratio()));

    let vram = AmdHeap::new().device().memory_info(); // the device's capacity snapshot
    assert!(vram.total() >= vram.available());

    // A plain value — Clone/Eq/Hash, with used / usage_ratio derived from the pair.
    assert_eq!(MemoryInfo::new(1000, 400), MemoryInfo::new(1000, 400));
    assert_eq!(MemoryInfo::new(1000, 400).used(), 600);
    assert!(MemoryInfo::unknown().is_unknown());
    ```

# The GPU device-memory layer

`gpu` is yggdryl's **device-memory layer** — memory that lives on a compute device (the host CPU,
an AMD Radeon, later a CUDA GPU) and **is an `IOBase`**: it reads, writes, and runs the same
vectorized bulk numeric kernels a `Heap` does, plus a host↔device `upload` / `download` transfer.
The layer is organized **by GPU architecture** — `cpu` (the portable backend), `amd` (AMD Radeon),
`cuda` (NVIDIA) — and it **adapts to the hardware present**: the device probe enumerates whatever
the enabled architectures detect, always ending with the CPU device so a target is never missing.
It sits behind the `gpu` cargo feature (with `gpu-amd` enabling the AMD backend); the Python and
Node extensions build with `gpu-amd`, so `yggdryl.gpu` is always available there.

Because a device buffer is just another source, everything you already know about the byte contract
carries over — positioned typed access, the auto-vectorized bulk arrays, capacity — and the only new
surface is the host↔device transfer and the by-architecture device probe. A quick end-to-end: pick
the default device, allocate a buffer, upload host bytes, run a vectorized bulk op on the device, and
download the result.

=== "Python"

    ```python
    from yggdryl.gpu import available_devices, default_device, AmdBuffer

    # Adapt to the hardware: the enabled architectures, always ending with the CPU device.
    assert len(available_devices()) >= 1
    dev = default_device()                          # the first detected GPU, else the CPU

    # Device memory that IS an IOBase — allocate, upload, run a bulk op, download.
    buf = AmdBuffer.from_host(b"device bytes")
    buf.pwrite_i32_array(16, [1, -2, 3])            # vectorized bulk op, on device memory
    assert buf.pread_i32_array(16, 3) == [1, -2, 3]
    assert buf.download_vec()[:12] == b"device bytes"
    assert buf.device().backend() in ("amd", "cpu")
    ```

=== "Node"

    ```js
    const { availableDevices, defaultDevice, AmdBuffer } = require('yggdryl').gpu

    // Adapt to the hardware: the enabled architectures, always ending with the CPU device.
    console.assert(availableDevices().length >= 1)
    const dev = defaultDevice()                     // the first detected GPU, else the CPU

    // Device memory that IS an IOBase — allocate, upload, run a bulk op, download.
    const buf = AmdBuffer.fromHost(Buffer.from('device bytes'))
    buf.pwriteI32Array(16, [1, -2, 3])              // vectorized bulk op, on device memory
    console.assert(buf.preadI32Array(16, 3).join() === '1,-2,3')
    console.assert(buf.downloadVec().slice(0, 12).toString() === 'device bytes')
    console.assert(['amd', 'cpu'].includes(buf.device().backend()))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::gpu::{available_devices, default_device, AmdBuffer, GpuMemory};
    use yggdryl_core::io::memory::IOBase;

    // Adapt to the hardware: the enabled architectures, always ending with the CPU device.
    assert!(!available_devices().is_empty());
    let _dev = default_device();                    // the first detected GPU, else the CPU

    // Device memory that IS an IOBase — allocate, upload, run a bulk op, download.
    let mut buf = AmdBuffer::from_host(b"device bytes");
    buf.pwrite_i32_array(16, &[1, -2, 3]).unwrap(); // vectorized bulk op, on device memory
    let mut back = [0i32; 3];
    buf.pread_i32_array(16, &mut back).unwrap();
    assert_eq!(back, [1, -2, 3]);
    assert_eq!(&buf.download_vec()[..12], b"device bytes");
    ```

## Devices — adapting to what's available

`available_devices()` enumerates the compute devices this build can allocate on — each enabled
architecture contributes what it detects, and the portable CPU device is always **appended last**, so
the list is never empty. `default_device()` picks the first detected hardware GPU, else the CPU
fallback. A `GpuDevice` is a plain **value description** of one device — its architecture token
(`backend()` → `"cpu"` / `"amd"` / `"cuda"`), human `name()`, `total_memory()`, and `is_cpu()` — and
it answers "how much room is there?" with a live `memory_info()` capacity snapshot (queried fresh, not
baked into the descriptor). A `GpuDevice` is equal, hashable, and keys a map.

=== "Python"

    ```python
    from yggdryl.gpu import available_devices, default_device

    devices = available_devices()
    assert devices[-1].is_cpu()                     # the CPU device is always present, last

    dev = default_device()                          # first detected GPU, else the CPU fallback
    assert dev.backend() in ("amd", "cuda", "cpu")
    assert isinstance(dev.name(), str)
    assert dev.total_memory() >= 0

    info = dev.memory_info()                         # a live capacity snapshot for the device
    assert info.total() >= info.available()
    ```

=== "Node"

    ```js
    const { availableDevices, defaultDevice } = require('yggdryl').gpu

    const devices = availableDevices()
    console.assert(devices[devices.length - 1].isCpu())  // the CPU device is always present, last

    const dev = defaultDevice()                          // first detected GPU, else the CPU fallback
    console.assert(['amd', 'cuda', 'cpu'].includes(dev.backend()))
    console.assert(typeof dev.name() === 'string')
    console.assert(dev.totalMemory() >= 0)

    const info = dev.memoryInfo()                        // a live capacity snapshot for the device
    console.assert(info.total() >= info.available())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::gpu::{available_devices, default_device, GpuBackend};

    let devices = available_devices();
    assert!(devices.last().unwrap().is_cpu());           // the CPU device is always present, last

    let dev = default_device();                          // first detected GPU, else the CPU fallback
    assert!(matches!(dev.backend(), GpuBackend::Amd | GpuBackend::Cuda | GpuBackend::Cpu));
    assert!(!dev.name().is_empty());

    let info = dev.memory_info();                        // a live capacity snapshot for the device
    assert!(info.total() >= info.available());
    ```

## The CPU backend is our `Heap` (`CpuHeap`)

The **CPU architecture's** device memory *is* the ordinary `Heap` — the core aliases them
(`CpuHeap = Heap`), so with the `gpu` feature a `Heap` is a `GpuMemory` buffer on the CPU device and
gains `upload` / `download` / `device` on top of its byte surface (both transfers are plain memcpys).
There is deliberately no separate CPU class: the always-available fallback needs no wrapper, reusing
the same in-heap buffer, vectorized kernels, and cursor the rest of the crate already uses. In the
Rust core you name it `CpuHeap`; in the bindings the CPU device buffer is simply a
`yggdryl.memory.Heap` — it carries the full byte + bulk surface, while the host↔device transfer
methods are shown on `AmdBuffer` below.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    # With the gpu feature a Heap IS CpuHeap — CPU device memory. In the bindings it keeps its
    # ordinary byte surface; the host<->device transfer methods are shown on AmdBuffer below.
    h = Heap(b"device bytes")
    h.pwrite_i32_array(16, [1, -2, 3])              # vectorized bulk op, on CPU device memory
    assert h.pread_i32_array(16, 3) == [1, -2, 3]
    assert bytes(h)[:12] == b"device bytes"
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    // With the gpu feature a Heap IS CpuHeap — CPU device memory. In the bindings it keeps its
    // ordinary byte surface; the host<->device transfer methods are shown on AmdBuffer below.
    const h = new Heap(Buffer.from('device bytes'))
    h.pwriteI32Array(16, [1, -2, 3])                // vectorized bulk op, on CPU device memory
    console.assert(h.preadI32Array(16, 3).join() === '1,-2,3')
    console.assert(h.toBytes().slice(0, 12).toString() === 'device bytes')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::gpu::{CpuHeap, GpuMemory};
    use yggdryl_core::io::memory::IOBase;

    // CpuHeap == the memory Heap: with the gpu feature a Heap is a GpuMemory on the CPU device.
    let mut dev = CpuHeap::new();
    dev.upload(b"device bytes").unwrap();           // host -> CPU device memory (a memcpy)
    dev.pwrite_i32_array(16, &[1, -2, 3]).unwrap(); // vectorized bulk op, on device memory
    assert_eq!(&dev.download_vec()[..12], b"device bytes");
    assert!(dev.device().is_cpu());
    ```

## AMD Radeon — `AmdBuffer`

`AmdBuffer` is device memory over the detected AMD Radeon adapter (or the CPU fallback when none is
present) that **is a full `IOBase`**: the positioned byte primitives and the vectorized bulk
`i32` / `i64` array kernels run on it exactly as they do on a `Heap`. Construct it empty
(`AmdBuffer()`), pre-sized (`with_capacity(n)`), or from host bytes (`from_host(bytes)`), then move
data with `upload(bytes)` (host → device, replacing the content) and `download(len)` /
`download_vec()` (device → host); `device()` reports which `GpuDevice` the memory lives on and
`memory_info()` its capacity. Detection is **live** — the backend resolves to `"amd"` on a machine
with a Radeon adapter, else the `"cpu"` fallback — and the buffer is correct and usable everywhere the
feature builds; the resident store stages through host memory today, with the device-side VRAM queue
as the next increment behind a stable API.

=== "Python"

    ```python
    from yggdryl.gpu import AmdBuffer

    buf = AmdBuffer()                               # empty; or AmdBuffer.with_capacity(1024)
    buf.upload(b"radeon payload")                   # host -> device (replaces the content)
    assert buf.download(6) == b"radeon"             # download the first 6 bytes
    assert buf.download_vec() == b"radeon payload"  # the whole buffer
    assert len(buf) == 14

    # It runs the byte + vectorized bulk numeric kernels, exactly like a Heap.
    wide = AmdBuffer.from_host(b"")
    wide.pwrite_i64_array(0, [10, 20, 30])
    assert wide.pread_i64_array(0, 3) == [10, 20, 30]

    # The backend resolves to "amd" on a Radeon machine, else the CPU fallback.
    assert buf.device().backend() in ("amd", "cpu")
    ```

=== "Node"

    ```js
    const { AmdBuffer } = require('yggdryl').gpu

    const buf = new AmdBuffer()                     // empty; or AmdBuffer.withCapacity(1024)
    buf.upload(Buffer.from('radeon payload'))       // host -> device (replaces the content)
    console.assert(buf.download(6).toString() === 'radeon')          // the first 6 bytes
    console.assert(buf.downloadVec().toString() === 'radeon payload') // the whole buffer
    console.assert(buf.byteSize() === 14)

    // It runs the byte + vectorized bulk numeric kernels, exactly like a Heap.
    const wide = AmdBuffer.fromHost(Buffer.alloc(0))
    wide.pwriteI64Array(0, [10, 20, 30])
    console.assert(wide.preadI64Array(0, 3).join() === '10,20,30')

    // The backend resolves to "amd" on a Radeon machine, else the CPU fallback.
    console.assert(['amd', 'cpu'].includes(buf.device().backend()))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::gpu::{AmdBuffer, GpuMemory};
    use yggdryl_core::io::memory::IOBase;

    let mut buf = AmdBuffer::new();                 // empty; or AmdBuffer::with_capacity(1024)
    buf.upload(b"radeon payload").unwrap();         // host -> device (replaces the content)
    assert_eq!(buf.download_vec(), b"radeon payload");
    assert_eq!(buf.byte_size(), 14);

    // It runs the byte + vectorized bulk numeric kernels, exactly like a Heap.
    let mut wide = AmdBuffer::from_host(b"");
    wide.pwrite_i64_array(0, &[10, 20, 30]).unwrap();
    let mut back = [0i64; 3];
    wide.pread_i64_array(0, &mut back).unwrap();
    assert_eq!(back, [10, 20, 30]);

    // The backend resolves to "amd" on a Radeon machine, else the CPU fallback.
    assert!(buf.device().backend().as_str() == "amd" || buf.device().is_cpu());
    ```

## Auto-dispatched compute — aggregations, filters, copy

Beyond raw byte I/O, a device buffer runs **compute** operations — math **aggregations** (`sum` /
`min` / `max` / `mean`), a threshold **filter** (`count_ge` — how many values are `>= threshold`),
and a device-aware **copy** — that **auto-select GPU vs CPU**. Each op asks `compute_backend(n)`
which backend to use: the **GPU** when the buffer lives on a real device *and* the workload is large
enough to amortize the host↔device transfer, else the **CPU** (the dense, LLVM-vectorized reduction,
streamed through a stack chunk with no heap allocation). Today both arms run the CPU kernel — the
GPU arm is the optimization **seam** where a device reduction / filter / DMA kernel drops in behind a
hardware backend, so code written against these ops accelerates transparently when the kernels land.
The typed ops exist for `i32`, `i64`, `f32`, and `f64`.

=== "Python"

    ```python
    from yggdryl.gpu import AmdBuffer

    buf = AmdBuffer()
    buf.pwrite_i32_array(0, [4, 8, 15, 16, 23, 42])
    assert buf.sum_i32(0, 6) == 108
    assert buf.min_i32(0, 6) == 4 and buf.max_i32(0, 6) == 42
    assert buf.mean_i32(0, 6) == 18.0
    assert buf.count_ge_i32(0, 6, 16) == 3      # a filter: how many >= 16
    assert buf.compute_backend(8) == "cpu"      # small workload stays on the CPU

    dst = AmdBuffer()
    assert buf.compute_copy_into(dst) == 24     # device-aware copy, returns bytes moved
    ```

=== "Node"

    ```js
    const { AmdBuffer } = require('yggdryl').gpu

    const buf = new AmdBuffer()
    buf.pwriteI32Array(0, [4, 8, 15, 16, 23, 42])
    console.assert(buf.sumI32(0, 6) === 108)
    console.assert(buf.minI32(0, 6) === 4 && buf.maxI32(0, 6) === 42)
    console.assert(buf.meanI32(0, 6) === 18)
    console.assert(buf.countGeI32(0, 6, 16) === 3)   // filter: how many >= 16
    console.assert(buf.computeBackend(8) === 'cpu')

    const dst = new AmdBuffer()
    console.assert(buf.computeCopyInto(dst) === 24)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::gpu::{AmdBuffer, Compute, ComputeBackend, GpuMemory};
    use yggdryl_core::io::memory::IOBase;

    let mut buf = AmdBuffer::new();
    buf.pwrite_i32_array(0, &[4, 8, 15, 16, 23, 42]).unwrap();
    assert_eq!(buf.sum_i32(0, 6).unwrap(), 108);
    assert_eq!(buf.max_i32(0, 6).unwrap(), Some(42));
    assert_eq!(buf.mean_i32(0, 6).unwrap(), Some(18.0));
    assert_eq!(buf.count_ge_i32(0, 6, 16).unwrap(), 3); // filter: how many >= 16
    assert_eq!(buf.compute_backend(8), ComputeBackend::Cpu);

    let mut dst = AmdBuffer::new();
    assert_eq!(buf.compute_copy_into(&mut dst).unwrap(), 24); // device-aware copy
    ```

## Capacity — `MemoryInfo`

`MemoryInfo` is the **one capacity type** across the whole library — the single answer to "how much
room is there?" whether the backend is host RAM, disk, or device VRAM. `MemoryInfo.system()` snapshots
physical RAM (the CPU device's memory), a `LocalIO` reports its volume's free space through
`memory_info()`, and a `GpuDevice` reports its VRAM through `memory_info()` — all the same value. It
carries a backend's `total()` and `available()` (free) bytes, with `used()` and `usage_ratio()`
derived from the pair, and an `is_unknown()` sentinel (`0` / `0`) for a platform that cannot answer.
It is an immutable value — equal, hashable, and (in Python) picklable through its `(total, available)`
pair.

=== "Python"

    ```python
    from yggdryl.io import MemoryInfo
    from yggdryl.gpu import default_device

    # One capacity type across host RAM, disk, and device VRAM.
    ram = MemoryInfo.system()                        # host RAM — the CPU device's memory
    assert ram.total() >= ram.available()
    assert ram.used() == ram.total() - ram.available()
    assert 0.0 <= ram.usage_ratio() <= 1.0

    vram = default_device().memory_info()            # the default device's capacity snapshot
    assert vram.total() >= vram.available()

    # A plain immutable value — equal, hashable, and picklable through (total, available).
    assert MemoryInfo(1000, 400) == MemoryInfo(1000, 400)
    assert MemoryInfo(1000, 400).used() == 600
    assert MemoryInfo.unknown().is_unknown()
    ```

=== "Node"

    ```js
    const { MemoryInfo } = require('yggdryl').io
    const { defaultDevice } = require('yggdryl').gpu

    // One capacity type across host RAM, disk, and device VRAM.
    const ram = MemoryInfo.system()                  // host RAM — the CPU device's memory
    console.assert(ram.total() >= ram.available())
    console.assert(ram.used() === ram.total() - ram.available())
    console.assert(ram.usageRatio() >= 0 && ram.usageRatio() <= 1)

    const vram = defaultDevice().memoryInfo()        // the default device's capacity snapshot
    console.assert(vram.total() >= vram.available())

    // A plain value — equatable, with used / usageRatio derived from the pair.
    console.assert(new MemoryInfo(1000, 400).equals(new MemoryInfo(1000, 400)))
    console.assert(new MemoryInfo(1000, 400).used() === 600)
    console.assert(MemoryInfo.unknown().isUnknown())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::MemoryInfo;
    use yggdryl_core::io::gpu::default_device;

    // One capacity type across host RAM, disk, and device VRAM.
    let ram = MemoryInfo::system();                  // host RAM — the CPU device's memory
    assert!(ram.total() >= ram.available());
    assert_eq!(ram.used(), ram.total() - ram.available());
    assert!((0.0..=1.0).contains(&ram.usage_ratio()));

    let vram = default_device().memory_info();       // the default device's capacity snapshot
    assert!(vram.total() >= vram.available());

    // A plain value — Clone/Eq/Hash, with used / usage_ratio derived from the pair.
    assert_eq!(MemoryInfo::new(1000, 400), MemoryInfo::new(1000, 400));
    assert_eq!(MemoryInfo::new(1000, 400).used(), 600);
    assert!(MemoryInfo::unknown().is_unknown());
    ```

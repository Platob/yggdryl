# yggdryl (Python)

Python bindings for [**yggdryl**](https://github.com/Platob/yggdryl), backed by
the Rust `yggdryl` core crate.

## Install

```bash
pip install maturin
maturin develop          # build & install into the current virtualenv
# or build a wheel:
maturin build --release
```

## Usage

```python
import yggdryl

uri = yggdryl.Uri("urn:isbn:0451450523")
print(uri.scheme)              # urn
print(uri.path)                # isbn:0451450523

url = yggdryl.Url("https://user:pw@example.com:8443/api?v=1#top")
print(url.host)                # example.com
print(url.port)                # 8443
print(url.username)            # user
print(str(url))                # https://user:pw@example.com:8443/api?v=1#top
```

Invalid input raises `ValueError`.

# yggdryl-http

Generic HTTP-style header maps for [yggdryl](https://github.com/Platob/yggdryl).

[`Headers`](src/headers.rs) is an ordered **bytes → bytes** map (like an HTTP header
block) with both byte and UTF-8 string accessors/mutators, zero-copy in-place value
mutation ([`get_mut`]), a byte round-trip codec, and pre-built accessors for the common
keys (`name`, `comment`, `content-type`, `content-encoding`). [`HeadersBased`] is the
trait a header-carrying type (a field, a buffer) implements to get the whole
get / add / update / delete surface for free.

Dependency-free; upper layers (`yggdryl-field`, `yggdryl-buffer`) depend on it.

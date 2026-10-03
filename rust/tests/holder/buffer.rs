//! `rust/src/holder/buffer.rs`: the in-memory handle, and the `mem:` identity
//! it answers for a location it does not have.

use yggdryl::IOBase;
use yggdryl::holder::Buffer;

#[test]
fn a_buffer_reports_a_mem_identity_rather_than_a_location() {
    let buffer = Buffer::from_bytes(b"bytes".to_vec());
    let identity = buffer.url().expect("a buffer always has an identity");

    // The bytes are not stored anywhere, so this names the machine, the
    // process and the allocation rather than a place on disk:
    // `mem://localhost/<pid>/<address>`, never a process id for a host.
    assert_eq!(identity.scheme().as_str(), "mem");
    assert_eq!(identity.authority().as_str(), "localhost");
    let pid = format!("/{}/0x", std::process::id());
    assert!(identity.path().as_str().starts_with(&pid), "{identity}");

    // The identity is stable for one handle.
    assert_eq!(buffer.url(), Some(identity));

    // A distinct buffer is distinguishable from it.
    let other = Buffer::from_bytes(b"bytes".to_vec());
    assert_ne!(other.url(), Some(identity));
}

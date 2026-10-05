//! `rust/src/holder/buffer.rs`: the in-memory handle, and the `mem:` identity
//! it answers for a location it does not have.

use yggdryl::IOBase;
use yggdryl::holder::Buffer;

#[test]
fn a_buffer_reports_a_mem_identity_rather_than_a_location() {
    let buffer = Buffer::from_bytes(b"bytes".to_vec());
    let identity = buffer.url().expect("a buffer always has an identity");

    // The bytes are not stored anywhere, so this names the machine, the
    // process and the buffer's place in its sequence rather than a place on
    // disk: `mem://localhost/<pid>/<sequence>`, never a process id for a
    // host, and never an address, which every empty buffer would share.
    assert_eq!(identity.scheme().as_str(), "mem");
    assert_eq!(identity.authority().as_str(), "localhost");
    let pid = format!("/{}/", std::process::id());
    let path = identity.path().as_str();
    assert!(path.starts_with(&pid), "{identity}");
    assert!(
        !path[pid.len()..].is_empty()
            && path[pid.len()..].bytes().all(|byte| byte.is_ascii_digit()),
        "{identity}"
    );

    // Two empty buffers are two: nothing of what they hold tells them apart.
    assert_ne!(Buffer::new().url().cloned(), Buffer::new().url().cloned());

    // The identity is stable for one handle.
    assert_eq!(buffer.url(), Some(identity));

    // A distinct buffer is distinguishable from it.
    let other = Buffer::from_bytes(b"bytes".to_vec());
    assert_ne!(other.url(), Some(identity));
}

//! `rust/src/iobase/bytes.rs`: the forwarding and boxed-handle implementations.

mod positional {

    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    #[test]
    fn boxed_cursors_preserve_lifecycle_hierarchy_and_kind() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        use yggdryl::holder::Holder;
        use yggdryl::{IOKind, Result};
        use yggdryl::{IOMedia, Listing};

        struct Probe {
            bytes: Buffer,
            opened: Arc<AtomicBool>,
        }

        impl IOMedia for Probe {
            yggdryl::impl_default_iomedia!();
        }

        impl IOBase for Probe {
            yggdryl::delegate_iobase!(bytes: pread, pstream_bytes, pwrite, size, capacity, reserve,
                truncate, uri, url, media_type, set_media_type, flush, clear, remove);

            fn open(&mut self) -> Result<()> {
                self.opened.store(true, Ordering::SeqCst);
                Ok(())
            }

            fn opened(&self) -> bool {
                self.opened.load(Ordering::SeqCst)
            }

            fn close(&mut self) -> Result<()> {
                self.opened.store(false, Ordering::SeqCst);
                Ok(())
            }

            fn parent(&self) -> Option<Holder> {
                Some(Holder::buffer(Buffer::from_bytes(b"parent".to_vec())))
            }

            fn child_by_path(&self, path: &str) -> Result<Holder> {
                Ok(Holder::buffer(Buffer::from_bytes(path.as_bytes().to_vec())))
            }

            fn ls(&self, recursive: bool, include_private: bool) -> Listing {
                let value = format!("{recursive}:{include_private}");
                Listing::new(std::iter::once(Ok(Holder::buffer(Buffer::from_bytes(
                    value.into_bytes(),
                )))))
            }

            fn kind(&self) -> IOKind {
                IOKind::Directory
            }
        }

        let state = Arc::new(AtomicBool::new(false));
        let mut handle: Box<dyn IOBase> = Box::new(yggdryl::Cursor::new(Probe {
            bytes: Buffer::new(),
            opened: Arc::clone(&state),
        }));

        assert_eq!(handle.kind(), IOKind::Directory);
        assert!(handle.is_container());
        assert_eq!(
            handle.parent().unwrap().read_all_bytes().unwrap(),
            b"parent"
        );
        assert_eq!(
            handle
                .child_by_path("nested/leaf")
                .unwrap()
                .read_all_bytes()
                .unwrap(),
            b"nested/leaf"
        );
        assert_eq!(
            handle
                .ls(true, true)
                .next()
                .unwrap()
                .unwrap()
                .read_all_bytes()
                .unwrap(),
            b"true:true"
        );

        handle.open().unwrap();
        assert!(handle.opened());
        assert!(state.load(Ordering::SeqCst));
        handle.close().unwrap();
        assert!(handle.closed());
        assert!(!state.load(Ordering::SeqCst));
    }
}

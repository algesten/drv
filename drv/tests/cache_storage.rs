//! Cache storage retains ordinary memo semantics and thread-exit cleanup.

use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

static CALLS: AtomicUsize = AtomicUsize::new(0);
static DROPS: AtomicUsize = AtomicUsize::new(0);

struct Value(u8);

impl Drop for Value {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

#[drv::memo(single)]
fn inner(key: u8) -> Rc<Value> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    Rc::new(Value(key))
}

#[drv::memo(lru = 2)]
fn outer(key: u8) -> Rc<Value> {
    inner(key)
}

#[test]
fn caches_are_thread_local_and_drop_non_send_values_at_thread_exit() {
    // Sequential threads must each compute their own entries. Repeated calls
    // within one thread must return clones of the same cached Rc.
    for thread in 1..=2 {
        std::thread::spawn(|| {
            let first = outer(7);
            assert_eq!(first.0, 7);
            assert!(Rc::ptr_eq(&first, &outer(7)));
            assert_eq!(outer(8).0, 8);
            assert!(Rc::ptr_eq(&first, &outer(7)));
        })
        .join()
        .unwrap();
        assert_eq!(CALLS.load(Ordering::SeqCst), thread * 2);
        assert_eq!(DROPS.load(Ordering::SeqCst), thread * 2);
    }
}

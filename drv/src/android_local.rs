//! Android-specific shared thread-local storage for generated memo caches.

use std::any::Any;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

static NEXT_SLOT: AtomicUsize = AtomicUsize::new(0);

std::thread_local! {
    // Android's Rust targets implement each `thread_local!` through a pthread
    // key. Keep all memo states behind this one key instead of spending one
    // of Android's limited keys per generated memo.
    static MEMO_STATES: RefCell<Vec<Option<Rc<dyn Any>>>> =
        const { RefCell::new(Vec::new()) };
}

/// An Android thread-local value stored in drv's shared TLS registry.
///
/// Generated memo statics use this in place of `std::thread::LocalKey` on
/// Android. Every instance owns a process-wide slot number, while the value at
/// that slot remains local to each thread.
pub struct AndroidLocal<T> {
    slot: OnceLock<usize>,
    marker: PhantomData<fn() -> T>,
}

impl<T> Default for AndroidLocal<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AndroidLocal<T> {
    /// Creates an uninitialized shared-registry entry.
    pub const fn new() -> Self {
        Self {
            slot: OnceLock::new(),
            marker: PhantomData,
        }
    }

    /// Invokes `f` with this value's thread-local cell, initializing it with
    /// `T::default()` on the first access from each thread.
    pub fn with<R>(&'static self, f: impl FnOnce(&RefCell<T>) -> R) -> R
    where
        T: Any + Default,
    {
        let slot = *self
            .slot
            .get_or_init(|| NEXT_SLOT.fetch_add(1, Ordering::Relaxed));

        let erased = MEMO_STATES.with(|states| {
            let mut states = states.borrow_mut();
            if states.len() <= slot {
                states.resize_with(slot + 1, || None);
            }
            states[slot]
                .get_or_insert_with(|| Rc::new(RefCell::new(T::default())))
                .clone()
        });

        // Release the registry borrow before calling `f`: memo equality,
        // output cloning, and memo bodies may enter a different memo.
        let state = match erased.downcast::<RefCell<T>>() {
            Ok(state) => state,
            Err(_) => unreachable!("drv Android TLS slot contained the wrong type"),
        };
        f(&state)
    }
}

#[cfg(test)]
mod tests {
    use super::AndroidLocal;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIRST: AndroidLocal<u32> = AndroidLocal::new();
    static SECOND: AndroidLocal<String> = AndroidLocal::new();

    #[test]
    fn values_are_distinct_and_persistent() {
        FIRST.with(|value| *value.borrow_mut() = 42);
        SECOND.with(|value| value.borrow_mut().push_str("memo"));

        FIRST.with(|value| assert_eq!(*value.borrow(), 42));
        SECOND.with(|value| assert_eq!(&*value.borrow(), "memo"));
    }

    static NON_SEND: AndroidLocal<Rc<()>> = AndroidLocal::new();

    #[test]
    fn values_do_not_need_to_be_send_or_sync() {
        NON_SEND.with(|value| assert_eq!(Rc::strong_count(&value.borrow()), 1));
    }

    static THREAD_VALUE: AndroidLocal<u32> = AndroidLocal::new();

    #[test]
    fn values_are_isolated_between_threads() {
        THREAD_VALUE.with(|value| *value.borrow_mut() = 7);

        std::thread::spawn(|| {
            THREAD_VALUE.with(|value| {
                assert_eq!(*value.borrow(), 0);
                *value.borrow_mut() = 11;
            });
            THREAD_VALUE.with(|value| assert_eq!(*value.borrow(), 11));
        })
        .join()
        .unwrap();

        THREAD_VALUE.with(|value| assert_eq!(*value.borrow(), 7));
    }

    static OUTER: AndroidLocal<u32> = AndroidLocal::new();
    static INNER: AndroidLocal<u32> = AndroidLocal::new();

    #[test]
    fn a_value_callback_can_enter_another_value() {
        OUTER.with(|outer| {
            *outer.borrow_mut() = 1;
            INNER.with(|inner| *inner.borrow_mut() = 2);
        });

        OUTER.with(|value| assert_eq!(*value.borrow(), 1));
        INNER.with(|value| assert_eq!(*value.borrow(), 2));
    }

    static MANY: [AndroidLocal<usize>; 256] = [const { AndroidLocal::new() }; 256];

    #[test]
    fn registry_has_no_pthread_key_sized_cap() {
        for (index, value) in MANY.iter().enumerate() {
            value.with(|value| *value.borrow_mut() = index);
        }
        for (index, value) in MANY.iter().enumerate() {
            value.with(|value| assert_eq!(*value.borrow(), index));
        }
    }

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Default)]
    struct DropCounter;

    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }

    static DROPPED_VALUE: AndroidLocal<DropCounter> = AndroidLocal::new();

    #[test]
    fn values_drop_when_the_thread_exits() {
        let before = DROPS.load(Ordering::Relaxed);
        std::thread::spawn(|| DROPPED_VALUE.with(|_| {}))
            .join()
            .unwrap();
        assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);
    }
}

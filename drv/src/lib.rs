#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Memoize a function with `#[drv::memo]`. The attribute is liberal about
//! parameter types — owned values, references, struct refs, `&str`,
//! `&[u8]` — and caches results in a per-memo thread-local slot array
//! keyed by value equality.
//!
//! ```rust
//! #[derive(Debug, Clone, PartialEq)]
//! pub struct Config {
//!     pub threads: u32,
//!     pub timeout_ms: u64,
//! }
//!
//! #[drv::memo(single)]
//! fn worker_count(c: &Config) -> u32 {
//!     c.threads * 2
//! }
//!
//! # fn main() {
//! let c = Config { threads: 4, timeout_ms: 5000 };
//! assert_eq!(worker_count(&c), 8);   // computes
//! assert_eq!(worker_count(&c), 8);   // cache hit, no work
//! # }
//! ```
//!
//! The companion attribute `#[drv::input]` is a helper for one specific
//! situation: when you want to cache on a *subset* of a source struct's
//! fields without cloning the whole struct on every call. See
//! [Zero-copy projections](#zero-copy-projections-with-drvinput).
//!
//! # Why drv
//!
//! - **Equality-keyed, not hash-keyed.** No hashing on the hot path, no
//!   `HashMap` probe. Cache lookup is a linear scan through a fixed-shape
//!   slot array with per-field `PartialEq`.
//! - **Thread-local caches.** Every memo owns its own cache —
//!   single-writer, lock-free.
//! - **Zero allocations on cache hit.** A hit is `PartialEq` plus a
//!   `Clone` of the output.
//! - **O(1) cache-hit check for `Arc<T>` and `imbl` collections.** A
//!   pointer-equality fast path skips deep comparison when the field's
//!   pointer hasn't changed since the last miss.
//! - **Small public surface.** `#[drv::memo]` is all you need for the
//!   common case; `#[drv::input]` is an opt-in helper for borrowed
//!   projections.
//!
//! # Writing a memo
//!
//! Every memo picks a cache strategy:
//!
//! - `#[drv::memo(single)]` — one slot, last-call caching. A hit requires
//!   today's inputs to equal the most recent recompute's inputs.
//! - `#[drv::memo(lru = N)]` — N slots, least-recently-used eviction. For
//!   inputs that cycle between a small number of recurring states.
//!
//! ## Parameters
//!
//! | You write | Needs `#[drv::input]` | Notes |
//! |---|---|---|
//! | `x: T` |  | `T: Clone + PartialEq + 'static` |
//! | `x: &T` |  | `T: ?Sized + ToOwned` (e.g. `&str` → `String`) |
//! | `x: MyInput<'a>` | ✅ | |
//! | `x: &MyInput<'a>` | ✅ | |
//!
//! Any by-value type whose last path segment carries a lifetime argument
//! (`MyInput<'a>`) is treated as a borrowed projection and must be tagged
//! `#[drv::input]`. Everything else — owned values, `&str`, `&[u8]`,
//! `&MyStruct`, etc. — works without the helper.
//!
//! Bodies see the exact type you declared: strip `#[drv::memo]` and the
//! function still compiles.
//!
//! # Zero-copy projections with `#[drv::input]`
//!
//! Take the previous `Config` example. If `Config` grows a big
//! `Vec<Worker>` field that `worker_count` doesn't read, the default
//! `&Config` form still snapshots the whole struct into the cache slot
//! via `Clone`, and every cache-hit check compares the whole thing.
//!
//! `#[drv::input]` lets you declare a lightweight view that borrows only
//! the fields the memo actually depends on. The macro auto-generates the
//! owned snapshot, the `PartialEq` against it, and the snapshot method
//! `#[drv::memo]` uses internally:
//!
//! ```rust
//! # #[derive(Debug, Clone, PartialEq, Default)]
//! # pub struct Scoreboard {
//! #     pub hits: Vec<u32>,
//! #     pub player_x: u32,
//! # }
//! #[drv::input]
//! struct TotalInput<'a> {
//!     pub hits: &'a Vec<u32>,
//! }
//!
//! impl<'a> TotalInput<'a> {
//!     pub fn new(s: &'a Scoreboard) -> Self { Self { hits: &s.hits } }
//! }
//!
//! #[drv::memo(single)]
//! fn total_score<'a>(input: TotalInput<'a>) -> u32 {
//!     input.hits.iter().sum()
//! }
//!
//! # fn main() {
//! let mut game = Scoreboard { hits: vec![100, 250, 50], player_x: 0 };
//! assert_eq!(total_score(TotalInput::new(&game)), 400);   // computes
//! game.player_x = 42;                                      // not in TotalInput
//! assert_eq!(total_score(TotalInput::new(&game)), 400);   // cache hit
//! # }
//! ```
//!
//! Only `hits` enters the cache key. Changes to `player_x` don't
//! invalidate; changes to `hits` do. The projection is whatever code you
//! write — a `::new` method, a `From<&Source>` impl, or an inline struct
//! literal at the call site. drv doesn't prescribe one.
//!
//! # Performance
//!
//! Two sources of per-call work:
//!
//! 1. **Per-field `PartialEq`** on the cache-hit check.
//! 2. **Output `Clone`** on every return.
//!
//! drv wires `FastEq` into the per-field comparison so that `Arc<T>` and
//! (with the `imbl` feature) `imbl`'s persistent collections take a
//! pointer-equality fast path — O(1) when the field hasn't been mutated
//! since the last miss.
//!
//! | Type | `Clone` | Cache hit (same pointer) | Cache hit (equal contents) | Mutation |
//! |------|-------|--------------------------|----------------------------|----------|
//! | `Vec<T>` | O(n) | O(n) | O(n) | O(1) amortised |
//! | `HashMap<K, V>` | O(n) | O(n) | O(n) | O(1) amortised |
//! | `Arc<T>` | **O(1)** | **O(1)** | O(eq of T) | n/a |
//! | `imbl::Vector<T>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |
//! | `imbl::HashMap<K, V>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |
//!
//! Rule of thumb: scalars are free; small `Vec` / `String` is fine; for
//! collections with more than a handful of elements, wrap in `Arc<T>` or
//! reach for `imbl`.
//!
//! Enable `imbl`:
//!
//! ```toml
//! [dependencies]
//! drv = { version = "0.2", features = ["imbl"] }
//! ```
//!
//! # Comparison
//!
//! Ranked from most to least alike.
//!
//! - [`comemo`] — closest in spirit. Memoises functions with
//!   fine-grained dependency tracking via runtime access recording
//!   (`#[track]`). drv's static input struct is cheaper per call but
//!   asks you to declare dependencies up front rather than discovering
//!   them at runtime.
//! - [`salsa`] — incremental-computation database used by rust-analyzer.
//!   Tracks a dependency graph across queries; much more powerful than
//!   drv for deeply chained derivations, and much heavier.
//! - [`cached`] / [`memoize`] — general-purpose memoisation via `Hash` of
//!   arguments, backed by `HashMap` under a lock. Work for any hashable
//!   input; drv skips hashing entirely and trades generality for
//!   hot-path speed and field-level invalidation.
//! - [`moka`] / [`quick_cache`] / [`stretto`] — concurrent in-memory
//!   cache data structures (Caffeine / Ristretto ports). Not
//!   memoisation crates — they're backing stores you'd build a cache
//!   on top of. drv's thread-local single-writer model is the opposite
//!   design choice.
//!
//! [`cached`]: https://crates.io/crates/cached
//! [`memoize`]: https://crates.io/crates/memoize
//! [`moka`]: https://crates.io/crates/moka
//! [`quick_cache`]: https://crates.io/crates/quick_cache
//! [`stretto`]: https://crates.io/crates/stretto
//! [`salsa`]: https://crates.io/crates/salsa
//! [`comemo`]: https://crates.io/crates/comemo
//!
//! # License
//!
//! Dual-licensed under MIT or Apache-2.0, at your option.

/// Marker trait automatically implemented by `#[drv::input]`-tagged
/// structs. `#[drv::memo]` emits a compile-time bound against this trait
/// for every input-classified parameter, which turns "forgot
/// `#[drv::input]`" into a single pointed error instead of a cascade of
/// missing-method/missing-snapshot errors from the generated code.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a `#[drv::input]` type",
    label = "missing `#[drv::input]` attribute on the struct definition",
    note = "memo parameters written as `MyInput<'_>` or `&MyInput<'_>` require the target to be tagged with `#[drv::input]`"
)]
pub trait DrvInput {}

/// Internal helper used by generated code to pick between a pointer-equality
/// fast path (for `Arc<T>` and, under the `imbl` feature, `imbl`'s persistent
/// collections) and the regular `PartialEq` path. Not intended for direct
/// use — `drv` wires this into `#[drv::memo]` and `#[drv::input]` automatically.
#[doc(hidden)]
pub struct FastEq<'a, T: ?Sized>(pub &'a T);

/// Fallback trait for [`FastEq::fast_eq`]. Exists so generated code can call
/// `fast_eq` uniformly regardless of whether `T` has a specialized inherent
/// impl. Not intended for direct use.
#[doc(hidden)]
pub trait FastEqFallback<T: ?Sized> {
    fn fast_eq(&self, other: &T) -> bool;
}

impl<T: PartialEq + ?Sized> FastEqFallback<T> for FastEq<'_, T> {
    fn fast_eq(&self, other: &T) -> bool {
        self.0 == other
    }
}

// Arc<T>: always available, std.
impl<T: PartialEq + ?Sized> FastEq<'_, std::sync::Arc<T>> {
    pub fn fast_eq(&self, other: &std::sync::Arc<T>) -> bool {
        std::sync::Arc::ptr_eq(self.0, other) || **self.0 == **other
    }
}

// imbl collections: behind `feature = "imbl"`.
#[cfg(feature = "imbl")]
mod fasteq_imbl {
    use super::FastEq;
    use std::hash::{BuildHasher, Hash};

    use imbl::shared_ptr::SharedPointerKind;
    use imbl::{GenericHashMap, GenericHashSet, GenericVector};

    impl<T: PartialEq + Clone, P: SharedPointerKind> FastEq<'_, GenericVector<T, P>> {
        pub fn fast_eq(&self, other: &GenericVector<T, P>) -> bool {
            self.0.ptr_eq(other) || self.0 == other
        }
    }

    impl<K, V, S, P> FastEq<'_, GenericHashMap<K, V, S, P>>
    where
        K: Hash + Eq + Clone,
        V: PartialEq + Clone,
        S: BuildHasher + Clone,
        P: SharedPointerKind,
    {
        pub fn fast_eq(&self, other: &GenericHashMap<K, V, S, P>) -> bool {
            self.0.ptr_eq(other) || self.0 == other
        }
    }

    impl<K, V> FastEq<'_, imbl::OrdMap<K, V>>
    where
        K: Ord + Clone,
        V: PartialEq + Clone,
    {
        pub fn fast_eq(&self, other: &imbl::OrdMap<K, V>) -> bool {
            self.0.ptr_eq(other) || self.0 == other
        }
    }

    impl<T, S, P> FastEq<'_, GenericHashSet<T, S, P>>
    where
        T: Hash + Eq + Clone,
        S: BuildHasher + Clone,
        P: SharedPointerKind,
    {
        pub fn fast_eq(&self, other: &GenericHashSet<T, S, P>) -> bool {
            self.0.ptr_eq(other) || self.0 == other
        }
    }

    impl<T: Ord + Clone> FastEq<'_, imbl::OrdSet<T>> {
        pub fn fast_eq(&self, other: &imbl::OrdSet<T>) -> bool {
            self.0.ptr_eq(other) || self.0 == other
        }
    }
}

/// Declare a struct as a memo input.
///
/// Generates the owned snapshot and the machinery `#[drv::memo]` needs to
/// compare against it. You provide the projection from your source
/// struct however you like — an inherent method, a `From` impl, an
/// inline struct literal — the macro doesn't care.
///
/// # Example
///
/// ```rust
/// #[derive(Debug, Clone, PartialEq, Default)]
/// pub struct AppState {
///     pub items: Vec<u32>,
///     pub viewport_rows: u32,
/// }
///
/// #[drv::input]
/// struct VisibleInput<'a> {
///     pub items: &'a Vec<u32>,
///     pub viewport_rows: u32,
/// }
///
/// impl<'a> VisibleInput<'a> {
///     pub fn new(a: &'a AppState) -> Self {
///         Self { items: &a.items, viewport_rows: a.viewport_rows }
///     }
/// }
///
/// #[drv::memo(single)]
/// fn first_visible<'a>(input: VisibleInput<'a>) -> Option<u32> {
///     input.items.iter().take(input.viewport_rows as usize).next().copied()
/// }
///
/// # fn main() {
/// let a = AppState { items: vec![10, 20, 30], viewport_rows: 2 };
/// assert_eq!(first_visible(VisibleInput::new(&a)), Some(10));
/// # }
/// ```
pub use drv_macros::input;

/// Memoize a function.
///
/// Every memo must declare a cache strategy:
///
/// - `#[drv::memo(single)]` — one slot, last-call caching.
/// - `#[drv::memo(lru = N)]` — N slots, least-recently-used eviction.
///
/// # Parameters
///
/// Every parameter is a memo input. Parameters whose type carries a
/// lifetime argument must be declared as `#[drv::input]`. Plain owned
/// values and `&T` references don't need it.
///
/// | You write | Needs `#[drv::input]` | Notes |
/// |---|---|---|
/// | `x: T` |  | `T: Clone + PartialEq + 'static` |
/// | `x: &T` |  | `T: ?Sized + ToOwned` (e.g. `&str` → `String`) |
/// | `x: MyInput<'a>` | ✅ | |
/// | `x: &MyInput<'a>` | ✅ | |
///
/// If a lifetime-bearing type isn't tagged `#[drv::input]`, you get a
/// pointed compile error at the call site.
///
/// # Example
///
/// ```rust
/// #[derive(Debug, Clone, PartialEq, Default)]
/// pub struct Stats { pub xs: Vec<u32> }
///
/// #[drv::input]
/// struct SumInput<'a> { pub xs: &'a Vec<u32> }
///
/// impl<'a> SumInput<'a> {
///     pub fn new(s: &'a Stats) -> Self { Self { xs: &s.xs } }
/// }
///
/// #[drv::memo(single)]
/// fn sum_with_offset<'a>(input: SumInput<'a>, offset: u32) -> u32 {
///     input.xs.iter().sum::<u32>() + offset
/// }
///
/// # fn main() {
/// let a = Stats { xs: vec![1, 2, 3] };
/// assert_eq!(sum_with_offset(SumInput::new(&a), 10), 16);
/// # }
/// ```
pub use drv_macros::memo;

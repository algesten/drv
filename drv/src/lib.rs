#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Memoize a function with `#[drv::memo]`. The attribute is liberal about
//! parameter types — owned values, references, struct refs, `&str`,
//! `&[u8]`, `#[derive(drv::Input)]` projections — and caches results in a
//! per-memo thread-local slot array keyed by value equality.
//!
//! ```rust
//! #[derive(drv::Input)]
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
//! The companion derive `#[derive(drv::Input)]` is a helper for one
//! specific situation: when you want to cache on a *subset* of a source
//! struct's fields without cloning the whole struct on every call. See
//! [Zero-copy projections](#zero-copy-projections-with-drvinput).
//!
//! # Why drv
//!
//! - **Equality-keyed, not hash-keyed.** No hashing on the hot path, no
//!   `HashMap` probe. Cache lookup is a linear scan through a fixed-shape
//!   slot array with per-field equality.
//! - **Thread-local caches.** Every memo owns its own cache —
//!   single-writer, lock-free.
//! - **Zero allocations on cache hit.** A hit is an equality check plus a
//!   `Clone` of the output.
//! - **O(1) cache-hit check for `Arc<T>` and `imbl` collections.** A
//!   pointer-equality fast path skips deep comparison when the field's
//!   pointer hasn't changed since the last miss.
//! - **One concept for inputs.** `#[derive(drv::Input)]` is the single
//!   opt-in: any struct you want as a memo parameter derives it. Plain
//!   structs, borrowed projections, and nested-input bundles all work
//!   the same way.
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
//! Every parameter must implement [`ToStatic`]. That trait is
//! implemented by drv for primitives, `String`, `Arc<T>`, imbl
//! collections (feature-gated), and — via a reference blanket — `&T`
//! for any `T: ToStatic`. Containers (`Vec`, `Option`, tuples, arrays,
//! `HashMap` / `HashSet` / `BTreeMap` / `BTreeSet`) are recursive:
//! they implement `ToStatic` whenever their elements do, so nested
//! projections like `Vec<MyInput<'a>>` work without any extra
//! annotation. User types become inputs by adding
//! `#[derive(drv::Input)]`.
//!
//! | You write | Needs `#[derive(drv::Input)]` | Notes |
//! |---|---|---|
//! | `x: T` (primitive, std type) |  | Shipped impl. |
//! | `x: &str`, `x: &[u8]` |  | Reference blanket. |
//! | `x: Arc<T>` |  | ptr_eq fast path. |
//! | `x: MyInput<'a>` | ✅ | Borrowed projection. |
//! | `x: &MyStruct` | ✅ | Plain owned struct. |
//!
//! Bodies see the exact type you declared: strip `#[drv::memo]` and the
//! function still compiles.
//!
//! # Zero-copy projections with `#[derive(drv::Input)]`
//!
//! Take the previous `Config` example. If `Config` grows a big
//! `Vec<Worker>` field that `worker_count` doesn't read, the default
//! `&Config` form still snapshots the whole struct into the cache slot
//! and every cache-hit check compares the whole thing.
//!
//! `#[derive(drv::Input)]` lets you declare a lightweight view that
//! borrows only the fields the memo actually depends on. The derive
//! auto-generates the owned snapshot and the machinery `#[drv::memo]`
//! uses internally:
//!
//! ```rust
//! # #[derive(Debug, Clone, PartialEq, Default)]
//! # pub struct Scoreboard {
//! #     pub hits: Vec<u32>,
//! #     pub player_x: u32,
//! # }
//! #[derive(drv::Input)]
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
//! # Nested inputs
//!
//! A `#[derive(drv::Input)]` struct can have another
//! `#[derive(drv::Input)]` struct as a field — useful for bundling a
//! handful of sub-projections into one memo parameter:
//!
//! ```rust
//! # #[derive(Default)]
//! # pub struct App { pub a: Vec<u32>, pub b: Vec<u32> }
//! #[derive(drv::Input)]
//! struct ChildA<'a> { pub a: &'a Vec<u32> }
//!
//! #[derive(drv::Input)]
//! struct ChildB<'a> { pub b: &'a Vec<u32> }
//!
//! #[derive(drv::Input)]
//! struct Both<'a> {
//!     pub ca: ChildA<'a>,
//!     pub cb: ChildB<'a>,
//! }
//!
//! #[drv::memo(single)]
//! fn sum_both<'a>(input: Both<'a>) -> u32 {
//!     input.ca.a.iter().sum::<u32>() + input.cb.b.iter().sum::<u32>()
//! }
//! ```
//!
//! `Vec`, `Option`, tuples, `HashMap` / `BTreeMap` values, and arrays
//! are all recursive — a field type like `Vec<MyInput<'a>>` or
//! `HashMap<String, MyInput<'a>>` works without any extra annotation.
//! The derive handles generic type parameters, tuple structs, and unit
//! structs as well.
//!
//! # Performance
//!
//! Two sources of per-call work:
//!
//! 1. **Per-field equality check** on cache-hit lookup.
//! 2. **Output `Clone`** on every return.
//!
//! drv's `ToStatic` impls for `Arc<T>` and (under the `imbl` feature)
//! `imbl`'s persistent collections take a pointer-equality fast path —
//! O(1) when the field hasn't been mutated since the last miss.
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
//! drv = { version = "0.4", features = ["imbl"] }
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

/// Convert a memo input to its `'static` cache-storage form.
///
/// Every type passed to a `#[drv::memo]` must implement `ToStatic`. drv
/// ships impls for primitives, `String`, `Arc<T>`, `&T` (for any `T:
/// ToStatic`), `str`, and (under `feature = "imbl"`) imbl's persistent
/// collections. Container types — `Vec<T>`, `[T]`, `[T; N]`,
/// `Option<T>`, tuples up to arity 5, `HashMap<K, V>`, `HashSet<K>`,
/// `BTreeMap<K, V>`, `BTreeSet<K>` — are *recursive*: they implement
/// `ToStatic` whenever their element types do, so things like
/// `Vec<MyInput<'a>>` and `HashMap<String, MyInput<'a>>` work. User
/// types get `ToStatic` via `#[derive(drv::Input)]`.
///
/// `Arc<T>` and imbl collections take a pointer-equality fast path in
/// `eq_static`: if the stored snapshot and the current input share a
/// root pointer, the equality check short-circuits without walking the
/// contents. Because the recursive container impls go through each
/// element's own `eq_static`, this fast path is preserved inside
/// containers too (e.g. `Vec<Arc<T>>` benefits per element).
///
/// # Why "ToStatic"?
///
/// The memo cache lives in a `thread_local!`, which requires its
/// contents to be `'static`. An input like `MyInput<'a>` can't sit there
/// directly — it borrows. `ToStatic::to_static` converts any input into
/// a sibling `'static` form (the associated `Static` type) that *can*
/// live in the cache. The name reads as "to the `'static` version of
/// self," mirroring `ToOwned` (which this trait subsumes and generalises
/// for nested inputs).
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as a `#[drv::memo]` input: no `drv::ToStatic` impl",
    label = "type needs a `drv::ToStatic` impl",
    note = "for user structs, add `#[derive(drv::Input)]`; for ecosystem types, drv may need a shipped impl"
)]
pub trait ToStatic {
    /// The `'static` owned form used as the cache-slot storage type.
    type Static: 'static;

    /// Build the cache-storage snapshot.
    fn to_static(&self) -> Self::Static;

    /// Compare self against a stored snapshot.
    fn eq_static(&self, other: &Self::Static) -> bool;
}

// ────────────────────────────────────────────────────────────────────
// Reference blanket: &T for any T: ToStatic.
// Delegates to T's impl so specialized behavior (Arc ptr_eq, imbl
// ptr_eq) is preserved through a reference.
// ────────────────────────────────────────────────────────────────────

impl<T: ?Sized + ToStatic> ToStatic for &T {
    type Static = T::Static;
    fn to_static(&self) -> T::Static {
        <T as ToStatic>::to_static(self)
    }
    fn eq_static(&self, other: &T::Static) -> bool {
        <T as ToStatic>::eq_static(self, other)
    }
}

// ────────────────────────────────────────────────────────────────────
// Primitives — Static = Self.
// ────────────────────────────────────────────────────────────────────

macro_rules! impl_identity_copy {
    ($($t:ty),* $(,)?) => {
        $(
            impl ToStatic for $t {
                type Static = $t;
                fn to_static(&self) -> $t { *self }
                fn eq_static(&self, other: &$t) -> bool { self == other }
            }
        )*
    };
}

impl_identity_copy!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64, bool, char,
);

// ────────────────────────────────────────────────────────────────────
// String / str.
// ────────────────────────────────────────────────────────────────────

impl ToStatic for String {
    type Static = String;
    fn to_static(&self) -> String {
        self.clone()
    }
    fn eq_static(&self, other: &String) -> bool {
        self == other
    }
}

impl ToStatic for str {
    type Static = String;
    fn to_static(&self) -> String {
        self.to_owned()
    }
    fn eq_static(&self, other: &String) -> bool {
        self == other.as_str()
    }
}

// ────────────────────────────────────────────────────────────────────
// Vec<T> / [T] / [T; N] — recursive so `Vec<MyInput<'a>>` and friends
// work. For plain element types (`u32`, `Arc<T>`, etc.) where
// `T::Static = T`, this reduces to the identity behavior.
// ────────────────────────────────────────────────────────────────────

impl<T: ToStatic> ToStatic for Vec<T> {
    type Static = Vec<T::Static>;
    fn to_static(&self) -> Vec<T::Static> {
        self.iter().map(T::to_static).collect()
    }
    fn eq_static(&self, other: &Vec<T::Static>) -> bool {
        self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a.eq_static(b))
    }
}

impl<T: ToStatic> ToStatic for [T] {
    type Static = Vec<T::Static>;
    fn to_static(&self) -> Vec<T::Static> {
        self.iter().map(T::to_static).collect()
    }
    fn eq_static(&self, other: &Vec<T::Static>) -> bool {
        self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a.eq_static(b))
    }
}

impl<T: ToStatic, const N: usize> ToStatic for [T; N] {
    type Static = [T::Static; N];
    fn to_static(&self) -> [T::Static; N] {
        std::array::from_fn(|i| self[i].to_static())
    }
    fn eq_static(&self, other: &[T::Static; N]) -> bool {
        (0..N).all(|i| self[i].eq_static(&other[i]))
    }
}

// ────────────────────────────────────────────────────────────────────
// std collections.
// ────────────────────────────────────────────────────────────────────

// Recursive so `HashMap<String, MyInput<'a>>` and friends work. Lookups
// in `eq_static` go via `K::Static: Borrow<K>`, so no key allocation on
// the cache-hit path for the common cases (`K::Static = K`).

impl<K, V> ToStatic for std::collections::HashMap<K, V>
where
    K: ToStatic + Eq + std::hash::Hash,
    V: ToStatic,
    K::Static: Eq + std::hash::Hash + std::borrow::Borrow<K>,
{
    type Static = std::collections::HashMap<K::Static, V::Static>;
    fn to_static(&self) -> Self::Static {
        self.iter()
            .map(|(k, v)| (k.to_static(), v.to_static()))
            .collect()
    }
    fn eq_static(&self, other: &Self::Static) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.iter()
            .all(|(k, v)| other.get(k).is_some_and(|ov| v.eq_static(ov)))
    }
}

impl<K> ToStatic for std::collections::HashSet<K>
where
    K: ToStatic + Eq + std::hash::Hash,
    K::Static: Eq + std::hash::Hash + std::borrow::Borrow<K>,
{
    type Static = std::collections::HashSet<K::Static>;
    fn to_static(&self) -> Self::Static {
        self.iter().map(K::to_static).collect()
    }
    fn eq_static(&self, other: &Self::Static) -> bool {
        self.len() == other.len() && self.iter().all(|k| other.contains(k))
    }
}

impl<K, V> ToStatic for std::collections::BTreeMap<K, V>
where
    K: ToStatic + Ord,
    V: ToStatic,
    K::Static: Ord + std::borrow::Borrow<K>,
{
    type Static = std::collections::BTreeMap<K::Static, V::Static>;
    fn to_static(&self) -> Self::Static {
        self.iter()
            .map(|(k, v)| (k.to_static(), v.to_static()))
            .collect()
    }
    fn eq_static(&self, other: &Self::Static) -> bool {
        if self.len() != other.len() {
            return false;
        }
        // BTreeMap iteration is sorted; zip-compare avoids per-key lookups.
        self.iter().zip(other.iter()).all(|((k1, v1), (k2, v2))| {
            <K::Static as std::borrow::Borrow<K>>::borrow(k2) == k1 && v1.eq_static(v2)
        })
    }
}

impl<K> ToStatic for std::collections::BTreeSet<K>
where
    K: ToStatic + Ord,
    K::Static: Ord + std::borrow::Borrow<K>,
{
    type Static = std::collections::BTreeSet<K::Static>;
    fn to_static(&self) -> Self::Static {
        self.iter().map(K::to_static).collect()
    }
    fn eq_static(&self, other: &Self::Static) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other.iter())
                .all(|(a, b)| <K::Static as std::borrow::Borrow<K>>::borrow(b) == a)
    }
}

// ────────────────────────────────────────────────────────────────────
// Option — recursive: Static = Option<T::Static>.
// ────────────────────────────────────────────────────────────────────

impl<T: ToStatic> ToStatic for Option<T> {
    type Static = Option<T::Static>;
    fn to_static(&self) -> Option<T::Static> {
        self.as_ref().map(T::to_static)
    }
    fn eq_static(&self, other: &Option<T::Static>) -> bool {
        match (self, other) {
            (Some(a), Some(b)) => a.eq_static(b),
            (None, None) => true,
            _ => false,
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// Tuples — recursive.
// ────────────────────────────────────────────────────────────────────

macro_rules! impl_tuple_static {
    ($( ( $( $idx:tt : $ty:ident ),+ ) ),+ $(,)?) => {
        $(
            impl<$($ty: ToStatic),+> ToStatic for ($($ty,)+) {
                type Static = ($($ty::Static,)+);
                fn to_static(&self) -> Self::Static {
                    ($( self.$idx.to_static(), )+)
                }
                fn eq_static(&self, other: &Self::Static) -> bool {
                    $( self.$idx.eq_static(&other.$idx) )&&+
                }
            }
        )+
    };
}

impl_tuple_static! {
    (0: T0),
    (0: T0, 1: T1),
    (0: T0, 1: T1, 2: T2),
    (0: T0, 1: T1, 2: T2, 3: T3),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4),
}

// ────────────────────────────────────────────────────────────────────
// Arc<T> — ptr_eq fast path in eq_static.
// ────────────────────────────────────────────────────────────────────

impl<T: ?Sized + PartialEq + 'static> ToStatic for std::sync::Arc<T> {
    type Static = std::sync::Arc<T>;
    fn to_static(&self) -> std::sync::Arc<T> {
        std::sync::Arc::clone(self)
    }
    fn eq_static(&self, other: &std::sync::Arc<T>) -> bool {
        std::sync::Arc::ptr_eq(self, other) || **self == **other
    }
}

// ────────────────────────────────────────────────────────────────────
// imbl — feature-gated, with ptr_eq fast path.
// ────────────────────────────────────────────────────────────────────

#[cfg(feature = "imbl")]
mod imbl_impls {
    use super::ToStatic;
    use imbl::shared_ptr::SharedPointerKind;
    use imbl::{GenericHashMap, GenericHashSet, GenericVector};
    use std::hash::{BuildHasher, Hash};

    impl<T, P> ToStatic for GenericVector<T, P>
    where
        T: Clone + PartialEq + 'static,
        P: SharedPointerKind + 'static,
    {
        type Static = GenericVector<T, P>;
        fn to_static(&self) -> Self::Static {
            self.clone()
        }
        fn eq_static(&self, other: &Self::Static) -> bool {
            self.ptr_eq(other) || self == other
        }
    }

    impl<K, V, S, P> ToStatic for GenericHashMap<K, V, S, P>
    where
        K: Hash + Eq + Clone + 'static,
        V: PartialEq + Clone + 'static,
        S: BuildHasher + Clone + 'static,
        P: SharedPointerKind + 'static,
    {
        type Static = GenericHashMap<K, V, S, P>;
        fn to_static(&self) -> Self::Static {
            self.clone()
        }
        fn eq_static(&self, other: &Self::Static) -> bool {
            self.ptr_eq(other) || self == other
        }
    }

    impl<K, V> ToStatic for imbl::OrdMap<K, V>
    where
        K: Ord + Clone + 'static,
        V: PartialEq + Clone + 'static,
    {
        type Static = imbl::OrdMap<K, V>;
        fn to_static(&self) -> Self::Static {
            self.clone()
        }
        fn eq_static(&self, other: &Self::Static) -> bool {
            self.ptr_eq(other) || self == other
        }
    }

    impl<T, S, P> ToStatic for GenericHashSet<T, S, P>
    where
        T: Hash + Eq + Clone + 'static,
        S: BuildHasher + Clone + 'static,
        P: SharedPointerKind + 'static,
    {
        type Static = GenericHashSet<T, S, P>;
        fn to_static(&self) -> Self::Static {
            self.clone()
        }
        fn eq_static(&self, other: &Self::Static) -> bool {
            self.ptr_eq(other) || self == other
        }
    }

    impl<T: Ord + Clone + 'static> ToStatic for imbl::OrdSet<T> {
        type Static = imbl::OrdSet<T>;
        fn to_static(&self) -> Self::Static {
            self.clone()
        }
        fn eq_static(&self, other: &Self::Static) -> bool {
            self.ptr_eq(other) || self == other
        }
    }
}

/// Declare a struct as a memo input.
///
/// Generates a `ToStatic` impl (producing a hidden `__DrvMyStruct`
/// shadow struct as the cache-storage form) and nothing else. Composes
/// with other derives — `Clone`, `PartialEq`, `Debug`, etc. — without
/// interference.
///
/// Works for three shapes:
///
/// - Plain owned structs: `#[derive(drv::Input)] struct Config { threads: u32 }`.
///   Use as `&Config` in a memo.
/// - Borrowed projections: `#[derive(drv::Input)] struct MyInput<'a> { xs: &'a Vec<u32> }`.
///   Use by value in a memo.
/// - Nested bundles: a projection whose fields are themselves
///   `#[derive(drv::Input)]` projections.
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
/// #[derive(drv::Input)]
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
pub use drv_macros::Input;

/// Memoize a function.
///
/// Every memo must declare a cache strategy:
///
/// - `#[drv::memo(single)]` — one slot, last-call caching.
/// - `#[drv::memo(lru = N)]` — N slots, least-recently-used eviction.
///
/// # Parameters
///
/// Every parameter must implement [`ToStatic`]. Primitives, std
/// collections, `Arc<T>`, references to any `T: ToStatic`, and user
/// types with `#[derive(drv::Input)]` are supported out of the box.
///
/// # Example
///
/// ```rust
/// #[derive(Debug, Clone, PartialEq, Default)]
/// pub struct Stats { pub xs: Vec<u32> }
///
/// #[derive(drv::Input)]
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

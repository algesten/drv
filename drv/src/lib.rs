#![forbid(unsafe_code)]
//! Memoized derivations over plain Rust structs.
//!
//! `drv` lets you declare a struct of ground-truth data (an **atom**), project
//! subsets of its fields (a **lens**), and compute derived values (a **memo**)
//! that are automatically cached. When nothing changed, nothing recomputes.
//!
//! # Quick example
//!
//! ```rust
//! #[drv::atom]
//! // Clone, PartialEq, Debug, Default are auto-generated on atoms.
//! pub struct Scoreboard {
//!     #[drv::lens(TotalLens)]
//!     pub hits: Vec<u32>,
//!
//!     pub player_x: u32,
//!     pub player_y: u32,
//!     pub time_ms: u64,
//! }
//!
//! #[drv::memo]
//! fn total_score(lens: &TotalLens) -> u32 {
//!     lens.hits.iter().sum()
//! }
//!
//! drv::assemble!();
//!
//! # fn main() {
//! let mut game = Scoreboard {
//!     hits: vec![100, 250, 50],
//!
//!     // Atoms have a hidden __drv field for caching lenses
//!     ..Default::default()
//! };
//!
//! let score = total_score(&game);     // computes: 400
//! game.player_x = 42;                  // irrelevant to the lens
//! let score = total_score(&game);     // cache hit, no work
//! # }
//! ```
//!
//! Moving the player? Free. `total_score` only recomputes when `hits` change.
//!
//! # The three pieces
//!
//! **Atom** — a struct of ground-truth data. Declared with `#[drv::atom]`.
//! Field types must implement `PartialEq + Clone + Debug + Default + Send`.
//! Fields can be any visibility — standard Rust rules apply, so a private
//! field can only be projected into a lens in the same module.
//!
//! **Lens** — a projection: a subset of an atom's fields, by name and type.
//! It declares "this computation depends on exactly these fields and no others."
//!
//! **Memo** — a pure function from a lens (or an atom directly) to an output.
//! Annotated with `#[drv::memo]`. The result is cached and only recomputed when
//! the input fields change.
//!
//! At the end of your crate, `drv::assemble!()` stitches everything together.
//!
//! # Declaring lenses
//!
//! There are three ways to declare a lens.
//!
//! In all generated lenses, built-in scalar primitives (`u32`, `bool`, `f64`,
//! `usize`, etc.) are stored **by value** — `lens.x` gives `u32` directly, no
//! dereference needed. All other types are borrowed as `&T`. User-defined
//! `Copy` types are *not* auto-detected (the proc macro can only recognise
//! language built-ins); for full control over field representation, declare
//! the projection explicitly with `#[drv::proj]`.
//!
//! ## Inline: annotate fields on the atom
//!
//! Tag fields with `#[drv::lens(Name)]` directly on the atom. The macro generates
//! a lens called `Name` with those fields. This keeps the full dependency picture
//! in one place:
//!
//! ```rust
//! #[drv::atom]
//! pub struct AppState {
//!     #[drv::lens(TotalLens, StatusLens)]
//!     pub items: Vec<u32>,
//!
//!     #[drv::lens(StatusLens)]
//!     pub selected: Option<usize>,
//!
//!     #[drv::lens(Render)]
//!     pub viewport_rows: u32,
//!
//!     pub cursor_col: u32,
//! }
//! # drv::assemble!();
//! # fn main() {}
//! ```
//!
//! This generates three lenses: `TotalLens { items }`, `StatusLens { items, selected }`,
//! and `Render { viewport_rows }`. A field can appear in multiple lenses —
//! list them in one attribute `#[drv::lens(A, B)]` or as separate attributes.
//!
//! ## Standalone: declare a separate struct
//!
//! Declare the lens as its own struct with `#[drv::lens(Atom)]`. The macro verifies
//! that every field name and type matches the atom:
//!
//! ```rust
//! # #[drv::atom]
//! # pub struct AppState {
//! #     pub items: Vec<u32>,
//! #     pub viewport_rows: u32,
//! # }
//! #[drv::lens(AppState)]
//! struct MyLens<'a> {
//!     pub items: &'a Vec<u32>,
//!     pub viewport_rows: u32,
//! }
//! # drv::assemble!();
//! # fn main() {}
//! ```
//!
//! Only built-in primitive Copy types (`u8`..`u128`, `i8`..`i128`, `usize`, `isize`, `f32`,
//! `f64`, `bool`, `char`) may appear by value — any other type must be written
//! as `&'a T`. The lens struct must be valid Rust on its own (the
//! `#[drv::lens(...)]` attribute is a no-op when stripped), so any reference
//! field requires a lifetime parameter declared on the struct. If you need a
//! clone (or a projection into a nested struct, or a different type), write
//! a custom projection with `#[drv::proj]` instead.
//!
//! Use standalone lenses when the lens is defined closer to the memo that consumes
//! it, or when the atom is in another module and you don't want to modify it.
//!
//! ## Custom projection with `#[drv::proj]`
//!
//! When you need lens fields that don't match the atom — different names, different
//! types, or reaching into nested structs — write the projection yourself. You declare
//! the lens struct with `#[drv::lens(Atom)]` and annotate the `From` impl with
//! `#[drv::proj]`:
//!
//! ```rust
//! #[derive(Debug, Clone, PartialEq, Default)]
//! pub struct Inner {
//!     pub x: u32,
//!     pub label: String,
//! }
//!
//! #[drv::atom]
//! pub struct Container {
//!     pub inner: Inner,
//!     pub name: String,
//!     pub count: u32,
//! }
//!
//! // Fields don't match the atom — we provide our own projection.
//! #[drv::lens(Container)]
//! struct ProjectedLens<'a> {
//!     pub x: u32,            // owned copy of a nested field
//!     pub name: &'a str,     // borrow &str from a String field
//! }
//!
//! // The projection function. drv::proj injects the cache reference.
//! #[drv::proj]
//! impl<'a> From<&'a Container> for ProjectedLens<'a> {
//!     fn from(v: &'a Container) -> Self {
//!         Self {
//!             x: v.inner.x,
//!             name: &v.name,
//!         }
//!     }
//! }
//!
//! #[drv::memo]
//! fn display(lens: &ProjectedLens) -> String {
//!     format!("{}={}", lens.name, lens.x)
//! }
//!
//! # drv::assemble!();
//! # fn main() {
//! let c = Container {
//!     inner: Inner { x: 42, label: "ignored".into() },
//!     name: "hello".into(),
//!     ..Default::default()
//! };
//! assert_eq!(display(&c), "hello=42");
//! # }
//! ```
//!
//! The macro switches to this mode automatically when the lens struct's fields don't
//! match the atom. It keeps your struct definition (adding only a hidden `__drv`
//! field), generates the snapshot and comparison logic, and rewrites your `From` impl
//! to inject the cache reference. Lenses with a `#[drv::proj]` impl require a lifetime
//! parameter on the struct (for the cache reference).
//!
//! They work identically with memos — cache hits, misses, multi-lens parameters,
//! and value parameters all behave the same as standard lenses.
//!
//! # Calling memos
//!
//! `#[drv::memo]` generates a free function with the same name. The function
//! body reads from the lens; the generated wrapper handles memoization. You
//! call it with `&atom` — the macro auto-converts into the right lens:
//!
//! ```rust
//! #[drv::atom]
//! pub struct AppState {
//!     #[drv::lens(CountLens)]
//!     pub items: Vec<u32>,
//!     pub viewport_rows: u32,
//! }
//!
//! #[drv::memo]
//! fn item_count(lens: &CountLens) -> usize {
//!     lens.items.len()
//! }
//!
//! # drv::assemble!();
//! # fn main() {
//! let mut app = AppState {
//!     items: vec![10, 20, 30],
//!     ..Default::default()
//! };
//!
//! let n = item_count(&app);   // pass &AppState directly — converts to CountLens
//! assert_eq!(n, 3);
//! # }
//! ```
//!
//! Memoization happens behind the scenes — no cache struct, no setup.
//!
//! # Using an atom directly
//!
//! A memo can take the atom itself as input — treated as an "identity lens" over
//! all data fields:
//!
//! ```rust
//! #[drv::atom]
//! pub struct Stats {
//!     pub total: u32,
//!     pub count: u32,
//! }
//!
//! #[drv::memo]
//! fn average(s: &Stats) -> u32 {
//!     if s.count == 0 { 0 } else { s.total / s.count }
//! }
//! # drv::assemble!();
//! # fn main() {
//! # let s = Stats { total: 100, count: 4, ..Default::default() };
//! # assert_eq!(average(&s), 25);
//! # }
//! ```
//!
//! Any change to any field of the atom invalidates the cache. Useful when you
//! really do depend on everything.
//!
//! # Multiple lenses
//!
//! A memo can take lenses from multiple atoms. The cache lives in the first
//! parameter's atom:
//!
//! ```rust
//! #[drv::atom]
//! pub struct Game {
//!     #[drv::lens(HitsLens)]
//!     pub hits: Vec<u32>,
//! }
//!
//! #[drv::atom]
//! pub struct Settings {
//!     #[drv::lens(MultiplierLens)]
//!     pub multiplier: u32,
//! }
//!
//! #[drv::memo]
//! fn weighted_score(hits: &HitsLens, settings: &MultiplierLens) -> u32 {
//!     hits.hits.iter().sum::<u32>() * settings.multiplier
//! }
//!
//! # drv::assemble!();
//! # fn main() {
//! # let game = Game { hits: vec![10, 20, 30], ..Default::default() };
//! # let settings = Settings { multiplier: 2, ..Default::default() };
//! let score = weighted_score(&game, &settings);   // cache stored in `game`
//! # assert_eq!(score, 120);
//! # }
//! ```
//!
//! If any of the lens field comparisons fail, the memo recomputes. Two lenses from
//! the same atom (`fn foo(a: &LensA, b: &LensB)` called as `foo(&app, &app)`)
//! works too.
//!
//! # Value parameters
//!
//! Memos can also take owned values or borrowed references to types like
//! `&str` or `&[u8]` alongside lens parameters. Values participate in the
//! cache key just like lens fields — any change triggers a recompute.
//!
//! ```rust
//! # #[drv::atom]
//! # pub struct Stats {
//! #     #[drv::lens(CountLens)]
//! #     pub count: u32,
//! # }
//! #[drv::memo]
//! fn labeled(lens: &CountLens, label: &str, multiplier: u32) -> String {
//!     format!("{}={}", label, lens.count * multiplier)
//! }
//! # drv::assemble!();
//! # fn main() {
//! # let s = Stats { count: 3, ..Default::default() };
//! # assert_eq!(labeled(&s, "x", 2), "x=6");
//! # }
//! ```
//!
//! Parameter classification:
//!
//! - `&Lens` or `&Atom` — a lens parameter (required: at least one lens).
//! - Owned types (`u32`, `String`, `MyStruct`) — stored via `Clone`.
//! - Borrowed types with `ToOwned` (`&str`, `&[u8]`, `&Path`, ...) — stored
//!   as `<T as ToOwned>::Owned` (so `&str` stores as `String`).
//!
//! Value types must implement `PartialEq + Clone + Send + 'static`; borrowed
//! value types must satisfy `T: ToOwned` with the owned form matching the
//! usual bounds. Declared order is preserved at the call site; the cache is
//! always stored on the first lens parameter's atom.
//!
//! # Chaining
//!
//! A memo's output can feed into another memo. Mark the output type as an atom too.
//! When constructing it inside the memo body, close with `..Default::default()` so
//! `drv` can set up its internal state:
//!
//! ```rust
//! // Root atom
//! #[drv::atom]
//! pub struct Game {
//!     pub hits: Vec<u32>,
//!     pub time_ms: u64,
//! }
//!
//! // Lens over Game → produces Stats (itself an atom).
//! #[drv::lens(Game)]
//! struct HitsLens<'a> {
//!     pub hits: &'a Vec<u32>,
//! }
//!
//! #[drv::memo]
//! fn stats(lens: &HitsLens) -> Stats {
//!     Stats {
//!         total: lens.hits.iter().sum(),
//!         count: lens.hits.len() as u32,
//!         best: lens.hits.iter().copied().max().unwrap_or(0),
//!         ..Default::default()   // lets drv initialize its internal state
//!     }
//! }
//!
//! // Stats is an atom, so more lenses can project from it.
//! #[drv::atom]
//! pub struct Stats {
//!     #[drv::lens(AverageLens)]
//!     pub total: u32,
//!
//!     #[drv::lens(AverageLens)]
//!     pub count: u32,
//!
//!     pub best: u32,
//! }
//!
//! #[drv::memo]
//! fn average(lens: &AverageLens) -> u32 {
//!     if lens.count == 0 { 0 } else { lens.total / lens.count }
//! }
//! # drv::assemble!();
//! # fn main() {
//! # let game = Game { hits: vec![10, 20, 30], ..Default::default() };
//! # let s = stats(&game);
//! # assert_eq!(average(&s), 20);
//! # }
//! ```
//!
//! Each link is independently memoized. If `hits` didn't change, `stats` returns
//! cached, `average` sees the same input, also returns cached. Zero work.
//!
//! ## Conceptual chaining
//!
//! ```text
//!                                   ┌──────────────────────────┐
//!                                   │      DerivedOutputB      │
//!                                   │                          │
//!                                   └──────────────────────────┘
//!                                                 ▲
//!                                                 │
//!                                                 │
//!                                       transform_function_3
//!                                              (memo)
//!                                                 ▲
//!                                                 │
//!                                                 │
//!                                        ┌ ─ ─ ─ ─ ─ ─ ─ ─
//!                                          PartialDerived │
//!                                        │     (lens)
//!                                         ─ ─ ─ ─ ─ ─ ─ ─ ┘
//!                                                 ▲
//!                                                 │
//! ┌──────────────────────────┐      ┌──────────────────────────┐
//! │      DerivedOutputA      │      │    OtherDerivedState     │
//! │                          │      │          (atom)          │
//! └──────────────────────────┘      └──────────────────────────┘
//!               ▲                                 ▲
//!               │                                 │
//!     transform_function_1              transform_function_2
//!            (memo)                            (memo)
//!               ▲                                 ▲
//!               │                                 │
//!               │                  ┌──────────────┴──────────────┐
//!               │                  │                             │
//!               │                  │                             │
//!       ┌ ─ ─ ─ ─ ─ ─ ─ ┐  ┌ ─ ─ ─ ─ ─ ─ ─ ┐         ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
//!         PartialState1      PartialState2              PartialOtherState   │
//!       │    (lens)     │  │    (lens)     │         │        (lens)
//!        ─ ─ ─ ─ ─ ─ ─ ─    ─ ─ ─ ─ ─ ─ ─ ─           ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┘
//!               ▲                  ▲                             ▲
//!               └──────────┬───────┘                             │
//!                          │                                     │
//!            ┌──────────────────────────┐          ┌──────────────────────────┐
//!            │        SomeState         │          │        OtherState        │
//!            │          (atom)          │          │          (atom)          │
//!            └──────────────────────────┘          └──────────────────────────┘
//! ```
//!
//! # Choosing field types
//!
//! Atom fields must implement `PartialEq + Clone + Debug + Default + Send`.
//!
//! ## What runs when
//!
//! Constructing a lens from an atom is nearly free — built-in primitives are
//! copied (trivial), other fields are borrowed by reference. What runs on
//! each call:
//!
//! - **Every call — `PartialEq` on each lens field.** Used to check whether
//!   the input has changed since the last call.
//! - **Every call — `Clone` on the output.** Memos return by value.
//! - **Cache miss only — `Clone` on each lens field.** A copy is kept for the
//!   next call's comparison.
//! - **Cache miss only — the memo body runs.**
//!
//! Two kinds of costs matter on the hot path:
//!
//! 1. Per-field `PartialEq` cost.
//! 2. Output `Clone` cost.
//!
//! For `PartialEq`, the field type matters. Scalars are free. Collections
//! (both `Vec`/`HashMap` and `imbl::Vector`/`imbl::HashMap`) iterate and
//! compare pairwise — O(n) worst case on a cache hit. `imbl` does not
//! short-circuit via pointer equality; its advantage is O(1) `Clone` on
//! cache miss, not faster comparison. For output `Clone`, wrap expensive
//! outputs in `Rc`/`Arc` for O(1) cloning.
//!
//! **Principle:** choose types where `PartialEq` is proportional to the amount of
//! change, not the total size of the data.
//!
//! ## Scalars: free
//!
//! `u32`, `bool`, `f64`, small enums — comparison and clone are one or a few
//! machine instructions. No special consideration needed.
//!
//! ## Small owned types: fine
//!
//! `String`, `PathBuf`, small `Vec<T>` with a handful of elements — comparison
//! is O(n) but n is small. Perfectly fine.
//!
//! ## Large collections: use `imbl` or `Arc`
//!
//! An atom tracking 50 open buffers in a `HashMap<Path, Buffer>` compares the
//! entire map on every access — O(n). Every memoized call pays full traversal
//! cost, defeating the point.
//!
//! The fix is a type with **O(1) `Clone` and O(1) equality when the value
//! hasn't been mutated**. `drv` recognises two families automatically and
//! short-circuits the cache check with a pointer compare:
//!
//! - **`Arc<T>`** — works out of the box, no feature flag needed.
//! - **[`imbl`](https://docs.rs/imbl) persistent collections** — enable the
//!   `imbl` feature. Covers `Vector`, `HashMap`, `OrdMap`, `HashSet`, `OrdSet`.
//!
//! ```toml
//! [dependencies]
//! drv = { version = "0.1", features = ["imbl"] }
//! ```
//!
//! With the feature on, a 10,000-entry `imbl::HashMap` that hasn't been
//! touched since the last memo call compares in **constant time**. If it was
//! mutated, the check falls back to element-wise `==`, which itself
//! short-circuits on the first difference. There is no scenario where you
//! pay *more* than plain `Vec`/`HashMap` would.
//!
//! | Type | Clone | Cache hit (same pointer) | Cache hit (equal contents) | Mutation |
//! |------|-------|--------------------------|----------------------------|----------|
//! | `Vec<T>` | O(n) | O(n) | O(n) | O(1) amortized |
//! | `HashMap<K,V>` | O(n) | O(n) | O(n) | O(1) amortized |
//! | `Arc<T>` | **O(1)** | **O(1)** | O(eq of T) | n/a |
//! | `imbl::Vector<T>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |
//! | `imbl::HashMap<K,V>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |
//!
//! *"Same pointer"* is the common case: the atom cloned its field into the
//! snapshot on the last cache miss, and nothing has mutated it since — so the
//! two point at the same underlying node. *"Equal contents"* is the worst case,
//! when the collection was rebuilt from scratch but happens to compare equal.
//!
//! ## Rule of thumb
//!
//! - **< 50 elements?** `Vec`/`HashMap` are fine.
//! - **50+ or expensive-to-compare elements?** Use `imbl` with the
//!   `imbl` feature — you get O(1) cache-hit comparison for free.
//! - **Single large blob of data you never mutate piece-wise?** Wrap it in
//!   `Arc<T>`; `drv` uses `Arc::ptr_eq` automatically.
//!
//! ## Example
//!
//! ```rust
//! use imbl::{Vector, HashMap};
//! # #[derive(Debug, Clone, PartialEq, Default)]
//! # pub struct Buffer;
//!
//! #[drv::atom]
//! pub struct AppState {
//!     pub active_tab: Option<String>,       // scalar — trivial
//!     pub viewport_rows: u32,               // scalar — trivial
//!     pub show_sidebar: bool,               // scalar — trivial
//!
//!     pub tabs: Vector<String>,             // persistent — O(1) clone, O(n) eq
//!     pub buffers: HashMap<String, Buffer>, // persistent — O(1) clone, O(n) eq
//! }
//! # drv::assemble!();
//! # fn main() {}
//! ```
//!
//! # Multiple atoms, multiple crates
//!
//! Each atom and its memos must live in the same crate. This is by design —
//! `drv::assemble!()` collects everything within a single compilation unit.
//!
//! For larger applications, split your state into domain-specific atoms in
//! separate crates:
//!
//! ```text
//! crate: my-app-buffers   → BufferState atom + buffer memos
//! crate: my-app-ui        → UiState atom + ui memos
//! crate: my-app-lsp       → LspState atom + lsp memos
//! crate: my-app           → composes the above
//! ```
//!
//! Each crate is self-contained. The top-level crate can define its own atoms
//! that compose fields from the domain crates, with its own lenses and memos
//! over the combined state.
//!
//! # Assembly
//!
//! `drv::assemble!()` must appear once, after all `#[drv::atom]`, `#[drv::lens]`,
//! and `#[drv::memo]` declarations in the crate. It collects all registrations
//! and emits the cache types, the lens types, and the memoized free functions.
//!
//! ```text
//! // lib.rs
//! mod state;       // atoms
//! mod views;       // lenses + memos
//! mod rendering;   // more lenses + memos
//!
//! drv::assemble!();
//! ```
//!
//! # Design goals
//!
//! - **Plain Rust structs.** Atoms and lenses are regular structs you can construct
//!   with literal syntax, clone, compare, debug.
//! - **Static dependency declaration.** The lens struct _is_ the dependency list.
//!   The compiler verifies field names and types at compile time.
//! - **Zero runtime tracking.** No proxy objects, no access instrumentation, no
//!   subscription management. Just field-by-field `PartialEq`.
//! - **Memoization is automatic.** No cache struct to construct, no explicit
//!   setup — just call the memo.
//! - **Free functions, not methods.** Memos are ordinary functions. Call sites
//!   don't need to know any generated type names.

use std::cell::RefCell;

/// Trait implemented by `#[drv::atom]` structs. The associated `State` type
/// is emitted by `drv::assemble!()` and contains one `Option<snapshot>` +
/// `Option<output>` pair per memo targeting this atom.
pub trait Atom {
    type State: Default + Send + 'static;
}

/// Stack-allocated memoization cache, embedded in atom structs.
///
/// Uses interior mutability so lenses can borrow `&self` instead of `&mut self`.
/// The `State` type is supplied by the atom's `Atom` impl, letting the cache
/// layout be fully known at compile time — no heap allocation, no type erasure.
///
/// Transparent trait impls so derives on atom structs work correctly:
/// - `PartialEq`: always `true` (cache state doesn't affect equality)
/// - `Clone`: clones to a fresh empty cache
/// - `Default`: empty cache
/// - `Hash`: no-op
/// - `Debug`: prints `Cache(..)`
pub struct Cache<A: Atom> {
    #[doc(hidden)]
    pub inner: RefCell<A::State>,
}

impl<A: Atom> Cache<A> {
    pub fn new() -> Self {
        Cache {
            inner: RefCell::new(A::State::default()),
        }
    }
}

impl<A: Atom> Default for Cache<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Atom> Clone for Cache<A> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl<A: Atom> PartialEq for Cache<A> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<A: Atom> Eq for Cache<A> {}

impl<A: Atom> std::fmt::Debug for Cache<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Cache(..)")
    }
}

impl<A: Atom> std::hash::Hash for Cache<A> {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl<A: Atom> PartialOrd for Cache<A> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<A: Atom> Ord for Cache<A> {
    fn cmp(&self, _: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

/// Internal helper used by generated code to pick between a pointer-equality
/// fast path (for `Arc<T>` and, under the `imbl` feature, `imbl`'s persistent
/// collections) and the regular `PartialEq` path. Not intended for direct
/// use — `drv` wires this into `#[drv::atom]` and `#[drv::memo]`
/// automatically.
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

// Re-export proc macros.
pub use drv_macros::assemble;
pub use drv_macros::atom;
pub use drv_macros::lens;
pub use drv_macros::memo;
pub use drv_macros::proj;

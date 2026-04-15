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
//! All fields must be `pub` and must implement `PartialEq + Clone + Debug + Default + Send`.
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
//! There are two ways to declare a lens. Both produce identical generated code.
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
//! struct MyLens {
//!     pub items: Vec<u32>,
//!     pub viewport_rows: u32,
//! }
//! # drv::assemble!();
//! # fn main() {}
//! ```
//!
//! Use standalone lenses when the lens is defined closer to the memo that consumes
//! it, or when the atom is in another module and you don't want to modify it.
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
//!     format!("{}={}", label, *lens.count * multiplier)
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
//! struct HitsLens {
//!     pub hits: Vec<u32>,
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
//!     if *lens.count == 0 { 0 } else { *lens.total / *lens.count }
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
//! Constructing a lens from an atom is free (zero-copy — the lens holds
//! references into the atom). What runs on each call:
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
//! ## Large collections: use `imbl`
//!
//! An atom tracking 50 open buffers in a `HashMap<Path, Buffer>` compares the
//! entire map on every access — O(n). Every memoized call pays full traversal
//! cost, defeating the point.
//!
//! [imbl](https://docs.rs/imbl) provides **persistent** (structurally-shared)
//! collections. Its main advantage for memoization is that **`Clone` is
//! O(1)** — cache-miss snapshots are nearly free regardless of size. Its
//! `PartialEq`, however, is implemented by iterating and comparing elements
//! pairwise (just like `Vec`/`HashMap`), with no pointer-equality
//! short-circuit — so cache-hit comparison is O(n) regardless of whether
//! nothing, some, or everything changed.
//!
//! | Type | Clone | PartialEq | Mutation |
//! |------|-------|-----------|---------|
//! | `Vec<T>` | O(n) | O(n) | O(1) amortized |
//! | `HashMap<K,V>` | O(n) | O(n) | O(1) amortized |
//! | `imbl::Vector<T>` | **O(1)** | O(n) | O(log n) |
//! | `imbl::HashMap<K,V>` | **O(1)** | O(n) | O(log n) |
//!
//! For both kinds of collection, `==` short-circuits on the first differing
//! element, so returning `false` is often faster than returning `true`.
//!
//! - **Clone is O(1) for `imbl`**: bumps a reference count on the root node.
//!   This makes snapshot creation on cache miss nearly free.
//! - **PartialEq walks elements pairwise for both `imbl` and std**: no
//!   structural-sharing optimization happens in `==`. The comparison is
//!   O(position of first difference) on miss, O(n) on a confirmed hit.
//! - **Mutation is O(log n) for `imbl`**: the trade-off. For typical sizes,
//!   negligible.
//!
//! **Practical upshot:** use `imbl` when you expect frequent cache misses
//! (so snapshot cost matters) or when your atom's collection is modified
//! often (so `Clone`-on-write under interior mutation matters). For atoms
//! where the collection changes rarely, `Vec`/`HashMap` are often fine.
//!
//! If you need truly O(1) comparison on unchanged collections, wrap the
//! field in `Arc<T>` and compare via `Arc::ptr_eq` in a custom `PartialEq`
//! wrapper — `drv` does not provide this out of the box.
//!
//! ## Rule of thumb
//!
//! - **< 50 elements?** `Vec`/`HashMap` are fine.
//! - **50+ or expensive-to-compare elements?** Use `imbl`.
//! - **Deeply nested?** Flatten into the atom, or wrap the outer collection in `imbl`.
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

// Re-export proc macros.
pub use drv_macros::assemble;
pub use drv_macros::atom;
pub use drv_macros::lens;
pub use drv_macros::memo;

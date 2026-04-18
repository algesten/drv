#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Derived, memoized values over plain Rust structs.
//!
//! `drv` lets you declare a struct of ground-truth data (an **atom**), project
//! subsets of its fields (a **lens**), and compute derived values (a **memo**)
//! that are automatically cached. When nothing changed, nothing recomputes.
//!
//! # Quick example
//!
//! ```rust
//! use drv::Atom;
//!
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! let mut game = Atom::new(Scoreboard {
//!     hits: vec![100, 250, 50],
//!     ..Default::default()
//! });
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
//! **Atom** — a plain struct of ground-truth data, tagged with
//! [`#[drv::atom]`][atom-attr]. You derive whatever you need (`Clone`,
//! `PartialEq`, `Debug`, `Default`, `serde`, …) with normal `#[derive(...)]`;
//! drv does not inject fields or impls. The struct is wrapped in
//! [`drv::Atom<T>`][atom-type] at construction so each instance carries its own
//! memoization cache alongside the data.
//!
//! **Lens** — a projection: a subset of an atom's fields, by name and type.
//! It declares "this computation depends on exactly these fields and no others."
//!
//! **Memo** — a pure function from a lens (or an atom directly) to an output.
//! Annotated with [`#[drv::memo]`][memo-attr]. The result is cached and only
//! recomputed when the input fields change.
//!
//! At the end of your crate, [`drv::assemble!()`][assemble] stitches
//! everything together.
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
//! the projection explicitly with [`#[drv::proj]`][proj-attr].
//!
//! ## Inline: annotate fields on the atom
//!
//! Tag fields with [`#[drv::lens(Name)]`][lens-attr] directly on the atom.
//! The macro generates a lens called `Name` with those fields. This keeps
//! the full dependency picture in one place:
//!
//! ```rust
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! list them in one attribute [`#[drv::lens(A, B)]`][lens-attr] or as
//! separate attributes.
//!
//! ## Standalone: declare a separate struct
//!
//! Declare the lens as its own struct with [`#[drv::lens(Atom)]`][lens-attr].
//! The macro verifies that every field name and type matches the atom:
//!
//! ```rust
//! # #[derive(Debug, Clone, PartialEq, Default)]
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
//! [`#[drv::lens(...)]`][lens-attr] attribute is a no-op when stripped), so
//! any reference field requires a lifetime parameter declared on the struct.
//! If you need a clone (or a projection into a nested struct, or a different
//! type), write a custom projection with [`#[drv::proj]`][proj-attr] instead.
//!
//! Use standalone lenses when the lens is defined closer to the memo that consumes
//! it, or when the atom is in another module and you don't want to modify it.
//!
//! ## Custom projection with [`#[drv::proj]`][proj-attr]
//!
//! When you need lens fields that don't match the atom — different names, different
//! types, or reaching into nested structs — write the projection yourself. You declare
//! the lens struct with [`#[drv::lens(Atom)]`][lens-attr] and annotate the
//! `From` impl with [`#[drv::proj]`][proj-attr]:
//!
//! ```rust
//! use drv::Atom;
//!
//! #[derive(Debug, Clone, PartialEq, Default)]
//! pub struct Inner {
//!     pub x: u32,
//!     pub label: String,
//! }
//!
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! // The projection function. drv::proj wires the cache handle and rewrites
//! // the signature to take a `&Atom<Container>` under the hood.
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
//! let c = Atom::new(Container {
//!     inner: Inner { x: 42, label: "ignored".into() },
//!     name: "hello".into(),
//!     ..Default::default()
//! });
//! assert_eq!(display(&c), "hello=42");
//! # }
//! ```
//!
//! You can always take control of the projection by writing the `From` impl
//! yourself and annotating it with [`#[drv::proj]`][proj-attr].
//! This is required when the lens's fields don't match the atom — different
//! names, nested fields, or different types — since the macro has nothing to
//! infer from. You can also supply a [`#[drv::proj]`][proj-attr] impl for a
//! lens whose fields *do* match the atom: the macro's default projection is
//! suppressed and your impl is used instead.
//!
//! Your struct definition stays exactly as written; the attribute only rewrites
//! the `From` body to wire the cache reference. Lenses with a
//! [`#[drv::proj]`][proj-attr] impl require
//! a lifetime parameter on the struct (for the cache reference).
//!
//! They work identically with memos — cache hits, misses, multi-lens parameters,
//! and value parameters all behave the same as standard lenses.
//!
//! # Calling memos
//!
//! [`#[drv::memo]`][memo-attr] generates a free function with the same name.
//! The function body reads from the lens; the generated wrapper handles
//! memoization. You call it with [`&Atom<YourStruct>`][atom-type] — the macro
//! auto-converts into the right lens:
//!
//! ```rust
//! use drv::Atom;
//!
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! let app = Atom::new(AppState {
//!     items: vec![10, 20, 30],
//!     ..Default::default()
//! });
//!
//! let n = item_count(&app);   // pass &Atom<AppState>; projects to CountLens
//! assert_eq!(n, 3);
//! # }
//! ```
//!
//! Memoization happens behind the scenes — no cache struct, no setup.
//!
//! # Using an atom directly
//!
//! A memo can take the atom itself as input — treated as an "identity lens" over
//! all data fields. Write the parameter as `&YourStruct`; callers pass
//! [`&Atom<YourStruct>`][atom-type] and the generated wrapper derefs before
//! invoking the body:
//!
//! ```rust
//! use drv::Atom;
//!
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! # let s = Atom::new(Stats { total: 100, count: 4 });
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
//! use drv::Atom;
//!
//! #[derive(Debug, Clone, PartialEq, Default)]
//! #[drv::atom]
//! pub struct Game {
//!     #[drv::lens(HitsLens)]
//!     pub hits: Vec<u32>,
//! }
//!
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! # let game = Atom::new(Game { hits: vec![10, 20, 30] });
//! # let settings = Atom::new(Settings { multiplier: 2 });
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
//! # use drv::Atom;
//! # #[derive(Debug, Clone, PartialEq, Default)]
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
//! # let s = Atom::new(Stats { count: 3 });
//! # assert_eq!(labeled(&s, "x", 2), "x=6");
//! # }
//! ```
//!
//! Parameter classification:
//!
//! - `&Lens` or [`&Atom<MyAtom>`][atom-type] — a lens parameter (required: at least one lens).
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
//! A memo's output can feed into another memo. Mark the output type as an atom too,
//! and return it wrapped in [`Atom<...>`][atom-type] so downstream memos can project
//! from it:
//!
//! ```rust
//! use drv::Atom;
//!
//! // Root atom
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! fn stats(lens: &HitsLens) -> Atom<Stats> {
//!     Atom::new(Stats {
//!         total: lens.hits.iter().sum(),
//!         count: lens.hits.len() as u32,
//!         best: lens.hits.iter().copied().max().unwrap_or(0),
//!     })
//! }
//!
//! // Stats is an atom, so more lenses can project from it.
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! # let game = Atom::new(Game { hits: vec![10, 20, 30], ..Default::default() });
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
//! `#[drv::atom]` alone imposes **no trait bounds** on the struct or its
//! fields — it just registers the type for lens/memo machinery. Bounds
//! accrue only through the lenses that actually project a field:
//!
//! - **Any field reached by a lens** (explicit or identity) must implement
//!   `PartialEq + Clone + Debug`. `PartialEq` drives the freshness check;
//!   `Clone` is used to snapshot the field for the next comparison; `Debug`
//!   comes from the lens struct's `#[derive]`.
//! - **Fields whose snapshot is stored in cache state** (i.e. reached by a
//!   memo via any lens) must additionally satisfy `Send + 'static`, because
//!   the snapshot lives in [`Atomized::State`][atomized].
//! - **Fields that never appear in a lens and whose atom has no memo taking
//!   `&Atom<T>`** carry no bounds at all.
//!
//! The identity lens (reached when a memo takes `&Atom<T>` directly) is
//! emitted only when some memo consumes it, so atoms without identity-lens
//! consumers don't pay for bounds on unrelated fields.
//!
//! If you want to [`Clone`], [`PartialEq`]-compare, [`Debug`]-print, or
//! [`Default`]-construct your atom itself (via `Atom<T>`), derive those traits
//! on your struct as usual — drv's forwarding impls simply require the same
//! bound on `T`.
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
//! For `PartialEq`, the field type matters. Scalars are free. Plain
//! collections (`Vec`, `HashMap`) iterate and compare pairwise — O(n) worst
//! case on a cache hit. `Arc<T>` and (with the `imbl` feature) `imbl`'s
//! persistent collections take a pointer-equality fast path: when the field
//! hasn't been mutated since the last cache miss, comparison is O(1).
//! For output `Clone`, wrap expensive outputs in `Rc`/`Arc` for O(1) cloning.
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
//! - **[`imbl`][imbl] persistent collections** — enable the
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
//! #[derive(Debug, Clone, PartialEq, Default)]
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
//! `drv::assemble!()` must appear once, after every `#[drv::atom]`,
//! `#[drv::lens]`, `#[drv::memo]`, and `#[drv::proj]` declaration in the
//! crate. It collects every registration and emits the per-atom state types
//! (the [`Atomized`][atomized] impls) and the memoized free functions.
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
//! - **Plain Rust structs.** Your atom is a plain data struct with whatever
//!   `#[derive(...)]` you choose. drv injects nothing into it; the
//!   [`Atom<T>`][atom-type] wrapper holds the cache *next to* the data, not inside it.
//! - **Static dependency declaration.** The lens struct _is_ the dependency
//!   list. The compiler verifies field names and types at compile time
//!   (or, with [`#[drv::proj]`][proj-attr], the projection function).
//! - **Zero runtime tracking.** No proxy objects, no access instrumentation,
//!   no subscription management. Just field-by-field `PartialEq` against a
//!   stashed snapshot.
//! - **Memoization is automatic.** Wrap your atom with [`Atom::new`][atom-new] and
//!   call the memo. No cache struct to manage, no explicit setup.
//! - **Free functions, not methods.** Memos are ordinary functions. Call
//!   sites don't need to know any generated type names.
//!
//! [atom-attr]: https://docs.rs/drv/latest/drv/attr.atom.html
//! [memo-attr]: https://docs.rs/drv/latest/drv/attr.memo.html
//! [lens-attr]: https://docs.rs/drv/latest/drv/attr.lens.html
//! [proj-attr]: https://docs.rs/drv/latest/drv/attr.proj.html
//! [assemble]: https://docs.rs/drv/latest/drv/macro.assemble.html
//! [atom-type]: https://docs.rs/drv/latest/drv/struct.Atom.html
//! [atom-new]: https://docs.rs/drv/latest/drv/struct.Atom.html#method.new
//! [atomized]: https://docs.rs/drv/latest/drv/trait.Atomized.html
//! [imbl]: https://docs.rs/imbl

use std::cell::RefCell;

#[doc(hidden)]
pub mod __sealed {
    /// Sealed super-trait of [`super::Atomized`]. Only `drv::assemble!()`
    /// emits impls of this; users cannot satisfy the bound.
    pub trait Sealed {}
}

/// Implemented by using [`#[drv::atom]`](https://docs.rs/drv/latest/drv/attr.atom.html); users never implement
/// it themselves.
///
/// [`drv::assemble!()`](https://docs.rs/drv/latest/drv/macro.assemble.html) emits the [`Atomized`] impl for every
/// [`#[drv::atom]`](https://docs.rs/drv/latest/drv/attr.atom.html)-tagged struct automatically. You'll see this
/// trait name only as the bound on [`drv::Atom<T>`](https://docs.rs/drv/latest/drv/struct.Atom.html) and in compile
/// errors like `Foo: Atomized is not satisfied` — usually meaning you forgot
/// [`#[drv::atom]`](https://docs.rs/drv/latest/drv/attr.atom.html) on `Foo` or didn't call
/// [`drv::assemble!()`](https://docs.rs/drv/latest/drv/macro.assemble.html).
///
/// The trait is sealed: external impls won't compile.
///
/// # Example
///
/// You only encounter `Atomized` indirectly — for instance, as the bound on
/// a generic helper that takes an [`Atom<T>`]:
///
/// ```rust
/// use drv::{Atom, Atomized};
///
/// fn debug_print<T: Atomized + std::fmt::Debug>(a: &Atom<T>) {
///     println!("{:?}", &**a);
/// }
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// #[drv::atom]
/// pub struct Counter { pub n: u32 }
/// drv::assemble!();
///
/// # fn main() {
/// debug_print(&Atom::new(Counter { n: 5 }));
/// # }
/// ```
pub trait Atomized: __sealed::Sealed {
    #[doc(hidden)]
    type State: Default + Send + 'static;
}

/// Stack-allocated memoization cache owned by [`Atom<T>`] alongside the atom.
///
/// Internal type: users do not construct or interact with `Cache<T>` directly —
/// the `Atom<T>` wrapper owns one and the macros wire it through. Exposed only
/// because macro-generated code in user crates references it through `::drv`.
#[doc(hidden)]
pub struct Cache<A: Atomized> {
    pub inner: RefCell<A::State>,
}

impl<A: Atomized> Cache<A> {
    pub fn new() -> Self {
        Cache {
            inner: RefCell::new(A::State::default()),
        }
    }
}

impl<A: Atomized> Default for Cache<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Atomized> Clone for Cache<A> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl<A: Atomized> PartialEq for Cache<A> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<A: Atomized> Eq for Cache<A> {}

impl<A: Atomized> std::fmt::Debug for Cache<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Cache(..)")
    }
}

impl<A: Atomized> std::hash::Hash for Cache<A> {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl<A: Atomized> PartialOrd for Cache<A> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<A: Atomized> Ord for Cache<A> {
    fn cmp(&self, _: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

/// Holds your data `T` together with the cache of values derived from it.
///
/// Construct one with [`Atom::new`]. Pass `&atom` to memos: they project the
/// fields they need, look up the cached output, and recompute only when those
/// fields have actually changed. Reads and mutations on `atom` go straight
/// through to `T` via `Deref`/`DerefMut`, so the wrapper is invisible at the
/// usage site (`atom.field`, `atom.field = x`).
///
/// Each `Atom<T>` carries its own cache. Trait impls forward to `T` and
/// ignore cache state:
///
/// - `Clone`: clones `T` and gives the clone a fresh, empty cache.
/// - `PartialEq` / `Eq` / `Hash` / `Debug`: compare, hash, and print `T` only.
/// - `Default`: wraps `T::default()` in an empty cache.
///
/// # Example
///
/// ```rust
/// use drv::Atom;
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// #[drv::atom]
/// pub struct Counter {
///     #[drv::lens(NLens)]
///     pub n: u32,
/// }
///
/// #[drv::memo]
/// fn doubled(lens: &NLens) -> u32 { lens.n * 2 }
///
/// drv::assemble!();
///
/// # fn main() {
/// let mut a = Atom::new(Counter { n: 5 });
/// assert_eq!(a.n, 5);            // Deref to Counter
/// assert_eq!(doubled(&a), 10);    // memo computes
/// a.n = 7;                        // DerefMut
/// assert_eq!(doubled(&a), 14);    // memo recomputes
/// # }
/// ```
pub struct Atom<T: Atomized> {
    inner: T,
    cache: Cache<T>,
}

impl<T: Atomized> Atom<T> {
    /// Wrap a value of an atom-tagged type with a fresh memoization cache.
    ///
    /// # Example
    ///
    /// ```rust
    /// use drv::Atom;
    ///
    /// #[derive(Debug, Clone, PartialEq, Default)]
    /// #[drv::atom]
    /// pub struct Counter { pub n: u32 }
    ///
    /// drv::assemble!();
    ///
    /// # fn main() {
    /// let a = Atom::new(Counter { n: 5 });
    /// assert_eq!(a.n, 5);
    /// # }
    /// ```
    pub fn new(value: T) -> Self {
        Self {
            inner: value,
            cache: Cache::new(),
        }
    }

    /// Consume the wrapper and return the inner atom, dropping the cache.
    ///
    /// # Example
    ///
    /// ```rust
    /// use drv::Atom;
    ///
    /// #[derive(Debug, Clone, PartialEq, Default)]
    /// #[drv::atom]
    /// pub struct Counter { pub n: u32 }
    ///
    /// drv::assemble!();
    ///
    /// # fn main() {
    /// let a = Atom::new(Counter { n: 5 });
    /// let inner: Counter = a.into_inner();
    /// assert_eq!(inner.n, 5);
    /// # }
    /// ```
    pub fn into_inner(self) -> T {
        self.inner
    }

    #[doc(hidden)]
    pub fn __drv_cache(&self) -> &Cache<T> {
        &self.cache
    }
}

impl<T: Atomized> std::ops::Deref for Atom<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: Atomized> std::ops::DerefMut for Atom<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Atomized + Clone> Clone for Atom<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            cache: Cache::new(),
        }
    }
}

impl<T: Atomized + PartialEq> PartialEq for Atom<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T: Atomized + Eq> Eq for Atom<T> {}

impl<T: Atomized + std::fmt::Debug> std::fmt::Debug for Atom<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: Atomized + Default> Default for Atom<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Atomized + std::hash::Hash> std::hash::Hash for Atom<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl<T: Atomized> From<T> for Atom<T> {
    fn from(value: T) -> Self {
        Self::new(value)
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

/// Atomize a struct to be used in [`Atom<T>`].
///
/// Registers the struct for memo/lens machinery; the struct body is emitted
/// verbatim, and any `#[derive(...)]` you add stays on it. The attribute alone
/// imposes no trait bounds on the struct or its fields — bounds are introduced
/// by the lenses and memos that actually touch a field. See the crate-level
/// [Choosing field types](crate#choosing-field-types) section for the details.
/// Fields can be any visibility (Rust's normal privacy rules apply for
/// cross-module lens projection). Annotate fields with
/// [`#[drv::lens(Name)]`](https://docs.rs/drv/latest/drv/attr.lens.html) to
/// declare inline lenses.
///
/// # Example
///
/// ```rust
/// use drv::Atom;
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// #[drv::atom]
/// pub struct Counter {
///     #[drv::lens(NLens)]
///     pub n: u32,
///     pub label: String,
/// }
///
/// #[drv::memo]
/// fn doubled(lens: &NLens) -> u32 { lens.n * 2 }
///
/// drv::assemble!();
///
/// # fn main() {
/// let a = Atom::new(Counter { n: 5, label: "x".into() });
/// assert_eq!(doubled(&a), 10);
/// # }
/// ```
pub use drv_macros::atom;

/// Declare a lens — a projection of an atom's fields.
///
/// Has two forms:
///
/// 1. **Inline**, on a field of an [`#[drv::atom]`](https://docs.rs/drv/latest/drv/attr.atom.html)-tagged struct
///    (`#[drv::lens(LensName)]`, or `#[drv::lens(LensA, LensB)]` for several).
///    The atom macro generates a lens struct named `LensName` containing
///    every field tagged with that lens name.
///
/// 2. **Standalone**, on its own struct (`#[drv::lens(AtomName)]`) where each
///    field has the same name and type (or `&'a T`) as a field on the atom.
///    Use this when the lens lives near the memo that consumes it. If the
///    fields don't structurally match the atom, the lens becomes a custom
///    projection — provide a [`#[drv::proj]`](https://docs.rs/drv/latest/drv/attr.proj.html)-annotated `From` impl.
///
/// Only built-in primitive Copy types may be stored by value; non-Copy types
/// must be borrowed as `&'a T` with a lifetime parameter on the lens struct.
///
/// # Example
///
/// ```rust
/// use drv::Atom;
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// #[drv::atom]
/// pub struct AppState {
///     pub items: Vec<u32>,
///     pub viewport_rows: u32,
/// }
///
/// // Standalone lens; matches AppState's `items` and `viewport_rows`.
/// #[drv::lens(AppState)]
/// struct VisibleLens<'a> {
///     pub items: &'a Vec<u32>,
///     pub viewport_rows: u32,
/// }
///
/// #[drv::memo]
/// fn first_visible(lens: &VisibleLens) -> Option<u32> {
///     lens.items.iter().take(lens.viewport_rows as usize).next().copied()
/// }
///
/// drv::assemble!();
///
/// # fn main() {
/// let a = Atom::new(AppState { items: vec![10, 20, 30], viewport_rows: 2 });
/// assert_eq!(first_visible(&a), Some(10));
/// # }
/// ```
pub use drv_macros::lens;

/// Memoize a function over its lens parameters.
///
/// The function takes one or more `&LensName` parameters (or `&YourAtom`
/// for an identity lens over an [`#[drv::atom]`](https://docs.rs/drv/latest/drv/attr.atom.html)-tagged struct)
/// plus optional value parameters. The result is automatically cached and
/// only recomputed when any input changes. Multiple lens parameters may
/// reference lenses from different atoms; the cache lives on the first
/// lens parameter's atom.
///
/// # Example
///
/// ```rust
/// use drv::Atom;
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// #[drv::atom]
/// pub struct Stats {
///     #[drv::lens(SumLens)]
///     pub xs: Vec<u32>,
/// }
///
/// #[drv::memo]
/// fn sum_with_offset(lens: &SumLens, offset: u32) -> u32 {
///     lens.xs.iter().sum::<u32>() + offset
/// }
///
/// drv::assemble!();
///
/// # fn main() {
/// let a = Atom::new(Stats { xs: vec![1, 2, 3] });
/// assert_eq!(sum_with_offset(&a, 10), 16);
/// # }
/// ```
pub use drv_macros::memo;

/// Turn a `From` impl into a user-controlled projection to a lens.
///
/// Use this when a [`#[drv::lens(Atom)]`](https://docs.rs/drv/latest/drv/attr.lens.html)-tagged struct has
/// fields with different names, types, or nested-access shapes than the
/// atom. You write the `From<&Atom>` conversion explicitly; this attribute
/// rewrites the signature to take `&Atom<Atom>` under the hood and wires
/// the cache handle into the resulting lens.
///
/// You can also attach [`#[drv::proj]`](https://docs.rs/drv/latest/drv/attr.proj.html) to a lens whose
/// fields structurally match the atom. The macro would normally
/// auto-generate the `From` impl for such a lens; supplying a `#[drv::proj]`
/// impl suppresses the default and uses yours instead.
///
/// # Example
///
/// ```rust
/// use drv::Atom;
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// pub struct Inner { pub value: u32, pub label: String }
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// #[drv::atom]
/// pub struct Container { pub inner: Inner, pub name: String }
///
/// // Field names and shapes don't match Container — needs a custom projection.
/// #[drv::lens(Container)]
/// struct Surface<'a> {
///     pub value: u32,         // owned copy from inner.value
///     pub name: &'a str,      // borrowed from name: String
/// }
///
/// #[drv::proj]
/// impl<'a> From<&'a Container> for Surface<'a> {
///     fn from(v: &'a Container) -> Self {
///         Self { value: v.inner.value, name: &v.name }
///     }
/// }
///
/// #[drv::memo]
/// fn render(s: &Surface) -> String { format!("{}={}", s.name, s.value) }
///
/// drv::assemble!();
///
/// # fn main() {
/// let a = Atom::new(Container {
///     inner: Inner { value: 42, label: "ignored".into() },
///     name: "x".into(),
/// });
/// assert_eq!(render(&a), "x=42");
/// # }
/// ```
pub use drv_macros::proj;

/// Collect every atom, lens, memo, and projection declared in this crate
/// and emit the generated `Atomized` impls and memoized functions.
///
/// Must appear once, after every [`#[drv::atom]`](https://docs.rs/drv/latest/drv/attr.atom.html),
/// [`#[drv::lens]`](https://docs.rs/drv/latest/drv/attr.lens.html), [`#[drv::memo]`](https://docs.rs/drv/latest/drv/attr.memo.html), and
/// [`#[drv::proj]`](https://docs.rs/drv/latest/drv/attr.proj.html) declaration in the crate.
///
/// # Example
///
/// ```rust
/// use drv::Atom;
///
/// #[derive(Debug, Clone, PartialEq, Default)]
/// #[drv::atom]
/// pub struct Counter {
///     #[drv::lens(NLens)]
///     pub n: u32,
/// }
///
/// #[drv::memo]
/// fn squared(lens: &NLens) -> u32 { lens.n * lens.n }
///
/// drv::assemble!();   // <-- collects everything above
///
/// # fn main() {
/// let a = Atom::new(Counter { n: 6 });
/// assert_eq!(squared(&a), 36);
/// # }
/// ```
pub use drv_macros::assemble;

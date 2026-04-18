# drv

> [!WARNING]
> **Vibe coded.** I've focused on the overall architecture rather than reviewing
> the code output in detail. For projects I've written by hand, see
> [ureq](https://github.com/algesten/ureq) and
> [str0m](https://github.com/algesten/str0m).

Derived, memoized values over plain Rust structs.

`drv` lets you declare a struct of ground-truth data (an **atom**), project
subsets of its fields (a **lens**), and compute derived values (a **memo**)
that are automatically cached. When nothing changed, nothing recomputes.

## Quick example

```rust
use drv::Atom;

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Scoreboard {
    #[drv::lens(TotalLens)]
    pub hits: Vec<u32>,

    pub player_x: u32,
    pub player_y: u32,
    pub time_ms: u64,
}

#[drv::memo]
fn total_score(lens: &TotalLens) -> u32 {
    lens.hits.iter().sum()
}

drv::assemble!();

let mut game = Atom::new(Scoreboard {
    hits: vec![100, 250, 50],
    ..Default::default()
});

let score = total_score(&game);     // computes: 400
game.player_x = 42;                  // irrelevant to the lens
let score = total_score(&game);     // cache hit, no work
```

Moving the player? Free. `total_score` only recomputes when `hits` change.

## The three pieces

**Atom** — a plain struct of ground-truth data, tagged with
[`#[drv::atom]`][atom-attr]. You derive whatever you need (`Clone`,
`PartialEq`, `Debug`, `Default`, `serde`, …) with normal `#[derive(...)]`;
drv does not inject fields or impls. The struct is wrapped in
[`drv::Atom<T>`][atom-type] at construction so each instance carries its own
memoization cache alongside the data.

**Lens** — a projection: a subset of an atom's fields, by name and type.
It declares "this computation depends on exactly these fields and no others."

**Memo** — a pure function from a lens (or an atom directly) to an output.
Annotated with [`#[drv::memo]`][memo-attr]. The result is cached and only
recomputed when the input fields change.

At the end of your crate, [`drv::assemble!()`][assemble] stitches
everything together.

## Declaring lenses

There are three ways to declare a lens.

In all generated lenses, built-in scalar primitives (`u32`, `bool`, `f64`,
`usize`, etc.) are stored **by value** — `lens.x` gives `u32` directly, no
dereference needed. All other types are borrowed as `&T`. User-defined
`Copy` types are *not* auto-detected (the proc macro can only recognise
language built-ins); for full control over field representation, declare
the projection explicitly with [`#[drv::proj]`][proj-attr].

### Inline: annotate fields on the atom

Tag fields with [`#[drv::lens(Name)]`][lens-attr] directly on the atom.
The macro generates a lens called `Name` with those fields. This keeps
the full dependency picture in one place:

```rust
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    #[drv::lens(TotalLens, StatusLens)]
    pub items: Vec<u32>,

    #[drv::lens(StatusLens)]
    pub selected: Option<usize>,

    #[drv::lens(Render)]
    pub viewport_rows: u32,

    pub cursor_col: u32,
}
```

This generates three lenses: `TotalLens { items }`, `StatusLens { items, selected }`,
and `Render { viewport_rows }`. A field can appear in multiple lenses —
list them in one attribute [`#[drv::lens(A, B)]`][lens-attr] or as
separate attributes.

### Standalone: declare a separate struct

Declare the lens as its own struct with [`#[drv::lens(Atom)]`][lens-attr].
The macro verifies that every field name and type matches the atom:

```rust
#[drv::lens(AppState)]
struct MyLens<'a> {
    pub items: &'a Vec<u32>,
    pub viewport_rows: u32,
}
```

Only built-in primitive Copy types (`u8`..`u128`, `i8`..`i128`, `usize`, `isize`, `f32`,
`f64`, `bool`, `char`) may appear by value — any other type must be written
as `&'a T`. The lens struct must be valid Rust on its own (the
[`#[drv::lens(...)]`][lens-attr] attribute is a no-op when stripped), so
any reference field requires a lifetime parameter declared on the struct.
If you need a clone (or a projection into a nested struct, or a different
type), write a custom projection with [`#[drv::proj]`][proj-attr] instead.

Use standalone lenses when the lens is defined closer to the memo that consumes
it, or when the atom is in another module and you don't want to modify it.

### Custom projection with [`#[drv::proj]`][proj-attr]

When you need lens fields that don't match the atom — different names, different
types, or reaching into nested structs — write the projection yourself. You declare
the lens struct with [`#[drv::lens(Atom)]`][lens-attr] and annotate the
`From` impl with [`#[drv::proj]`][proj-attr]:

```rust
use drv::Atom;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inner {
    pub x: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Container {
    pub inner: Inner,
    pub name: String,
    pub count: u32,
}

// Fields don't match the atom — we provide our own projection.
#[drv::lens(Container)]
struct ProjectedLens<'a> {
    pub x: u32,            // owned copy of a nested field
    pub name: &'a str,     // borrow &str from a String field
}

// The projection function. drv::proj wires the cache handle and rewrites
// the signature to take a `&Atom<Container>` under the hood.
#[drv::proj]
impl<'a> From<&'a Container> for ProjectedLens<'a> {
    fn from(v: &'a Container) -> Self {
        Self {
            x: v.inner.x,
            name: &v.name,
        }
    }
}

#[drv::memo]
fn display(lens: &ProjectedLens) -> String {
    format!("{}={}", lens.name, lens.x)
}

let c = Atom::new(Container {
    inner: Inner { x: 42, label: "ignored".into() },
    name: "hello".into(),
    ..Default::default()
});
assert_eq!(display(&c), "hello=42");
```

You can always take control of the projection by writing the `From` impl
yourself and annotating it with [`#[drv::proj]`][proj-attr].
This is required when the lens's fields don't match the atom — different
names, nested fields, or different types — since the macro has nothing to
infer from. You can also supply a [`#[drv::proj]`][proj-attr] impl for a
lens whose fields *do* match the atom: the macro's default projection is
suppressed and your impl is used instead.

Your struct definition stays exactly as written; the attribute only rewrites
the `From` body to wire the cache reference. Lenses with a
[`#[drv::proj]`][proj-attr] impl require
a lifetime parameter on the struct (for the cache reference).

They work identically with memos — cache hits, misses, multi-lens parameters,
and value parameters all behave the same as standard lenses.

## Calling memos

[`#[drv::memo]`][memo-attr] generates a free function with the same name.
The function body reads from the lens; the generated wrapper handles
memoization. You call it with [`&Atom<YourStruct>`][atom-type] — the macro
auto-converts into the right lens:

```rust
use drv::Atom;

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    #[drv::lens(CountLens)]
    pub items: Vec<u32>,
    pub viewport_rows: u32,
}

#[drv::memo]
fn item_count(lens: &CountLens) -> usize {
    lens.items.len()
}

let app = Atom::new(AppState {
    items: vec![10, 20, 30],
    ..Default::default()
});

let n = item_count(&app);   // pass &Atom<AppState>; projects to CountLens
assert_eq!(n, 3);
```

Memoization happens behind the scenes — no cache struct, no setup.

## Using an atom directly

A memo can take the atom itself as input — treated as an "identity lens" over
all data fields. Write the parameter as `&YourStruct`; callers pass
[`&Atom<YourStruct>`][atom-type] and the generated wrapper derefs before
invoking the body:

```rust
use drv::Atom;

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Stats {
    pub total: u32,
    pub count: u32,
}

#[drv::memo]
fn average(s: &Stats) -> u32 {
    if s.count == 0 { 0 } else { s.total / s.count }
}
```

Any change to any field of the atom invalidates the cache. Useful when you
really do depend on everything.

## Memos calling other memos

A memo body may invoke another memo on the same atom — composition across
derived values, with each memo's cache short-circuiting independently.
Inner-memo re-entry is safe because the cache's `RefCell` is released
before the body runs.

```rust
use drv::Atom;

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Counter {
    pub value: u32,
}

#[drv::memo]
fn doubled(c: &Counter) -> u32 {
    c.value * 2
}

#[drv::memo]
fn doubled_plus_one(c: &Counter) -> u32 {
    doubled(c) + 1   // calls a sibling memo on the same atom
}

drv::assemble!();

let a = Atom::new(Counter { value: 10 });
assert_eq!(doubled_plus_one(&a), 21);
```

## Multiple lenses

A memo can take lenses from multiple atoms. The cache lives in the first
parameter's atom:

```rust
use drv::Atom;

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Game {
    #[drv::lens(HitsLens)]
    pub hits: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Settings {
    #[drv::lens(MultiplierLens)]
    pub multiplier: u32,
}

#[drv::memo]
fn weighted_score(hits: &HitsLens, settings: &MultiplierLens) -> u32 {
    hits.hits.iter().sum::<u32>() * settings.multiplier
}

let score = weighted_score(&game, &settings);   // cache stored in `game`
```

If any of the lens field comparisons fail, the memo recomputes. Two lenses from
the same atom (`fn foo(a: &LensA, b: &LensB)` called as `foo(&app, &app)`)
works too.

## Value parameters

Memos can also take owned values or borrowed references to types like
`&str` or `&[u8]` alongside lens parameters. Values participate in the
cache key just like lens fields — any change triggers a recompute.

```rust
#[drv::memo]
fn labeled(lens: &CountLens, label: &str, multiplier: u32) -> String {
    format!("{}={}", label, lens.count * multiplier)
}
```

Parameter classification:

- `&Lens` or [`&Atom<MyAtom>`][atom-type] — a lens parameter (required: at least one lens).
- Owned types (`u32`, `String`, `MyStruct`) — stored via `Clone`.
- Borrowed types with `ToOwned` (`&str`, `&[u8]`, `&Path`, ...) — stored
  as `<T as ToOwned>::Owned` (so `&str` stores as `String`).

Value types must implement `PartialEq + Clone + Send + 'static`; borrowed
value types must satisfy `T: ToOwned` with the owned form matching the
usual bounds. Declared order is preserved at the call site; the cache is
always stored on the first lens parameter's atom.

## Chaining

A memo's output can feed into another memo. Mark the output type as an atom too,
and return it wrapped in [`Atom<...>`][atom-type] so downstream memos can project
from it:

```rust
use drv::Atom;

// Root atom
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Game {
    pub hits: Vec<u32>,
    pub time_ms: u64,
}

// Lens over Game → produces Stats (itself an atom).
#[drv::lens(Game)]
struct HitsLens<'a> {
    pub hits: &'a Vec<u32>,
}

#[drv::memo]
fn stats(lens: &HitsLens) -> Atom<Stats> {
    Atom::new(Stats {
        total: lens.hits.iter().sum(),
        count: lens.hits.len() as u32,
        best: lens.hits.iter().copied().max().unwrap_or(0),
    })
}

// Stats is an atom, so more lenses can project from it.
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Stats {
    #[drv::lens(AverageLens)]
    pub total: u32,

    #[drv::lens(AverageLens)]
    pub count: u32,

    pub best: u32,
}

#[drv::memo]
fn average(lens: &AverageLens) -> u32 {
    if lens.count == 0 { 0 } else { lens.total / lens.count }
}
```

Each link is independently memoized. If `hits` didn't change, `stats` returns
cached, `average` sees the same input, also returns cached. Zero work.

### Conceptual chaining

```
                                  ┌──────────────────────────┐
                                  │      DerivedOutputB      │
                                  │                          │
                                  └──────────────────────────┘
                                                ▲
                                                │
                                                │
                                      transform_function_3
                                             (memo)
                                                ▲
                                                │
                                                │
                                       ┌ ─ ─ ─ ─ ─ ─ ─ ─
                                         PartialDerived │
                                       │     (lens)
                                        ─ ─ ─ ─ ─ ─ ─ ─ ┘
                                                ▲
                                                │
┌──────────────────────────┐      ┌──────────────────────────┐
│      DerivedOutputA      │      │    OtherDerivedState     │
│                          │      │          (atom)          │
└──────────────────────────┘      └──────────────────────────┘
              ▲                                 ▲
              │                                 │
    transform_function_1              transform_function_2
           (memo)                            (memo)
              ▲                                 ▲
              │                                 │
              │                  ┌──────────────┴──────────────┐
              │                  │                             │
              │                  │                             │
      ┌ ─ ─ ─ ─ ─ ─ ─ ┐  ┌ ─ ─ ─ ─ ─ ─ ─ ┐         ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
        PartialState1      PartialState2              PartialOtherState   │
      │    (lens)     │  │    (lens)     │         │        (lens)
       ─ ─ ─ ─ ─ ─ ─ ─    ─ ─ ─ ─ ─ ─ ─ ─           ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┘
              ▲                  ▲                             ▲
              └──────────┬───────┘                             │
                         │                                     │
           ┌──────────────────────────┐          ┌──────────────────────────┐
           │        SomeState         │          │        OtherState        │
           │          (atom)          │          │          (atom)          │
           └──────────────────────────┘          └──────────────────────────┘
```

## Choosing field types

`#[drv::atom]` alone imposes **no trait bounds** on the struct or its
fields — it just registers the type for lens/memo machinery. Bounds
accrue only through the lenses that actually project a field:

- **Any field reached by a lens** (explicit or identity) must implement
  `PartialEq + Clone + Debug`. `PartialEq` drives the freshness check;
  `Clone` is used to snapshot the field for the next comparison; `Debug`
  comes from the lens struct's `#[derive]`.
- **Fields whose snapshot is stored in cache state** (i.e. reached by a
  memo via any lens) must additionally satisfy `Send + 'static`, because
  the snapshot lives in [`Atomized::State`][atomized].
- **Fields that never appear in a lens and whose atom has no memo taking
  `&Atom<T>`** carry no bounds at all.

The identity lens (reached when a memo takes `&Atom<T>` directly) is
emitted only when some memo consumes it, so atoms without identity-lens
consumers don't pay for bounds on unrelated fields.

If you want to [`Clone`], [`PartialEq`]-compare, [`Debug`]-print, or
[`Default`]-construct your atom itself (via `Atom<T>`), derive those traits
on your struct as usual — drv's forwarding impls simply require the same
bound on `T`.

### What runs when

Constructing a lens from an atom is nearly free — built-in primitives are
copied (trivial), other fields are borrowed by reference. What runs on
each call:

- **Every call — `PartialEq` on each lens field.** Used to check whether
  the input has changed since the last call.
- **Every call — `Clone` on the output.** Memos return by value.
- **Cache miss only — `Clone` on each lens field.** A copy is kept for the
  next call's comparison.
- **Cache miss only — the memo body runs.**

Two kinds of costs matter on the hot path:

1. Per-field `PartialEq` cost.
2. Output `Clone` cost.

For `PartialEq`, the field type matters. Scalars are free. Plain
collections (`Vec`, `HashMap`) iterate and compare pairwise — O(n) worst
case on a cache hit. `Arc<T>` and (with the `imbl` feature) `imbl`'s
persistent collections take a pointer-equality fast path: when the field
hasn't been mutated since the last cache miss, comparison is O(1).
For output `Clone`, wrap expensive outputs in `Rc`/`Arc` for O(1) cloning.

**Principle:** choose types where `PartialEq` is proportional to the amount of
change, not the total size of the data.

### Scalars: free

`u32`, `bool`, `f64`, small enums — comparison and clone are one or a few
machine instructions. No special consideration needed.

### Small owned types: fine

`String`, `PathBuf`, small `Vec<T>` with a handful of elements — comparison
is O(n) but n is small. Perfectly fine.

### Large collections: use `imbl` or `Arc`

An atom tracking 50 open buffers in a `HashMap<Path, Buffer>` compares the
entire map on every access — O(n). Every memoized call pays full traversal
cost, defeating the point.

The fix is a type with **O(1) `Clone` and O(1) equality when the value
hasn't been mutated**. `drv` recognises two families automatically and
short-circuits the cache check with a pointer compare:

- **`Arc<T>`** — works out of the box, no feature flag needed.
- **[`imbl`][imbl] persistent collections** — enable the
  `imbl` feature. Covers `Vector`, `HashMap`, `OrdMap`, `HashSet`, `OrdSet`.

```toml
[dependencies]
drv = { version = "0.1", features = ["imbl"] }
```

With the feature on, a 10,000-entry `imbl::HashMap` that hasn't been
touched since the last memo call compares in **constant time**. If it was
mutated, the check falls back to element-wise `==`, which itself
short-circuits on the first difference. There is no scenario where you
pay *more* than plain `Vec`/`HashMap` would.

| Type | Clone | Cache hit (same pointer) | Cache hit (equal contents) | Mutation |
|------|-------|--------------------------|----------------------------|----------|
| `Vec<T>` | O(n) | O(n) | O(n) | O(1) amortized |
| `HashMap<K,V>` | O(n) | O(n) | O(n) | O(1) amortized |
| `Arc<T>` | **O(1)** | **O(1)** | O(eq of T) | n/a |
| `imbl::Vector<T>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |
| `imbl::HashMap<K,V>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |

*"Same pointer"* is the common case: the atom cloned its field into the
snapshot on the last cache miss, and nothing has mutated it since — so the
two point at the same underlying node. *"Equal contents"* is the worst case,
when the collection was rebuilt from scratch but happens to compare equal.

### Rule of thumb

- **< 50 elements?** `Vec`/`HashMap` are fine.
- **50+ or expensive-to-compare elements?** Use `imbl` with the
  `imbl` feature — you get O(1) cache-hit comparison for free.
- **Single large blob of data you never mutate piece-wise?** Wrap it in
  `Arc<T>`; `drv` uses `Arc::ptr_eq` automatically.

### Example

```rust
use imbl::{Vector, HashMap};

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    pub active_tab: Option<String>,       // scalar — trivial
    pub viewport_rows: u32,               // scalar — trivial
    pub show_sidebar: bool,               // scalar — trivial

    pub tabs: Vector<String>,             // persistent — O(1) clone, O(n) eq
    pub buffers: HashMap<String, Buffer>, // persistent — O(1) clone, O(n) eq
}
```

## Multiple atoms, multiple crates

Each atom and its memos must live in the same crate. This is by design —
`drv::assemble!()` collects everything within a single compilation unit.

For larger applications, split your state into domain-specific atoms in
separate crates:

```
crate: my-app-buffers   → BufferState atom + buffer memos
crate: my-app-ui        → UiState atom + ui memos
crate: my-app-lsp       → LspState atom + lsp memos
crate: my-app           → composes the above
```

Each crate is self-contained. The top-level crate can define its own atoms
that compose fields from the domain crates, with its own lenses and memos
over the combined state.

## Assembly

`drv::assemble!()` must appear once, after every `#[drv::atom]`,
`#[drv::lens]`, `#[drv::memo]`, and `#[drv::proj]` declaration in the
crate. It collects every registration and emits the per-atom state types
(the [`Atomized`][atomized] impls) and the memoized free functions.

```
// lib.rs
mod state;       // atoms
mod views;       // lenses + memos
mod rendering;   // more lenses + memos

drv::assemble!();
```

## Serde

Enable the `serde` feature to make `Atom<T>` forward [`serde::Serialize`]
and [`serde::Deserialize`] whenever `T` implements them. Only the inner
data is serialized; the cache is reconstructed empty on deserialize, so a
roundtripped atom is observably equivalent to the original but starts cold.

```toml
[dependencies]
drv = { version = "0.1", features = ["serde"] }
```

## Design goals

- **Plain Rust structs.** Your atom is a plain data struct with whatever
  `#[derive(...)]` you choose. drv injects nothing into it; the
  [`Atom<T>`][atom-type] wrapper holds the cache *next to* the data, not inside it.
- **Static dependency declaration.** The lens struct _is_ the dependency
  list. The compiler verifies field names and types at compile time
  (or, with [`#[drv::proj]`][proj-attr], the projection function).
- **Zero runtime tracking.** No proxy objects, no access instrumentation,
  no subscription management. Just field-by-field `PartialEq` against a
  stashed snapshot.
- **Memoization is automatic.** Wrap your atom with [`Atom::new`][atom-new] and
  call the memo. No cache struct to manage, no explicit setup.
- **Free functions, not methods.** Memos are ordinary functions. Call
  sites don't need to know any generated type names.

[atom-attr]: https://docs.rs/drv/latest/drv/attr.atom.html
[memo-attr]: https://docs.rs/drv/latest/drv/attr.memo.html
[lens-attr]: https://docs.rs/drv/latest/drv/attr.lens.html
[proj-attr]: https://docs.rs/drv/latest/drv/attr.proj.html
[assemble]: https://docs.rs/drv/latest/drv/macro.assemble.html
[atom-type]: https://docs.rs/drv/latest/drv/struct.Atom.html
[atom-new]: https://docs.rs/drv/latest/drv/struct.Atom.html#method.new
[atomized]: https://docs.rs/drv/latest/drv/trait.Atomized.html
[imbl]: https://docs.rs/imbl

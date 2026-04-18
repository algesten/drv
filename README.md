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
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Scoreboard {
    #[drv::lens(TotalLens)]
    pub hits: Vec<u32>,

    pub player_x: u32,
    pub player_y: u32,
    pub time_ms: u64,
}

#[drv::memo(single)]
fn total_score<'a>(lens: impl Into<TotalLens<'a>>) -> u32 {
    lens.hits.iter().sum()
}

drv::assemble!();

let mut game = Scoreboard {
    hits: vec![100, 250, 50],
    ..Default::default()
};

let score = total_score(&game);     // computes: 400
game.player_x = 42;                  // irrelevant to the lens
let score = total_score(&game);     // cache hit, no work
```

Moving the player? Free. `total_score` only recomputes when `hits` change.

## The three pieces

**Atom** — a plain struct of ground-truth data, tagged with
[`#[drv::atom]`][atom-attr]. You derive whatever you need (`Clone`,
`PartialEq`, `Debug`, `Default`, `serde`, …) with normal `#[derive(...)]`;
drv does not inject fields or impls. The struct is a plain Rust value — no
wrapper to construct, no cache held alongside the data. Each memo owns its
own per-thread cache, keyed by input value.

**Lens** — a projection: a subset of an atom's fields, by name and type.
It declares "this computation depends on exactly these fields and no others."

**Memo** — a pure function from a lens (or an atom directly) to an output.
Annotated with [`#[drv::memo]`][memo-attr] plus a required **cache
strategy** — either `#[drv::memo(single)]` (one slot, last-call caching)
or `#[drv::memo(lru = N)]` (N-slot LRU). The result is cached and only
recomputed when the input fields change.

At the end of your crate, [`drv::assemble!()`][assemble] stitches
everything together.

## Declaring lenses

Two forms. **Inline** lenses are declared by tagging fields on the atom
with `#[drv::lens(Name)]`; the macro builds the lens struct and the
`From<&Atom>` projection for you. **Standalone** lenses are a separate
struct tagged with `#[drv::lens]`, accompanied by a `From<&Atom>` impl
you write — use this when you want fields that don't match the atom
1:1, reach into nested structs, project computed values, or target an
atom from another crate.

For inline lenses, built-in scalar primitives (`u32`, `bool`, `f64`,
`usize`, etc.) are stored **by value** — `lens.x` gives `u32` directly, no
dereference needed. All other types are borrowed as `&T`. User-defined
`Copy` types are *not* auto-detected (the proc macro can only recognise
language built-ins); for full control over field representation, use a
standalone lens.

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

Tag any struct with [`#[drv::lens]`][lens-attr]. You build it however
you like and pass it to memos. Pairing it with `impl From<&Atom>` lets
memos taking `impl Into<MyLens<'_>>` be called as `memo(&atom)`:

```rust
#[drv::lens]
struct MyLens<'a> {
    pub items: &'a Vec<u32>,
    pub viewport_rows: u32,
}

impl<'a> From<&'a AppState> for MyLens<'a> {
    fn from(a: &'a AppState) -> Self {
        Self { items: &a.items, viewport_rows: a.viewport_rows }
    }
}
```

## Calling memos

[`#[drv::memo]`][memo-attr] generates a free function with the same name.
The function body reads from the lens; the generated wrapper handles
memoization. You call it with `&your_atom` — the macro auto-converts
into the right lens:

```rust
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    #[drv::lens(CountLens)]
    pub items: Vec<u32>,
    pub viewport_rows: u32,
}

#[drv::memo(single)]
fn item_count<'a>(lens: impl Into<CountLens<'a>>) -> usize {
    lens.items.len()
}

let app = AppState {
    items: vec![10, 20, 30],
    ..Default::default()
};

let n = item_count(&app);   // pass &AppState; projects to CountLens
assert_eq!(n, 3);
```

Memoization happens behind the scenes — no cache struct, no setup.

### Cache strategy

Every memo must declare a cache strategy. There's no default — you pick
one explicitly so the choice is visible at the memo definition:

```rust
// Single-slot: last-call caching. A hit requires today's inputs to
// equal the most recent recompute's inputs. Cheapest; predictable.
#[drv::memo(single)]
fn sum(s: &S) -> u32 { s.x * 2 }

// LRU-N: remember up to N distinct recent `(input, output)` pairs,
// evicting least-recently-used on overflow. Buys cross-state reuse
// (ping-pong, undo/redo) at slightly higher per-call scan cost.
#[drv::memo(lru = 16)]
fn product(s: &S) -> u32 { s.x * 3 }
```

Pick `single` when the atom moves forward monotonically (most
application state — cursor positions, scroll offsets, selection IDs).
Pick `lru = N` for memos where the input cycles between a small
number of recurring states, and size N to the working set. Lookup
is a linear scan, so `single` and small `lru` values are ~free; large
N trades scan cost for hit rate.

## Using an atom directly

A memo can take the atom itself as input — treated as an "identity lens" over
all data fields. Write the parameter as `&YourStruct` and pass `&your_atom`:

```rust
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Stats {
    pub total: u32,
    pub count: u32,
}

#[drv::memo(single)]
fn average(s: &Stats) -> u32 {
    if s.count == 0 { 0 } else { s.total / s.count }
}
```

Any change to any field of the atom invalidates the cache. Useful when you
really do depend on everything.

## Memos calling other memos

A memo body may invoke another memo on the same atom — composition across
derived values, with each memo's cache short-circuiting independently.
Each memo owns its own thread-local cache, so inner-memo re-entry is
always safe: lookup and install are scoped borrows that don't overlap
with the body.

```rust
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Counter {
    pub value: u32,
}

#[drv::memo(single)]
fn doubled(c: &Counter) -> u32 {
    c.value * 2
}

#[drv::memo(single)]
fn doubled_plus_one(c: &Counter) -> u32 {
    doubled(c) + 1   // calls a sibling memo on the same atom
}

drv::assemble!();

let a = Counter { value: 10 };
assert_eq!(doubled_plus_one(&a), 21);
```

## Multiple lenses

A memo can take lenses from multiple atoms:

```rust
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

#[drv::memo(single)]
fn weighted_score<'a, 'b>(
    hits: impl Into<HitsLens<'a>>,
    settings: impl Into<MultiplierLens<'b>>,
) -> u32 {
    hits.hits.iter().sum::<u32>() * settings.multiplier
}

let score = weighted_score(&game, &settings);
```

If any of the lens field comparisons fail, the memo recomputes. Two lenses from
the same atom (`fn foo(a: &LensA, b: &LensB)` called as `foo(&app, &app)`)
works too.

## Value parameters

Memos can also take owned values or borrowed references to types like
`&str` or `&[u8]` alongside lens parameters. Values participate in the
cache key just like lens fields — any change triggers a recompute.

```rust
#[drv::memo(single)]
fn labeled<'a>(lens: impl Into<CountLens<'a>>, label: &str, multiplier: u32) -> String {
    format!("{}={}", label, lens.count * multiplier)
}
```

Parameter classification:

- `&Lens` or `&MyAtom` — a lens parameter (required: at least one lens).
- Owned types (`u32`, `String`, `MyStruct`) — stored via `Clone`.
- Borrowed types with `ToOwned` (`&str`, `&[u8]`, `&Path`, ...) — stored
  as `<T as ToOwned>::Owned` (so `&str` stores as `String`).

Value types must implement `PartialEq + Clone + Send + 'static`; borrowed
value types must satisfy `T: ToOwned` with the owned form matching the
usual bounds. Declared order is preserved at the call site.

## Chaining

A memo's output can feed into another memo. Mark the output type as an atom too,
and return it by value so downstream memos can project from it:

```rust
// Root atom
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Game {
    pub hits: Vec<u32>,
    pub time_ms: u64,
}

// Lens over Game → produces Stats (itself an atom).
#[drv::lens]
struct HitsLens<'a> {
    pub hits: &'a Vec<u32>,
}

impl<'a> From<&'a Game> for HitsLens<'a> {
    fn from(g: &'a Game) -> Self { Self { hits: &g.hits } }
}

#[drv::memo(single)]
fn stats<'a>(lens: impl Into<HitsLens<'a>>) -> Stats {
    Stats {
        total: lens.hits.iter().sum(),
        count: lens.hits.len() as u32,
        best: lens.hits.iter().copied().max().unwrap_or(0),
    }
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

#[drv::memo(single)]
fn average<'a>(lens: impl Into<AverageLens<'a>>) -> u32 {
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
  the snapshot lives in the memo's thread-local cache.
- **Fields that never appear in a lens and whose atom has no memo taking
  `&MyAtom`** carry no bounds at all.

The identity lens (reached when a memo takes `&MyAtom` directly) is
emitted only when some memo consumes it, so atoms without identity-lens
consumers don't pay for bounds on unrelated fields.

Trait impls on the atom itself (`Clone`, `PartialEq`, `Debug`, `Default`)
are whatever you `#[derive]` — drv doesn't inject or forward anything.

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

Since each memo owns its own thread-local cache keyed by input value,
memos and their atoms are decoupled. Memos can live in a different crate
than the atom they target — no registry coordination is needed across
crates. Split your state into domain crates as fits your app:

```
crate: my-app-buffers   → BufferState atom + buffer memos
crate: my-app-ui        → UiState atom + ui memos
crate: my-app-lsp       → LspState atom + lsp memos
crate: my-app           → composes the above
```

Each crate calls [`drv::assemble!()`][assemble] once after its own
declarations; different crates' assemblies don't interfere.

## Assembly

`drv::assemble!()` must appear once, after every `#[drv::atom]`,
`#[drv::lens]`, and `#[drv::memo]` declaration in the
crate. It collects every registration, emits the memoized free functions,
and sets up each memo's thread-local cache.

```
// lib.rs
mod state;       // atoms
mod views;       // lenses + memos
mod rendering;   // more lenses + memos

drv::assemble!();
```

## Serde

Atoms are plain structs — derive `Serialize`/`Deserialize` on them
directly, with no drv-specific wiring. Roundtripped values are equal
to the originals; whatever cache entries a memo has already built for
their field values are still hits.

## Design goals

- **Plain Rust structs.** Your atom is a plain data struct with whatever
  `#[derive(...)]` you choose. drv injects nothing into it — no wrapper,
  no hidden fields, no cache held alongside the data.
- **Static dependency declaration.** The lens struct _is_ the dependency
  list. The compiler verifies field names and types at compile time
  (or, for a standalone lens, the `From<&Atom>` impl the user wrote).
- **Zero runtime tracking.** No proxy objects, no access instrumentation,
  no subscription management. Just field-by-field `PartialEq` against a
  stashed snapshot.
- **Value-keyed memoization.** Each memo owns a thread-local LRU cache
  keyed by lens/value inputs. Equal inputs from any call — same or
  different atom instance — hit the same cache entry.
- **Free functions, not methods.** Memos are ordinary functions. Call
  sites don't need to know any generated type names.

[atom-attr]: https://docs.rs/drv/latest/drv/attr.atom.html
[memo-attr]: https://docs.rs/drv/latest/drv/attr.memo.html
[lens-attr]: https://docs.rs/drv/latest/drv/attr.lens.html
[assemble]: https://docs.rs/drv/latest/drv/macro.assemble.html
[imbl]: https://docs.rs/imbl

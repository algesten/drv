# drv

Memoized derivations over plain Rust structs.

`drv` lets you declare a struct of ground-truth data (an **atom**), project
subsets of its fields (a **lens**), and compute derived values (a **memo**)
that are automatically cached. When nothing changed, nothing recomputes.

## Quick example

```rust
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

**Atom** — a struct of ground-truth data. Declared with `#[drv::atom]`.
All fields must be `pub` and must implement `PartialEq + Clone + Debug + Default + Send`.

**Lens** — a projection: a subset of an atom's fields, by name and type.
It declares "this computation depends on exactly these fields and no others."

**Memo** — a pure function from a lens (or an atom directly) to an output.
Annotated with `#[drv::memo]`. The result is cached and only recomputed when
the input fields change.

At the end of your crate, `drv::assemble!()` stitches everything together.

## Declaring lenses

There are two ways to declare a lens. Both produce identical generated code.

### Inline: annotate fields on the atom

Tag fields with `#[drv::lens(Name)]` directly on the atom. The macro generates
a lens called `Name` with those fields. This keeps the full dependency picture
in one place:

```rust
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
list them in one attribute `#[drv::lens(A, B)]` or as separate attributes.

### Standalone: declare a separate struct

Declare the lens as its own struct with `#[drv::lens(Atom)]`. The macro verifies
that every field name and type matches the atom:

```rust
#[drv::lens(AppState)]
struct MyLens {
    pub items: Vec<u32>,
    pub viewport_rows: u32,
}
```

Use standalone lenses when the lens is defined closer to the memo that consumes
it, or when the atom is in another module and you don't want to modify it.

## Calling memos

`#[drv::memo]` generates a free function with the same name. The function
body reads from the lens; the generated wrapper handles memoization. You
call it with `&atom` — the macro auto-converts into the right lens:

```rust
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

let mut app = AppState {
    items: vec![10, 20, 30],
    ..Default::default()
};

let n = item_count(&app);   // pass &AppState directly — converts to CountLens
assert_eq!(n, 3);
```

Memoization happens behind the scenes — no cache struct, no setup.

## Using an atom directly

A memo can take the atom itself as input — treated as an "identity lens" over
all data fields:

```rust
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

## Multiple lenses

A memo can take lenses from multiple atoms. The cache lives in the first
parameter's atom:

```rust
#[drv::atom]
pub struct Game {
    #[drv::lens(HitsLens)]
    pub hits: Vec<u32>,
}

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
    format!("{}={}", label, *lens.count * multiplier)
}
```

Parameter classification:

- `&Lens` or `&Atom` — a lens parameter (required: at least one lens).
- Owned types (`u32`, `String`, `MyStruct`) — stored via `Clone`.
- Borrowed types with `ToOwned` (`&str`, `&[u8]`, `&Path`, ...) — stored
  as `<T as ToOwned>::Owned` (so `&str` stores as `String`).

Value types must implement `PartialEq + Clone + Send + 'static`; borrowed
value types must satisfy `T: ToOwned` with the owned form matching the
usual bounds. Declared order is preserved at the call site; the cache is
always stored on the first lens parameter's atom.

## Chaining

A memo's output can feed into another memo. Mark the output type as an atom too.
When constructing it inside the memo body, close with `..Default::default()` so
`drv` can set up its internal state:

```rust
// Root atom
#[drv::atom]
pub struct Game {
    pub hits: Vec<u32>,
    pub time_ms: u64,
}

// Lens over Game → produces Stats (itself an atom).
#[drv::lens(Game)]
struct HitsLens {
    pub hits: Vec<u32>,
}

#[drv::memo]
fn stats(lens: &HitsLens) -> Stats {
    Stats {
        total: lens.hits.iter().sum(),
        count: lens.hits.len() as u32,
        best: lens.hits.iter().copied().max().unwrap_or(0),
        ..Default::default()   // lets drv initialize its internal state
    }
}

// Stats is an atom, so more lenses can project from it.
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
    if *lens.count == 0 { 0 } else { *lens.total / *lens.count }
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

Atom fields must implement `PartialEq + Clone + Debug + Default + Send`.

### What runs when

Constructing a lens from an atom is free (zero-copy — the lens holds
references into the atom). What runs on each call:

- **Every call — `PartialEq` on each lens field.** Used to check whether
  the input has changed since the last call.
- **Every call — `Clone` on the output.** Memos return by value.
- **Cache miss only — `Clone` on each lens field.** A copy is kept for the
  next call's comparison.
- **Cache miss only — the memo body runs.**

Two kinds of costs matter on the hot path:

1. Per-field `PartialEq` cost.
2. Output `Clone` cost.

For `PartialEq`, the field type matters. Scalars are free. Collections
(both `Vec`/`HashMap` and `imbl::Vector`/`imbl::HashMap`) iterate and
compare pairwise — O(n) worst case on a cache hit. `imbl` does not
short-circuit via pointer equality; its advantage is O(1) `Clone` on
cache miss, not faster comparison. For output `Clone`, wrap expensive
outputs in `Rc`/`Arc` for O(1) cloning.

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
- **[`imbl`](https://docs.rs/imbl) persistent collections** — enable the
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

`drv::assemble!()` must appear once, after all `#[drv::atom]`, `#[drv::lens]`,
and `#[drv::memo]` declarations in the crate. It collects all registrations
and emits the cache types, the lens types, and the memoized free functions.

```
// lib.rs
mod state;       // atoms
mod views;       // lenses + memos
mod rendering;   // more lenses + memos

drv::assemble!();
```

## Design goals

- **Plain Rust structs.** Atoms and lenses are regular structs you can construct
  with literal syntax, clone, compare, debug.
- **Static dependency declaration.** The lens struct _is_ the dependency list.
  The compiler verifies field names and types at compile time.
- **Zero runtime tracking.** No proxy objects, no access instrumentation, no
  subscription management. Just field-by-field `PartialEq`.
- **Memoization is automatic.** No cache struct to construct, no explicit
  setup — just call the memo.
- **Free functions, not methods.** Memos are ordinary functions. Call sites
  don't need to know any generated type names.

License: MIT OR Apache-2.0

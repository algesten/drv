# Theory

## The problem

An interactive application has a set of ground-truth data — cursor position,
buffer contents, open tabs, viewport dimensions — and many values derived from
that data: visible lines on screen, syntax highlights in view, status bar text,
which files to watch.

The naive approach recomputes every derived value on every state change. This is
correct but wasteful: a cursor move should not recompute syntax highlights, and a
scroll should not rebuild the tab list.

The FRP approach wires streams with deduplication combinators (`dedupe`,
`sample_combine`, `filter`) to propagate only relevant changes. This is
efficient but imposes significant cognitive load: the programmer must
manually maintain dedup/combine chains, reason about combinator ordering,
and follow subtle rules to avoid bugs (e.g., map-dedupe-filter ordering,
don't-derive-from-state-then-sample-state).

`drv` takes a different approach: **declare the dependency as a type, and let
the compiler and runtime do the rest.**

## Core model

The system has three concepts:

### Atom

An atom is a struct of observable facts. `#[drv::atom]` itself imposes no
trait bounds on the struct or its fields — it only registers the type for
lens/memo machinery. Bounds accrue only through the lenses that actually
project a field: any field reached by a lens must implement
`PartialEq + Clone + Debug`; any field whose snapshot is stored in a memo's
cache state must additionally be `Send + 'static`. Fields that are never
projected — and whose atom is never consumed as an identity lens by any memo
— carry no bounds at all. An atom is the ground truth — it is mutated
directly (plain field assignment) and serves as the root of the dependency
graph.

At use, the atom is just the struct value itself — no wrapper to construct,
no cache held alongside the data. Each memo maintains its own thread-local
cache keyed by input value, so cloning an atom is cheap and sharing-preserving:
two equal-valued atom instances hit the same cache entries.

In database terms, an atom is a **base table**. In signal frameworks, it is a
**state signal** or **observable**. In Datalog, it is the **extensional
database (EDB)**.

### Lens

A lens is a struct that declares the inputs a computation depends on. It
serves two purposes:

1. **Dependency declaration.** The set of fields in the lens is the set of
   values the computation reads. There is no way to accidentally depend on
   a value that isn't declared.

2. **Change detection.** Each lens field is compared against the stored
   snapshot via `PartialEq`. If all fields match, the cached result is
   returned.

There are two forms:

**Inline lenses** (`#[drv::lens(LensName)]` on atom fields): the macro
aggregates all fields tagged with a given lens name into a generated
`LensName` struct. Built-in scalar primitives (`u8`–`u128`, `i8`–`i128`,
`usize`, `isize`, `f32`, `f64`, `bool`, `char`) are stored by value; all
other fields are borrowed as `&'drv T`. Inline lenses are the simplest form
and keep the full dependency picture in one place on the atom.

**Standalone lenses** (`#[drv::lens]` on a separate struct): a plain
struct you build and pass to memos. Pairing it with an
`impl From<&AtomName>` lets memos taking `impl Into<MyLens<'_>>` be
called as `memo(&atom)`. The fields can have any shape — pulling from
nested atom fields, borrowing `&str` from `String`, projecting computed
values, projecting from an atom in another crate.

Standalone lenses compose across crate boundaries: the atom lives in one
crate, the lens + memo in another, no registry coordination needed. The
atom's home crate doesn't need to know about downstream memos.

In FP optics, a lens is a composable accessor into a product type. Here we use
only the "getter" half (projection). In database terms, the lens is a **view
definition** — a projection query. In Redux/Reselect, it is an **input
selector**.

The connection to the incremental computation literature is precise: the lens
is a **verifying trace** (Build Systems a la Carte, Mokhov et al. 2018). The
trace records which inputs were read; the rebuilder compares the trace against
current values to decide whether to recompute. In `drv`, the trace is the
lens struct itself, and the comparison is `PartialEq`. For standalone
lenses with a custom projection, the trace is the *projected* values —
the comparison happens on the lens output, not on the raw atom fields. This
means changes to atom fields that produce the same projected values do not
trigger recomputation.

The atom itself can also be used as a lens — the "identity lens" over all data
fields. This is a convenience for computations that genuinely depend on
everything. The identity lens is generated on demand by `drv::assemble!()`,
only for atoms that some memo actually consumes via `&AtomName` directly:
an atom with no identity-lens consumer pays no cost for it, and imposes no
bounds on fields that aren't reached by some other lens.

### Memo

A memo is a pure function from one or more lenses to an output value. It is
automatically memoized. Every memo declares its cache strategy explicitly:
`#[drv::memo(single)]` for a single-slot last-call cache, or
`#[drv::memo(lru = N)]` for an N-slot LRU cache. Recomputation is skipped
when a slot's stored input matches the current one field-by-field.

Memo signatures are emitted verbatim — the macro does not rewrite parameter
types. A lens parameter can be written as `&MyLens` (honest, caller
projects), `impl Into<MyLens<'a>>` (ergonomic sugar, caller passes `&atom`),
or `&MyAtom` (identity lens). Non-lens parameters (owned values, `&str`,
`&[u8]`, …) pass through unchanged; see *Value parameters* below.

In database terms, a memo is a **materialized view** — a precomputed query
result that is maintained incrementally. In signal frameworks, it is a
**computed signal**. In the incremental computation literature, it is a
**thunk** (Adapton) or **query** (Salsa).

A memo's output can itself be marked as an atom, enabling chaining: the output
of one memo becomes the input to another's lens. This creates a DAG of
derivations, each independently memoized.

## The projection-as-dependency insight

Most memoization systems discover dependencies at runtime. MobX instruments
property access. Comemo wraps values in `Tracked<T>`. Salsa records which
queries a query calls. These approaches are flexible but add runtime overhead
and cognitive indirection.

`drv` makes the dependency declaration a **type**:

```rust
#[drv::atom]
pub struct AppState {
    #[drv::lens(MyLens)]
    pub scroll_row: u32,

    #[drv::lens(MyLens)]
    pub viewport_rows: u32,

    pub cursor_col: u32,  // not in MyLens — changes to it don't invalidate
}
```

This type (the generated `MyLens` struct) *is* the dependency. It is
declared either as inline tags on atom fields (the macro synthesises the
struct and a `From<&Atom>` projection) or as a standalone struct that
the user constructs however they like before passing it to a memo. The
runtime uses the lens for field-by-field `PartialEq`. There is nothing
else — no tracking table, no subscription list, no proxy object.

This is a form of **static dependency analysis** achieved through the type
system rather than program analysis. The programmer writes the projection;
the compiler verifies it; the runtime executes it. Each layer has a clear
role.

In the terminology of "A Theory of Changes for Higher-Order Languages" (Cai
et al., PLDI 2014), a lens is a projection function, and its **derivative**
is trivially the projected fields' changes. The change-propagation step
reduces to: compare the projected fields, skip if equal.

## Change detection: PartialEq

The change detection strategy is field-by-field `PartialEq` of the lens
fields against the atom. This is deliberately simple.

For scalar types (`u32`, `bool`, `String`, small enums), comparison is
trivial — one or a few machine instructions. No optimization needed.

For collections, comparison cost depends on the collection type and on how
`drv` chooses to compare it:

- `Vec<T>`, `HashMap<K,V>`: standard element-wise `PartialEq`, O(n) on a
  cache hit, O(position of first difference) on a miss.

- `Arc<T>`: `drv` recognises `Arc<T>` and uses `Arc::ptr_eq` as a fast
  path before falling back to `PartialEq`. When the snapshot still points
  at the same allocation as the live field (the common case if nothing
  has mutated wholesale), comparison is O(1).

- `imbl::Vector<T>`, `imbl::HashMap<K,V>` and friends (with the `imbl`
  feature): `drv` uses `imbl`'s built-in pointer-equality check on the
  shared root. After a snapshot is cloned, the snapshot and the live field
  share their structural-sharing root, so as long as no mutation has
  reshaped the root, comparison is O(1). On a true mismatch the check
  falls back to element-wise `==`, which itself short-circuits on the
  first difference.

Recommendation: use `Arc<T>` for single large blobs you replace wholesale,
and `imbl` collections for fields you mutate incrementally. Scalar fields
need no special treatment.

Alternative strategies (hash-based comparison, version counters, pointer
equality) are not used because they each introduce either lossy comparison
(hash collisions), controlled mutation (version counters require setters),
or wrapper types (pointer equality requires `Rc`/`Arc`). `PartialEq` is
exact, works with plain field assignment, and requires no wrapper types.
With `imbl`, it is also efficient.

## Memoization strategy

Each `#[drv::memo]` function is transformed by the macro into a free function
with the same name that performs the memoization inline.

Atoms are plain Rust structs — no wrapper, no hidden fields, no `Atomized`
trait. Each memo owns its cache in a `thread_local!` static. The declared
strategy determines `N`: `single` → one slot, `lru = N` → N slots. Both
share the same slot-array codegen (the enum leaves room for new strategies
later to diverge).

```rust
thread_local! {
    static __DRV_FOO_CACHE: RefCell<__DrvFooCacheState> = ...;
}

struct __DrvFooCacheState {
    slots: [Option<(Input, Output, u64)>; N],  // N from strategy
    next_stamp: u64,
}
```

Slots are keyed by **value**, not by atom identity: the `Input` is a snapshot
of the lens and value parameters, and lookup is a linear scan with
`PartialEq<Snapshot>` against each occupied slot. On hit, the slot's LRU
stamp is bumped to `next_stamp + 1`. On miss, the body runs and installs
into an empty slot; if none, the slot with the smallest stamp is evicted.
For `single` (N=1), the scan and the eviction are both over one slot —
the result is last-call caching.

The memo's parameter type — whatever the user wrote — is emitted verbatim.
Three shapes are supported:

- `&Lens[<'_>]` — honest signature; caller must have a `&Lens` already.
  Projection happens at the call site (`(&atom).into()` to get the lens).
- `impl Into<Lens<'a>>` — ergonomic sugar; caller passes any type
  convertible to `Lens<'a>` (typically `&atom`, via the auto-generated
  `From<&Atom>` impl). The body converts once via `.into()`.
- `&MyAtom` — identity-lens form; caller passes `&atom` directly.

The evaluation flow (same for all three — only the entry step differs):

```
call memo(x)
  → (entry depends on sig)
      impl Into<Lens<'a>>     — x.into() produces the Lens
      &Lens                    — x is already the Lens
      &Atom                    — x is the atom; an identity-lens view
                                 is constructed internally for the cache
  → take a short exclusive borrow on the memo's thread-local cache
  → linear-scan slots for the first whose stored input matches the current one
      (field-by-field PartialEq via FastEq, which short-circuits on Arc::ptr_eq
       and imbl ptr_eq fast paths)
    → found: bump its stamp, clone the output, DROP the borrow, return it (FAST PATH)
    → not found: drop the borrow
        → run the memo function with the lens  [no borrow held]
        → take a short exclusive borrow of the cache
        → clone the lens fields + value params into an owned snapshot
        → pick an empty slot or the LRU victim
        → write the (input, output, stamp) tuple
        → drop the borrow
        → return the computed output
```

Comparison is field-by-field `PartialEq` via `drv`'s `FastEq` helper, which
short-circuits on `Arc::ptr_eq` for `Arc<T>` and on the equivalent pointer
check for `imbl` persistent collections (when the `imbl` feature is enabled).
Plain `Vec`/`HashMap` fall through to standard element-wise comparison.

The user's compute function runs **without holding any borrow on the cache**.
The borrow taken for the lookup scan is dropped before the body runs (or
before the cached output is cloned and returned). On a cache miss, a separate
`borrow_mut` is taken *after* compute returns, scoped just to the store step.
The borrow is never held across user code.

This gives a hard re-entrancy guarantee: a memo can re-enter its own cache
or a sibling memo's cache from anywhere — recursively, through a captured
reference in a closure, through user-controlled call paths that loop back —
without triggering a `RefCell` double-borrow panic. Each memo's cache is
independent; inner-memo re-entry operates on a different `RefCell` entirely.
The common case is direct: a memo body receives `&Lens` (or `&T` for an
identity-lens memo) and invokes sibling memos by passing that reference
straight through.

The caches live in thread-locals, so each thread builds up its own cache
independently. Cross-thread mutation of the atom doesn't affect other
threads' caches beyond the usual "new value, miss, recompute" path. Atoms
themselves carry no cache state, so `T: Send` → `atom: Send` trivially.

The return value is returned by value (`Clone::clone` of the cached output).
For cheap types (`usize`, `String`, `imbl` collections), this is effectively
free. For expensive types, the programmer should consider whether full-output
returns are appropriate, or wrap the output in `Rc`/`Arc` for cheap cloning.

### Value-keyed vs per-instance

A consequence of the value-keyed design: two distinct atom instances with
equal field values hit the same cache slot. Cloning an atom doesn't give
it a "fresh empty cache" — cache entries are keyed by the input snapshot,
so equal-valued clones share whatever entries already exist.

This also enables ping-pong hits: if the atom cycles between two input
states (undo/redo, A↔B toggles), both states stay cached (up to the LRU
capacity) and subsequent visits are hits rather than misses.

### Multi-lens memos

When a memo takes multiple lens parameters, each is compared field-by-field
against the slot's snapshot. All parameters must match for a hit.

This handles two cases uniformly:
- Lenses from different atoms (e.g., `fn header(tabs: &TabsLens, user: &UserLens)`)
- Multiple lenses from the same atom (e.g.,
  `fn foo<'a, 'b>(a: impl Into<LensA<'a>>, b: impl Into<LensB<'b>>)`
  called as `foo(&app, &app)`)

### Value parameters

Memos can mix lens parameters with owned value parameters:

```rust
#[drv::memo(single)]
fn compute<'a>(lens: impl Into<MyLens<'a>>, thing: usize, other: String) -> Output { ... }
```

Value parameters participate in the cache key just like lens fields — the
snapshot struct holds them alongside the lens snapshots, each compared by
`PartialEq` on every call. Declared order is preserved.

Requirements on value types: `PartialEq + Clone + Send + 'static`. They are
cloned into storage on cache miss.

### Reference value parameters (`&str`, `&[u8]`, ...)

Non-lens reference parameters (e.g., `&str`, `&[u8]`, `&Path`) are stored via
[`ToOwned`](https://doc.rust-lang.org/std/borrow/trait.ToOwned.html): the
storage type is `<T as ToOwned>::Owned`, the store operation is
`<T as ToOwned>::to_owned(param)`, and comparison happens between the borrow
(e.g., `&str`) and the owned form (`String`) via standard `PartialEq` impls.

Classification in the macro:
- `&Ident` where `Ident` names a known lens or atom → lens parameter.
- `&Ident` where `Ident` is *not* a known lens or atom → promoted to a
  reference value parameter (this is what lets `foo: &str` work; `str` isn't
  a registered lens, so it falls through to `ToOwned`).
- `&NonIdent` (like `&[u8]`, `&dyn Trait`) → reference value parameter directly.
- Non-reference types → owned value parameter.

To catch typos early (e.g., `foo: &MyLen` meant as `&MyLens`), the memo macro
emits a compile-time assertion at the function's definition site:

```rust
const _: fn() = || {
    fn __drv_assert_to_owned<T: ?Sized + ::std::borrow::ToOwned>() {}
    __drv_assert_to_owned::<MyLen>();
};
```

If `MyLen` doesn't exist, the error (`cannot find type 'MyLen' in this scope`)
points at the user's parameter — not at generated code. If it exists but
doesn't implement `ToOwned`, the trait-bound error points at the same place.

## Chaining and transitive memoization

When a memo's output is marked as an atom, other lenses can project from it:

```
Atom A → Lens L1 → Memo M1 → Atom B → Lens L2 → Memo M2 → Output
```

Each link is independently memoized. If A changes but only in fields that
L1 doesn't select, M1 returns cached, B is unchanged, L2 sees no change,
M2 returns cached. Zero work propagates through the entire chain.

If A changes in a field that L1 selects, M1 recomputes and produces a new B.
L2 then compares its fields against the new B. If those specific fields
haven't changed (the recomputation produced the same values), M2 still
returns cached.

This is the **early cutoff** property (Build Systems a la Carte): even when
an upstream computation re-runs, downstream computations are skipped if the
output didn't actually change. In `drv`, early cutoff falls out naturally
from the `PartialEq` check — it doesn't require special support.

For an atom used as a memo output, the programmer constructs it in the memo
body as a plain struct literal. Downstream lenses project from it in exactly
the same way as any other atom.

## Memory layout

An atom is a plain Rust struct. No wrapper. No hidden cache.

```rust
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Editor { /* user fields */ }

// Construct and mutate directly.
let mut e = Editor::default();
e.scroll_row = 5;
```

Each `#[drv::memo]` owns a thread-local cache whose shape is determined at
expansion time by the memo's own `(Input, Output)`:

```rust
// Generated by drv::assemble!() for
// `fn visible_lines<'a>(lens: impl Into<VisibleLens<'a>>) -> Vec<String>`

pub struct __DrvVisibleLinesInput {
    lens: __DrvVisibleLens,  // snapshot of the lens's fields
}

pub struct __DrvVisibleLinesSlot {
    input: __DrvVisibleLinesInput,
    output: Vec<String>,
    stamp: u64,
}

pub struct __DrvVisibleLinesCacheState {
    slots: [Option<__DrvVisibleLinesSlot>; 16],  // N = cache size
    next_stamp: u64,
}

thread_local! {
    static __DRV_VISIBLE_LINES_CACHE: RefCell<__DrvVisibleLinesCacheState> = ...;
}
```

For a memo with mixed parameters like
`fn compute<'a>(lens: impl Into<MyLens<'a>>, thing: usize, prefix: &str) -> Output`,
the input struct carries one field per declared parameter:

```rust
pub struct __DrvComputeInput {
    lens: __DrvMyLens,
    thing: usize,
    prefix: <str as ::std::borrow::ToOwned>::Owned,  // = String
}
```

Access cost on the hot path:

- `thread_local!` lookup (`.with`) — one pointer-ish read on most targets
- `RefCell::borrow_mut()` — a single atomic-like flag check
- Linear scan over the fixed-size slot array — ~1–3ns per slot with short-
  circuiting PartialEq (`Arc::ptr_eq` / `imbl::ptr_eq` fast paths)

On cache miss, a second `borrow_mut` is taken (after the user's compute
function returns) to install into an empty slot or evict the LRU victim.
The borrow is never held across user code.

No `HashMap` lookup, no `TypeId` compare, no subscription table, no graph
traversal.

The trade-off: each memo's cache holds at most N entries per-thread-per-memo.
When the working set exceeds N, the oldest-accessed entry is evicted. Every
memo picks its N explicitly via `#[drv::memo(single)]` (N=1) or
`#[drv::memo(lru = N)]`.

### Cross-crate

Because each memo's cache lives in a thread-local inside its own crate,
memos and their atoms are fully decoupled. A crate can:

- define an atom with no memos (the atom is just a struct with a `#[drv::atom]`
  registration tag);
- define memos over atoms from other crates by declaring a standalone
  `#[drv::lens] struct MyLens { ... }` with an
  `impl From<&foreign_crate::Foo> for MyLens`.

No cross-crate macro coordination is required.

## Relationship to prior art

### vs Reselect (Redux/JS)

Reselect's `createSelector(inputSelectors, resultFn)` is the same pattern:
input selectors project fields, the result function computes a derived value,
memoization skips recomputation when inputs are reference-equal. `drv` differs
in that the input selector is a struct type verified at compile time, and
comparison is `PartialEq` (value equality) rather than reference equality.

### vs Comemo (Typst/Rust)

Comemo discovers dependencies dynamically by wrapping values in `Tracked<T>`
and intercepting method calls. `drv` declares dependencies statically via lens
structs. Comemo is more flexible (dependencies can vary between calls); `drv`
is cheaper (no runtime tracking, no hashing).

### vs Salsa (rust-analyzer/Rust)

Salsa is a full incremental computation framework with a database abstraction,
interned keys, and a red/green algorithm for cycle detection. `drv` is much
simpler: no database, no query keys, no cycle detection. Salsa is appropriate
for large-scale compiler workloads with deep, dynamic dependency graphs. `drv`
is appropriate for applications with a known, static set of derivations over
a flat state.

### vs Signal frameworks (Solid.js, Leptos, Angular)

Signal frameworks track dependencies at runtime via reactive primitives
(`createSignal`, `createMemo`, `createEffect`). `drv` tracks dependencies
at compile time via struct types. Signal frameworks are designed for UI
component trees where dependencies are discovered dynamically as components
render. `drv` is designed for application state where the dependency structure
is known upfront.

### vs Self-Adjusting Computation (Acar, 2005)

SAC records a dynamic dependence graph (DDG) during execution and replays
change-propagation through the graph when inputs change. `drv` uses a simpler
model: the dependency graph is static (declared via lens structs), and
change detection is a flat comparison (no graph traversal). SAC handles
arbitrary programs; `drv` handles the specific pattern of projection +
memoized computation over a flat state.

### vs Incremental Lambda-Calculus (Cai et al., PLDI 2014)

ILC formalizes incremental computation as differentiation: the "derivative"
of a function maps input changes to output changes. For projections, the
derivative is trivial — it is the projected fields' changes. `drv` implements
exactly this special case: the lens is a projection, the change detection is
the trivial derivative (field-by-field comparison), and the "re-evaluation"
is a full recomputation rather than an incremental update. This is optimal
when the cost of recomputation is small relative to the cost of maintaining
incremental update machinery.

## Limitations

- **No dynamic dependencies.** A lens is fixed at compile time. If a
  computation conditionally reads different fields, all possible fields must
  be in the lens. This may cause unnecessary recomputation when an irrelevant
  field changes.

- **No incremental update.** When the cache is stale, the memo recomputes from
  scratch. There is no mechanism to pass "what changed" to the memo and let it
  update incrementally. For most UI derivations (rendering a visible slice,
  computing a status bar), full recomputation is cheap. For expensive
  derivations over large collections, this may be a bottleneck.

- **Bounded cache.** Each memo's cache holds at most N slots per-thread,
  with N chosen at the declaration site: `#[drv::memo(single)]` for N=1
  or `#[drv::memo(lru = N)]`. Working sets that exceed N trigger LRU
  eviction; undersize is a per-memo tuning decision, not a global one.

- **Owned returns.** Memos return their output by value (via `Clone`). For
  cheap types this is free; for expensive outputs, wrap in `Rc`/`Arc`.

- **No cycle detection.** If memo A depends on memo B which depends on memo A,
  the result is infinite recursion. The programmer must ensure the dependency
  graph is acyclic. In practice, lens-based projections rarely create cycles
  because they select from existing data rather than computed data.

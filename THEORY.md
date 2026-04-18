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

An atom is a struct of observable facts. Each field must implement
`PartialEq + Clone + Debug + Default` (derived with plain `#[derive(...)]` —
`#[drv::atom]` itself adds nothing). An atom is the ground truth — it is
mutated directly (plain field assignment) and serves as the root of the
dependency graph.

At use, the data struct is wrapped in `Atom<T>`, which owns both the data and
its memoization cache. Construction is explicit — `Atom::new(MyAtom { ... })`
— but the wrapper derefs transparently to `T`, so field access reads like
normal struct access. Cloning an `Atom<T>` clones the data and starts the
clone with a fresh empty cache.

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

There are two kinds of lens:

**Standard lenses** have fields that are a strict subset of an atom's
fields — same names, same types. The macro verifies this at compile time
and auto-generates the projection. Built-in scalar primitives (`u8`–`u128`,
`i8`–`i128`, `usize`, `isize`, `f32`, `f64`, `bool`, `char`) may be declared
by value, so `lens.x` gives `u32` directly with no dereference needed. All
other types must be declared as `&'a T` — the lens struct itself must be
valid Rust, so any reference field requires a lifetime parameter on the
struct (`struct MyLens<'a> { ... }`). A built-in primitive may also be
written as `&'a T` to force a reference.

The restriction to built-in primitives is deliberate: the proc macro cannot
query trait implementations (like `Copy`) at expansion time — it only sees
syntax. Rather than guessing based on heuristics, `drv` recognises the fixed
set of language primitives that are always `Copy` and always trivially cheap
to copy. A user-defined `#[derive(Copy)] struct Foo(u32)` looks like any
other path type to the macro, and so must be written as `&Foo`. For fields
that should be owned clones, computed values, or different types from the
atom, write a custom projection with `#[drv::proj]`.

**Lenses with a `#[drv::proj]` impl** have user-defined fields that may
differ from the atom in name, type, or nesting depth. The user writes the
`From<&Atom>` conversion (annotated with `#[drv::proj]`), and the macro
generates the snapshot and comparison logic. This allows reaching into
nested structs, borrowing `&str` from `String` fields, or projecting
computed values. This mode is triggered automatically when the lens
struct's fields don't match the atom.

In FP optics, a lens is a composable accessor into a product type. Here we use
only the "getter" half (projection). In database terms, the lens is a **view
definition** — a projection query. In Redux/Reselect, it is an **input
selector**.

The connection to the incremental computation literature is precise: the lens
is a **verifying trace** (Build Systems a la Carte, Mokhov et al. 2018). The
trace records which inputs were read; the rebuilder compares the trace against
current values to decide whether to recompute. In `drv`, the trace is the
lens struct itself, and the comparison is `PartialEq`. For lenses with a
custom `#[drv::proj]` projection, the trace is the *projected* values —
the comparison happens on the lens output, not on the raw atom fields. This
means changes to atom fields that produce the same projected values do not
trigger recomputation.

The atom itself can also be used as a lens — the "identity lens" over all data
fields. This is a convenience for computations that genuinely depend on
everything.

### Memo

A memo is a pure function from one or more lenses to an output value. It is
automatically memoized: the runtime maintains a cache of
`(last_inputs, last_output)` and skips recomputation when every lens matches.

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
#[drv::lens(AppState)]
struct MyLens {
    pub scroll_row: u32,
    pub viewport_rows: u32,
}
```

This type is the dependency. For standard lenses, the compiler verifies it
(field names and types must match the atom). For lenses with a `#[drv::proj]`
impl, the user writes the projection and the compiler verifies the `From`
impl compiles.
The runtime uses the lens for field-by-field `PartialEq`. There is nothing
else — no tracking table, no subscription list, no proxy object.

This is a form of **static dependency analysis** achieved through the type
system rather than program analysis. The programmer writes the projection
(either declaratively via matching field names, or explicitly via a `From`
impl); the compiler verifies it; the runtime executes it. Each layer has a
clear role.

In the terminology of "A Theory of Changes for Higher-Order Languages" (Cai
et al., PLDI 2014), a lens is a projection function, and its **derivative**
is trivially the projected fields' changes. The change-propagation step
reduces to: compare the projected fields, skip if equal.

## Change detection: PartialEq

The change detection strategy is field-by-field `PartialEq` of the lens
fields against the atom. This is deliberately simple.

For scalar types (`u32`, `bool`, `String`, small enums), comparison is
trivial — one or a few machine instructions. No optimization needed.

For collections, comparison cost depends on the collection type:

- `Vec<T>`, `HashMap<K,V>`: comparison is O(n). This is correct but
  potentially expensive for large collections.

- `imbl::Vector<T>`, `imbl::HashMap<K,V>`: `PartialEq` iterates elements
  pairwise (just like `Vec`/`HashMap`) and does *not* short-circuit via
  pointer equality. Cost is O(n) on a cache hit, O(position of first
  difference) on a miss. The structural-sharing advantage of `imbl` applies
  to `Clone` (O(1), refcount bump on the root) and mutation (O(log n)), not
  to equality.

  So `imbl`'s advantage for memoization is primarily that snapshot creation
  on cache miss is cheap. The cache-hit comparison cost is the same order
  as with `Vec`/`HashMap`.

  `imbl::Vector` also exposes a separate `ptr_eq` method that checks whether
  two values share the same root node — explicit pointer equality — but this
  is not used by `==`. Users who want constant-time cache-hit comparison on
  large collections would need to wrap the field in a newtype that uses
  `ptr_eq` inside its `PartialEq`.

The recommendation: use `imbl` collections for any collection field in an
atom. Scalar fields need no special treatment.

Alternative strategies (hash-based comparison, version counters, pointer
equality) are not used because they each introduce either lossy comparison
(hash collisions), controlled mutation (version counters require setters),
or wrapper types (pointer equality requires `Rc`/`Arc`). `PartialEq` is
exact, works with plain field assignment, and requires no wrapper types.
With `imbl`, it is also efficient.

## Memoization strategy

Each `#[drv::memo]` function is transformed by the macro into a free function
with the same name that performs the memoization inline.

The `Atom<A>` wrapper owns both the atom and a `drv::Cache<A>`. `Cache<A: Atom>`
holds `RefCell<A::State>` — a stack-allocated, interior-mutable state struct
unique to each atom type. The state type is supplied by the atom's `impl Atom`,
which `drv::assemble!()` emits. It holds `(input, output)` pairs — one for each
memo that targets this atom.

The evaluation flow:

```
call memo(&a)                      // a: &Atom<MyAtom>
  → convert &drv into the lens (auto via Into):
      built-in primitive Copy types are copied by value, others borrowed by reference
  → take a short shared borrow of a.__drv_cache()
  → compare each lens field against the cached snapshot
    → all equal: clone the cached output, DROP the borrow, return it (FAST PATH)
    → any differ: drop the borrow
        → run the memo function with the new lens  [no borrow held]
        → take a short exclusive borrow of the cache
        → clone the lens fields into an owned snapshot
        → clone the output into storage
        → drop the borrow
        → return the computed output
```

The fast path does no heap allocation at all. The comparison is field-by-field
`PartialEq`. Collection fields (both `Vec`/`HashMap` and `imbl::Vector`/
`imbl::HashMap`) compare element-by-element — O(n) worst case on a hit,
O(position of first difference) on a miss. `imbl`'s structural sharing does
not accelerate `==`; it accelerates `Clone` (O(1)) and mutation (O(log n)).

The user's compute function runs **without holding any borrow on the cache**.
This is important for reentrancy: a memo body can safely call another memo on
the same atom without triggering a `RefCell` double-borrow panic. The memo can
even call itself indirectly (through another memo) — each call scopes its own
short borrow around the cache check and the cache write.

The return value is returned by value (`Clone::clone` of the cached output).
For cheap types (`usize`, `String`, `imbl` collections), this is effectively
free. For expensive types, the programmer should consider whether full-output
returns are appropriate, or wrap the output in `Rc`/`Arc` for cheap cloning.

### Multi-lens memos

When a memo takes multiple lens parameters, the cache lives in the first
lens's atom. The stored input is a single struct (`__Drv{MemoName}Input`)
holding an owned snapshot per lens. On each call, every lens is compared
against its stored snapshot; if any differ, the memo recomputes.

This handles two cases uniformly:
- Lenses from different atoms (e.g., `fn header(tabs: &TabsLens, user: &UserLens)`)
- Multiple lenses from the same atom (e.g., `fn foo(a: &LensA, b: &LensB)` called as `foo(&app, &app)`)

### Value parameters

Memos can mix lens parameters with owned value parameters:

```rust
#[drv::memo]
fn compute(lens: &MyLens, thing: usize, other: String) -> Output { ... }
```

Value parameters participate in the cache key just like lens fields — the
snapshot struct holds them alongside the lens snapshots, each compared by
`PartialEq` on every call. Declared order is preserved; the cache location
is still pinned to the first lens parameter's atom, regardless of whether
value params come before or after it.

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
body wrapped in `Atom::new(...)`. The cache is fresh on every recomputation of
the upstream memo — which is correct because the atom value itself is new.

## Memory layout

`Atom<A>` owns both the atom data and its cache:

```rust
pub trait Atom {
    type State: Default + Send + 'static;
}

pub struct Cache<A: Atom> {
    inner: RefCell<A::State>,
}

pub struct Atom<T: Atomized> {
    inner: T,
    cache: Cache<T>,
}
```

`drv::assemble!()` emits a per-atom state struct and implements `Atom` for
each atom:

```rust
// Generated by drv::assemble!()

// Per-memo input snapshot (fields for each declared parameter, in order).
#[derive(Default)]
pub struct __DrvVisibleLinesInput {
    lens: __DrvVisibleLens,
}

#[derive(Default)]
pub struct __DrvStatusBarInput {
    lens: __DrvStatusBarLens,
}

// Per-atom state: one (input, output) pair per memo targeting the atom.
#[derive(Default)]
pub struct __DrvEditorState {
    visible_lines_input: Option<__DrvVisibleLinesInput>,
    visible_lines_output: Option<Vec<String>>,
    status_bar_input: Option<__DrvStatusBarInput>,
    status_bar_output: Option<StatusBar>,
    // ... one pair per memo targeting Editor
}

impl drv::Atom for Editor {
    type State = __DrvEditorState;
}
```

For a memo with mixed parameters like
`fn compute(lens: &MyLens, thing: usize, prefix: &str) -> Output`,
the input struct carries one field per declared parameter:

```rust
#[derive(Default)]
pub struct __DrvComputeInput {
    lens: __DrvMyLens,
    thing: usize,
    prefix: <str as ::std::borrow::ToOwned>::Owned,  // = String
}
```

The associated type `Atomized::State` bridges the ordering mismatch: `Atom<T>`'s
field type (`Cache<T>`) is known at atom-definition time, while the concrete
state layout depends on which memos exist and is resolved later via the trait
impl. The `Send` bound on `State` propagates to `Atom<T>`, so wrapped atoms can
be moved across threads.

No heap allocation, no type erasure, no `Box`. The state struct is inlined
directly next to the atom inside `Atom<T>`. Access cost on the hot path:

- `RefCell::borrow()` — a single atomic-like flag check
- Direct field access into `A::State` — known offset, no downcast

On cache miss, one additional `RefCell::borrow_mut()` is taken (after the user's
compute function returns) to store the new snapshot. The borrow is never held
across user code.

No `HashMap` lookup, no `TypeId` compare, no subscription table, no graph
traversal.

The trade-off: every atom instance carries its full state struct inline, so
atoms get bigger as more memos target them. For applications with a few atoms
and many memos this is strictly better than heap-allocated type-erased storage.
For applications creating many atom instances with many memos, the size
increase may matter.

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

- **Single-crate scope.** All memos for an atom must be in the same crate.
  Cross-crate memos are not supported. `drv::assemble!()` collects everything
  within a single compilation unit.

- **Owned returns.** Memos return their output by value (via `Clone`). For
  cheap types this is free; for expensive outputs, wrap in `Rc`/`Arc`.

- **No cycle detection.** If memo A depends on memo B which depends on memo A,
  the result is infinite recursion. The programmer must ensure the dependency
  graph is acyclic. In practice, lens-based projections rarely create cycles
  because they select from existing data rather than computed data.

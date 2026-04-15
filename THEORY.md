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
`PartialEq + Clone + Debug + Default`. An atom is the ground truth — it is
mutated directly (plain field assignment) and serves as the root of the
dependency graph.

An atom also carries its own memoization cache as a hidden field (`__drv`). The
cache is invisible to the programmer: it is transparent to `PartialEq`, `Clone`,
`Debug`, and `Default`, so atoms behave like plain data for all observation
purposes.

In database terms, an atom is a **base table**. In signal frameworks, it is a
**state signal** or **observable**. In Datalog, it is the **extensional
database (EDB)**.

### Lens

A lens is a struct whose fields are a strict subset of an atom's fields — same
names, same types. It declares "this computation depends on exactly these
fields and no others."

The lens serves two purposes:

1. **Dependency declaration.** The set of fields in the lens is the set of
   fields the computation reads. This is verified at compile time. There is no
   way to accidentally depend on a field that isn't declared.

2. **Change detection.** Each lens field is compared against the corresponding
   atom field via `PartialEq`. If all fields match, the cached result is
   returned.

In FP optics, a lens is a composable accessor into a product type. Here we use
only the "getter" half (projection). In database terms, the lens is a **view
definition** — a projection query. In Redux/Reselect, it is an **input
selector**.

The connection to the incremental computation literature is precise: the lens
is a **verifying trace** (Build Systems a la Carte, Mokhov et al. 2018). The
trace records which inputs were read; the rebuilder compares the trace against
current values to decide whether to recompute. In `drv`, the trace is the
lens struct itself, and the comparison is `PartialEq`.

The atom itself can also be used as a lens — the "identity lens" over all data
fields. This is a convenience for computations that genuinely depend on
everything.

### Expr

An expr is a pure function from one or more lenses to an output value. It is
automatically memoized: the runtime maintains a cache of
`(last_inputs, last_output)` and skips recomputation when every lens matches.

In database terms, the expr is a **materialized view** — a precomputed query
result that is maintained incrementally. In signal frameworks, it is a
**computed signal** or **memo**. In the incremental computation literature, it
is a **thunk** (Adapton) or **query** (Salsa).

An expr's output can itself be marked as an atom, enabling chaining: the output
of one expr becomes the input to another's lens. This creates a DAG of
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

This type is the dependency. The compiler verifies it (field names and types
must match the atom). The runtime uses it (field-by-field `PartialEq`). There
is nothing else — no tracking table, no subscription list, no proxy object.

This is a form of **static dependency analysis** achieved through the type
system rather than program analysis. The programmer writes the projection;
the compiler verifies it; the runtime executes it. Each layer has a clear role.

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

- `imbl::Vector<T>`, `imbl::HashMap<K,V>`: comparison is O(log n) between
  structurally related versions. These persistent data structures share
  internal nodes via reference counting. Two versions derived from the same
  original share most of their structure. Comparison walks only the subtrees
  that differ.

  Clone is O(1) (reference count bump on the root node), so constructing a
  lens from an atom is also cheap.

The recommendation: use `imbl` collections for any collection field in an
atom. Scalar fields need no special treatment.

Alternative strategies (hash-based comparison, version counters, pointer
equality) are not used because they each introduce either lossy comparison
(hash collisions), controlled mutation (version counters require setters),
or wrapper types (pointer equality requires `Rc`/`Arc`). `PartialEq` is
exact, works with plain field assignment, and requires no wrapper types.
With `imbl`, it is also efficient.

## Memoization strategy

Each `#[drv::expr]` function is transformed by the macro into a free function
with the same name that performs the memoization inline.

The atom carries a `__drv: drv::Cache` field — a type-erased, interior-mutable
slot (`RefCell<Option<Box<dyn Any>>>`). On first call, the slot is initialized
with a per-atom state struct holding `(input, output)` pairs — one for each
expr that targets this atom.

The evaluation flow:

```
call expr(&atom)
  → convert &atom into the lens (auto via Into)
  → borrow atom.__drv mutably
  → compare each lens field against the cached snapshot
    → all equal: clone and return the cached output (FAST PATH)
    → any differ:
        → run the expr function with the new lens
        → clone the lens fields into an owned snapshot
        → store new snapshot + output in the cache
        → clone and return the new output
```

The fast path does no heap allocation beyond the `RefCell` borrow. The
comparison is field-by-field `PartialEq`. For `imbl` collections, this may
short-circuit on pointer-equal subtrees, making the comparison sublinear.

The return value is returned by value (`Clone::clone` of the cached output).
For cheap types (`usize`, `String`, `imbl` collections), this is effectively
free. For expensive types, the programmer should consider whether full-output
returns are appropriate, or wrap the output in `Rc`/`Arc` for cheap cloning.

### Multi-lens exprs

When an expr takes multiple lens parameters, the cache lives in the first
parameter's atom. The stored input is a tuple of owned snapshots, one per
lens. On each call, every lens is compared against its stored snapshot;
if any differ, the expr recomputes.

This handles two cases uniformly:
- Lenses from different atoms (e.g., `fn header(tabs: &TabsLens, user: &UserLens)`)
- Multiple lenses from the same atom (e.g., `fn foo(a: &LensA, b: &LensB)` called as `foo(&app, &app)`)

## Chaining and transitive memoization

When an expr's output is marked as an atom, other lenses can project from it:

```
Atom A → Lens L1 → Expr E1 → Atom B → Lens L2 → Expr E2 → Output
```

Each link is independently memoized. If A changes but only in fields that
L1 doesn't select, E1 returns cached, B is unchanged, L2 sees no change,
E2 returns cached. Zero work propagates through the entire chain.

If A changes in a field that L1 selects, E1 recomputes and produces a new B.
L2 then compares its fields against the new B. If those specific fields
haven't changed (the recomputation produced the same values), E2 still
returns cached.

This is the **early cutoff** property (Build Systems a la Carte): even when
an upstream computation re-runs, downstream computations are skipped if the
output didn't actually change. In `drv`, early cutoff falls out naturally
from the `PartialEq` check — it doesn't require special support.

For an atom used as an expr output, the programmer constructs it in the expr
body with `..Default::default()` to fill in the hidden cache field. The
cache is fresh on every recomputation of the upstream expr — which is correct
because the atom value itself is new.

## Memory layout

Each atom carries its own cache via the `__drv: drv::Cache` field:

```rust
pub struct Cache {
    inner: RefCell<Option<Box<dyn Any>>>,
}
```

On first call to any expr targeting this atom, the `Box<dyn Any>` is
initialized with a state struct generated by `drv::assemble!()`:

```rust
// Generated by drv::assemble!()
pub struct __DrvEditorState {
    visible_lines_input: Option<__DrvVisibleLens>,
    visible_lines_output: Option<Vec<String>>,
    status_bar_input: Option<__DrvStatusBarLens>,
    status_bar_output: Option<StatusBar>,
    // ... one pair per expr targeting Editor
}
```

The type erasure (`Box<dyn Any>`) allows the atom type to be defined without
forward-referencing the state type — which is important because the state
type depends on what exprs exist, and exprs can be declared in any order
relative to the atom.

Access cost:
- `RefCell::borrow_mut()` — a single atomic-like flag check
- `Box<dyn Any>::downcast_mut::<T>()` — a single `TypeId` comparison
- Lazy initialization on first use — one `Box::new` allocation per atom
  instance per expr-owning atom

After initialization, the hot path is: flag check, downcast, field comparison.
No `HashMap` lookup, no subscription table, no graph traversal.

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

- **No incremental update.** When the cache is stale, the expr recomputes from
  scratch. There is no mechanism to pass "what changed" to the expr and let it
  update incrementally. For most UI derivations (rendering a visible slice,
  computing a status bar), full recomputation is cheap. For expensive
  derivations over large collections, this may be a bottleneck.

- **Single-crate scope.** All exprs for an atom must be in the same crate.
  Cross-crate exprs are not supported. `drv::assemble!()` collects everything
  within a single compilation unit.

- **Owned returns.** Exprs return their output by value (via `Clone`). For
  cheap types this is free; for expensive outputs, wrap in `Rc`/`Arc`.

- **No cycle detection.** If expr A depends on expr B which depends on expr A,
  the result is infinite recursion. The programmer must ensure the dependency
  graph is acyclic. In practice, lens-based projections rarely create cycles
  because they select from existing data rather than computed data.

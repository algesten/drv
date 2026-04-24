# drv

> [!WARNING]
> **Vibe coded.** I've focused on the overall architecture rather than reviewing
> the code output in detail. For projects I've written by hand, see
> [ureq](https://github.com/algesten/ureq) and
> [str0m](https://github.com/algesten/str0m).

Memoize a function with `#[drv::memo]`. The attribute is liberal about
parameter types — owned values, references, struct refs, `&str`,
`&[u8]`, `#[derive(drv::Input)]` projections — and caches results in a
per-memo thread-local slot array keyed by value equality.

```rust
#[derive(drv::Input)]
pub struct Config {
    pub threads: u32,
    pub timeout_ms: u64,
}

#[drv::memo(single)]
fn worker_count(c: &Config) -> u32 {
    c.threads * 2
}

let c = Config { threads: 4, timeout_ms: 5000 };
assert_eq!(worker_count(&c), 8);   // computes
assert_eq!(worker_count(&c), 8);   // cache hit, no work
```

The companion derive `#[derive(drv::Input)]` is a helper for one
specific situation: when you want to cache on a *subset* of a source
struct's fields without cloning the whole struct on every call. See
[Zero-copy projections](#zero-copy-projections-with-drvinput).

## Why drv

- **Equality-keyed, not hash-keyed.** No hashing on the hot path, no
  `HashMap` probe. Cache lookup is a linear scan through a fixed-shape
  slot array with per-field equality.
- **Thread-local caches.** Every memo owns its own cache —
  single-writer, lock-free.
- **Zero allocations on cache hit.** A hit is an equality check plus a
  `Clone` of the output.
- **O(1) cache-hit check for `Arc<T>` and `imbl` collections.** A
  pointer-equality fast path skips deep comparison when the field's
  pointer hasn't changed since the last miss.
- **One concept for inputs.** `#[derive(drv::Input)]` is the single
  opt-in: any struct you want as a memo parameter derives it. Plain
  structs, borrowed projections, and nested-input bundles all work
  the same way.

## Writing a memo

Every memo picks a cache strategy:

- `#[drv::memo(single)]` — one slot, last-call caching. A hit requires
  today's inputs to equal the most recent recompute's inputs.
- `#[drv::memo(lru = N)]` — N slots, least-recently-used eviction. For
  inputs that cycle between a small number of recurring states.

### Parameters

Every parameter must implement [`ToStatic`]. That trait is
implemented by drv for primitives, `String`, `Arc<T>`, imbl
collections (feature-gated), and — via a reference blanket — `&T`
for any `T: ToStatic`. Containers (`Vec`, `Option`, tuples, arrays,
`HashMap` / `HashSet` / `BTreeMap` / `BTreeSet`) are recursive:
they implement `ToStatic` whenever their elements do, so nested
projections like `Vec<MyInput<'a>>` work without any extra
annotation. User types become inputs by adding
`#[derive(drv::Input)]`.

| You write | Needs `#[derive(drv::Input)]` | Notes |
|---|---|---|
| `x: T` (primitive, std type) |  | Shipped impl. |
| `x: &str`, `x: &[u8]` |  | Reference blanket. |
| `x: Arc<T>` |  | ptr_eq fast path. |
| `x: MyInput<'a>` | ✅ | Borrowed projection. |
| `x: &MyStruct` | ✅ | Plain owned struct. |

Bodies see the exact type you declared: strip `#[drv::memo]` and the
function still compiles.

## Zero-copy projections with `#[derive(drv::Input)]`

Take the previous `Config` example. If `Config` grows a big
`Vec<Worker>` field that `worker_count` doesn't read, the default
`&Config` form still snapshots the whole struct into the cache slot
and every cache-hit check compares the whole thing.

`#[derive(drv::Input)]` lets you declare a lightweight view that
borrows only the fields the memo actually depends on. The derive
auto-generates the owned snapshot and the machinery `#[drv::memo]`
uses internally:

```rust
#[derive(drv::Input)]
struct TotalInput<'a> {
    pub hits: &'a Vec<u32>,
}

impl<'a> TotalInput<'a> {
    pub fn new(s: &'a Scoreboard) -> Self { Self { hits: &s.hits } }
}

#[drv::memo(single)]
fn total_score<'a>(input: TotalInput<'a>) -> u32 {
    input.hits.iter().sum()
}

let mut game = Scoreboard { hits: vec![100, 250, 50], player_x: 0 };
assert_eq!(total_score(TotalInput::new(&game)), 400);   // computes
game.player_x = 42;                                      // not in TotalInput
assert_eq!(total_score(TotalInput::new(&game)), 400);   // cache hit
```

Only `hits` enters the cache key. Changes to `player_x` don't
invalidate; changes to `hits` do. The projection is whatever code you
write — a `::new` method, a `From<&Source>` impl, or an inline struct
literal at the call site. drv doesn't prescribe one.

## Nested inputs

A `#[derive(drv::Input)]` struct can have another
`#[derive(drv::Input)]` struct as a field — useful for bundling a
handful of sub-projections into one memo parameter:

```rust
#[derive(drv::Input)]
struct ChildA<'a> { pub a: &'a Vec<u32> }

#[derive(drv::Input)]
struct ChildB<'a> { pub b: &'a Vec<u32> }

#[derive(drv::Input)]
struct Both<'a> {
    pub ca: ChildA<'a>,
    pub cb: ChildB<'a>,
}

#[drv::memo(single)]
fn sum_both<'a>(input: Both<'a>) -> u32 {
    input.ca.a.iter().sum::<u32>() + input.cb.b.iter().sum::<u32>()
}
```

`Vec`, `Option`, tuples, `HashMap` / `BTreeMap` values, and arrays
are all recursive — a field type like `Vec<MyInput<'a>>` or
`HashMap<String, MyInput<'a>>` works without any extra annotation.
The derive handles generic type parameters, tuple structs, and unit
structs as well.

## Performance

Two sources of per-call work:

1. **Per-field equality check** on cache-hit lookup.
2. **Output `Clone`** on every return.

drv's `ToStatic` impls for `Arc<T>` and (under the `imbl` feature)
`imbl`'s persistent collections take a pointer-equality fast path —
O(1) when the field hasn't been mutated since the last miss.

| Type | `Clone` | Cache hit (same pointer) | Cache hit (equal contents) | Mutation |
|------|-------|--------------------------|----------------------------|----------|
| `Vec<T>` | O(n) | O(n) | O(n) | O(1) amortised |
| `HashMap<K, V>` | O(n) | O(n) | O(n) | O(1) amortised |
| `Arc<T>` | **O(1)** | **O(1)** | O(eq of T) | n/a |
| `imbl::Vector<T>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |
| `imbl::HashMap<K, V>` (`imbl` feature) | **O(1)** | **O(1)** | O(n) | O(log n) |

Rule of thumb: scalars are free; small `Vec` / `String` is fine; for
collections with more than a handful of elements, wrap in `Arc<T>` or
reach for `imbl`.

Enable `imbl`:

```toml
[dependencies]
drv = { version = "0.4", features = ["imbl"] }
```

## Comparison

Ranked from most to least alike.

- [`comemo`] — closest in spirit. Memoises functions with
  fine-grained dependency tracking via runtime access recording
  (`#[track]`). drv's static input struct is cheaper per call but
  asks you to declare dependencies up front rather than discovering
  them at runtime.
- [`salsa`] — incremental-computation database used by rust-analyzer.
  Tracks a dependency graph across queries; much more powerful than
  drv for deeply chained derivations, and much heavier.
- [`cached`] / [`memoize`] — general-purpose memoisation via `Hash` of
  arguments, backed by `HashMap` under a lock. Work for any hashable
  input; drv skips hashing entirely and trades generality for
  hot-path speed and field-level invalidation.
- [`moka`] / [`quick_cache`] / [`stretto`] — concurrent in-memory
  cache data structures (Caffeine / Ristretto ports). Not
  memoisation crates — they're backing stores you'd build a cache
  on top of. drv's thread-local single-writer model is the opposite
  design choice.

[`cached`]: https://crates.io/crates/cached
[`memoize`]: https://crates.io/crates/memoize
[`moka`]: https://crates.io/crates/moka
[`quick_cache`]: https://crates.io/crates/quick_cache
[`stretto`]: https://crates.io/crates/stretto
[`salsa`]: https://crates.io/crates/salsa
[`comemo`]: https://crates.io/crates/comemo

## License

Dual-licensed under MIT or Apache-2.0, at your option.

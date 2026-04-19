# drv

> [!WARNING]
> **Vibe coded.** I've focused on the overall architecture rather than reviewing
> the code output in detail. For projects I've written by hand, see
> [ureq](https://github.com/algesten/ureq) and
> [str0m](https://github.com/algesten/str0m).

Memoize a function with `#[drv::memo]`. The attribute is liberal about
parameter types — owned values, references, struct refs, `&str`,
`&[u8]` — and caches results in a per-memo thread-local slot array
keyed by value equality.

```rust
#[derive(Debug, Clone, PartialEq)]
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

The companion attribute `#[drv::input]` is a helper for one specific
situation: when you want to cache on a *subset* of a source struct's
fields without cloning the whole struct on every call. See
[Zero-copy projections](#zero-copy-projections-with-drvinput).

## Why drv

- **Equality-keyed, not hash-keyed.** No hashing on the hot path, no
  `HashMap` probe. Cache lookup is a linear scan through a fixed-shape
  slot array with per-field `PartialEq`.
- **Thread-local caches.** Every memo owns its own cache —
  single-writer, lock-free.
- **Zero allocations on cache hit.** A hit is `PartialEq` plus a
  `Clone` of the output.
- **O(1) cache-hit check for `Arc<T>` and `imbl` collections.** A
  pointer-equality fast path skips deep comparison when the field's
  pointer hasn't changed since the last miss.
- **Small public surface.** `#[drv::memo]` is all you need for the
  common case; `#[drv::input]` is an opt-in helper for borrowed
  projections.

## Writing a memo

Every memo picks a cache strategy:

- `#[drv::memo(single)]` — one slot, last-call caching. A hit requires
  today's inputs to equal the most recent recompute's inputs.
- `#[drv::memo(lru = N)]` — N slots, least-recently-used eviction. For
  inputs that cycle between a small number of recurring states.

### Parameters

| You write | Needs `#[drv::input]` | Notes |
|---|---|---|
| `x: T` |  | `T: Clone + PartialEq + 'static` |
| `x: &T` |  | `T: ?Sized + ToOwned` (e.g. `&str` → `String`) |
| `x: MyInput<'a>` | ✅ | |
| `x: &MyInput<'a>` | ✅ | |

Any by-value type whose last path segment carries a lifetime argument
(`MyInput<'a>`) is treated as a borrowed projection and must be tagged
`#[drv::input]`. Everything else — owned values, `&str`, `&[u8]`,
`&MyStruct`, etc. — works without the helper.

Bodies see the exact type you declared: strip `#[drv::memo]` and the
function still compiles.

## Zero-copy projections with `#[drv::input]`

Take the previous `Config` example. If `Config` grows a big
`Vec<Worker>` field that `worker_count` doesn't read, the default
`&Config` form still snapshots the whole struct into the cache slot
via `Clone`, and every cache-hit check compares the whole thing.

`#[drv::input]` lets you declare a lightweight view that borrows only
the fields the memo actually depends on. The macro auto-generates the
owned snapshot, the `PartialEq` against it, and the snapshot method
`#[drv::memo]` uses internally:

```rust
#[drv::input]
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

## Performance

Two sources of per-call work:

1. **Per-field `PartialEq`** on the cache-hit check.
2. **Output `Clone`** on every return.

drv wires `FastEq` into the per-field comparison so that `Arc<T>` and
(with the `imbl` feature) `imbl`'s persistent collections take a
pointer-equality fast path — O(1) when the field hasn't been mutated
since the last miss.

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
drv = { version = "0.2", features = ["imbl"] }
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

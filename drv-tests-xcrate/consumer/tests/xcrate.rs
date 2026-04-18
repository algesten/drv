use drv_tests_xcrate_consumer::count_plus_one;
use drv_tests_xcrate_core::Config;

#[test]
fn memo_on_foreign_atom() {
    let c = Config {
        count: 10,
        label: "hi".into(),
    };
    assert_eq!(count_plus_one(&c), 11);

    // Same value → cache hit (no recompute; we don't observe it directly,
    // but the second call must at least produce the same result).
    assert_eq!(count_plus_one(&c), 11);

    // Different instance, same field value → value-keyed cache hits.
    let c2 = Config {
        count: 10,
        label: "world".into(), // `label` isn't in the lens; doesn't matter
    };
    assert_eq!(count_plus_one(&c2), 11);

    // Different projected value → recompute.
    let c3 = Config {
        count: 42,
        label: "hi".into(),
    };
    assert_eq!(count_plus_one(&c3), 43);
}

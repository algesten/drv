use drv_tests_xcrate_consumer::count_plus_one;
use drv_tests_xcrate_core::Config;

#[test]
fn memo_on_foreign_source() {
    let c = Config {
        count: 10,
        label: "hi".into(),
    };
    assert_eq!(count_plus_one((&c).into()), 11);

    // Same value → cache hit (no recompute; we don't observe it directly,
    // but the second call must at least produce the same result).
    assert_eq!(count_plus_one((&c).into()), 11);

    // Different instance, same field value → value-keyed cache hits.
    let c2 = Config {
        count: 10,
        label: "world".into(), // `label` isn't in the input; doesn't matter
    };
    assert_eq!(count_plus_one((&c2).into()), 11);

    // Different projected value → recompute.
    let c3 = Config {
        count: 42,
        label: "hi".into(),
    };
    assert_eq!(count_plus_one((&c3).into()), 43);
}

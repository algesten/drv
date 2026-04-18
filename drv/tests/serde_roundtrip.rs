//! Verify that `#[drv::atom]` doesn't interfere with user-derived
//! `Serialize` / `Deserialize` on the atom struct, under the `serde`
//! feature.
//!
//! Atoms are plain structs now — drv doesn't inject a wrapper or forward
//! trait impls. This test exists mostly to catch regressions from future
//! macro changes that might add hidden fields or bounds.

#![cfg(feature = "serde")]

use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[drv::atom]
pub struct Counter {
    #[drv::lens(NLens)]
    pub n: u32,
    pub label: String,
}

#[drv::memo(single)]
fn doubled(lens: &NLens) -> u32 {
    lens.n * 2
}

drv::assemble!();

fn assert_ser_de<T: Serialize + DeserializeOwned>() {}

#[test]
fn atom_supports_user_derived_serde() {
    assert_ser_de::<Counter>();

    // Smoke-test: build an atom, roundtrip it via serde_json (only if the
    // tests bring in the crate themselves) or just exercise the memo to
    // prove the generated code compiled with serde derives in place.
    let a = Counter {
        n: 21,
        label: "answer".into(),
    };
    assert_eq!(doubled(&a), 42);
}

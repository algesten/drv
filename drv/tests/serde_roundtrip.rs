//! Verify that `Atom<T>` forwards `Serialize` / `Deserialize` when `T`
//! implements them, under the `serde` feature.
//!
//! The impls are trivial delegations to `T`, so this test only needs to prove
//! the bounds line up — no actual format involved. A function that requires
//! `Serialize + DeserializeOwned` on `Atom<Counter>` either compiles (the
//! impls are wired correctly) or doesn't (regression).

#![cfg(feature = "serde")]

use drv::Atom;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[drv::atom]
pub struct Counter {
    #[drv::lens(NLens)]
    pub n: u32,
    pub label: String,
}

#[drv::memo]
fn doubled(lens: &NLens) -> u32 {
    lens.n * 2
}

drv::assemble!();

fn assert_ser_de<T: Serialize + DeserializeOwned>() {}

#[test]
fn atom_forwards_serde_traits() {
    assert_ser_de::<Atom<Counter>>();

    // Smoke-test the actual machinery: create an atom, use it, and confirm
    // the inner state is what we expect. Serialization correctness itself
    // is guaranteed by `T`'s derived impls.
    let a = Atom::new(Counter {
        n: 21,
        label: "answer".into(),
    });
    assert_eq!(doubled(&a), 42);
}

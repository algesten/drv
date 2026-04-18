//! Consumer crate — defines a lens + memo targeting an atom from a
//! foreign crate (`drv-tests-xcrate-core`).
//!
//! The pattern for a lens over an atom in another crate: declare the lens
//! as a standalone `#[drv::lens]` struct here, then write the
//! `From<&foreign::Atom> for MyLens` impl that projects the fields you
//! care about. drv doesn't track the atom/lens association — it's only
//! known via the `From` impl's signature.

use drv_tests_xcrate_core::Config;

#[drv::lens]
pub struct CountLens {
    pub count: u32,
}

impl From<&Config> for CountLens {
    fn from(c: &Config) -> Self {
        Self { count: c.count }
    }
}

#[drv::memo(single)]
pub fn count_plus_one(lens: impl Into<CountLens>) -> u32 {
    lens.count + 1
}

drv::assemble!();

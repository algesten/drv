//! Consumer crate — defines an input + memo targeting a source struct from
//! a foreign crate (`drv-tests-xcrate-core`).
//!
//! Pattern for an input over a struct in another crate: declare the input as
//! a standalone `#[derive(drv::Input)]` struct here, then write the
//! `From<&foreign::Source> for MyInput` impl that projects the fields you
//! care about. drv doesn't track the source/input association — it's only
//! known via the `From` impl's signature.

use std::marker::PhantomData;

use drv_tests_xcrate_core::Config;

#[derive(drv::Input)]
pub struct CountInput<'a> {
    pub count: u32,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a Config> for CountInput<'a> {
    fn from(c: &'a Config) -> Self {
        Self {
            count: c.count,
            _p: PhantomData,
        }
    }
}

#[drv::memo(single)]
pub fn count_plus_one<'a>(input: CountInput<'a>) -> u32 {
    input.count + 1
}

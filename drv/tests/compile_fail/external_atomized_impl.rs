// `Atomized` is sealed: only `drv::assemble!()` may emit impls of it.
// User code attempting to implement it manually must fail to compile
// because the private super-trait `__sealed::Sealed` cannot be satisfied
// from outside the `drv` crate.

pub struct Foo {
    pub x: u32,
}

impl drv::Atomized for Foo {
    type State = ();
}

fn main() {}

//! Source-only crate — demonstrates that a source struct can live in a
//! foreign crate with no knowledge of the memos that will target it. The
//! struct is plain Rust; `drv` is not a dependency here.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub count: u32,
    pub label: String,
}

// A standalone lens cannot store a non-Copy type by value.
// `String` must be written as `&String`.

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Dashboard {
    pub user_name: String,
    pub count: u32,
}

#[drv::lens(Dashboard)]
struct BadLens {
    pub user_name: String,
}

drv::assemble!();

fn main() {}

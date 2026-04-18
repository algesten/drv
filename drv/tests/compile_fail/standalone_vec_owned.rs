// A standalone lens cannot store a non-Copy type by value.
// `Vec<u32>` must be written as `&Vec<u32>`.

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    pub items: Vec<u32>,
    pub count: u32,
}

#[drv::lens(AppState)]
struct BadLens {
    pub items: Vec<u32>,
}

drv::assemble!();

fn main() {}

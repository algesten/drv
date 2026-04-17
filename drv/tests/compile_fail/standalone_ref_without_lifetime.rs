// A standalone lens with a reference field must declare a lifetime on the
// struct — removing the `#[drv::lens]` attribute must leave valid Rust.

#[drv::atom]
pub struct AppState {
    pub items: Vec<u32>,
    pub count: u32,
}

#[drv::lens(AppState)]
struct BadLens {
    pub items: &Vec<u32>,
}

drv::assemble!();

fn main() {}

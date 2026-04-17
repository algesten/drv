// A standalone lens with only Copy-primitive fields must not declare a lifetime
// (it would be unused).

#[drv::atom]
pub struct AppState {
    pub count: u32,
    pub offset: u32,
}

#[drv::lens(AppState)]
struct BadLens<'a> {
    pub count: u32,
    pub offset: u32,
}

drv::assemble!();

fn main() {}

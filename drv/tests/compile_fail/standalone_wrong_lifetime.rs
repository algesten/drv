// A standalone lens reference field must use the lifetime declared on the struct,
// not `'static` or any other unrelated lifetime.

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    pub items: Vec<u32>,
    pub count: u32,
}

#[drv::lens(AppState)]
struct BadLens<'a> {
    pub items: &'static Vec<u32>,
    pub count: u32,
}

drv::assemble!();

fn main() {
    let _ = core::marker::PhantomData::<&'static ()>;
}

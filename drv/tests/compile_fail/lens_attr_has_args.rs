//! `#[drv::lens]` on a struct takes no arguments — the atom is named
//! by the `impl From<&Atom>` the user writes alongside.

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    pub items: Vec<u32>,
}

#[drv::lens(AppState)]
struct MyLens {
    pub items: Vec<u32>,
}

drv::assemble!();

fn main() {}

//! Atom-only crate — demonstrates that an atom can live in a foreign crate
//! with no knowledge of the memos that will target it.

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Config {
    pub count: u32,
    pub label: String,
}

// No memos; just the atom declaration. assemble!() still runs to wire up
// the identity-lens type (not needed here because no local memo consumes
// it, so assemble!() is effectively a no-op).
drv::assemble!();

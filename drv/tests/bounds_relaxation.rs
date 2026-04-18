//! Verify that `#[drv::atom]` alone imposes no field-trait requirements and
//! that fields outside any consumed lens carry no bounds.

use drv::Atom;

/// Holds only the traits drv genuinely requires for lens participation:
/// `PartialEq + Clone` (+ `Debug` for the lens struct's derive).
#[derive(Debug, Clone, PartialEq)]
pub struct LensReady(pub u32);

impl Default for LensReady {
    fn default() -> Self {
        LensReady(0)
    }
}

/// Deliberately bare — no `PartialEq`, no `Clone`, no `Debug`, no `Default`.
/// Safe to place on an atom as long as no memo consumes the identity lens and
/// no explicit lens projects this field.
pub struct Opaque {
    pub _private: std::sync::Mutex<u32>,
}

#[drv::atom]
pub struct RelaxedAtom {
    #[drv::lens(Simple)]
    pub n: LensReady,

    // Bare field — never projected into a lens, never reached via identity.
    pub opaque: Opaque,
}

#[drv::memo]
fn doubled(lens: &Simple) -> u32 {
    lens.n.0 * 2
}

drv::assemble!();

#[test]
fn atom_without_identity_consumer_and_no_bounds_on_unused_field() {
    let atom = Atom::new(RelaxedAtom {
        n: LensReady(21),
        opaque: Opaque {
            _private: std::sync::Mutex::new(7),
        },
    });
    assert_eq!(doubled(&atom), 42);
}

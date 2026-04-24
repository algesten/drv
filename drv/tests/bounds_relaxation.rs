//! Verify that a plain source struct places no trait requirements on fields
//! that are never projected by any input and never reached by any memo.
//!
//! Fields touched by an input need a `ToStatic` impl (via
//! `#[derive(drv::Input)]` or a hand impl). Fields outside any input —
//! and not reached via a `&Source` value-ref consumer — carry no bounds
//! at all.

use std::marker::PhantomData;

/// Hand-impls `ToStatic` to exercise the escape hatch and to keep the
/// tuple-struct shape. The only trait drv needs for an input field is
/// `ToStatic`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tracked(pub u32);

impl drv::ToStatic for Tracked {
    type Static = Tracked;
    fn to_static(&self) -> Tracked {
        self.clone()
    }
    fn eq_static(&self, other: &Tracked) -> bool {
        self == other
    }
}

/// Deliberately bare — no `PartialEq`, no `Clone`, no `Debug`, no `Default`.
/// Safe on a source as long as no input projects it and no memo consumes the
/// source as `&Source`.
pub struct Opaque {
    pub _private: std::sync::Mutex<u32>,
}

pub struct Relaxed {
    pub n: Tracked,
    // Bare field — never projected, never reached via `&Source`.
    pub opaque: Opaque,
}

#[derive(drv::Input)]
struct Simple<'a> {
    pub n: &'a Tracked,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a Relaxed> for Simple<'a> {
    fn from(a: &'a Relaxed) -> Self {
        Self {
            n: &a.n,
            _p: PhantomData,
        }
    }
}

#[drv::memo(single)]
fn doubled<'a>(input: Simple<'a>) -> u32 {
    input.n.0 * 2
}

#[test]
fn source_with_bare_field_compiles_and_memoizes() {
    let source = Relaxed {
        n: Tracked(21),
        opaque: Opaque {
            _private: std::sync::Mutex::new(7),
        },
    };
    assert_eq!(doubled((&source).into()), 42);
}

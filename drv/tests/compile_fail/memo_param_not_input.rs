//! Using a non-`#[drv::input]` type as a memo input must fail with the
//! `DrvInput` diagnostic.

pub struct NotAnInput<'a> {
    pub x: &'a u32,
}

#[drv::memo(single)]
fn bad<'a>(input: NotAnInput<'a>) -> u32 {
    *input.x
}

fn main() {
    let n = 5u32;
    let i = NotAnInput { x: &n };
    let _ = bad(i);
}

//! Link regression for targets with a small native TLS budget (arm64_32 watchOS).
//! The cache payload exceeds 64 KiB even though its TLS handle must stay small.

#[drv::memo(lru = 32)]
fn large_memo(value: u8) -> [u8; 4096] {
    [value; 4096]
}

#[drv::memo(lru = 32)]
fn second_memo(value: u8) -> [u8; 4096] {
    [value; 4096]
}

fn main() {
    for value in 0..64 {
        assert_eq!(large_memo(value), [value; 4096]);
        assert_eq!(second_memo(value), [value; 4096]);
        assert_eq!(large_memo(value), [value; 4096]);
    }
}

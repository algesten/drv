// trybuild compares rustc stderr byte-for-byte, and error-message formatting
// changes between rustc versions. The checked-in .stderr files target the
// current stable release; older toolchains skip the test so CI on the MSRV
// job still passes.
#[rustversion::since(1.95)]
#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}

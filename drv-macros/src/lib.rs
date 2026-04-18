#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Procedural macros for the [`drv`](https://docs.rs/drv) crate.
//!
//! See the `drv` crate for user-facing documentation. This crate exposes the
//! proc-macro entry points (`#[drv::atom]`, `#[drv::lens]`, `#[drv::memo]`,
//! `#[drv::proj]`, `drv::assemble!()`) and should not be used directly.

mod assemble;
mod atom;
mod lens;
mod memo;
mod proj;
mod registry;

use proc_macro::TokenStream;

/// Mark a struct as an atom — a ground-truth data source.
///
/// `drv::atom` registers the struct for memo/lens machinery but makes no
/// other changes to it. The struct body is emitted verbatim, and any
/// `#[derive(...)]` you add stays on it. Atoms flow through memos
/// wrapped as `drv::Drv<T>`.
///
/// Fields can be any visibility; Rust's normal privacy rules apply, so a
/// private field can only be projected into a lens declared in the same
/// module. Fields can be annotated with `#[drv::lens(Name)]` to declare
/// inline lenses.
///
/// `Clone`, `PartialEq`, `Debug`, and `Default` are always generated.
/// Additional derives like `Hash` and `Eq` can be requested via
/// `#[drv::atom(derive(Hash, Eq))]`.
#[proc_macro_attribute]
pub fn atom(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemStruct);
    let attr = proc_macro2::TokenStream::from(attr);
    match atom::expand(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Declare a standalone lens — a projection of an atom's fields.
///
/// Usage: `#[drv::lens(AtomName)]` on a struct. Each field must have the same
/// name and type as a field on the atom.
#[proc_macro_attribute]
pub fn lens(attr: TokenStream, item: TokenStream) -> TokenStream {
    let atom_name = syn::parse_macro_input!(attr as syn::Ident);
    let item = syn::parse_macro_input!(item as syn::ItemStruct);
    match lens::expand(&atom_name, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Mark a function as a memoized derivation.
///
/// The function takes one or more `&LensName` parameters and returns a value.
/// The result is automatically cached and only recomputed when any lens fields
/// change. Multiple parameters can reference lenses from different atoms.
#[proc_macro_attribute]
pub fn memo(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemFn);
    match memo::expand(item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Mark a `From` impl as the projection function for a lens.
///
/// When a `#[drv::lens(Atom)]` struct has fields that don't match the atom
/// (different names, different types, nested access), the user writes the
/// `From<&Atom>` conversion explicitly. This attribute injects the hidden
/// `__drv` cache reference into the struct construction.
///
/// Usage: `#[drv::proj]` on `impl<'a> From<&'a Atom> for MyLens<'a> { ... }`
#[proc_macro_attribute]
pub fn proj(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemImpl);
    match proj::expand(item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Collect all atoms, lenses, and memos declared in this crate and generate
/// the memoized functions.
///
/// Must appear once, after all `#[drv::atom]`, `#[drv::lens]`, and `#[drv::memo]`
/// declarations.
#[proc_macro]
pub fn assemble(_input: TokenStream) -> TokenStream {
    match assemble::expand() {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

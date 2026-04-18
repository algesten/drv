#![forbid(unsafe_code)]
//! Internal proc-macro implementations for the `drv` crate. Use via `drv`'s
//! re-exports — see <https://docs.rs/drv> for user-facing documentation.

mod assemble;
mod atom;
mod lens;
mod memo;
mod proj;
mod registry;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn atom(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemStruct);
    let attr = proc_macro2::TokenStream::from(attr);
    match atom::expand(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn lens(attr: TokenStream, item: TokenStream) -> TokenStream {
    let atom_name = syn::parse_macro_input!(attr as syn::Ident);
    let item = syn::parse_macro_input!(item as syn::ItemStruct);
    match lens::expand(&atom_name, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn memo(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemFn);
    match memo::expand(item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn proj(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemImpl);
    match proj::expand(item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro]
pub fn assemble(_input: TokenStream) -> TokenStream {
    match assemble::expand() {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

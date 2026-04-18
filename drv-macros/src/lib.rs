#![forbid(unsafe_code)]
//! Internal proc-macro implementations for the `drv` crate. Use via `drv`'s
//! re-exports — see <https://docs.rs/drv> for user-facing documentation.

mod assemble;
mod atom;
mod lens;
mod memo;
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
    // Struct-level `#[drv::lens]` takes no arguments. The atom/lens
    // association is established by the `From<&AtomName>` impl the user
    // writes; the macro never inspects it (the type checker does, at
    // memo call sites).
    let attr = proc_macro2::TokenStream::from(attr);
    let item = syn::parse_macro_input!(item as syn::ItemStruct);
    match lens::expand(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn memo(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item as syn::ItemFn);
    let attr = proc_macro2::TokenStream::from(attr);
    match memo::expand(attr, item) {
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

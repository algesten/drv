#![forbid(unsafe_code)]
//! Internal proc-macro implementations for the `drv` crate. Use via `drv`'s
//! re-exports — see <https://docs.rs/drv> for user-facing documentation.

mod input;
mod memo;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn input(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = proc_macro2::TokenStream::from(attr);
    let item = syn::parse_macro_input!(item as syn::ItemStruct);
    match input::expand(attr, item) {
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

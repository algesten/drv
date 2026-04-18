use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Fields, Ident, ItemStruct};

use crate::registry::{self, LensRegistration};

pub fn expand(attr: TokenStream, item: ItemStruct) -> Result<TokenStream, syn::Error> {
    // `#[drv::lens]` on a struct takes no arguments. The atom/lens
    // association is supplied by the `impl From<&AtomName> for MyLens`
    // the user writes; the macro never inspects it.
    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            &attr,
            "#[drv::lens] takes no arguments on a struct — the atom is named by the `impl From<&AtomName>` you write alongside\n\
             hint: for field-level inline lenses, use #[drv::lens(LensName)] on atom fields instead",
        ));
    }

    let lens_name = &item.ident;

    let fields = match &item.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &item,
                "drv::lens requires a struct with named fields",
            ));
        }
    };

    // Register the lens with no atom association — standalone lenses just
    // exist as named slots in the registry; the atom is never tracked
    // because nothing in the generated code needs it.
    registry::with(|reg| {
        if reg.lens_name_exists(&lens_name.to_string()) {
            return Err(syn::Error::new_spanned(
                lens_name,
                format!(
                    "lens '{}' is already declared -- lens names must be unique within a crate",
                    lens_name
                ),
            ));
        }

        reg.lenses.push(LensRegistration {
            name: lens_name.to_string(),
            atom_name: String::new(), // standalone lens; user provides From impl
            is_identity: false,
            from_impl_tokens: None,
        });

        Ok(())
    })?;

    // Emit the user's struct verbatim. The lens carries no cache handle
    // (caches are per-memo thread-locals), so no field injection needed.
    let vis = &item.vis;
    let other_attrs: Vec<_> = item.attrs.iter().collect();
    let generics = &item.generics;
    let user_fields: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let attrs = &f.attrs;
            let vis = &f.vis;
            let ident = &f.ident;
            let ty = &f.ty;
            quote! { #(#attrs)* #vis #ident: #ty }
        })
        .collect();

    let snapshot_ident = format_ident!("__Drv{}", lens_name);
    let lifetime = item
        .generics
        .lifetimes()
        .next()
        .map(|lt| lt.lifetime.clone());

    let mut output = quote! {
        #(#other_attrs)*
        #vis struct #lens_name #generics {
            #(#user_fields,)*
        }
    };

    output.extend(generate_proj_lens_types(
        lens_name,
        &snapshot_ident,
        fields,
        lifetime.as_ref(),
    ));

    Ok(output)
}

/// Generate the snapshot struct, PartialEq<Snapshot>, and __drv_snapshot
/// method for a standalone lens. The user writes the `impl From<&Atom>`
/// themselves — drv only needs the snapshot machinery to operate the cache.
fn generate_proj_lens_types(
    lens_ident: &Ident,
    snapshot_ident: &Ident,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    lifetime: Option<&syn::Lifetime>,
) -> TokenStream {
    let mut snap_fields = Vec::new();
    let mut eq_checks = Vec::new();
    let mut snap_stores = Vec::new();

    for field in fields {
        let fname = field.ident.as_ref().unwrap();
        let fty = &field.ty;

        if let syn::Type::Reference(r) = fty {
            // Reference field: snapshot stores a clone of the referent type.
            // Only `ToOwned::Owned` that equals the referent (e.g. most
            // containers: `<Vec<T> as ToOwned>::Owned = Vec<T>`) is supported
            // — `&str` → `String` is the notable special case.
            let referent = &*r.elem;
            snap_fields.push(quote! {
                pub #fname: <#referent as ::std::borrow::ToOwned>::Owned
            });
            // FastEq's ptr_eq specialisations work on `&Referent` vs
            // `&Referent`; the snapshot's `ToOwned::Owned` is the referent
            // for Arc/imbl/Vec/HashMap/etc., so `&other.#fname` coerces.
            // For `&str` vs `String`, the Fallback path runs `PartialEq`
            // which has a `str == String` impl via auto-deref.
            eq_checks.push(quote! {
                ({
                    use ::drv::FastEqFallback as _;
                    ::drv::FastEq(self.#fname).fast_eq(&other.#fname)
                })
            });
            // Store via to_owned (works for both `&T`→`T` and `&str`→`String`).
            snap_stores.push(quote! {
                #fname: ::std::borrow::ToOwned::to_owned(self.#fname)
            });
        } else {
            // Owned field: snapshot stores same type. Use FastEq for ptr_eq
            // short-circuit on Arc/imbl types.
            eq_checks.push(quote! {
                ({
                    use ::drv::FastEqFallback as _;
                    ::drv::FastEq(&self.#fname).fast_eq(&other.#fname)
                })
            });
            snap_fields.push(quote! { pub #fname: #fty });
            snap_stores.push(quote! { #fname: self.#fname.clone() });
        }
    }

    let impl_generics = match lifetime {
        Some(lt) => quote! { <#lt> },
        None => quote! {},
    };
    let lens_generics = match lifetime {
        Some(lt) => quote! { <#lt> },
        None => quote! {},
    };

    quote! {
        // Owned snapshot for cache storage.
        #[doc(hidden)]
        pub struct #snapshot_ident {
            #(#snap_fields,)*
        }

        // Cross-type PartialEq: lens vs owned snapshot.
        impl #impl_generics ::core::cmp::PartialEq<#snapshot_ident> for #lens_ident #lens_generics {
            fn eq(&self, other: &#snapshot_ident) -> bool {
                #(#eq_checks)&&*
            }
        }

        // Snapshot the lens to an owned value for cache storage (cache miss only).
        impl #impl_generics #lens_ident #lens_generics {
            #[doc(hidden)]
            pub fn __drv_snapshot(&self) -> #snapshot_ident {
                #snapshot_ident {
                    #(#snap_stores,)*
                }
            }
        }
    }
}

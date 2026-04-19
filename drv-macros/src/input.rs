use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Fields, Ident, ItemStruct};

pub fn expand(attr: TokenStream, item: ItemStruct) -> Result<TokenStream, syn::Error> {
    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            &attr,
            "#[drv::input] takes no arguments",
        ));
    }

    let input_name = &item.ident;

    let fields = match &item.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &item,
                "drv::input requires a struct with named fields",
            ));
        }
    };

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

    let snapshot_ident = format_ident!("__Drv{}", input_name);
    let lifetime = item
        .generics
        .lifetimes()
        .next()
        .map(|lt| lt.lifetime.clone());

    let mut output = quote! {
        #(#other_attrs)*
        #vis struct #input_name #generics {
            #(#user_fields,)*
        }
    };

    output.extend(generate_snapshot_machinery(
        input_name,
        &snapshot_ident,
        fields,
        lifetime.as_ref(),
    ));

    Ok(output)
}

/// `true` if `ty` names `PhantomData` (regardless of path qualification).
fn is_phantom_data(ty: &syn::Type) -> bool {
    if let syn::Type::Path(p) = ty {
        if let Some(last) = p.path.segments.last() {
            return last.ident == "PhantomData";
        }
    }
    false
}

/// Generate the snapshot struct, `PartialEq<Snapshot>` impl, the hidden
/// `__drv_snapshot` method, and the `DrvInput` marker impl.
fn generate_snapshot_machinery(
    input_ident: &Ident,
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

        // Skip PhantomData fields — they carry no data and their lifetime
        // generics would break the non-generic snapshot struct.
        if is_phantom_data(fty) {
            continue;
        }

        if let syn::Type::Reference(r) = fty {
            // Reference field: snapshot stores a clone of the referent type.
            // Only `ToOwned::Owned` that equals the referent (e.g. most
            // containers: `<Vec<T> as ToOwned>::Owned = Vec<T>`) is supported
            // — `&str` → `String` is the notable special case.
            let referent = &*r.elem;
            snap_fields.push(quote! {
                pub #fname: <#referent as ::std::borrow::ToOwned>::Owned
            });
            // FastEq's ptr_eq specialisations operate on `&Referent` vs
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
            // Owned field: snapshot stores same type. FastEq for ptr_eq
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
    let input_generics = match lifetime {
        Some(lt) => quote! { <#lt> },
        None => quote! {},
    };

    quote! {
        // Owned snapshot for cache storage.
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #snapshot_ident {
            #(#snap_fields,)*
        }

        // Cross-type PartialEq: borrowed input vs owned snapshot.
        impl #impl_generics ::core::cmp::PartialEq<#snapshot_ident> for #input_ident #input_generics {
            fn eq(&self, other: &#snapshot_ident) -> bool {
                #(#eq_checks)&&*
            }
        }

        // Snapshot method — inherent rather than trait so the memo's
        // generated code doesn't run into associated-type normalization
        // issues when the input type is reached through a where-clause
        // bound (see Rust issue #152409).
        impl #impl_generics #input_ident #input_generics {
            #[doc(hidden)]
            pub fn __drv_snapshot(&self) -> #snapshot_ident {
                #snapshot_ident {
                    #(#snap_stores,)*
                }
            }
        }

        // Marker trait — `#[drv::memo]` emits a compile-time bound against
        // this to produce a clear "forgot `#[drv::input]`" diagnostic when
        // a memo parameter references an un-tagged type.
        impl #impl_generics ::drv::DrvInput for #input_ident #input_generics {}
    }
}

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Fields};

/// Expand `#[derive(drv::Input)]` on a struct.
///
/// Emits:
/// 1. A `#[doc(hidden)]` shadow struct `__Drv<Name>` whose fields are
///    each field's `<FieldType as ToStatic>::Static`.
/// 2. `impl ToStatic for <Name>` with field-by-field `to_static` and
///    `eq_static` bodies.
///
/// Per-field codegen is uniform: no branching on reference vs owned vs
/// nested drv::Input. Rust's method resolution picks the right
/// `ToStatic` impl at typeck.
pub fn expand(item: DeriveInput) -> Result<TokenStream, syn::Error> {
    let input_name = &item.ident;

    let fields = match &item.data {
        syn::Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &s.fields,
                    "drv::Input requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &item,
                "drv::Input can only be derived on structs",
            ));
        }
    };

    let snapshot_ident = format_ident!("__Drv{}", input_name);

    let mut snap_fields = Vec::new();
    let mut eq_checks = Vec::new();
    let mut snap_stores = Vec::new();

    for field in fields {
        let fname = field.ident.as_ref().unwrap();
        let fty = &field.ty;

        // Skip PhantomData fields — they carry no data and their
        // lifetime generics would break the non-generic snapshot struct.
        if is_phantom_data(fty) {
            continue;
        }

        // Uniform per-field codegen via ToStatic method resolution.
        // Reference fields (&T): method syntax auto-derefs to T, so
        // `self.f.to_static()` resolves to T's ToStatic::to_static with
        // self=&T — same as `<T as ToStatic>::to_static`. The shadow
        // struct's field type must name the 'static form; we strip
        // references and substitute lifetimes with 'static so the
        // resulting type is well-formed inside a non-generic struct.
        let fty_static = type_to_static_form(fty);

        snap_fields.push(quote! {
            pub #fname: <#fty_static as ::drv::ToStatic>::Static
        });
        snap_stores.push(quote! {
            #fname: ::drv::ToStatic::to_static(&self.#fname)
        });
        eq_checks.push(quote! {
            ::drv::ToStatic::eq_static(&self.#fname, &other.#fname)
        });
    }

    // The shadow struct comparison body needs a `true` fallback when the
    // field list is empty (e.g. only PhantomData fields present).
    let eq_body = if eq_checks.is_empty() {
        quote! { true }
    } else {
        quote! { #(#eq_checks)&&* }
    };

    // If all fields were filtered (PhantomData only), build the
    // snapshot struct as a unit-style empty struct — struct with no
    // fields is valid.
    let snap_struct_body = if snap_fields.is_empty() {
        quote! {}
    } else {
        quote! { #(#snap_fields,)* }
    };

    let snap_stores_body = if snap_stores.is_empty() {
        quote! {}
    } else {
        quote! { #(#snap_stores,)* }
    };

    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #snapshot_ident {
            #snap_struct_body
        }

        impl #impl_generics ::drv::ToStatic for #input_name #ty_generics #where_clause {
            type Static = #snapshot_ident;

            fn to_static(&self) -> Self::Static {
                #snapshot_ident {
                    #snap_stores_body
                }
            }

            fn eq_static(&self, other: &Self::Static) -> bool {
                #eq_body
            }
        }
    })
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

/// Rewrite a field type so it's valid inside a non-generic shadow struct:
/// strip outer reference, substitute every lifetime with `'static`.
///
/// For `&'a T`, returns the referent type with lifetimes made `'static`
/// — method resolution on the original `&T` field dispatches via
/// auto-deref to T's ToStatic impl, so this referent type is what the
/// snapshot stores.
///
/// For owned `T<'a>`, returns `T<'static>`.
fn type_to_static_form(ty: &syn::Type) -> syn::Type {
    let stripped = match ty {
        syn::Type::Reference(r) => (*r.elem).clone(),
        other => other.clone(),
    };
    let mut out = stripped;
    substitute_lifetimes_static(&mut out);
    out
}

/// Replace every `Lifetime` node inside `ty` with `'static`. Walks
/// references, paths (generic arguments), tuples, arrays, and slices.
fn substitute_lifetimes_static(ty: &mut syn::Type) {
    use syn::{GenericArgument, PathArguments, Type};

    let static_lt = syn::Lifetime::new("'static", proc_macro2::Span::call_site());

    match ty {
        Type::Reference(r) => {
            r.lifetime = Some(static_lt.clone());
            substitute_lifetimes_static(&mut r.elem);
        }
        Type::Path(p) => {
            for seg in &mut p.path.segments {
                if let PathArguments::AngleBracketed(args) = &mut seg.arguments {
                    for arg in &mut args.args {
                        match arg {
                            GenericArgument::Lifetime(lt) => {
                                *lt = static_lt.clone();
                            }
                            GenericArgument::Type(t) => substitute_lifetimes_static(t),
                            _ => {}
                        }
                    }
                }
            }
        }
        Type::Tuple(t) => {
            for elem in &mut t.elems {
                substitute_lifetimes_static(elem);
            }
        }
        Type::Array(a) => substitute_lifetimes_static(&mut a.elem),
        Type::Slice(s) => substitute_lifetimes_static(&mut s.elem),
        Type::Ptr(p) => substitute_lifetimes_static(&mut p.elem),
        Type::Paren(p) => substitute_lifetimes_static(&mut p.elem),
        Type::Group(g) => substitute_lifetimes_static(&mut g.elem),
        _ => {}
    }
}

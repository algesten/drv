use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Fields, Index};

/// Expand `#[derive(drv::Input)]` on a struct.
///
/// Emits:
/// 1. A `#[doc(hidden)]` shadow struct `__Drv<Name>` whose fields are
///    each field's `<FieldType as ToStatic>::Static`. Mirrors the input
///    struct's shape — named, tuple, or unit.
/// 2. `impl ToStatic for <Name>` with field-by-field `to_static` and
///    `eq_static` bodies.
///
/// Per-field codegen is uniform: no branching on reference vs owned vs
/// nested drv::Input. Rust's method resolution picks the right
/// `ToStatic` impl at typeck.
pub fn expand(item: DeriveInput) -> Result<TokenStream, syn::Error> {
    let input_name = &item.ident;

    let data = match &item.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                &item,
                "drv::Input can only be derived on structs",
            ));
        }
    };

    let snapshot_ident = format_ident!("__Drv{}", input_name);

    let fields: Vec<FieldInfo> = match &data.fields {
        Fields::Named(f) => f
            .named
            .iter()
            .map(|field| FieldInfo {
                accessor: {
                    let id = field.ident.as_ref().unwrap();
                    quote! { #id }
                },
                decl_head: {
                    let attrs = &field.attrs;
                    let vis = &field.vis;
                    let id = field.ident.as_ref().unwrap();
                    quote! { #(#attrs)* #vis #id: }
                },
                build_head: {
                    let id = field.ident.as_ref().unwrap();
                    quote! { #id: }
                },
                ty: field.ty.clone(),
            })
            .collect(),
        Fields::Unnamed(f) => f
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let idx = Index::from(i);
                FieldInfo {
                    accessor: quote! { #idx },
                    decl_head: {
                        let attrs = &field.attrs;
                        let vis = &field.vis;
                        quote! { #(#attrs)* #vis }
                    },
                    build_head: quote! {},
                    ty: field.ty.clone(),
                }
            })
            .collect(),
        Fields::Unit => Vec::new(),
    };

    let mut snap_decls = Vec::new();
    let mut eq_checks = Vec::new();
    let mut snap_stores = Vec::new();

    for f in &fields {
        // Skip PhantomData fields — they carry no data and their
        // lifetime generics would break the non-generic snapshot struct.
        if is_phantom_data(&f.ty) {
            continue;
        }

        // Uniform per-field codegen via ToStatic method resolution.
        // Reference fields (&T): method syntax auto-derefs to T, so
        // `self.f.to_static()` resolves to T's ToStatic::to_static with
        // self=&T — same as `<T as ToStatic>::to_static`. The shadow
        // struct's field type must name the 'static form; we strip
        // references and substitute lifetimes with 'static so the
        // resulting type is well-formed inside a non-generic struct.
        let fty_static = type_to_static_form(&f.ty);
        let decl_head = &f.decl_head;
        let build_head = &f.build_head;
        let accessor = &f.accessor;

        snap_decls.push(quote! {
            #decl_head <#fty_static as ::drv::ToStatic>::Static
        });
        snap_stores.push(quote! {
            #build_head ::drv::ToStatic::to_static(&self.#accessor)
        });
        eq_checks.push(quote! {
            ::drv::ToStatic::eq_static(&self.#accessor, &other.#accessor)
        });
    }

    let eq_body = if eq_checks.is_empty() {
        quote! { true }
    } else {
        quote! { #(#eq_checks)&&* }
    };

    // Build the shadow struct's generics: drop lifetime params (the
    // shadow is `'static`), keep type + const params, add `'static`
    // bound to each type param so the shadow itself is `'static`.
    let (shadow_generics, shadow_ty_args) = shadow_struct_generics(&item.generics);

    // Shadow struct inherits the input's where-clause so field types
    // like `<T as ToStatic>::Static` are well-formed. Lifetime
    // predicates (which reference lifetimes we drop) are filtered out.
    let shadow_where = build_shadow_where_clause(&item.generics);

    // Build the shadow-struct declaration and its constructor
    // expression, mirroring the input struct's shape.
    let (snap_struct_with_generics, snap_build_expr) = match &data.fields {
        Fields::Named(_) => (
            quote! {
                pub struct #snapshot_ident #shadow_generics #shadow_where {
                    #(#snap_decls,)*
                }
            },
            quote! {
                #snapshot_ident {
                    #(#snap_stores,)*
                }
            },
        ),
        Fields::Unnamed(_) => (
            quote! {
                pub struct #snapshot_ident #shadow_generics (
                    #(#snap_decls,)*
                ) #shadow_where;
            },
            quote! {
                #snapshot_ident(
                    #(#snap_stores,)*
                )
            },
        ),
        Fields::Unit => (
            quote! {
                pub struct #snapshot_ident #shadow_generics #shadow_where;
            },
            quote! { #snapshot_ident },
        ),
    };

    // The impl reuses the input's generics verbatim and adds `'static`
    // bounds on each type param so the concrete Static type (the
    // shadow, parameterized by those type params) is `'static`.
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();
    let extra_static_bounds = extra_static_bounds_for_type_params(&item.generics);
    let mut all_preds: Vec<TokenStream> = Vec::new();
    if let Some(w) = where_clause {
        for p in &w.predicates {
            all_preds.push(quote! { #p });
        }
    }
    all_preds.extend(extra_static_bounds);
    let impl_where = if all_preds.is_empty() {
        quote! {}
    } else {
        quote! { where #(#all_preds),* }
    };

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #snap_struct_with_generics

        impl #impl_generics ::drv::ToStatic for #input_name #ty_generics #impl_where {
            type Static = #snapshot_ident #shadow_ty_args;

            fn to_static(&self) -> Self::Static {
                #snap_build_expr
            }

            fn eq_static(&self, other: &Self::Static) -> bool {
                #eq_body
            }
        }
    })
}

/// Build generics for the shadow struct: drops lifetime params, keeps
/// type + const params, adds `'static` to each type param's bounds so
/// the shadow-struct type is itself `'static`.
///
/// Returns `(generics-for-declaration, ty-args-for-use)`.
fn shadow_struct_generics(generics: &syn::Generics) -> (TokenStream, TokenStream) {
    let mut decl_params: Vec<TokenStream> = Vec::new();
    let mut use_args: Vec<TokenStream> = Vec::new();

    for p in &generics.params {
        match p {
            syn::GenericParam::Lifetime(_) => {} // drop
            syn::GenericParam::Type(t) => {
                let id = &t.ident;
                let bounds = &t.bounds;
                let decl = if bounds.is_empty() {
                    quote! { #id: 'static }
                } else {
                    quote! { #id: 'static + #bounds }
                };
                decl_params.push(decl);
                use_args.push(quote! { #id });
            }
            syn::GenericParam::Const(c) => {
                let id = &c.ident;
                let ty = &c.ty;
                decl_params.push(quote! { const #id: #ty });
                use_args.push(quote! { #id });
            }
        }
    }

    let decl = if decl_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#decl_params),*> }
    };
    let use_ = if use_args.is_empty() {
        quote! {}
    } else {
        quote! { <#(#use_args),*> }
    };
    (decl, use_)
}

/// Build the shadow struct's where-clause by copying the input's
/// predicates, dropping any that reference a dropped lifetime. The
/// remaining predicates (mostly type-param trait bounds) need to carry
/// over so field types like `<T as ToStatic>::Static` are well-formed.
fn build_shadow_where_clause(generics: &syn::Generics) -> TokenStream {
    let Some(w) = &generics.where_clause else {
        return quote! {};
    };

    // Names of lifetime params we drop in the shadow struct.
    let dropped_lifetimes: Vec<syn::Ident> = generics
        .params
        .iter()
        .filter_map(|p| match p {
            syn::GenericParam::Lifetime(lt) => Some(lt.lifetime.ident.clone()),
            _ => None,
        })
        .collect();

    let preds: Vec<TokenStream> = w
        .predicates
        .iter()
        .filter(|p| match p {
            syn::WherePredicate::Lifetime(l) => !dropped_lifetimes.contains(&l.lifetime.ident),
            _ => true,
        })
        .map(|p| quote! { #p })
        .collect();

    if preds.is_empty() {
        quote! {}
    } else {
        quote! { where #(#preds),* }
    }
}

/// Extra `T: 'static` bounds appended to the impl's where-clause for
/// every type param on the input struct. Ensures that the Static
/// associated type (`__Drv<Name><T...>`) is `'static`.
fn extra_static_bounds_for_type_params(generics: &syn::Generics) -> Vec<TokenStream> {
    generics
        .params
        .iter()
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => {
                let id = &t.ident;
                Some(quote! { #id: 'static })
            }
            _ => None,
        })
        .collect()
}

/// Per-field data needed to emit codegen regardless of named vs tuple
/// shape. `accessor` is the token used for field access (`name` or `0`).
/// `decl_head` is everything before the type in a field declaration
/// (attrs + vis + `name:` or attrs + vis for tuple). `build_head` is
/// either `name:` or empty (for tuple positional construction).
struct FieldInfo {
    accessor: TokenStream,
    decl_head: TokenStream,
    build_head: TokenStream,
    ty: syn::Type,
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

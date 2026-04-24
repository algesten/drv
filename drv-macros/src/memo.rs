use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, GenericArgument, Ident, ItemFn, Pat, PathArguments, ReturnType, Type};

pub fn expand(attr: TokenStream, item: ItemFn) -> Result<TokenStream, syn::Error> {
    let fn_name = &item.sig.ident;
    let cache_size = parse_cache_strategy(&attr, fn_name)?;

    if item.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.sig.inputs,
            format!(
                "#[drv::memo] function '{}' must take at least one parameter",
                fn_name,
            ),
        ));
    }

    let output_ty = match &item.sig.output {
        ReturnType::Type(_, ty) => (**ty).clone(),
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                &item.sig,
                format!(
                    "#[drv::memo] function '{}' must have an explicit return type",
                    fn_name,
                ),
            ));
        }
    };

    let mut params: Vec<ParsedParam> = Vec::new();
    for p in &item.sig.inputs {
        params.push(parse_param(p, fn_name)?);
    }

    let vis = &item.vis;
    let body = &item.block;

    let pascal = snake_to_pascal(&fn_name.to_string());
    let key_ident = format_ident!("__Drv{}Key", pascal);
    let slot_ident = format_ident!("__Drv{}Slot", pascal);
    let state_ident = format_ident!("__Drv{}CacheState", pascal);
    let static_ident = format_ident!("__DRV_{}_CACHE", fn_name.to_string().to_uppercase());

    // Cache-key field type: see `ParsedParam::key_field_ty` for the
    // two-shape rationale.
    let key_fields: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let ty = &p.key_field_ty;
            quote! { pub #name: #ty }
        })
        .collect();

    // Outer signature — emitted verbatim as the user wrote it. Stripping
    // `#[drv::memo]` leaves a valid Rust function.
    let outer_params: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let ty = &p.orig_ty;
            quote! { #name: #ty }
        })
        .collect();

    // No explicit `T: ToStatic` where-clause. Rust issue #152409 — a
    // where-clause bound on a lifetime-parametric type shadows the impl's
    // associated-type definition, preventing `<T<'a> as ToStatic>::Static`
    // from normalizing to the concrete snapshot struct. The trait bound
    // is still required at the first `ToStatic` call in the body; the
    // diagnostic there uses our `#[diagnostic::on_unimplemented]`
    // message.
    let existing_where = item.sig.generics.where_clause.as_ref();
    let where_clause: TokenStream = match existing_where {
        None => quote! {},
        Some(w) => quote! { #w },
    };

    // Generics sans where-clause so we can splice the merged where in.
    let generics_no_where = {
        let mut g = item.sig.generics.clone();
        g.where_clause = None;
        g
    };

    // Pre-body snap: capture each parameter's snapshot before running the
    // user's body, so it survives if the body moves the parameter.
    //
    // Uses a fresh variable so the body still sees the original parameter
    // (owned or borrowed, as declared).
    let pre_body_snaps: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let snap = format_ident!("__drv_snap_{}", name);
            quote! { let #snap = ::drv::ToStatic::to_static(&#name); }
        })
        .collect();

    // Freshness check per param — combined later with `&&`.
    let fresh_checks: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let field = &p.name;
            quote! { ::drv::ToStatic::eq_static(&#name, &__drv_key.#field) }
        })
        .collect();

    let fresh_check_expr: TokenStream = {
        let first = &fresh_checks[0];
        let rest = &fresh_checks[1..];
        quote! { #first #( && #rest )* }
    };

    // Cache-key construction on miss-install — uses the pre-snapshotted
    // values captured before the body ran.
    let key_build_fields: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let field = &p.name;
            let snap = format_ident!("__drv_snap_{}", name);
            quote! { #field: #snap }
        })
        .collect();

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #key_ident {
            #(#key_fields,)*
        }

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #slot_ident {
            pub key: #key_ident,
            pub output: #output_ty,
            pub stamp: u64,
        }

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #state_ident {
            pub slots: [::core::option::Option<#slot_ident>; #cache_size],
            pub next_stamp: u64,
        }

        impl ::core::default::Default for #state_ident {
            fn default() -> Self {
                Self {
                    slots: [const { ::core::option::Option::None }; #cache_size],
                    next_stamp: 0,
                }
            }
        }

        thread_local! {
            #[allow(non_upper_case_globals)]
            static #static_ident: ::core::cell::RefCell<#state_ident> =
                ::core::cell::RefCell::new(#state_ident::default());
        }

        #vis fn #fn_name #generics_no_where (#(#outer_params),*) -> #output_ty
        #where_clause
        {
            #(#pre_body_snaps)*

            // ── Try cache hit. Linear scan; bump the hit slot's stamp to
            // mark it most-recently-used. Borrow released before body runs.
            let __drv_hit: ::core::option::Option<#output_ty> = #static_ident.with(|__drv_cell| {
                let mut __drv_state = ::core::cell::RefCell::borrow_mut(__drv_cell);
                for __drv_idx in 0..__drv_state.slots.len() {
                    let __drv_match = if let ::core::option::Option::Some(__drv_slot) = __drv_state.slots[__drv_idx].as_ref() {
                        let __drv_key: &#key_ident = &__drv_slot.key;
                        #fresh_check_expr
                    } else {
                        false
                    };
                    if __drv_match {
                        __drv_state.next_stamp = __drv_state.next_stamp.wrapping_add(1);
                        let __drv_new_stamp = __drv_state.next_stamp;
                        if let ::core::option::Option::Some(__drv_slot) = __drv_state.slots[__drv_idx].as_mut() {
                            __drv_slot.stamp = __drv_new_stamp;
                        }
                        let __drv_out = <#output_ty as ::core::clone::Clone>::clone(
                            &__drv_state.slots[__drv_idx].as_ref().unwrap().output
                        );
                        return ::core::option::Option::Some(__drv_out);
                    }
                }
                ::core::option::Option::None
            });
            if let ::core::option::Option::Some(__drv_out) = __drv_hit {
                return __drv_out;
            }

            // ── Miss: run the user body with no cache borrow held.
            let __drv_out: #output_ty = { #body };

            // ── Install: empty slot if available, else LRU victim.
            #static_ident.with(|__drv_cell| {
                let mut __drv_state = ::core::cell::RefCell::borrow_mut(__drv_cell);
                __drv_state.next_stamp = __drv_state.next_stamp.wrapping_add(1);
                let __drv_stamp = __drv_state.next_stamp;
                let __drv_key = #key_ident {
                    #(#key_build_fields,)*
                };
                let __drv_idx = __drv_state.slots.iter().position(|__s| __s.is_none())
                    .unwrap_or_else(|| {
                        let mut __drv_min_idx = 0usize;
                        let mut __drv_min_stamp = u64::MAX;
                        for (__drv_i, __drv_s) in __drv_state.slots.iter().enumerate() {
                            if let ::core::option::Option::Some(__drv_slot) = __drv_s.as_ref() {
                                if __drv_slot.stamp < __drv_min_stamp {
                                    __drv_min_stamp = __drv_slot.stamp;
                                    __drv_min_idx = __drv_i;
                                }
                            }
                        }
                        __drv_min_idx
                    });
                __drv_state.slots[__drv_idx] = ::core::option::Option::Some(#slot_ident {
                    key: __drv_key,
                    output: <#output_ty as ::core::clone::Clone>::clone(&__drv_out),
                    stamp: __drv_stamp,
                });
            });

            __drv_out
        }
    })
}

fn parse_cache_strategy(attr: &TokenStream, fn_name: &Ident) -> Result<usize, syn::Error> {
    if attr.is_empty() {
        return Err(syn::Error::new_spanned(
            fn_name,
            format!(
                "#[drv::memo] on '{}' must specify a cache strategy: \
                 `#[drv::memo(single)]` for a single-slot cache, or \
                 `#[drv::memo(lru = N)]` for an N-slot LRU cache",
                fn_name
            ),
        ));
    }
    let meta: syn::Meta = syn::parse2(attr.clone()).map_err(|e| {
        syn::Error::new(
            e.span(),
            format!(
                "#[drv::memo] on '{}': expected `single` or `lru = N`",
                fn_name
            ),
        )
    })?;
    match meta {
        syn::Meta::Path(path) if path.is_ident("single") => Ok(1),
        syn::Meta::NameValue(nv) if nv.path.is_ident("lru") => {
            let lit = match &nv.value {
                syn::Expr::Lit(expr_lit) => match &expr_lit.lit {
                    syn::Lit::Int(lit) => lit,
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "#[drv::memo(lru = N)]: N must be an integer literal",
                        ));
                    }
                },
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "#[drv::memo(lru = N)]: N must be an integer literal",
                    ));
                }
            };
            let n: usize = lit.base10_parse().map_err(|e| {
                syn::Error::new(lit.span(), format!("#[drv::memo(lru = N)]: {}", e))
            })?;
            if n == 0 {
                return Err(syn::Error::new_spanned(
                    lit,
                    "#[drv::memo(lru = N)]: N must be at least 1",
                ));
            }
            Ok(n)
        }
        other => Err(syn::Error::new_spanned(
            &other,
            "#[drv::memo] expects `single` or `lru = N`",
        )),
    }
}

struct ParsedParam {
    name: Ident,
    orig_ty: syn::Type,
    /// Cache-key field type — what we write in the `__DrvFnKey` struct
    /// definition. Must be `'static`. Two shapes:
    ///
    /// - For a param whose (possibly-dereferenced) path carries a
    ///   lifetime arg (e.g. `MyInput<'a>` or `&MyInput<'a>`), this is
    ///   the concrete snapshot-struct path `__DrvMyInput`. Computed
    ///   syntactically to bypass Rust's inability to normalize
    ///   `<T<'a> as ToStatic>::Static` uniformly with
    ///   `<T<'static> as ToStatic>::Static`.
    /// - Otherwise, `<T_static as ToStatic>::Static` where `T_static`
    ///   is the original type with every lifetime substituted with
    ///   `'static`. Safe because `ToStatic::Static` is `'static` by
    ///   trait contract.
    key_field_ty: TokenStream,
}

fn parse_param(arg: &FnArg, fn_name: &Ident) -> Result<ParsedParam, syn::Error> {
    let typed = match arg {
        FnArg::Typed(t) => t,
        FnArg::Receiver(_) => {
            return Err(syn::Error::new_spanned(
                arg,
                format!("#[drv::memo] function '{}' cannot take self", fn_name),
            ));
        }
    };

    let name = match typed.pat.as_ref() {
        Pat::Ident(pi) => pi.ident.clone(),
        _ => {
            return Err(syn::Error::new_spanned(
                &typed.pat,
                format!(
                    "#[drv::memo] function '{}': parameter must be a simple name",
                    fn_name
                ),
            ));
        }
    };

    let orig_ty = (*typed.ty).clone();

    // `impl Trait` — not supported. Users write concrete types.
    if matches!(typed.ty.as_ref(), Type::ImplTrait(_)) {
        return Err(syn::Error::new_spanned(
            &typed.ty,
            "#[drv::memo]: `impl Trait` parameters are not supported — use a concrete type",
        ));
    }

    // `&mut T` — not supported.
    if let Type::Reference(r) = typed.ty.as_ref() {
        if r.mutability.is_some() {
            return Err(syn::Error::new_spanned(
                &typed.ty,
                format!(
                    "#[drv::memo] function '{}': `&mut` parameters are not supported",
                    fn_name
                ),
            ));
        }
    }

    let key_field_ty = compute_key_field_ty(&orig_ty);

    Ok(ParsedParam {
        name,
        orig_ty,
        key_field_ty,
    })
}

/// Compute the cache-key field type for a memo parameter.
///
/// If the (possibly-dereferenced) type is a path whose last segment
/// carries a lifetime argument, it's assumed to be a `#[derive(drv::Input)]`
/// struct; the cache stores its snapshot struct directly by path name
/// (`__Drv<Name>`). This bypasses a Rust normalization limitation where
/// `<T<'a> as ToStatic>::Static` and `<T<'static> as ToStatic>::Static`
/// aren't unified despite resolving to the same concrete type.
///
/// Otherwise, emit `<T_static as ::drv::ToStatic>::Static` where
/// `T_static` is `ty` with every lifetime replaced by `'static`. This
/// works uniformly for primitives, owned collections, `Arc<T>`, `&str`,
/// `&[u8]`, and `&T` over plain lifetime-free types.
fn compute_key_field_ty(ty: &syn::Type) -> TokenStream {
    // Peel outer reference for the drv::Input path-with-lifetime check.
    let inner = match ty {
        Type::Reference(r) => &*r.elem,
        other => other,
    };

    if let Type::Path(p) = inner {
        if path_last_has_lifetime(&p.path) {
            if let Ok(snap) = snapshot_path_from_input(&p.path) {
                return quote! { #snap };
            }
        }
    }

    let mut ty_static = ty.clone();
    substitute_lifetimes_static(&mut ty_static);
    quote! { <#ty_static as ::drv::ToStatic>::Static }
}

/// `true` if the path's last segment has at least one lifetime generic
/// argument — the syntactic signal distinguishing a drv::Input type
/// from a plain lifetime-free type.
fn path_last_has_lifetime(path: &syn::Path) -> bool {
    match path.segments.last() {
        Some(seg) => matches!(
            &seg.arguments,
            PathArguments::AngleBracketed(a)
                if a.args.iter().any(|g| matches!(g, GenericArgument::Lifetime(_)))
        ),
        None => false,
    }
}

/// Derive the snapshot struct path from a drv::Input type's path:
/// replace the last segment's ident with `__Drv<Ident>` and drop its
/// generic args (the snapshot struct is `'static` and has no generics).
fn snapshot_path_from_input(input_path: &syn::Path) -> Result<syn::Path, syn::Error> {
    let mut snap = input_path.clone();
    let last = snap.segments.last_mut().ok_or_else(|| {
        syn::Error::new_spanned(input_path, "input type must have at least one path segment")
    })?;
    last.ident = format_ident!("__Drv{}", last.ident);
    last.arguments = PathArguments::None;
    Ok(snap)
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

fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

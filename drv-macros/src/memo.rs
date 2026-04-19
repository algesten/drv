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
        params.push(classify_param(p, fn_name)?);
    }

    let vis = &item.vis;
    let body = &item.block;

    let pascal = snake_to_pascal(&fn_name.to_string());
    let key_ident = format_ident!("__Drv{}Key", pascal);
    let slot_ident = format_ident!("__Drv{}Slot", pascal);
    let state_ident = format_ident!("__Drv{}CacheState", pascal);
    let static_ident = format_ident!("__DRV_{}_CACHE", fn_name.to_string().to_uppercase());

    // Cache-key struct fields — one per param, typed by cache-storage form.
    let key_fields: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            match &p.kind {
                ParamKind::InputByValue { snapshot_path, .. }
                | ParamKind::InputByRef { snapshot_path, .. } => {
                    quote! { pub #name: #snapshot_path }
                }
                ParamKind::Value { ty } => quote! { pub #name: #ty },
                ParamKind::ValueRef { referent } => {
                    quote! { pub #name: <#referent as ::std::borrow::ToOwned>::Owned }
                }
            }
        })
        .collect();

    // Outer signature — emitted verbatim as the user wrote it. No sugar,
    // no rewriting, no rescue. Stripping `#[drv::memo]` leaves a valid
    // Rust function.
    let outer_params: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let ty = &p.orig_ty;
            quote! { #name: #ty }
        })
        .collect();

    // Compile-time bounds — every input-classified parameter must satisfy
    // `drv::DrvInput`. Emitted as a `where` clause so that a missing
    // `#[drv::input]` surfaces as a single clear diagnostic.
    let input_where_bounds: Vec<TokenStream> = params
        .iter()
        .filter_map(|p| match &p.kind {
            ParamKind::InputByValue { input_ty, .. } | ParamKind::InputByRef { input_ty, .. } => {
                Some(quote! { #input_ty: ::drv::DrvInput })
            }
            _ => None,
        })
        .collect();

    // Merge with any existing where-clause predicates on the user's fn so
    // we don't drop them.
    let existing_where = item.sig.generics.where_clause.as_ref();
    let where_clause: TokenStream = match (existing_where, input_where_bounds.is_empty()) {
        (None, true) => quote! {},
        (None, false) => quote! { where #(#input_where_bounds),* },
        (Some(w), true) => quote! { #w },
        (Some(w), false) => {
            let preds = &w.predicates;
            quote! { where #preds, #(#input_where_bounds),* }
        }
    };

    // `generics` without the where clause, so we can splice it in manually.
    let generics_no_where = {
        let mut g = item.sig.generics.clone();
        g.where_clause = None;
        g
    };

    // Pre-freshness snap bindings — captured before the body runs so
    // snapshots survive a moving body.
    //
    // - `Value`: Clone the param so install can use it after the body.
    // - `InputByValue`: `__drv_snapshot()` via auto-ref, keeping `x`
    //   usable in the body (owned).
    // - `InputByRef` / `ValueRef`: no pre-snap needed — the body holds
    //   a reference that stays valid through install.
    let pre_body_snaps: Vec<TokenStream> = params
        .iter()
        .filter_map(|p| match &p.kind {
            ParamKind::Value { .. } => {
                let name = &p.name;
                let snap = format_ident!("__drv_snap_{}", name);
                Some(quote! { let #snap = <_ as ::core::clone::Clone>::clone(&#name); })
            }
            ParamKind::InputByValue { .. } => {
                let name = &p.name;
                let snap = format_ident!("__drv_snap_{}", name);
                Some(quote! { let #snap = (&#name).__drv_snapshot(); })
            }
            _ => None,
        })
        .collect();

    // Freshness check per param — combined later with `&&`.
    let fresh_checks: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let field = &p.name;
            match &p.kind {
                ParamKind::InputByValue { .. } | ParamKind::InputByRef { .. } => {
                    quote! { (#name.eq(&__drv_key.#field)) }
                }
                ParamKind::Value { .. } => quote! {
                    ({
                        use ::drv::FastEqFallback as _;
                        ::drv::FastEq(&#name).fast_eq(&__drv_key.#field)
                    })
                },
                ParamKind::ValueRef { .. } => quote! { (#name == &__drv_key.#field) },
            }
        })
        .collect();

    let fresh_check_expr: TokenStream = {
        let first = &fresh_checks[0];
        let rest = &fresh_checks[1..];
        quote! { #first #( && #rest )* }
    };

    // Cache-key construction on miss-install.
    let key_build_fields: Vec<TokenStream> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let field = &p.name;
            match &p.kind {
                ParamKind::InputByValue { .. } | ParamKind::Value { .. } => {
                    let snap = format_ident!("__drv_snap_{}", name);
                    quote! { #field: #snap }
                }
                ParamKind::InputByRef { .. } => {
                    quote! { #field: #name.__drv_snapshot() }
                }
                ParamKind::ValueRef { referent } => quote! {
                    #field: <#referent as ::std::borrow::ToOwned>::to_owned(#name)
                },
            }
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
    kind: ParamKind,
}

/// Classification of a memo parameter. Purely syntactic — the macro reads
/// the type you wrote, nothing more. No sugar, no rewriting: stripping
/// `#[drv::memo]` leaves a valid Rust function whose body sees exactly
/// the declared types.
enum ParamKind {
    /// `x: MyInput<'a>` — by-value input. Body owns `x`; snapshot is
    /// captured before the body runs so it survives a moving body.
    InputByValue {
        input_ty: syn::Type,
        snapshot_path: syn::Path,
    },
    /// `x: &MyInput<'a>` — borrowed input. Body uses the reference;
    /// snapshot is taken at install time via the ref.
    InputByRef {
        input_ty: syn::Type,
        snapshot_path: syn::Path,
    },
    /// `x: T` (no lifetime argument on `T`). Stored via `Clone`,
    /// compared via `FastEq`.
    Value { ty: syn::Type },
    /// `x: &T` (no lifetime argument on `T`). Stored as
    /// `<T as ToOwned>::Owned`, compared via the `impl PartialEq<&B> for &A`
    /// blanket.
    ValueRef { referent: syn::Type },
}

fn classify_param(arg: &FnArg, fn_name: &Ident) -> Result<ParsedParam, syn::Error> {
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
            "#[drv::memo]: `impl Trait` parameters are not supported — use a concrete type \
             (e.g. `MyInput<'a>`, `&MyInput<'a>`, `T`, or `&T`)",
        ));
    }

    // `&T` — split on whether `T`'s path carries a lifetime argument.
    // `&MyInput<'a>` → InputByRef; plain `&str` / `&MyStruct` → ValueRef.
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
        if let Type::Path(p) = r.elem.as_ref() {
            if path_last_has_lifetime(&p.path) {
                let snapshot_path = snapshot_path_from_input(&p.path)?;
                return Ok(ParsedParam {
                    name,
                    orig_ty,
                    kind: ParamKind::InputByRef {
                        input_ty: (*r.elem).clone(),
                        snapshot_path,
                    },
                });
            }
        }
        return Ok(ParsedParam {
            name,
            orig_ty,
            kind: ParamKind::ValueRef {
                referent: (*r.elem).clone(),
            },
        });
    }

    // By-value path — split on lifetime-arg presence. `MyInput<'a>` →
    // InputByValue; plain `u32` / `String` / `Vec<u8>` → Value.
    if let Type::Path(p) = typed.ty.as_ref() {
        if path_last_has_lifetime(&p.path) {
            let snapshot_path = snapshot_path_from_input(&p.path)?;
            return Ok(ParsedParam {
                name,
                orig_ty: orig_ty.clone(),
                kind: ParamKind::InputByValue {
                    input_ty: orig_ty,
                    snapshot_path,
                },
            });
        }
    }

    // Everything else by-value → owned value.
    Ok(ParsedParam {
        name,
        orig_ty: orig_ty.clone(),
        kind: ParamKind::Value { ty: orig_ty },
    })
}

/// `true` if the path's last segment has at least one lifetime generic
/// argument — the syntactic signal distinguishing "input type" from
/// "owned value type."
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

/// Derive the snapshot type path from an input type's path: replace the
/// last segment's ident with `__Drv<Ident>` and drop its generic arguments
/// (the snapshot is always `'static`, so it carries no lifetime).
fn snapshot_path_from_input(input_path: &syn::Path) -> Result<syn::Path, syn::Error> {
    let mut snap = input_path.clone();
    let last = snap.segments.last_mut().ok_or_else(|| {
        syn::Error::new_spanned(input_path, "input type must have at least one path segment")
    })?;
    last.ident = format_ident!("__Drv{}", last.ident);
    last.arguments = PathArguments::None;
    Ok(snap)
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

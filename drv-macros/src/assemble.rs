use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

use crate::registry::{self, MemoParam};

/// Struct name for an atom's identity-lens view, generated only when some memo
/// takes the atom directly (i.e. `fn foo(a: &Editor)` with `Editor` an atom).
fn identity_lens_ident(atom_name: &str) -> Ident {
    format_ident!("__DrvIdentity{}", atom_name)
}

/// Snapshot struct name paired with [`identity_lens_ident`].
fn identity_snapshot_ident(atom_name: &str) -> Ident {
    format_ident!("__Drv__DrvIdentity{}", atom_name)
}

pub fn expand() -> Result<TokenStream, syn::Error> {
    registry::with(|reg| {
        let mut output = TokenStream::new();

        // Identity-lens generation. The lens struct + snapshot + PartialEq +
        // From are emitted only for atoms that some memo actually consumes
        // as an identity lens. Atoms without an identity-lens consumer impose
        // no bounds on their fields from the identity path.
        let identity_atoms: std::collections::BTreeSet<String> = reg
            .memos
            .iter()
            .flat_map(|m| m.params.iter())
            .filter_map(|p| match p {
                MemoParam::Lens { lens_name, .. } => reg
                    .find_lens(lens_name)
                    .filter(|l| l.is_identity)
                    .map(|l| l.atom_name.clone()),
                _ => None,
            })
            .collect();

        for atom_name in &identity_atoms {
            let atom = reg
                .find_atom(atom_name)
                .expect("identity lens references a registered atom");
            let atom_ident = Ident::new(&atom.name, Span::call_site());
            let lens_ident = identity_lens_ident(&atom.name);
            let snapshot_ident = identity_snapshot_ident(&atom.name);
            let field_names: Vec<Ident> = atom
                .fields
                .iter()
                .map(|f| Ident::new(&f.name, Span::call_site()))
                .collect();
            let field_types: Vec<syn::Type> = atom
                .fields
                .iter()
                .map(|f| syn::parse_str(&f.ty_tokens).expect("atom field type should parse"))
                .collect();
            let (base, from_impl) = crate::atom::generate_lens_types(
                &lens_ident,
                &snapshot_ident,
                &atom_ident,
                &field_names,
                &field_types,
            );
            output.extend(base);
            output.extend(from_impl);
        }

        // Emit auto-generated `From<&Atom> for Lens` impls for every inline
        // lens (registered by `#[drv::atom]` from `#[drv::lens(Name)]`
        // field annotations). Standalone lenses leave `from_impl_tokens`
        // as `None` — the user writes their own `impl From<...>`.
        for lens in &reg.lenses {
            if let Some(tokens) = &lens.from_impl_tokens {
                let parsed: TokenStream = tokens
                    .parse()
                    .expect("registered from_impl_tokens should re-parse");
                output.extend(parsed);
            }
        }

        // Build per-memo info.
        let memos: Vec<MemoInfo> = reg
            .memos
            .iter()
            .map(|m| build_memo_info(m, reg))
            .collect::<Result<Vec<_>, _>>()?;

        // Emit the per-memo input struct, cache-state struct, thread-local
        // cache, and rewritten memo function.
        for memo in &memos {
            output.extend(generate_input_struct(memo));
            output.extend(generate_cache_types(memo));
            output.extend(generate_memo_fn(memo));
        }

        Ok(output)
    })
}

fn input_struct_ident(fn_name: &str) -> Ident {
    format_ident!("__Drv{}Input", snake_to_pascal(fn_name))
}

fn slot_struct_ident(fn_name: &str) -> Ident {
    format_ident!("__Drv{}Slot", snake_to_pascal(fn_name))
}

fn cache_state_ident(fn_name: &str) -> Ident {
    format_ident!("__Drv{}CacheState", snake_to_pascal(fn_name))
}

fn cache_static_ident(fn_name: &str) -> Ident {
    format_ident!("__DRV_{}_CACHE", fn_name.to_uppercase())
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

/// Generate the `__Drv{MemoName}Input` struct that holds one call's cached
/// input snapshot. Contains lens snapshots + value fields, in declared order.
fn generate_input_struct(memo: &MemoInfo) -> TokenStream {
    let struct_ident = input_struct_ident(&memo.fn_name);
    let fields: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let name = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens { snapshot_ident, .. } => quote! { #name: #snapshot_ident },
                MemoParamKind::Value { ty_tokens } => {
                    let ty: syn::Type = syn::parse_str(ty_tokens).expect("value type should parse");
                    quote! { #name: #ty }
                }
                MemoParamKind::ValueRef { referent_tokens } => {
                    let referent: syn::Type =
                        syn::parse_str(referent_tokens).expect("referent type should parse");
                    quote! { #name: <#referent as ::std::borrow::ToOwned>::Owned }
                }
            }
        })
        .collect();

    quote! {
        #[doc(hidden)]
        pub struct #struct_ident {
            #(pub #fields,)*
        }
    }
}

/// Generate the slot struct, cache-state struct, and thread-local for the
/// memo's LRU-scanned, value-keyed cache.
fn generate_cache_types(memo: &MemoInfo) -> TokenStream {
    let slot_ident = slot_struct_ident(&memo.fn_name);
    let state_ident = cache_state_ident(&memo.fn_name);
    let static_ident = cache_static_ident(&memo.fn_name);
    let input_ident = input_struct_ident(&memo.fn_name);
    let output_ty: syn::Type =
        syn::parse_str(&memo.output_ty_tokens).expect("output type should parse");
    let cache_size = memo.cache_size;

    quote! {
        #[doc(hidden)]
        pub struct #slot_ident {
            pub input: #input_ident,
            pub output: #output_ty,
            pub stamp: u64,
        }

        #[doc(hidden)]
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
    }
}

fn generate_memo_fn(memo: &MemoInfo) -> TokenStream {
    let fn_ident = Ident::new(&memo.fn_name, Span::call_site());
    let output_ty: syn::Type =
        syn::parse_str(&memo.output_ty_tokens).expect("output type should parse");

    let vis: TokenStream = memo.vis_tokens.parse().unwrap_or_else(|_| quote! {});
    let body: TokenStream = memo.body_tokens.parse().expect("body should parse");
    let input_struct = input_struct_ident(&memo.fn_name);
    let slot_ident = slot_struct_ident(&memo.fn_name);
    let static_ident = cache_static_ident(&memo.fn_name);

    // Outer fn params, in declared order. The user's original type is
    // emitted verbatim — either `&Lens[<'_>]` or `impl Into<Lens<'..>>` for
    // lens params; `T` or `&Referent` for value / value-ref params. Any
    // lifetimes in the user's signature are theirs to declare.
    let outer_params: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens { ty_tokens, .. } => {
                    let ty: syn::Type =
                        syn::parse_str(ty_tokens).expect("lens type should re-parse");
                    quote! { #pname: #ty }
                }
                MemoParamKind::Value { ty_tokens } => {
                    let ty: syn::Type = syn::parse_str(ty_tokens).expect("value type should parse");
                    quote! { #pname: #ty }
                }
                MemoParamKind::ValueRef { referent_tokens } => {
                    let referent: syn::Type =
                        syn::parse_str(referent_tokens).expect("referent type should parse");
                    quote! { #pname: &#referent }
                }
            }
        })
        .collect();

    // Preserve the user's own fn generics (lifetimes + type params),
    // captured at #[drv::memo] time. The macro doesn't synthesize any
    // extra lifetimes — the user declares whatever they wrote in the sig.
    let generics: TokenStream = memo.generics_tokens.parse().unwrap_or_else(|_| quote! {});

    // Inside the function, rebind each lens param so the user's body sees the
    // type they declared:
    // - Regular lens: convert `impl Into<Lens>` to an owned `Lens` under a
    //   hidden name, then rebind `pname` to `&Lens`. The body sees `&Lens`
    //   (matching the user's declared signature) and can pass it to sibling
    //   memos whose outer sig is `impl Into<Lens>` via the auto-generated
    //   `From<&Lens> for Lens`.
    // - Identity lens: keep `pname` as `&T` (matching the user's body); bind
    //   a sibling `__drvl_<pname>` to the identity-lens view used by the
    //   cache machinery's freshness + snapshot paths. Re-entry `bar(a)`
    //   works directly because sibling identity memos also take `&T`.
    let lens_expr = |p: &MemoParamLocal| -> TokenStream {
        let pname = Ident::new(&p.param_name, Span::call_site());
        match &p.kind {
            MemoParamKind::Lens { is_identity, .. } if *is_identity => {
                let v = format_ident!("__drvl_{}", p.param_name);
                quote! { #v }
            }
            _ => quote! { #pname },
        }
    };

    let conversions: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens {
                    is_identity,
                    atom_name,
                    ..
                } if *is_identity => {
                    let v = format_ident!("__drvl_{}", p.param_name);
                    let lens_ident = identity_lens_ident(atom_name);
                    quote! { let #v: #lens_ident<'_> = #pname.into(); }
                }
                MemoParamKind::Lens { is_impl_into, .. } => {
                    if *is_impl_into {
                        // User wrote `impl Into<Lens<'..>>` — convert once,
                        // then rebind as `&Lens` for the body.
                        let owned = format_ident!("__drvl_owned_{}", p.param_name);
                        quote! {
                            let #owned = #pname.into();
                            let #pname = &#owned;
                        }
                    } else {
                        // User wrote `&Lens[<'_>]` — it's already the shape
                        // the body wants, no conversion needed.
                        quote! {}
                    }
                }
                MemoParamKind::Value { .. } => quote! {},
                MemoParamKind::ValueRef { .. } => quote! {},
            }
        })
        .collect();

    // Freshness check: compare each param against the stored snapshot field
    // on a slot's `input`. Each check is wrapped in parens so the combined
    // `&&`-chain parses unambiguously.
    let fresh_checks: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            let field = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens { .. } => {
                    let lens = lens_expr(p);
                    quote! { (#lens.eq(&__drv_input.#field)) }
                }
                MemoParamKind::Value { .. } => {
                    quote! { ({
                        use ::drv::FastEqFallback as _;
                        ::drv::FastEq(&#pname).fast_eq(&__drv_input.#field)
                    }) }
                }
                MemoParamKind::ValueRef { .. } => {
                    // &T vs <T as ToOwned>::Owned via PartialEq
                    // (e.g. `&str == String`, `&[u8] == Vec<u8>`).
                    quote! { (#pname == __drv_input.#field) }
                }
            }
        })
        .collect();

    // Build an explicit `&&`-joined expression; a single-token `&&` separator
    // in quote's repeat syntax can produce malformed output in some contexts.
    let fresh_check_expr: TokenStream = if fresh_checks.is_empty() {
        quote! { true }
    } else {
        let first = &fresh_checks[0];
        let rest = &fresh_checks[1..];
        quote! { #first #( && #rest )* }
    };

    // Pre-body clones of owned value params, so the snapshot can be built
    // after the body runs even if the body moved the param.
    let value_snap_bindings: Vec<TokenStream> = memo
        .params
        .iter()
        .filter_map(|p| match &p.kind {
            MemoParamKind::Value { .. } => {
                let pname = Ident::new(&p.param_name, Span::call_site());
                let snap = format_ident!("__drv_snap_{}", p.param_name);
                Some(quote! {
                    let #snap = <_ as ::core::clone::Clone>::clone(&#pname);
                })
            }
            _ => None,
        })
        .collect();

    // Build the input snapshot at install time.
    let snapshot_fields: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            let field = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens { .. } => {
                    let lens = lens_expr(p);
                    quote! { #field: #lens.__drv_snapshot() }
                }
                MemoParamKind::Value { .. } => {
                    let snap = format_ident!("__drv_snap_{}", p.param_name);
                    quote! { #field: #snap }
                }
                MemoParamKind::ValueRef { referent_tokens } => {
                    let referent: syn::Type =
                        syn::parse_str(referent_tokens).expect("referent type should parse");
                    quote! { #field: <#referent as ::std::borrow::ToOwned>::to_owned(#pname) }
                }
            }
        })
        .collect();

    quote! {
        #vis fn #fn_ident #generics (#(#outer_params),*) -> #output_ty {
            #(#conversions)*
            #(#value_snap_bindings)*

            // ── Try cache hit: linear scan for a slot whose stored input
            // matches the current one. Bump the stamp on hit so this entry
            // is the most recently used. Short borrow released before body.
            let __drv_hit: ::core::option::Option<#output_ty> = #static_ident.with(|__drv_cell| {
                let mut __drv_state = ::core::cell::RefCell::borrow_mut(__drv_cell);
                for __drv_idx in 0..__drv_state.slots.len() {
                    let __drv_match = if let ::core::option::Option::Some(__drv_slot) = __drv_state.slots[__drv_idx].as_ref() {
                        let __drv_input: &#input_struct = &__drv_slot.input;
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

            // ── Miss: recompute body with no cache borrow held. Re-entrancy
            // to sibling memos is safe — each memo has its own cache, and
            // this memo's borrow was released before the body runs.
            let __drv_out: #output_ty = { #body };

            // ── Install: find empty slot, else LRU victim (smallest stamp).
            #static_ident.with(|__drv_cell| {
                let mut __drv_state = ::core::cell::RefCell::borrow_mut(__drv_cell);
                __drv_state.next_stamp = __drv_state.next_stamp.wrapping_add(1);
                let __drv_stamp = __drv_state.next_stamp;
                let __drv_input = #input_struct {
                    #(#snapshot_fields,)*
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
                    input: __drv_input,
                    output: <#output_ty as ::core::clone::Clone>::clone(&__drv_out),
                    stamp: __drv_stamp,
                });
            });

            __drv_out
        }
    }
}

fn build_memo_info(
    memo: &registry::MemoRegistration,
    reg: &registry::Registry,
) -> Result<MemoInfo, syn::Error> {
    let mut params = Vec::new();

    for p in &memo.params {
        match p {
            MemoParam::Lens {
                param_name,
                lens_name,
                ty_tokens,
                is_impl_into,
            } => {
                let lens = reg
                    .find_lens(lens_name)
                    .expect("lens should exist (validated during #[drv::memo])");

                let snapshot_ident = if lens.is_identity {
                    identity_snapshot_ident(&lens.atom_name)
                } else {
                    format_ident!("__Drv{}", lens.name)
                };

                params.push(MemoParamLocal {
                    param_name: param_name.clone(),
                    kind: MemoParamKind::Lens {
                        atom_name: lens.atom_name.clone(),
                        is_identity: lens.is_identity,
                        snapshot_ident,
                        ty_tokens: ty_tokens.clone(),
                        is_impl_into: *is_impl_into,
                    },
                });
            }
            MemoParam::Value {
                param_name,
                ty_tokens,
            } => {
                params.push(MemoParamLocal {
                    param_name: param_name.clone(),
                    kind: MemoParamKind::Value {
                        ty_tokens: ty_tokens.clone(),
                    },
                });
            }
            MemoParam::ValueRef {
                param_name,
                referent_tokens,
            } => {
                params.push(MemoParamLocal {
                    param_name: param_name.clone(),
                    kind: MemoParamKind::ValueRef {
                        referent_tokens: referent_tokens.clone(),
                    },
                });
            }
        }
    }

    let cache_size = match memo.cache_strategy {
        registry::CacheStrategy::Single => 1,
        registry::CacheStrategy::Lru(n) => n,
    };

    Ok(MemoInfo {
        fn_name: memo.fn_name.clone(),
        vis_tokens: memo.vis_tokens.clone(),
        generics_tokens: memo.generics_tokens.clone(),
        output_ty_tokens: memo.output_ty_tokens.clone(),
        body_tokens: memo.body_tokens.clone(),
        params,
        cache_size,
    })
}

#[derive(Clone)]
struct MemoInfo {
    fn_name: String,
    vis_tokens: String,
    generics_tokens: String,
    output_ty_tokens: String,
    body_tokens: String,
    params: Vec<MemoParamLocal>,
    cache_size: usize,
}

#[derive(Clone)]
struct MemoParamLocal {
    param_name: String,
    kind: MemoParamKind,
}

#[derive(Clone)]
enum MemoParamKind {
    Lens {
        atom_name: String,
        is_identity: bool,
        snapshot_ident: Ident,
        /// The user's original parameter type — emitted verbatim in the
        /// memo's outer fn signature.
        ty_tokens: String,
        /// `true` → user wrote `impl Into<Lens<'..>>`; body converts via
        /// `.into()`. `false` → user wrote `&Lens[<'_>]`; body uses the
        /// param directly.
        is_impl_into: bool,
    },
    Value {
        ty_tokens: String,
    },
    ValueRef {
        /// Token string for the referent type (e.g. "str", "[u8]").
        referent_tokens: String,
    },
}

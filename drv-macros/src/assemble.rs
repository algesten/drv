use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::Ident;

use crate::registry::{self, MemoParam};

/// Struct name for an atom's identity-lens view, generated only when some memo
/// actually takes `&Atom<T>` as a parameter.
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

        // Validate: every lens marked as is_proj must have a matching #[drv::proj] impl.
        for lens in &reg.lenses {
            if lens.is_proj && !reg.proj_exists(&lens.name) {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "lens '{}' requires a `#[drv::proj]` annotated \
                         `impl From<&{}> for {}`\n\
                         hint: if this should be a standard lens, ensure all field \
                         names and types match atom '{}'",
                        lens.name, lens.atom_name, lens.name, lens.atom_name
                    ),
                ));
            }
        }

        // Identity-lens generation. The lens struct + snapshot + PartialEq +
        // From are emitted only for atoms that some memo actually consumes via
        // `&Atom<T>`. Atoms without an identity-lens consumer impose no bounds
        // on their fields from the identity path.
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

        // Emit auto-generated `From<&Atom> for Lens` impls for every standard
        // lens that doesn't have a user-written `#[drv::proj]` impl shadowing
        // it. Deferred to assemble time so users can override the generated
        // projection with their own — when a `#[drv::proj]` exists for a
        // standard lens, the user's impl wins and the auto-generated one is
        // skipped to avoid a coherence conflict.
        for lens in &reg.lenses {
            if let Some(tokens) = &lens.from_impl_tokens {
                if reg.proj_exists(&lens.name) {
                    continue;
                }
                let parsed: TokenStream = tokens
                    .parse()
                    .expect("registered from_impl_tokens should re-parse");
                output.extend(parsed);
            }
        }

        // Build per-memo info, group by primary atom (first lens param).
        let memos: Vec<MemoInfo> = reg
            .memos
            .iter()
            .map(|m| build_memo_info(m, reg))
            .collect::<Result<Vec<_>, _>>()?;

        let mut atom_memos: std::collections::HashMap<String, Vec<MemoInfo>> =
            std::collections::HashMap::new();
        for m in &memos {
            atom_memos
                .entry(m.primary_atom.clone())
                .or_default()
                .push(m.clone());
        }

        // Emit per-memo input struct (the cache snapshot for that memo).
        for memo in &memos {
            output.extend(generate_input_struct(memo));
        }

        // Emit per-atom state struct + Atom impl. Every atom gets an impl,
        // even if no memos target it (the atom's `Cache<Self>` field requires it).
        for atom in &reg.atoms {
            let atom_ident = Ident::new(&atom.name, Span::call_site());
            let memos = atom_memos.get(&atom.name).cloned().unwrap_or_default();

            let state_name = format_ident!("__Drv{}State", atom.name);
            let state_fields: Vec<TokenStream> = memos
                .iter()
                .flat_map(|m| {
                    let in_field = format_ident!("{}_input", m.fn_name);
                    let out_field = format_ident!("{}_output", m.fn_name);
                    let input_struct = input_struct_ident(&m.fn_name);
                    let output_ty: syn::Type =
                        syn::parse_str(&m.output_ty_tokens).expect("output type should parse");
                    vec![
                        quote! { #in_field: Option<#input_struct> },
                        quote! { #out_field: Option<#output_ty> },
                    ]
                })
                .collect();

            output.extend(quote! {
                #[doc(hidden)]
                #[derive(Default)]
                pub struct #state_name {
                    #(#state_fields,)*
                }

                impl ::drv::__sealed::Sealed for #atom_ident {}
                impl ::drv::Atomized for #atom_ident {
                    type State = #state_name;
                }
            });
        }

        // Emit the rewritten memo functions.
        for memo in &memos {
            output.extend(generate_memo_fn(memo));
        }

        Ok(output)
    })
}

fn input_struct_ident(fn_name: &str) -> Ident {
    format_ident!("__Drv{}Input", snake_to_pascal(fn_name))
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

/// Generate the `__Drv{MemoName}Input` struct that holds the cache snapshot.
/// Contains: lens snapshots + value fields, in declared order.
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

fn generate_memo_fn(memo: &MemoInfo) -> TokenStream {
    let fn_ident = Ident::new(&memo.fn_name, Span::call_site());
    let input_field = format_ident!("{}_input", memo.fn_name);
    let output_field = format_ident!("{}_output", memo.fn_name);
    let output_ty: syn::Type =
        syn::parse_str(&memo.output_ty_tokens).expect("output type should parse");

    let vis: TokenStream = memo.vis_tokens.parse().unwrap_or_else(|_| quote! {});
    let body: TokenStream = memo.body_tokens.parse().expect("body should parse");
    let input_struct = input_struct_ident(&memo.fn_name);

    // __compute fn signature: preserves the user's original signature.
    let compute_params: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens { lens_type, .. } => {
                    quote! { #pname: &#lens_type }
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

    // Outer fn params, in declared order. Collect lifetime params as we go.
    let mut lifetime_params: Vec<TokenStream> = Vec::new();
    let outer_params: Vec<TokenStream> = memo
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens {
                    lens_name,
                    is_identity,
                    atom_name,
                    ..
                } => {
                    if *is_identity {
                        let atom_ident = Ident::new(atom_name, Span::call_site());
                        quote! { #pname: &::drv::Atom<#atom_ident> }
                    } else {
                        let lens_ident = Ident::new(lens_name, Span::call_site());
                        let lt = syn::Lifetime::new(&format!("'drv{}", i), Span::call_site());
                        lifetime_params.push(quote! { #lt });
                        quote! { #pname: impl ::core::convert::Into<#lens_ident<#lt>> }
                    }
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

    let generics = if lifetime_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#lifetime_params),*> }
    };

    // Inside the function: convert each lens param into the lens value.
    // - Regular lens: shadow `pname` with the converted lens, so downstream
    //   references (cache, fresh-check, snapshot, compute) all see the lens.
    // - Identity lens: keep `pname` as the atom reference (needed for the
    //   user's `__compute`), and bind a sibling `__drvl_<pname>` to the
    //   identity-lens view used by the cache machinery.
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
                MemoParamKind::Lens { .. } => {
                    quote! { let #pname: _ = #pname.into(); }
                }
                MemoParamKind::Value { .. } => quote! {},
                MemoParamKind::ValueRef { .. } => quote! {},
            }
        })
        .collect();

    // First lens param determines where the cache lives.
    let first_lens = memo
        .params
        .iter()
        .find(|p| matches!(p.kind, MemoParamKind::Lens { .. }))
        .expect("memo must have at least one lens param (validated in memo.rs)");
    let first_lens_expr = lens_expr(first_lens);
    let cache_expr = quote! { #first_lens_expr.__drv };

    // Freshness check: compare each param against the stored snapshot field.
    let fresh_checks: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            let field = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens { .. } => {
                    // Regular and identity lenses both compare as `lens == prev.field`.
                    let lens = lens_expr(p);
                    quote! { #lens == __prev.#field }
                }
                MemoParamKind::Value { .. } => {
                    // FastEq enables ptr_eq short-circuit for Arc and imbl.
                    quote! { {
                        use ::drv::FastEqFallback as _;
                        ::drv::FastEq(&#pname).fast_eq(&__prev.#field)
                    } }
                }
                MemoParamKind::ValueRef { .. } => {
                    // &T vs <T as ToOwned>::Owned: relies on PartialEq impls
                    // between them (e.g. `&str == String`, `&[u8] == Vec<u8>`).
                    quote! { #pname == __prev.#field }
                }
            }
        })
        .collect();

    // Build the snapshot at cache-miss storage time.
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
                    quote! { #field: #pname }
                }
                MemoParamKind::ValueRef { referent_tokens } => {
                    // Store the owned form via ToOwned::to_owned(&T) → T::Owned.
                    let referent: syn::Type =
                        syn::parse_str(referent_tokens).expect("referent type should parse");
                    quote! { #field: <#referent as ::std::borrow::ToOwned>::to_owned(#pname) }
                }
            }
        })
        .collect();

    // Compute call args: pass lenses by reference, values by move. For value
    // params we clone the value so it survives for the snapshot afterwards.
    let compute_args: Vec<TokenStream> = memo
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            match &p.kind {
                MemoParamKind::Lens { is_identity, .. } if *is_identity => {
                    // #pname: &Atom<Atom>; user __compute takes &Atom, so deref.
                    quote! { &**#pname }
                }
                MemoParamKind::Lens { .. } => {
                    quote! { &#pname }
                }
                MemoParamKind::Value { .. } => {
                    // Clone to pass to compute; the original is moved into the
                    // snapshot below.
                    quote! { #pname.clone() }
                }
                MemoParamKind::ValueRef { .. } => {
                    // Pass the reference through directly; no clone needed
                    // for the compute call.
                    quote! { #pname }
                }
            }
        })
        .collect();

    quote! {
        #vis fn #fn_ident #generics (#(#outer_params),*) -> #output_ty {
            fn __compute(#(#compute_params),*) -> #output_ty {
                #body
            }

            #(#conversions)*

            // Short shared borrow: cache hit returns a clone and releases.
            {
                let __state = ::core::cell::RefCell::borrow(&#cache_expr.inner);
                if let Some(__prev) = __state.#input_field.as_ref() {
                    if #(#fresh_checks)&&* {
                        return <#output_ty as ::core::clone::Clone>::clone(
                            __state.#output_field.as_ref().unwrap()
                        );
                    }
                }
            }

            // Cache miss: compute with no borrow held.
            let __out = __compute(#(#compute_args),*);

            // Short exclusive borrow to store.
            {
                let mut __state = ::core::cell::RefCell::borrow_mut(&#cache_expr.inner);
                __state.#output_field = Some(<#output_ty as ::core::clone::Clone>::clone(&__out));
                __state.#input_field = Some(#input_struct {
                    #(#snapshot_fields,)*
                });
            }

            __out
        }
    }
}

fn build_memo_info(
    memo: &registry::MemoRegistration,
    reg: &registry::Registry,
) -> Result<MemoInfo, syn::Error> {
    let mut params = Vec::new();
    let mut primary_atom: Option<String> = None;

    for p in &memo.params {
        match p {
            MemoParam::Lens {
                param_name,
                lens_name,
            } => {
                let lens = reg
                    .find_lens(lens_name)
                    .expect("lens should exist (validated during #[drv::memo])");

                if primary_atom.is_none() {
                    primary_atom = Some(lens.atom_name.clone());
                }

                let snapshot_ident = if lens.is_identity {
                    identity_snapshot_ident(&lens.atom_name)
                } else {
                    format_ident!("__Drv{}", lens.name)
                };
                let lens_type = if lens.is_identity {
                    // The user's `__compute` receives `&T` (not `&Lens`); the
                    // identity-lens machinery is used only internally for
                    // cache lookup / freshness / snapshot.
                    Ident::new(&lens.atom_name, Span::call_site()).into_token_stream()
                } else {
                    let i = Ident::new(&lens.name, Span::call_site());
                    quote! { #i<'_> }
                };

                params.push(MemoParamLocal {
                    param_name: param_name.clone(),
                    kind: MemoParamKind::Lens {
                        lens_name: lens.name.clone(),
                        atom_name: lens.atom_name.clone(),
                        is_identity: lens.is_identity,
                        snapshot_ident,
                        lens_type,
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

    Ok(MemoInfo {
        fn_name: memo.fn_name.clone(),
        vis_tokens: memo.vis_tokens.clone(),
        output_ty_tokens: memo.output_ty_tokens.clone(),
        body_tokens: memo.body_tokens.clone(),
        params,
        primary_atom: primary_atom.expect("memo must have at least one lens param"),
    })
}

#[derive(Clone)]
struct MemoInfo {
    fn_name: String,
    vis_tokens: String,
    output_ty_tokens: String,
    body_tokens: String,
    params: Vec<MemoParamLocal>,
    primary_atom: String,
}

#[derive(Clone)]
struct MemoParamLocal {
    param_name: String,
    kind: MemoParamKind,
}

#[derive(Clone)]
enum MemoParamKind {
    Lens {
        lens_name: String,
        atom_name: String,
        is_identity: bool,
        snapshot_ident: Ident,
        lens_type: TokenStream,
    },
    Value {
        ty_tokens: String,
    },
    ValueRef {
        /// Token string for the referent type (e.g. "str", "[u8]").
        referent_tokens: String,
    },
}

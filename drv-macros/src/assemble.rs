use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

use crate::registry;

pub fn expand() -> Result<TokenStream, syn::Error> {
    registry::with(|reg| {
        let mut output = TokenStream::new();

        // Group exprs by primary atom (first param's lens → atom).
        // Also collect per-atom state fields.
        let mut atom_exprs: std::collections::HashMap<String, Vec<ExprInfo>> =
            std::collections::HashMap::new();

        for expr in &reg.exprs {
            let first = &expr.lens_params[0];
            let first_lens = reg.find_lens(&first.lens_name).unwrap();
            let primary_atom = first_lens.atom_name.clone();

            let params: Vec<ExprParamInfo> = expr
                .lens_params
                .iter()
                .map(|lp| {
                    let lens = reg.find_lens(&lp.lens_name).unwrap();
                    ExprParamInfo {
                        param_name: lp.param_name.clone(),
                        lens_name: lp.lens_name.clone(),
                        atom_name: lens.atom_name.clone(),
                        is_identity: lens.is_identity,
                        fields: lens.fields.clone(),
                    }
                })
                .collect();

            atom_exprs.entry(primary_atom).or_default().push(ExprInfo {
                fn_name: expr.fn_name.clone(),
                vis_tokens: expr.vis_tokens.clone(),
                params,
                output_ty_tokens: expr.output_ty_tokens.clone(),
                body_tokens: expr.body_tokens.clone(),
            });
        }

        // Per-atom state struct (one slot pair per expr).
        for atom in &reg.atoms {
            let exprs = atom_exprs.get(&atom.name).cloned().unwrap_or_default();
            if exprs.is_empty() {
                continue;
            }

            let state_name = format_ident!("__Drv{}State", atom.name);
            let state_fields: Vec<TokenStream> = exprs
                .iter()
                .flat_map(|e| {
                    let in_field = format_ident!("{}_input", e.fn_name);
                    let out_field = format_ident!("{}_output", e.fn_name);
                    let input_ty = input_storage_type(e, reg);
                    let output_ty: syn::Type =
                        syn::parse_str(&e.output_ty_tokens).expect("output type should parse");
                    vec![
                        quote! { #in_field: Option<#input_ty> },
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
            });
        }

        // Generate the rewritten expr functions.
        for expr in &reg.exprs {
            let first = &expr.lens_params[0];
            let first_lens = reg.find_lens(&first.lens_name).unwrap();
            let primary_atom = first_lens.atom_name.clone();
            let params: Vec<ExprParamInfo> = expr
                .lens_params
                .iter()
                .map(|lp| {
                    let lens = reg.find_lens(&lp.lens_name).unwrap();
                    ExprParamInfo {
                        param_name: lp.param_name.clone(),
                        lens_name: lp.lens_name.clone(),
                        atom_name: lens.atom_name.clone(),
                        is_identity: lens.is_identity,
                        fields: lens.fields.clone(),
                    }
                })
                .collect();

            let info = ExprInfo {
                fn_name: expr.fn_name.clone(),
                vis_tokens: expr.vis_tokens.clone(),
                params,
                output_ty_tokens: expr.output_ty_tokens.clone(),
                body_tokens: expr.body_tokens.clone(),
            };

            output.extend(generate_expr_fn(&info, &primary_atom, reg));
        }

        Ok(output)
    })
}

/// What type is stored in the cache for this expr's input?
/// - Single param, lens: the lens's snapshot type (`__DrvCountLens`)
/// - Single param, identity (atom): the atom's identity snapshot (`__Drv{Atom}Identity`)
/// - Multiple params: tuple of snapshots
fn input_storage_type(expr: &ExprInfo, _reg: &registry::Registry) -> TokenStream {
    let snapshot_types: Vec<Ident> = expr.params.iter().map(snapshot_ident_for).collect();

    if snapshot_types.len() == 1 {
        let s = &snapshot_types[0];
        quote! { #s }
    } else {
        quote! { (#(#snapshot_types),*) }
    }
}

fn snapshot_ident_for(param: &ExprParamInfo) -> Ident {
    if param.is_identity {
        format_ident!("__Drv{}Identity", param.atom_name)
    } else {
        format_ident!("__Drv{}", param.lens_name)
    }
}

fn generate_expr_fn(expr: &ExprInfo, primary_atom: &str, _reg: &registry::Registry) -> TokenStream {
    let fn_ident = Ident::new(&expr.fn_name, Span::call_site());
    let state_name = format_ident!("__Drv{}State", primary_atom);
    let input_field = format_ident!("{}_input", expr.fn_name);
    let output_field = format_ident!("{}_output", expr.fn_name);
    let output_ty: syn::Type =
        syn::parse_str(&expr.output_ty_tokens).expect("output type should parse");

    // Parse visibility and body tokens from strings back to TokenStream.
    let vis: TokenStream = expr.vis_tokens.parse().unwrap_or_else(|_| quote! {});
    let body: TokenStream = expr.body_tokens.parse().expect("body should parse");

    // Build compute fn: reuses user's signature (lens types) and body.
    let compute_params: Vec<TokenStream> = expr
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            let lens_ty = lens_type_for(p);
            quote! { #pname: &#lens_ty }
        })
        .collect();

    // Outer fn params: each becomes `impl Into<LensType<'drvN>>`.
    // For atoms used as identity lens, the param is `&AtomName`.
    // We need a lifetime parameter per non-identity lens param.
    let mut lifetime_params: Vec<TokenStream> = Vec::new();
    let outer_params: Vec<TokenStream> = expr
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            if p.is_identity {
                let atom_ident = Ident::new(&p.atom_name, Span::call_site());
                quote! { #pname: &#atom_ident }
            } else {
                let lens_ident = Ident::new(&p.lens_name, Span::call_site());
                let lt = syn::Lifetime::new(&format!("'drv{}", i), Span::call_site());
                lifetime_params.push(quote! { #lt });
                quote! { #pname: impl ::core::convert::Into<#lens_ident<#lt>> }
            }
        })
        .collect();

    let generics = if lifetime_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#lifetime_params),*> }
    };

    // Inside the function, convert each param into the lens.
    let conversions: Vec<TokenStream> = expr
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            if p.is_identity {
                // For identity, the atom IS what we use. Just rebind.
                quote! { let #pname = #pname; }
            } else {
                quote! { let #pname: _ = #pname.into(); }
            }
        })
        .collect();

    // First param determines where the cache is.
    let first_pname = Ident::new(&expr.params[0].param_name, Span::call_site());
    let cache_expr = if expr.params[0].is_identity {
        quote! { &#first_pname.__drv }
    } else {
        quote! { #first_pname.__drv }
    };

    // Freshness check: compare each param against the stored snapshot.
    let input_idents: Vec<Ident> = (0..expr.params.len())
        .map(|i| format_ident!("__drv_in_{}", i))
        .collect();

    let single = expr.params.len() == 1;

    let fresh_check = if single {
        let pname = Ident::new(&expr.params[0].param_name, Span::call_site());
        // For identity: compare *atom == *snapshot via atom's PartialEq<Snapshot>
        // For lens: compare *lens == *snapshot via lens's PartialEq<Snapshot>
        if expr.params[0].is_identity {
            quote! { *#pname == *__prev }
        } else {
            quote! { #pname == *__prev }
        }
    } else {
        let checks: Vec<TokenStream> = expr
            .params
            .iter()
            .zip(input_idents.iter())
            .map(|(p, inp)| {
                let pname = Ident::new(&p.param_name, Span::call_site());
                if p.is_identity {
                    quote! { *#pname == *#inp }
                } else {
                    quote! { #pname == *#inp }
                }
            })
            .collect();
        quote! { #(#checks)&&* }
    };

    let fresh_pattern: TokenStream = if single {
        quote! { __prev }
    } else {
        quote! { (#(#input_idents),*) }
    };

    // Build snapshot on cache miss.
    let snapshot_build: TokenStream = if single {
        let p = &expr.params[0];
        build_snapshot_expr(p)
    } else {
        let builders: Vec<TokenStream> = expr.params.iter().map(build_snapshot_expr).collect();
        quote! { (#(#builders),*) }
    };

    // Compute call args.
    let compute_args: Vec<TokenStream> = expr
        .params
        .iter()
        .map(|p| {
            let pname = Ident::new(&p.param_name, Span::call_site());
            if p.is_identity {
                quote! { #pname }
            } else {
                quote! { &#pname }
            }
        })
        .collect();

    quote! {
        #vis fn #fn_ident #generics (#(#outer_params),*) -> #output_ty {
            fn __compute(#(#compute_params),*) -> #output_ty {
                #body
            }

            #(#conversions)*

            let mut __cache_ref = ::core::cell::RefCell::borrow_mut(&#cache_expr.inner);
            let __state: &mut #state_name = __cache_ref
                .get_or_insert_with(|| ::std::boxed::Box::new(#state_name::default()))
                .downcast_mut::<#state_name>()
                .expect("drv: internal cache type mismatch");

            let __fresh = __state.#input_field.as_ref().is_some_and(|#fresh_pattern| #fresh_check);

            if !__fresh {
                let __out = __compute(#(#compute_args),*);
                __state.#output_field = Some(__out);
                __state.#input_field = Some(#snapshot_build);
            }
            <#output_ty as ::core::clone::Clone>::clone(__state.#output_field.as_ref().unwrap())
        }
    }
}

/// The lens type used inside the user's compute fn.
/// For identity lens: the atom itself. For regular lens: `LensName<'_>`.
fn lens_type_for(param: &ExprParamInfo) -> TokenStream {
    if param.is_identity {
        let atom = Ident::new(&param.atom_name, Span::call_site());
        quote! { #atom }
    } else {
        let lens = Ident::new(&param.lens_name, Span::call_site());
        quote! { #lens<'_> }
    }
}

/// Build a snapshot value from a lens/atom param.
fn build_snapshot_expr(param: &ExprParamInfo) -> TokenStream {
    let pname = Ident::new(&param.param_name, Span::call_site());
    if param.is_identity {
        // Identity snapshot: clone each field from the atom.
        let snap_ident = format_ident!("__Drv{}Identity", param.atom_name);
        let fields: Vec<TokenStream> = param
            .fields
            .iter()
            .map(|f| {
                let fname = Ident::new(&f.name, Span::call_site());
                quote! { #fname: #pname.#fname.clone() }
            })
            .collect();
        quote! { #snap_ident { #(#fields),* } }
    } else {
        quote! { #pname.__drv_snapshot() }
    }
}

#[derive(Clone)]
struct ExprInfo {
    fn_name: String,
    vis_tokens: String,
    params: Vec<ExprParamInfo>,
    output_ty_tokens: String,
    body_tokens: String,
}

#[derive(Clone)]
struct ExprParamInfo {
    param_name: String,
    lens_name: String,
    atom_name: String,
    is_identity: bool,
    fields: Vec<registry::LensField>,
}

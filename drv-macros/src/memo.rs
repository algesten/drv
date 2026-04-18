use proc_macro2::TokenStream;
use quote::quote;
use syn::{FnArg, Ident, ItemFn, Pat, ReturnType, Type};

use crate::registry::{self, CacheStrategy, MemoParam, MemoRegistration};

pub fn expand(attr: TokenStream, item: ItemFn) -> Result<TokenStream, syn::Error> {
    let fn_name = &item.sig.ident;

    let cache_strategy = parse_cache_strategy(&attr, fn_name)?;

    if item.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.sig.inputs,
            format!(
                "#[drv::memo] function '{}' must take at least one parameter",
                fn_name,
            ),
        ));
    }

    // First-pass classification. `&Ident` is tentatively Lens; `&NonIdent`
    // (e.g. &[u8], &dyn Trait) is ValueRef; non-references are Value.
    let mut parsed: Vec<ParsedParam> = Vec::new();
    for param in &item.sig.inputs {
        parsed.push(classify_param(param, fn_name)?);
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

    let fn_name_str = fn_name.to_string();

    registry::with(|reg| {
        // Promote tentative Lens params to ValueRef if they don't name a
        // registered lens or atom. This lets users write things like `foo: &str`
        // — `str` isn't a lens, so it's treated as a ToOwned'able reference.
        for p in parsed.iter_mut() {
            if let ParsedParam::Lens {
                lens_name,
                param_name,
                is_impl_into,
                ..
            } = p
            {
                let lens_name_str = lens_name.to_string();
                if !reg.lens_name_exists(&lens_name_str) && !reg.atom_name_exists(&lens_name_str) {
                    if *is_impl_into {
                        // `impl Into<X>` where X isn't a registered lens or
                        // atom — can't quietly fall back to ValueRef (the
                        // syntax is unambiguous). Surface a clear error.
                        return Err(syn::Error::new_spanned(
                            &lens_name,
                            format!(
                                "#[drv::memo] on '{}': `impl Into<{}<'..>>` names '{}' which is not a registered lens or atom",
                                fn_name, lens_name, lens_name
                            ),
                        ));
                    }
                    // Not a lens or an atom — treat as a ToOwned-style reference.
                    *p = ParsedParam::ValueRef {
                        param_name: param_name.clone(),
                        referent: syn::Type::Path(syn::TypePath {
                            qself: None,
                            path: lens_name.clone().into(),
                        }),
                    };
                }
            }
        }

        // Require at least one lens (otherwise we have no atom for the cache).
        if !parsed.iter().any(|p| matches!(p, ParsedParam::Lens { .. })) {
            return Err(syn::Error::new_spanned(
                fn_name,
                format!(
                    "#[drv::memo] function '{}' must have at least one `&LensName` parameter",
                    fn_name,
                ),
            ));
        }

        if reg.memos.iter().any(|e| e.fn_name == fn_name_str) {
            return Err(syn::Error::new_spanned(
                fn_name,
                format!(
                    "memo '{}' is already declared -- memo names must be unique within a crate",
                    fn_name
                ),
            ));
        }

        let body = &item.block;
        let body_tokens = quote!(#body).to_string();
        let vis = &item.vis;
        let generics = &item.sig.generics;

        reg.memos.push(MemoRegistration {
            fn_name: fn_name_str,
            vis_tokens: quote!(#vis).to_string(),
            generics_tokens: quote!(#generics).to_string(),
            params: parsed
                .iter()
                .map(|p| match p {
                    ParsedParam::Lens {
                        param_name,
                        lens_name,
                        ty,
                        is_impl_into,
                    } => MemoParam::Lens {
                        param_name: param_name.to_string(),
                        lens_name: lens_name.to_string(),
                        ty_tokens: registry::type_to_tokens(ty),
                        is_impl_into: *is_impl_into,
                    },
                    ParsedParam::Value { param_name, ty } => MemoParam::Value {
                        param_name: param_name.to_string(),
                        ty_tokens: registry::type_to_tokens(ty),
                    },
                    ParsedParam::ValueRef {
                        param_name,
                        referent,
                    } => MemoParam::ValueRef {
                        param_name: param_name.to_string(),
                        referent_tokens: registry::type_to_tokens(referent),
                    },
                })
                .collect(),
            output_ty_tokens: registry::type_to_tokens(&output_ty),
            body_tokens,
            cache_strategy,
        });

        Ok(())
    })?;

    // Emit compile-time assertions that each ValueRef referent (a) names a
    // real type, and (b) implements ToOwned. Using the original syn::Type
    // preserves spans, so errors point at the user's function signature.
    // This catches common typos like `foo: &MyLen` (meant `&MyLens`) —
    // instead of a confusing error deep in generated code, the user sees
    // "cannot find type `MyLen` in this scope" pointed at their param.
    let assertions: Vec<TokenStream> = parsed
        .iter()
        .filter_map(|p| match p {
            ParsedParam::ValueRef { referent, .. } => Some(quote! {
                const _: fn() = || {
                    fn __drv_assert_to_owned<T: ?Sized + ::std::borrow::ToOwned>() {}
                    __drv_assert_to_owned::<#referent>();
                };
            }),
            _ => None,
        })
        .collect();

    // Swallow the function body (assemble!() emits the rewritten version);
    // only the compile-time assertions remain at the #[drv::memo] call site.
    Ok(quote! { #(#assertions)* })
}

fn parse_cache_strategy(attr: &TokenStream, fn_name: &Ident) -> Result<CacheStrategy, syn::Error> {
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
        syn::Meta::Path(path) if path.is_ident("single") => Ok(CacheStrategy::Single),
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
            Ok(CacheStrategy::Lru(n))
        }
        other => Err(syn::Error::new_spanned(
            &other,
            "#[drv::memo] expects `single` or `lru = N`",
        )),
    }
}

enum ParsedParam {
    Lens {
        param_name: Ident,
        lens_name: Ident,
        /// The user's original parameter type, emitted verbatim at the
        /// memo's outer fn signature. Either `&MyLens[<'_>]` (body uses it
        /// directly) or `impl Into<MyLens<'a>>` (body calls `.into()`).
        ty: syn::Type,
        /// `true` if `ty` is `impl Into<Lens<'..>>`, meaning the body must
        /// convert via `.into()` before using it.
        is_impl_into: bool,
    },
    Value {
        param_name: Ident,
        ty: syn::Type,
    },
    ValueRef {
        param_name: Ident,
        referent: syn::Type,
    },
}

fn classify_param(param: &FnArg, fn_name: &syn::Ident) -> Result<ParsedParam, syn::Error> {
    let typed = match param {
        FnArg::Typed(t) => t,
        FnArg::Receiver(_) => {
            return Err(syn::Error::new_spanned(
                param,
                format!("#[drv::memo] function '{}' cannot take self", fn_name),
            ));
        }
    };

    let param_name = match typed.pat.as_ref() {
        Pat::Ident(pat_ident) => pat_ident.ident.clone(),
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

    // A shared reference is either a lens (`&MyLens`) or a ToOwned reference
    // (`&str`, `&[u8]`, etc.). We classify `&Ident` tentatively as Lens; the
    // caller promotes to ValueRef if the ident isn't a registered lens/atom.
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
            if let Some(lens_name) = p.path.get_ident() {
                return Ok(ParsedParam::Lens {
                    param_name,
                    lens_name: lens_name.clone(),
                    ty: (*typed.ty).clone(),
                    is_impl_into: false,
                });
            }
        }
        // Non-ident referent (e.g., &[u8], &(A, B), &dyn Trait) — ToOwned ref.
        return Ok(ParsedParam::ValueRef {
            param_name,
            referent: (*r.elem).clone(),
        });
    }

    // `impl Into<LensName<'...>>` — the user-written equivalent of `&LensName`
    // with the ergonomic sugar that lets callers pass `&atom` directly. We
    // accept it verbatim and the body calls `.into()` before use.
    if let Type::ImplTrait(it) = typed.ty.as_ref() {
        if let Some(lens_name) = extract_lens_name_from_impl_into(it) {
            return Ok(ParsedParam::Lens {
                param_name,
                lens_name,
                ty: (*typed.ty).clone(),
                is_impl_into: true,
            });
        }
    }

    // Anything else: treat as an owned value parameter.
    Ok(ParsedParam::Value {
        param_name,
        ty: (*typed.ty).clone(),
    })
}

/// If `it` is `impl Into<LensName[<'..>]>`, return `LensName`. Other bounds
/// on the `impl Trait` (e.g. `+ Send`) aren't accepted — the sugar form
/// should be a single `Into<...>` bound.
fn extract_lens_name_from_impl_into(it: &syn::TypeImplTrait) -> Option<Ident> {
    if it.bounds.len() != 1 {
        return None;
    }
    let tb = match it.bounds.first()? {
        syn::TypeParamBound::Trait(tb) => tb,
        _ => return None,
    };
    let last = tb.path.segments.last()?;
    if last.ident != "Into" {
        return None;
    }
    let args = match &last.arguments {
        syn::PathArguments::AngleBracketed(a) => a,
        _ => return None,
    };
    let target = match args.args.first()? {
        syn::GenericArgument::Type(ty) => ty,
        _ => return None,
    };
    let p = match target {
        Type::Path(p) => p,
        _ => return None,
    };
    Some(p.path.segments.last()?.ident.clone())
}

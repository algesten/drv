use proc_macro2::TokenStream;
use quote::quote;
use syn::{FnArg, Ident, ItemFn, Pat, ReturnType, Type};

use crate::registry::{self, MemoParam, MemoRegistration};

pub fn expand(item: ItemFn) -> Result<TokenStream, syn::Error> {
    let fn_name = &item.sig.ident;

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
            } = p
            {
                let lens_name_str = lens_name.to_string();
                if !reg.lens_name_exists(&lens_name_str) && !reg.atom_name_exists(&lens_name_str) {
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

        reg.memos.push(MemoRegistration {
            fn_name: fn_name_str,
            vis_tokens: quote!(#vis).to_string(),
            params: parsed
                .iter()
                .map(|p| match p {
                    ParsedParam::Lens {
                        param_name,
                        lens_name,
                    } => MemoParam::Lens {
                        param_name: param_name.to_string(),
                        lens_name: lens_name.to_string(),
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

enum ParsedParam {
    Lens {
        param_name: Ident,
        lens_name: Ident,
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
                });
            }
        }
        // Non-ident referent (e.g., &[u8], &(A, B), &dyn Trait) — ToOwned ref.
        return Ok(ParsedParam::ValueRef {
            param_name,
            referent: (*r.elem).clone(),
        });
    }

    // Anything else: treat as an owned value parameter.
    Ok(ParsedParam::Value {
        param_name,
        ty: (*typed.ty).clone(),
    })
}

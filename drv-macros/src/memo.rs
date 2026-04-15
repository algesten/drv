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

    // Classify each parameter as either a lens reference or an owned value.
    let mut parsed: Vec<ParsedParam> = Vec::new();
    for param in &item.sig.inputs {
        parsed.push(classify_param(param, fn_name)?);
    }

    // Require at least one lens (otherwise we have no atom for the cache).
    if !parsed.iter().any(|p| matches!(p, ParsedParam::Lens { .. })) {
        return Err(syn::Error::new_spanned(
            &item.sig.inputs,
            format!(
                "#[drv::memo] function '{}' must have at least one `&LensName` parameter",
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

    let fn_name_str = fn_name.to_string();

    registry::with(|reg| {
        // Verify each lens param exists; if it names an atom, create an
        // identity-lens registration on the fly.
        for p in &parsed {
            if let ParsedParam::Lens { lens_name, .. } = p {
                let lens_name_str = lens_name.to_string();
                if !reg.lens_name_exists(&lens_name_str) {
                    let available: Vec<_> = reg
                        .lenses
                        .iter()
                        .filter(|l| !l.is_identity)
                        .map(|l| l.name.clone())
                        .chain(reg.atoms.iter().map(|a| a.name.clone()))
                        .collect();
                    let hint = if available.is_empty() {
                        "no lenses or atoms have been declared yet".to_string()
                    } else {
                        format!("available lenses/atoms: {}", available.join(", "))
                    };
                    return Err(syn::Error::new_spanned(
                        lens_name,
                        format!("'{}' is not a known lens or atom\n{}", lens_name, hint),
                    ));
                }
            }
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
                })
                .collect(),
            output_ty_tokens: registry::type_to_tokens(&output_ty),
            body_tokens,
        });

        Ok(())
    })?;

    // Swallow the function entirely — assemble!() emits the rewritten version.
    Ok(TokenStream::new())
}

enum ParsedParam {
    Lens { param_name: Ident, lens_name: Ident },
    Value { param_name: Ident, ty: syn::Type },
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

    // A shared reference to a simple path = lens/atom parameter.
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
        return Err(syn::Error::new_spanned(
            &typed.ty,
            format!(
                "#[drv::memo] function '{}': references are only allowed for lens/atom types \
                 (e.g. `&MyLens`); owned types are required for value parameters",
                fn_name
            ),
        ));
    }

    // Anything else: treat as an owned value parameter.
    Ok(ParsedParam::Value {
        param_name,
        ty: (*typed.ty).clone(),
    })
}

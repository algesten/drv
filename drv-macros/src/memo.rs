use proc_macro2::TokenStream;
use quote::quote;
use syn::{FnArg, ItemFn, Pat, ReturnType, Type};

use crate::registry::{self, MemoLensParam, MemoRegistration};

pub fn expand(item: ItemFn) -> Result<TokenStream, syn::Error> {
    let fn_name = &item.sig.ident;

    if item.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.sig.inputs,
            format!(
                "#[drv::memo] function '{}' must take at least one parameter: &LensName",
                fn_name,
            ),
        ));
    }

    let mut lens_params = Vec::new();
    for param in &item.sig.inputs {
        let (param_name, lens_name) = extract_param_info(param, fn_name)?;
        lens_params.push((param_name, lens_name));
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
        for (_, lens_name) in &lens_params {
            let lens_name_str = lens_name.to_string();
            if !reg.lens_name_exists(&lens_name_str) {
                // Not a lens — but every atom also has an identity-lens registration
                // (added in atom.rs). So if neither found, it's truly missing.
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
            lens_params: lens_params
                .iter()
                .map(|(param_name, lens_name)| MemoLensParam {
                    param_name: param_name.to_string(),
                    lens_name: lens_name.to_string(),
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

fn extract_param_info(
    param: &FnArg,
    fn_name: &syn::Ident,
) -> Result<(syn::Ident, syn::Ident), syn::Error> {
    let typed = match param {
        FnArg::Typed(t) => t,
        FnArg::Receiver(_) => {
            return Err(syn::Error::new_spanned(
                param,
                format!(
                    "#[drv::memo] function '{}' must take &LensName, not self",
                    fn_name
                ),
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

    let inner_ty = match typed.ty.as_ref() {
        Type::Reference(r) => {
            if r.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    &typed.ty,
                    format!(
                        "#[drv::memo] function '{}' must take &LensName (shared reference)",
                        fn_name
                    ),
                ));
            }
            r.elem.as_ref()
        }
        _ => {
            return Err(syn::Error::new_spanned(
                &typed.ty,
                format!(
                    "#[drv::memo] function '{}' must take &LensName (a shared reference to a lens)",
                    fn_name,
                ),
            ));
        }
    };

    let lens_name = match inner_ty {
        Type::Path(p) => {
            if let Some(ident) = p.path.get_ident() {
                ident.clone()
            } else {
                return Err(syn::Error::new_spanned(
                    inner_ty,
                    format!(
                        "#[drv::memo] function '{}': the lens type must be a simple name",
                        fn_name
                    ),
                ));
            }
        }
        _ => {
            return Err(syn::Error::new_spanned(
                inner_ty,
                format!(
                    "#[drv::memo] function '{}': expected a lens type name",
                    fn_name
                ),
            ));
        }
    };

    Ok((param_name, lens_name))
}

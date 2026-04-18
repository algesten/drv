use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, ExprStruct, Ident, ItemImpl, Pat};

use crate::registry::{self, ProjRegistration};

pub fn expand(mut item: ItemImpl) -> Result<TokenStream, syn::Error> {
    // Extract lens name from the Self type (e.g., MyLens<'a>).
    let lens_name = extract_lens_name(&item)?;

    // Extract atom name from the From trait (e.g., `From<&'a Bar>` → `Bar`).
    let atom_name = extract_atom_name(&item)?;

    // Extract the user's parameter name from the from() method.
    let user_param = extract_from_param_name(&item)?;

    // Extract the lifetime on the reference parameter (used when we rewrite
    // the signature — we preserve whatever lifetime the user wrote).
    let lifetime = extract_from_lifetime(&item);

    // Validate and register.
    registry::with(|reg| {
        let lens_name_str = lens_name.to_string();
        let atom_name_str = atom_name.to_string();

        // Verify the lens expects a user-written projection.
        let lens = reg.find_lens(&lens_name_str);
        match lens {
            Some(l) if l.is_proj && l.atom_name == atom_name_str => {}
            Some(l) if !l.is_proj => {
                return Err(syn::Error::new_spanned(
                    &item.self_ty,
                    format!(
                        "lens '{}' is a standard lens and does not need a \
                         projection impl -- #[drv::proj] is only for lenses \
                         whose fields don't match the atom",
                        lens_name_str
                    ),
                ));
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    &item.self_ty,
                    format!(
                        "lens '{}' for atom '{}' not found -- \
                         #[drv::lens({})] must appear before #[drv::proj]",
                        lens_name_str, atom_name_str, atom_name_str
                    ),
                ));
            }
        }

        // Register that the projection impl exists.
        reg.proj_impls.push(ProjRegistration {
            lens_name: lens_name_str,
            atom_name: atom_name_str,
        });

        Ok(())
    })?;

    // Rewrite the impl so the user never has to type the wrapper:
    //
    // - Change the trait argument from `&'a MyAtom` to `&'a ::drv::Atom<MyAtom>`.
    // - Rename the `from` parameter to an internal name that holds the wrapper.
    // - Re-bind the user's original name to a deref'd `&MyAtom` at the top of
    //   the body, so field access (`v.inner`) reads the atom directly.
    // - Inject `__drv: #internal.__drv_cache()` into struct literals.
    let internal_param = Ident::new("__drv_proj_src", user_param.span());
    rewrite_trait_arg(&mut item, &atom_name)?;
    rewrite_from_method(
        &mut item,
        &user_param,
        &internal_param,
        &atom_name,
        lifetime.as_ref(),
    )?;
    inject_drv_field(&mut item, &internal_param)?;

    Ok(quote! { #item })
}

/// Extract the lens name from `impl ... for LensName<'a>`.
fn extract_lens_name(item: &ItemImpl) -> Result<Ident, syn::Error> {
    if let syn::Type::Path(p) = item.self_ty.as_ref() {
        if let Some(seg) = p.path.segments.last() {
            return Ok(seg.ident.clone());
        }
    }
    Err(syn::Error::new_spanned(
        &item.self_ty,
        "#[drv::proj] expects `impl From<&Atom> for LensName<'_>`",
    ))
}

/// Extract the atom name from `impl From<&'a AtomName> for ...`.
fn extract_atom_name(item: &ItemImpl) -> Result<Ident, syn::Error> {
    let trait_path = match &item.trait_ {
        Some((_, path, _)) => path,
        None => {
            return Err(syn::Error::new_spanned(
                item,
                "#[drv::proj] must be on a `From` impl",
            ));
        }
    };

    let last_seg = trait_path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(trait_path, "#[drv::proj] expects `From<&Atom>`"))?;

    if last_seg.ident != "From" {
        return Err(syn::Error::new_spanned(
            &last_seg.ident,
            "#[drv::proj] must be on a `From` impl",
        ));
    }

    // Extract `From<&'a AtomName>` → `AtomName`.
    if let syn::PathArguments::AngleBracketed(args) = &last_seg.arguments {
        if let Some(syn::GenericArgument::Type(syn::Type::Reference(r))) = args.args.first() {
            if let syn::Type::Path(p) = r.elem.as_ref() {
                if let Some(seg) = p.path.segments.last() {
                    if seg.ident == "Atom" {
                        return Err(syn::Error::new_spanned(
                            &last_seg.arguments,
                            "#[drv::proj] expects `From<&YourAtom>` — the \
                             `Atom<_>` wrapper is added automatically",
                        ));
                    }
                    return Ok(seg.ident.clone());
                }
            }
        }
    }

    Err(syn::Error::new_spanned(
        trait_path,
        "#[drv::proj] expects `From<&AtomName>` or `From<&'a AtomName>`",
    ))
}

/// Extract the parameter name from `fn from(v: &Atom) -> Self`.
fn extract_from_param_name(item: &ItemImpl) -> Result<Ident, syn::Error> {
    for impl_item in &item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            if method.sig.ident == "from" {
                if let Some(syn::FnArg::Typed(pat_ty)) = method.sig.inputs.first() {
                    if let Pat::Ident(pat_ident) = pat_ty.pat.as_ref() {
                        return Ok(pat_ident.ident.clone());
                    }
                }
            }
        }
    }
    Err(syn::Error::new_spanned(
        item,
        "#[drv::proj] could not find `fn from(param: &Atom)` in impl",
    ))
}

/// Extract the lifetime written on the `from` parameter reference, if any.
fn extract_from_lifetime(item: &ItemImpl) -> Option<syn::Lifetime> {
    for impl_item in &item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            if method.sig.ident == "from" {
                if let Some(syn::FnArg::Typed(pat_ty)) = method.sig.inputs.first() {
                    if let syn::Type::Reference(r) = pat_ty.ty.as_ref() {
                        return r.lifetime.clone();
                    }
                }
            }
        }
    }
    None
}

/// Rewrite `impl ... From<&'a MyAtom> ...` to `impl ... From<&'a ::drv::Atom<MyAtom>> ...`.
fn rewrite_trait_arg(item: &mut ItemImpl, atom_name: &Ident) -> Result<(), syn::Error> {
    let trait_path = &mut item
        .trait_
        .as_mut()
        .expect("validated earlier: proj is on a trait impl")
        .1;
    let last_seg = trait_path
        .segments
        .last_mut()
        .expect("validated earlier: From<...> has segments");
    if let syn::PathArguments::AngleBracketed(args) = &mut last_seg.arguments {
        if let Some(syn::GenericArgument::Type(syn::Type::Reference(r))) = args.args.first_mut() {
            *r.elem = syn::parse_quote! { ::drv::Atom<#atom_name> };
            return Ok(());
        }
    }
    Err(syn::Error::new_spanned(
        trait_path,
        "#[drv::proj] expects `From<&Atom>`",
    ))
}

/// Rewrite the `from` method to take `&'a Atom<Atom>` under a fresh internal
/// name, and re-bind the user's original parameter name to a deref'd `&Atom`
/// so their body reads atom fields naturally.
fn rewrite_from_method(
    item: &mut ItemImpl,
    user_param: &Ident,
    internal_param: &Ident,
    atom_name: &Ident,
    lifetime: Option<&syn::Lifetime>,
) -> Result<(), syn::Error> {
    for impl_item in &mut item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            if method.sig.ident == "from" {
                // Rewrite first parameter.
                if let Some(syn::FnArg::Typed(pat_ty)) = method.sig.inputs.first_mut() {
                    if let syn::Pat::Ident(pat_ident) = pat_ty.pat.as_mut() {
                        pat_ident.ident = internal_param.clone();
                    }
                    *pat_ty.ty = match lifetime {
                        Some(lt) => syn::parse_quote! { &#lt ::drv::Atom<#atom_name> },
                        None => syn::parse_quote! { &::drv::Atom<#atom_name> },
                    };
                }
                // Insert `let #user_param: &'a Atom = &**#internal_param;` at the top.
                let binding: syn::Stmt = match lifetime {
                    Some(lt) => syn::parse_quote! {
                        let #user_param: &#lt #atom_name = &**#internal_param;
                    },
                    None => syn::parse_quote! {
                        let #user_param: &#atom_name = &**#internal_param;
                    },
                };
                method.block.stmts.insert(0, binding);
                return Ok(());
            }
        }
    }
    Err(syn::Error::new_spanned(
        &*item,
        "#[drv::proj] could not find `fn from` in impl",
    ))
}

/// Walk the from() body and inject `__drv: #param.__drv_cache()` into struct literals.
fn inject_drv_field(item: &mut ItemImpl, param_name: &Ident) -> Result<(), syn::Error> {
    for impl_item in &mut item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            if method.sig.ident == "from" {
                inject_drv_in_block(&mut method.block, param_name);
                return Ok(());
            }
        }
    }
    Err(syn::Error::new_spanned(
        &*item,
        "#[drv::proj] could not find `fn from` in impl",
    ))
}

/// Recursively walk a block and inject __drv into any struct literal expressions.
fn inject_drv_in_block(block: &mut syn::Block, param_name: &Ident) {
    for stmt in &mut block.stmts {
        inject_drv_in_stmt(stmt, param_name);
    }
}

fn inject_drv_in_stmt(stmt: &mut syn::Stmt, param_name: &Ident) {
    match stmt {
        syn::Stmt::Expr(expr, _) => inject_drv_in_expr(expr, param_name),
        syn::Stmt::Local(local) => {
            if let Some(init) = &mut local.init {
                inject_drv_in_expr(&mut init.expr, param_name);
            }
        }
        _ => {}
    }
}

fn inject_drv_in_expr(expr: &mut Expr, param_name: &Ident) {
    match expr {
        Expr::Struct(s) => inject_drv_in_struct_expr(s, param_name),
        Expr::Block(b) => inject_drv_in_block(&mut b.block, param_name),
        Expr::Return(r) => {
            if let Some(e) = &mut r.expr {
                inject_drv_in_expr(e, param_name);
            }
        }
        Expr::If(i) => {
            inject_drv_in_block(&mut i.then_branch, param_name);
            if let Some((_, else_branch)) = &mut i.else_branch {
                inject_drv_in_expr(else_branch, param_name);
            }
        }
        Expr::Match(m) => {
            for arm in &mut m.arms {
                inject_drv_in_expr(&mut arm.body, param_name);
            }
        }
        _ => {}
    }
}

fn inject_drv_in_struct_expr(s: &mut ExprStruct, param_name: &Ident) {
    // Only inject into struct literals that look like `Self { ... }` or
    // `LensName { ... }`. We inject `__drv: #param.__drv_cache()` as the
    // first field.
    let drv_field: syn::FieldValue = syn::parse_quote! {
        __drv: #param_name.__drv_cache()
    };
    s.fields.insert(0, drv_field);
}

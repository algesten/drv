use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Fields, Ident, ItemStruct};

use crate::atom::generate_lens_types_with;
use crate::registry::{self, LensField, LensRegistration};

pub fn expand(attr_args: &Ident, item: ItemStruct) -> Result<TokenStream, syn::Error> {
    let atom_name = attr_args;
    let lens_name = &item.ident;

    let fields = match &item.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &item,
                "drv::lens requires a struct with named fields",
            ));
        }
    };

    // Look up atom in registry.
    let atom = registry::with(|reg| {
        let atom_name_str = atom_name.to_string();
        match reg.find_atom(&atom_name_str) {
            Some(a) => Ok(a.clone()),
            None => {
                let available: Vec<_> = reg.atoms.iter().map(|a| a.name.clone()).collect();
                let hint = if available.is_empty() {
                    "no atoms have been declared yet".to_string()
                } else {
                    format!("available atoms: {}", available.join(", "))
                };
                Err(syn::Error::new_spanned(
                    atom_name,
                    format!(
                        "atom '{}' not found -- #[drv::atom] must appear before #[drv::lens({})]\n\
                         hint: {}\n\
                         hint: atoms are discovered in module order (the order `mod` declarations appear)",
                        atom_name, atom_name, hint
                    ),
                ))
            }
        }
    })?;

    // Check if all fields match the atom. Accept both `T` and `&T` when atom has `T`.
    // Track which fields the user wrote as explicit references.
    let mut force_ref: Vec<bool> = Vec::new();
    let mut all_match = true;
    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let field_ty_tokens = registry::type_to_tokens(field_ty);

        // Direct match: lens type == atom type
        let direct = atom.fields.iter().any(|af| {
            field_name == af.name.as_str() && registry::types_match(&af.ty_tokens, &field_ty_tokens)
        });
        if direct {
            force_ref.push(false);
            continue;
        }

        // Reference match: lens has &T, atom has T
        if let syn::Type::Reference(r) = field_ty {
            let inner_tokens = registry::type_to_tokens(&r.elem);
            let ref_match = atom.fields.iter().any(|af| {
                field_name == af.name.as_str()
                    && registry::types_match(&af.ty_tokens, &inner_tokens)
            });
            if ref_match {
                force_ref.push(true);
                continue;
            }
        }

        all_match = false;
        break;
    }

    if all_match {
        expand_standard(atom_name, lens_name, fields, &atom, &force_ref)
    } else {
        expand_factory(atom_name, lens_name, &item, fields)
    }
}

fn expand_standard(
    atom_name: &Ident,
    lens_name: &Ident,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    atom: &registry::AtomRegistration,
    force_ref: &[bool],
) -> Result<TokenStream, syn::Error> {
    // Validate each field. Already verified by the all_match check above,
    // but re-validate here for precise error messages on name mismatches.
    let mut lens_fields = Vec::new();
    registry::with(|reg| {
        let atom_name_str = atom_name.to_string();
        for (i, field) in fields.iter().enumerate() {
            let field_name = field.ident.as_ref().unwrap();
            let atom_field = atom.fields.iter().find(|af| field_name == af.name.as_str());
            if atom_field.is_none() {
                let available = reg.atom_field_names(&atom_name_str);
                return Err(syn::Error::new_spanned(
                    field_name,
                    format!(
                        "field '{}' does not exist on atom '{}'\n\
                         available fields: {}",
                        field_name,
                        atom_name,
                        available.join(", ")
                    ),
                ));
            }
            let _ = i; // force_ref already validated the type match
            lens_fields.push(LensField {
                name: field_name.to_string(),
                ty_tokens: None,
                is_ref: false,
                referent_tokens: None,
            });
        }

        if reg.lens_name_exists(&lens_name.to_string()) {
            return Err(syn::Error::new_spanned(
                lens_name,
                format!(
                    "lens '{}' is already declared -- lens names must be unique within a crate",
                    lens_name
                ),
            ));
        }

        reg.lenses.push(LensRegistration {
            name: lens_name.to_string(),
            atom_name: atom_name.to_string(),
            fields: lens_fields,
            is_identity: false,
            is_factory: false,
        });

        Ok(())
    })?;

    // We ignore the user-provided struct body and generate our own borrow lens.
    // The user's struct definition is purely a specification — we generate
    // both the borrow-lens and snapshot types. For fields the user wrote as
    // `&T` (when atom has `T`), we strip the `&` and use force_ref so the
    // generated lens keeps them as references.
    let field_names: Vec<Ident> = fields
        .iter()
        .map(|f| f.ident.as_ref().unwrap().clone())
        .collect();
    let field_types: Vec<syn::Type> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            if force_ref.get(i).copied().unwrap_or(false) {
                // User wrote &T — strip the & to get the atom's type T
                if let syn::Type::Reference(r) = &f.ty {
                    (*r.elem).clone()
                } else {
                    f.ty.clone()
                }
            } else {
                f.ty.clone()
            }
        })
        .collect();

    let snapshot_ident = format_ident!("__Drv{}", lens_name);
    let output = generate_lens_types_with(
        lens_name,
        &snapshot_ident,
        atom_name,
        &field_names,
        &field_types,
        force_ref,
    );
    Ok(output)
}

fn expand_factory(
    atom_name: &Ident,
    lens_name: &Ident,
    item: &ItemStruct,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
) -> Result<TokenStream, syn::Error> {
    // Register as factory lens with full type info.
    registry::with(|reg| {
        if reg.lens_name_exists(&lens_name.to_string()) {
            return Err(syn::Error::new_spanned(
                lens_name,
                format!(
                    "lens '{}' is already declared -- lens names must be unique within a crate",
                    lens_name
                ),
            ));
        }

        let mut lens_fields = Vec::new();
        for field in fields {
            let field_name = field.ident.as_ref().unwrap();
            let field_ty = &field.ty;
            let ty_tokens = registry::type_to_tokens(field_ty);

            let (is_ref, referent_tokens) = if let syn::Type::Reference(r) = field_ty {
                (true, Some(registry::type_to_tokens(&r.elem)))
            } else {
                (false, None)
            };

            lens_fields.push(LensField {
                name: field_name.to_string(),
                ty_tokens: Some(ty_tokens),
                is_ref,
                referent_tokens,
            });
        }

        reg.lenses.push(LensRegistration {
            name: lens_name.to_string(),
            atom_name: atom_name.to_string(),
            fields: lens_fields,
            is_identity: false,
            is_factory: true,
        });

        Ok(())
    })?;

    // Determine the lifetime parameter for the lens struct.
    // The user must provide a lifetime because __drv is always a reference.
    let lifetime = item
        .generics
        .lifetimes()
        .next()
        .map(|lt| lt.lifetime.clone());

    let lifetime = match lifetime {
        Some(lt) => lt,
        None => {
            return Err(syn::Error::new_spanned(
                &item.ident,
                "factory lens requires a lifetime parameter (e.g., `struct MyLens<'a> { ... }`)\n\
                 hint: the cache reference `__drv` borrows from the source atom",
            ));
        }
    };

    // Emit the user's struct with __drv injected.
    let vis = &item.vis;
    let other_attrs: Vec<_> = item.attrs.iter().collect();
    let generics = &item.generics;

    let user_fields: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let attrs = &f.attrs;
            let vis = &f.vis;
            let ident = &f.ident;
            let ty = &f.ty;
            quote! { #(#attrs)* #vis #ident: #ty }
        })
        .collect();

    let snapshot_ident = format_ident!("__Drv{}", lens_name);

    let mut output = quote! {
        #(#other_attrs)*
        #vis struct #lens_name #generics {
            #[doc(hidden)]
            pub __drv: &#lifetime ::drv::Cache<#atom_name>,
            #(#user_fields,)*
        }
    };

    // Generate snapshot struct, PartialEq, __drv_snapshot.
    output.extend(generate_factory_lens_types(
        lens_name,
        &snapshot_ident,
        fields,
        &lifetime,
    ));

    Ok(output)
}

/// Generate snapshot struct, PartialEq, and __drv_snapshot for a factory lens.
fn generate_factory_lens_types(
    lens_ident: &Ident,
    snapshot_ident: &Ident,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    lifetime: &syn::Lifetime,
) -> TokenStream {
    let mut snap_fields = Vec::new();
    let mut eq_checks = Vec::new();
    let mut snap_stores = Vec::new();

    for field in fields {
        let fname = field.ident.as_ref().unwrap();
        let fty = &field.ty;

        if let syn::Type::Reference(r) = fty {
            // Reference field: snapshot stores ToOwned::Owned
            let referent = &*r.elem;
            snap_fields.push(quote! {
                pub #fname: <#referent as ::std::borrow::ToOwned>::Owned
            });
            // Cross-type PartialEq (&T vs Owned) — can't use FastEq here.
            // Deref both sides so e.g. &str and String both become str.
            eq_checks.push(quote! { (*self.#fname == *other.#fname) });
            // Store via to_owned
            snap_stores.push(quote! {
                #fname: ::std::borrow::ToOwned::to_owned(self.#fname)
            });
        } else {
            // Owned field: snapshot stores same type. Use FastEq for ptr_eq
            // short-circuit on Arc/imbl types.
            eq_checks.push(quote! {
                ({
                    use ::drv::FastEqFallback as _;
                    ::drv::FastEq(&self.#fname).fast_eq(&other.#fname)
                })
            });
            snap_fields.push(quote! { pub #fname: #fty });
            // Clone for storage
            snap_stores.push(quote! { #fname: self.#fname.clone() });
        }
    }

    quote! {
        // Owned snapshot for cache storage.
        #[doc(hidden)]
        #[derive(Default)]
        pub struct #snapshot_ident {
            #(#snap_fields,)*
        }

        // Cross-type PartialEq: factory lens vs owned snapshot.
        impl<#lifetime> ::core::cmp::PartialEq<#snapshot_ident> for #lens_ident<#lifetime> {
            fn eq(&self, other: &#snapshot_ident) -> bool {
                #(#eq_checks)&&*
            }
        }

        // Snapshot the lens to an owned value for cache storage (cache miss only).
        impl<#lifetime> #lens_ident<#lifetime> {
            #[doc(hidden)]
            pub fn __drv_snapshot(&self) -> #snapshot_ident {
                #snapshot_ident {
                    #(#snap_stores,)*
                }
            }
        }
    }
}

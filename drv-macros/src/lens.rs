use proc_macro2::TokenStream;
use quote::format_ident;
use syn::{Fields, Ident, ItemStruct};

use crate::atom::generate_lens_types;
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

    // Validate against atom in registry.
    registry::with(|reg| {
        let atom_name_str = atom_name.to_string();
        let atom = match reg.find_atom(&atom_name_str) {
            Some(a) => a,
            None => {
                let available: Vec<_> = reg.atoms.iter().map(|a| a.name.clone()).collect();
                let hint = if available.is_empty() {
                    "no atoms have been declared yet".to_string()
                } else {
                    format!("available atoms: {}", available.join(", "))
                };
                return Err(syn::Error::new_spanned(
                    atom_name,
                    format!(
                        "atom '{}' not found -- #[drv::atom] must appear before #[drv::lens({})]\n\
                         hint: {}\n\
                         hint: atoms are discovered in module order (the order `mod` declarations appear)",
                        atom_name, atom_name, hint
                    ),
                ));
            }
        };

        let mut lens_fields = Vec::new();
        for field in fields {
            let field_name = field.ident.as_ref().unwrap();
            let field_ty = &field.ty;
            let field_ty_tokens = registry::type_to_tokens(field_ty);
            let atom_field = atom.fields.iter().find(|af| field_name == af.name.as_str());
            match atom_field {
                None => {
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
                Some(af) => {
                    if !registry::types_match(&af.ty_tokens, &field_ty_tokens) {
                        return Err(syn::Error::new_spanned(
                            field_ty,
                            format!(
                                "type mismatch for field '{}': lens has '{}' but atom '{}' has '{}'\n\
                                 hint: the type must be spelled identically in both the atom and the lens",
                                field_name,
                                field_ty_tokens,
                                atom_name,
                                af.ty_tokens,
                            ),
                        ));
                    }
                }
            }
            let _ = field_ty_tokens;
            lens_fields.push(LensField {
                name: field_name.to_string(),
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
        });

        Ok(())
    })?;

    // We ignore the user-provided struct body and generate our own borrow lens.
    // The user's struct definition is purely a specification — we generate
    // both the borrow-lens and snapshot types.
    let field_names: Vec<Ident> = fields
        .iter()
        .map(|f| f.ident.as_ref().unwrap().clone())
        .collect();
    let field_types: Vec<syn::Type> = fields.iter().map(|f| f.ty.clone()).collect();

    let snapshot_ident = format_ident!("__Drv{}", lens_name);
    let output = generate_lens_types(
        lens_name,
        &snapshot_ident,
        atom_name,
        &field_names,
        &field_types,
    );
    Ok(output)
}

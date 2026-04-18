use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Fields, Ident, ItemStruct};

use crate::registry::{self, AtomField, AtomRegistration, LensRegistration};

pub fn expand(attr: TokenStream, item: ItemStruct) -> Result<TokenStream, syn::Error> {
    let struct_name = &item.ident;

    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "drv::atom takes no arguments -- put derives in #[derive(...)] on the struct directly",
        ));
    }

    let fields = match &item.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &item,
                "drv::atom requires a struct with named fields",
            ));
        }
    };

    // Collect atom fields and inline lens annotations.
    let mut inline_lenses: std::collections::HashMap<String, Vec<(Ident, syn::Type)>> =
        std::collections::HashMap::new();
    let mut atom_fields = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap().clone();
        let field_ty = field.ty.clone();

        atom_fields.push(AtomField {
            name: field_name.to_string(),
            ty_tokens: registry::type_to_tokens(&field_ty),
        });

        for attr in &field.attrs {
            let lens_names = parse_lens_attr(attr)?;
            for lens_name in lens_names {
                inline_lenses
                    .entry(lens_name.to_string())
                    .or_default()
                    .push((field_name.clone(), field_ty.clone()));
            }
        }
    }

    registry::with(|reg| {
        for lens_name in inline_lenses.keys() {
            if reg.lens_name_exists(lens_name) {
                return Err(syn::Error::new_spanned(
                    struct_name,
                    format!(
                        "lens '{}' is already declared -- lens names must be unique within a crate",
                        lens_name
                    ),
                ));
            }
        }
        Ok(())
    })?;

    registry::with(|reg| {
        if reg.atom_name_exists(&struct_name.to_string()) {
            return Err(syn::Error::new_spanned(
                struct_name,
                format!(
                    "atom '{}' is already declared -- atom names must be unique within a crate",
                    struct_name
                ),
            ));
        }

        reg.atoms.push(AtomRegistration {
            name: struct_name.to_string(),
            fields: atom_fields,
        });

        for (lens_name_str, lens_fields) in &inline_lenses {
            let lens_ident = Ident::new(lens_name_str, struct_name.span());
            let snapshot_ident = format_ident!("__Drv{}", lens_ident);
            let field_names: Vec<_> = lens_fields.iter().map(|(n, _)| n.clone()).collect();
            let field_types: Vec<_> = lens_fields.iter().map(|(_, t)| t.clone()).collect();
            let (_, from_impl) = generate_lens_types(
                &lens_ident,
                &snapshot_ident,
                struct_name,
                &field_names,
                &field_types,
            );

            reg.lenses.push(LensRegistration {
                name: lens_name_str.clone(),
                atom_name: struct_name.to_string(),
                is_identity: false,
                is_proj: false,
                from_impl_tokens: Some(from_impl.to_string()),
            });
        }

        Ok(())
    })?;

    let other_attrs: Vec<_> = item.attrs.iter().filter(|a| !is_drv_attr(a)).collect();
    let vis = &item.vis;
    let generics = &item.generics;

    let clean_fields: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let ident = &f.ident;
            let ty = &f.ty;
            let vis = &f.vis;
            let attrs: Vec<_> = f.attrs.iter().filter(|a| !is_drv_attr(a)).collect();
            quote! {
                #(#attrs)*
                #vis #ident: #ty
            }
        })
        .collect();

    let mut output = quote! {
        #(#other_attrs)*
        #vis struct #struct_name #generics {
            #(#clean_fields,)*
        }
    };

    // Generate inline lens structs (borrow view + owned snapshot + PartialEq +
    // __drv_snapshot). The `From` impl is registered for `assemble!()` to emit
    // so user-written `#[drv::proj]` impls can shadow it.
    for (lens_name_str, lens_fields) in &inline_lenses {
        let lens_ident = Ident::new(lens_name_str, struct_name.span());
        let snapshot_ident = format_ident!("__Drv{}", lens_ident);
        let field_names: Vec<_> = lens_fields.iter().map(|(n, _)| n.clone()).collect();
        let field_types: Vec<_> = lens_fields.iter().map(|(_, t)| t.clone()).collect();

        let (base, _) = generate_lens_types(
            &lens_ident,
            &snapshot_ident,
            struct_name,
            &field_names,
            &field_types,
        );
        output.extend(base);
    }

    // Register the identity lens so memos taking `&AtomName` can find it.
    // The lens struct, snapshot, PartialEq, and From impls are emitted by
    // `drv::assemble!()` only when some memo actually consumes the identity
    // lens — atoms with no identity-lens consumers impose no bounds on their
    // fields beyond what their explicit lenses already demand.
    registry::with(|reg| {
        if !reg.lens_name_exists(&struct_name.to_string()) {
            reg.lenses.push(LensRegistration {
                name: struct_name.to_string(),
                atom_name: struct_name.to_string(),
                is_identity: true,
                is_proj: false,
                from_impl_tokens: None,
            });
        }
    });

    Ok(output)
}

/// Generate the user-facing lens + internal owned snapshot + PartialEq + From.
///
/// Built-in scalar primitives (u8–u128, i8–i128, usize, isize, f32, f64,
/// bool, char) are stored by value in the lens for ergonomics (`lens.x`
/// instead of `*lens.x`). All other types — including user-defined Copy
/// types — are borrowed (`&'drv T`), because the proc macro cannot query
/// trait implementations at expansion time.
/// Per-field control: should this field be a reference in the lens?
/// `None` means auto-detect (copy primitives → owned, others → ref).
/// `Some(true)` forces a reference, `Some(false)` forces owned.
pub(crate) fn generate_lens_types(
    lens_ident: &Ident,
    snapshot_ident: &Ident,
    atom_ident: &Ident,
    field_names: &[Ident],
    field_types: &[syn::Type],
) -> (TokenStream, TokenStream) {
    generate_lens_types_with(
        lens_ident,
        snapshot_ident,
        atom_ident,
        field_names,
        field_types,
        &[],
    )
}

/// Returns `(base_tokens, from_impl_tokens)`.
///
/// `base_tokens` holds the lens struct, the owned snapshot, the `PartialEq`
/// impl between them, and the `__drv_snapshot` method — these are emitted
/// inline from the `#[drv::atom]` / `#[drv::lens]` expansion.
///
/// `from_impl_tokens` holds the auto-generated `From<&Atom> for Lens` impl.
/// It is registered in the lens registration and emitted later by
/// `drv::assemble!()`, so a user-supplied `#[drv::proj]` impl (which also
/// implements `From<&Atom> for Lens`) can shadow it without a coherence
/// conflict.
pub(crate) fn generate_lens_types_with(
    lens_ident: &Ident,
    snapshot_ident: &Ident,
    atom_ident: &Ident,
    field_names: &[Ident],
    field_types: &[syn::Type],
    force_ref: &[bool],
) -> (TokenStream, TokenStream) {
    let mut struct_fields = Vec::new();
    let mut snap_fields = Vec::new();
    let mut eq_checks = Vec::new();
    let mut from_fields = Vec::new();
    let mut snap_stores = Vec::new();

    for (i, (name, ty)) in field_names.iter().zip(field_types.iter()).enumerate() {
        let is_ref = force_ref.get(i).copied().unwrap_or(false);
        if !is_ref && is_copy_primitive(ty) {
            // Owned field — copy from atom, no deref needed by user.
            struct_fields.push(quote! { pub #name: #ty });
            snap_fields.push(quote! { pub #name: #ty });
            eq_checks.push(quote! {
                (::drv::FastEq(&self.#name).fast_eq(&other.#name))
            });
            // Explicit deref: `source: &'drv Atom<Atom>` → `(*source): Atom`
            // so the field access borrows for the full `'drv` lifetime.
            from_fields.push(quote! { #name: (*source).#name });
            snap_stores.push(quote! { #name: self.#name.clone() });
        } else {
            // Reference field — borrow from atom, zero-copy.
            struct_fields.push(quote! { pub #name: &'drv #ty });
            snap_fields.push(quote! { pub #name: #ty });
            eq_checks.push(quote! {
                (::drv::FastEq(self.#name).fast_eq(&other.#name))
            });
            from_fields.push(quote! { #name: &(*source).#name });
            snap_stores.push(quote! { #name: (*self.#name).clone() });
        }
    }

    let base = quote! {
        // User-facing lens — Copy-type fields by value, others by reference.
        #[derive(Copy, Clone, Debug)]
        pub struct #lens_ident<'drv> {
            #[doc(hidden)]
            pub __drv: &'drv ::drv::Cache<#atom_ident>,
            #(#struct_fields,)*
        }

        // Internal owned snapshot for cache storage.
        #[doc(hidden)]
        pub struct #snapshot_ident {
            #(#snap_fields,)*
        }

        // PartialEq: lens vs owned snapshot.
        impl<'drv> ::core::cmp::PartialEq<#snapshot_ident> for #lens_ident<'drv> {
            fn eq(&self, other: &#snapshot_ident) -> bool {
                use ::drv::FastEqFallback as _;
                #(#eq_checks)&&*
            }
        }

        // Snapshot the lens to an owned value for cache storage (cache miss only).
        impl<'drv> #lens_ident<'drv> {
            #[doc(hidden)]
            pub fn __drv_snapshot(&self) -> #snapshot_ident {
                #snapshot_ident {
                    #(#snap_stores,)*
                }
            }
        }
    };

    // From<&Atom<Atom>> for the lens — copy primitives, borrow the rest.
    // Emitted from assemble!() unless a user #[drv::proj] shadows it.
    let from_impl = quote! {
        impl<'drv> ::core::convert::From<&'drv ::drv::Atom<#atom_ident>> for #lens_ident<'drv> {
            fn from(source: &'drv ::drv::Atom<#atom_ident>) -> Self {
                #lens_ident {
                    __drv: source.__drv_cache(),
                    #(#from_fields,)*
                }
            }
        }
    };

    (base, from_impl)
}

/// Returns true if the type is a known Copy primitive.
pub(crate) fn is_copy_primitive(ty: &syn::Type) -> bool {
    if let syn::Type::Path(p) = ty {
        if let Some(ident) = p.path.get_ident() {
            return matches!(
                ident.to_string().as_str(),
                "u8" | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "f32"
                    | "f64"
                    | "bool"
                    | "char"
            );
        }
    }
    false
}

fn parse_lens_attr(attr: &syn::Attribute) -> Result<Vec<Ident>, syn::Error> {
    let path = attr.path();
    let segments: Vec<_> = path.segments.iter().collect();
    let is_drv_lens = match segments.len() {
        1 => segments[0].ident == "lens",
        2 => segments[0].ident == "drv" && segments[1].ident == "lens",
        _ => false,
    };
    if !is_drv_lens {
        return Ok(vec![]);
    }
    let mut names = Vec::new();
    attr.parse_nested_meta(|meta| {
        if let Some(ident) = meta.path.get_ident() {
            names.push(ident.clone());
            Ok(())
        } else {
            Err(meta.error("expected a lens name, like #[drv::lens(MyLens)] or #[drv::lens(A, B)]"))
        }
    })?;
    if names.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[drv::lens(...)] requires at least one lens name, like #[drv::lens(MyLens)]",
        ));
    }
    Ok(names)
}

fn is_drv_attr(attr: &syn::Attribute) -> bool {
    let path = attr.path();
    let segments: Vec<_> = path.segments.iter().collect();
    match segments.len() {
        1 => segments[0].ident == "lens",
        2 => {
            segments[0].ident == "drv"
                && (segments[1].ident == "lens" || segments[1].ident == "atom")
        }
        _ => false,
    }
}

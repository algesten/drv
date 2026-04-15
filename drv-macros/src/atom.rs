use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Fields, Ident, ItemStruct, Visibility};

use crate::registry::{self, AtomField, AtomRegistration, LensField, LensRegistration};

pub fn expand(attr: TokenStream, item: ItemStruct) -> Result<TokenStream, syn::Error> {
    let struct_name = &item.ident;

    // Parse attribute args: #[drv::atom(derive(Hash, Eq, ...))]
    let derive_traits = parse_atom_attr(attr)?;

    let fields = match &item.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &item,
                "drv::atom requires a struct with named fields",
            ));
        }
    };

    for field in fields {
        if !matches!(&field.vis, Visibility::Public(_)) {
            let name = field
                .ident
                .as_ref()
                .map(|i| format!("'{}'", i))
                .unwrap_or_else(|| "unnamed".to_string());
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "atom field {} must be pub -- drv needs to project fields into lenses",
                    name
                ),
            ));
        }
    }

    // Reject #[derive(...)] on the struct.
    for attr in &item.attrs {
        if attr.path().is_ident("derive") {
            return Err(syn::Error::new_spanned(
                attr,
                "use #[drv::atom(derive(Hash, Eq, ...))] for extra derives on atoms\n\
                 hint: Clone, PartialEq, Debug, Default are always generated",
            ));
        }
    }

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
            reg.lenses.push(LensRegistration {
                name: lens_name_str.clone(),
                atom_name: struct_name.to_string(),
                fields: lens_fields
                    .iter()
                    .map(|(name, _ty)| LensField {
                        name: name.to_string(),
                    })
                    .collect(),
                is_identity: false,
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

    let data_field_idents: Vec<&Ident> = fields.iter().map(|f| f.ident.as_ref().unwrap()).collect();
    let data_field_types: Vec<&syn::Type> = fields.iter().map(|f| &f.ty).collect();
    let data_field_strs: Vec<String> = data_field_idents.iter().map(|i| i.to_string()).collect();

    let mut output = quote! {
        #(#other_attrs)*
        #vis struct #struct_name #generics {
            #(#clean_fields,)*
            #[doc(hidden)]
            pub __drv: ::drv::Cache,
        }
    };

    // Always generate Clone, PartialEq, Debug, Default.
    {
        let f1 = data_field_idents.iter();
        let f2 = data_field_idents.iter();
        output.extend(quote! {
            impl Clone for #struct_name {
                fn clone(&self) -> Self {
                    Self {
                        #(#f1: self.#f2.clone(),)*
                        __drv: ::drv::Cache::new(),
                    }
                }
            }
        });
    }
    {
        let f1 = data_field_idents.iter();
        let f2 = data_field_idents.iter();
        output.extend(quote! {
            impl PartialEq for #struct_name {
                fn eq(&self, other: &Self) -> bool {
                    #(self.#f1 == other.#f2)&&*
                }
            }
        });
    }
    {
        let strs = data_field_strs.iter();
        let ids = data_field_idents.iter();
        let name_str = struct_name.to_string();
        output.extend(quote! {
            impl ::core::fmt::Debug for #struct_name {
                fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                    f.debug_struct(#name_str)
                        #(.field(#strs, &self.#ids))*
                        .finish()
                }
            }
        });
    }
    {
        let f = data_field_idents.iter();
        output.extend(quote! {
            impl Default for #struct_name {
                fn default() -> Self {
                    Self {
                        #(#f: Default::default(),)*
                        __drv: Default::default(),
                    }
                }
            }
        });
    }

    // Opt-in derives.
    for trait_path in &derive_traits {
        if trait_path.is_ident("Clone")
            || trait_path.is_ident("PartialEq")
            || trait_path.is_ident("Debug")
            || trait_path.is_ident("Default")
        {
            // Already generated.
        } else if trait_path.is_ident("Eq") {
            output.extend(quote! { impl Eq for #struct_name {} });
        } else if trait_path.is_ident("Hash") {
            let f = data_field_idents.iter();
            output.extend(quote! {
                impl ::core::hash::Hash for #struct_name {
                    fn hash<H: ::core::hash::Hasher>(&self, state: &mut H) {
                        #(self.#f.hash(state);)*
                    }
                }
            });
        }
    }

    // Generate inline lens structs (borrow view + owned snapshot + PartialEq + From).
    for (lens_name_str, lens_fields) in &inline_lenses {
        let lens_ident = Ident::new(lens_name_str, struct_name.span());
        let snapshot_ident = format_ident!("__Drv{}", lens_ident);
        let field_names: Vec<_> = lens_fields.iter().map(|(n, _)| n.clone()).collect();
        let field_types: Vec<_> = lens_fields.iter().map(|(_, t)| t.clone()).collect();

        output.extend(generate_lens_types(
            &lens_ident,
            &snapshot_ident,
            struct_name,
            &field_names,
            &field_types,
        ));

        // All atom data fields (for From impl)
        let _ = &data_field_idents;
        let _ = &data_field_types;
    }

    // Generate identity lens: atom itself as lens.
    // Register it so exprs taking `&AtomName` work.
    {
        let snapshot_ident = format_ident!("__Drv{}Identity", struct_name);
        let field_names: Vec<Ident> = data_field_idents.iter().cloned().cloned().collect();
        let field_types: Vec<syn::Type> = data_field_types.iter().cloned().cloned().collect();

        // Snapshot struct for identity lens.
        let fn_iter = field_names.iter();
        let ft_iter = field_types.iter();
        output.extend(quote! {
            #[doc(hidden)]
            #[derive(Default)]
            pub struct #snapshot_ident {
                #(pub #fn_iter: #ft_iter,)*
            }
        });

        // PartialEq between &Atom and identity snapshot.
        let fe1 = field_names.iter();
        let fe2 = field_names.iter();
        output.extend(quote! {
            impl ::core::cmp::PartialEq<#snapshot_ident> for #struct_name {
                fn eq(&self, other: &#snapshot_ident) -> bool {
                    #(self.#fe1 == other.#fe2)&&*
                }
            }
        });

        registry::with(|reg| {
            // Check there's no user-defined lens with the same name as the atom.
            if !reg.lens_name_exists(&struct_name.to_string()) {
                reg.lenses.push(LensRegistration {
                    name: struct_name.to_string(),
                    atom_name: struct_name.to_string(),
                    fields: data_field_idents
                        .iter()
                        .map(|n| LensField {
                            name: n.to_string(),
                        })
                        .collect(),
                    is_identity: true,
                });
            }
        });
    }

    Ok(output)
}

/// Generate the user-facing lens + internal owned snapshot + PartialEq + From.
///
/// Lens fields are always `&'drv T` — zero-copy.
/// User bodies need to deref where values are required (e.g. `*lens.scroll_row as usize`).
pub(crate) fn generate_lens_types(
    lens_ident: &Ident,
    snapshot_ident: &Ident,
    atom_ident: &Ident,
    field_names: &[Ident],
    field_types: &[syn::Type],
) -> TokenStream {
    let fn1 = field_names.iter();
    let ft1 = field_types.iter();
    let fn2 = field_names.iter();
    let ft2 = field_types.iter();
    let fn3 = field_names.iter();
    let fn4 = field_names.iter();
    let fn5 = field_names.iter();
    let fn6 = field_names.iter();
    let fn7 = field_names.iter();

    quote! {
        // User-facing lens — all fields are references, zero-copy.
        #[derive(Copy, Clone, Debug)]
        pub struct #lens_ident<'drv> {
            #[doc(hidden)]
            pub __drv: &'drv ::drv::Cache,
            #(pub #fn1: &'drv #ft1,)*
        }

        // Internal owned snapshot for cache storage.
        #[doc(hidden)]
        #[derive(Default)]
        pub struct #snapshot_ident {
            #(pub #fn2: #ft2,)*
        }

        // Cross-type PartialEq: lens (refs) vs owned snapshot.
        impl<'drv> ::core::cmp::PartialEq<#snapshot_ident> for #lens_ident<'drv> {
            fn eq(&self, other: &#snapshot_ident) -> bool {
                #(*self.#fn3 == other.#fn4)&&*
            }
        }

        // From<&Atom> for the lens — borrow each field, no cloning.
        impl<'drv> ::core::convert::From<&'drv #atom_ident> for #lens_ident<'drv> {
            fn from(source: &'drv #atom_ident) -> Self {
                #lens_ident {
                    __drv: &source.__drv,
                    #(#fn5: &source.#fn6,)*
                }
            }
        }

        // Snapshot the lens to an owned value for cache storage (cache miss only).
        impl<'drv> #lens_ident<'drv> {
            #[doc(hidden)]
            pub fn __drv_snapshot(&self) -> #snapshot_ident {
                #snapshot_ident {
                    #(#fn7: (*self.#fn7).clone(),)*
                }
            }
        }
    }
}

fn parse_atom_attr(attr: TokenStream) -> Result<Vec<syn::Path>, syn::Error> {
    if attr.is_empty() {
        return Ok(Vec::new());
    }
    let parsed: AtomAttrArgs = syn::parse2(attr)?;
    Ok(parsed.derives)
}

struct AtomAttrArgs {
    derives: Vec<syn::Path>,
}

impl syn::parse::Parse for AtomAttrArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut derives = Vec::new();
        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            if ident == "derive" {
                let content;
                syn::parenthesized!(content in input);
                let paths = content.parse_terminated(syn::Path::parse, syn::Token![,])?;
                derives.extend(paths);
            } else {
                return Err(syn::Error::new_spanned(
                    ident,
                    "expected `derive(...)` in #[drv::atom(...)]",
                ));
            }
            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(AtomAttrArgs { derives })
    }
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

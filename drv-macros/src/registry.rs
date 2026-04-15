use std::cell::RefCell;

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

pub fn with<F, R>(f: F) -> R
where
    F: FnOnce(&mut Registry) -> R,
{
    REGISTRY.with(|r| f(&mut r.borrow_mut()))
}

#[derive(Default)]
pub struct Registry {
    pub atoms: Vec<AtomRegistration>,
    pub lenses: Vec<LensRegistration>,
    pub memos: Vec<MemoRegistration>,
}

#[derive(Clone)]
pub struct AtomRegistration {
    pub name: String,
    pub fields: Vec<AtomField>,
}

#[derive(Clone)]
pub struct AtomField {
    pub name: String,
    pub ty_tokens: String,
}

#[derive(Clone)]
pub struct LensRegistration {
    pub name: String,
    pub atom_name: String,
    pub fields: Vec<LensField>,
    /// True if this is an "identity lens" — the lens IS the atom (all fields).
    pub is_identity: bool,
}

#[derive(Clone)]
pub struct LensField {
    pub name: String,
}

#[derive(Clone)]
pub struct MemoRegistration {
    pub fn_name: String,
    pub vis_tokens: String,
    pub lens_params: Vec<MemoLensParam>,
    pub output_ty_tokens: String,
    /// The function body tokens (as a string), to be re-parsed by assemble.
    pub body_tokens: String,
}

#[derive(Clone)]
pub struct MemoLensParam {
    pub param_name: String,
    pub lens_name: String,
}

impl Registry {
    pub fn find_atom(&self, name: &str) -> Option<&AtomRegistration> {
        self.atoms.iter().find(|a| a.name == name)
    }

    pub fn find_lens(&self, name: &str) -> Option<&LensRegistration> {
        self.lenses.iter().find(|l| l.name == name)
    }

    pub fn lens_name_exists(&self, name: &str) -> bool {
        self.lenses.iter().any(|l| l.name == name)
    }

    pub fn atom_name_exists(&self, name: &str) -> bool {
        self.atoms.iter().any(|a| a.name == name)
    }

    pub fn atom_field_names(&self, atom_name: &str) -> Vec<String> {
        self.find_atom(atom_name)
            .map(|a| a.fields.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default()
    }
}

pub fn type_to_tokens(ty: &syn::Type) -> String {
    quote::quote!(#ty).to_string()
}

pub fn types_match(a: &str, b: &str) -> bool {
    a == b
}

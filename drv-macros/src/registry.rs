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
    pub proj_impls: Vec<ProjRegistration>,
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
    /// True if this is an "identity lens" — the lens IS the atom (all fields).
    pub is_identity: bool,
    /// True if this lens *requires* a user-written `#[drv::proj]` projection
    /// impl because the macro cannot infer one (field names/types don't match
    /// the atom). Standard lenses have `is_proj: false` — the macro can
    /// auto-generate a `From` impl, but the user may still override it by
    /// writing their own `#[drv::proj]` impl.
    pub is_proj: bool,
    /// Auto-generated `From<&Atom> for Lens` token string, emitted from
    /// `assemble!()` unless a user `#[drv::proj]` impl shadows it. `None` for
    /// proj-required lenses (`is_proj: true`) and identity lenses.
    pub from_impl_tokens: Option<String>,
}

#[derive(Clone)]
pub struct ProjRegistration {
    pub lens_name: String,
}

#[derive(Clone)]
pub struct MemoRegistration {
    pub fn_name: String,
    pub vis_tokens: String,
    /// Parameters in declared order (mix of lens and value).
    pub params: Vec<MemoParam>,
    pub output_ty_tokens: String,
    /// The function body tokens (as a string), to be re-parsed by assemble.
    pub body_tokens: String,
}

#[derive(Clone)]
pub enum MemoParam {
    /// A `&LensName` parameter. Contributes to the cache key via the lens's
    /// field-by-field comparison.
    Lens {
        param_name: String,
        lens_name: String,
    },
    /// An owned value parameter like `u32` or `String`. Contributes to the
    /// cache key via PartialEq; stored via Clone.
    Value {
        param_name: String,
        ty_tokens: String,
    },
    /// A reference value parameter like `&str` or `&[u8]`. The referent `T`
    /// must implement `ToOwned`. Stored as `<T as ToOwned>::Owned`, compared
    /// via `PartialEq` between `&T` and the owned form.
    ValueRef {
        param_name: String,
        /// Token string of the referent type (e.g. "str" for &str).
        referent_tokens: String,
    },
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

    pub fn proj_exists(&self, lens_name: &str) -> bool {
        self.proj_impls.iter().any(|p| p.lens_name == lens_name)
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

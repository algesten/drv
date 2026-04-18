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
    /// True if this is an "identity lens" — the lens IS the atom (all fields).
    pub is_identity: bool,
    /// Auto-generated `From<&Atom> for Lens` token string, emitted by
    /// `assemble!()` for inline lenses. `None` for standalone lenses
    /// (the user supplies their own `impl From<...>`) and identity lenses.
    pub from_impl_tokens: Option<String>,
}

#[derive(Clone)]
pub struct MemoRegistration {
    pub fn_name: String,
    pub vis_tokens: String,
    /// The user's fn generics (lifetimes + type params), preserved verbatim
    /// so `fn foo<'a>(lens: impl Into<Lens<'a>>) -> R` works.
    pub generics_tokens: String,
    /// Parameters in declared order (mix of lens and value).
    pub params: Vec<MemoParam>,
    pub output_ty_tokens: String,
    /// The function body tokens (as a string), to be re-parsed by assemble.
    pub body_tokens: String,
    /// Cache strategy chosen via `#[drv::memo(single)]` or
    /// `#[drv::memo(lru = N)]`. Every memo must pick one — there is no
    /// default, so the choice is explicit at the call site.
    pub cache_strategy: CacheStrategy,
}

#[derive(Clone, Copy)]
pub enum CacheStrategy {
    /// `#[drv::memo(single)]` — one slot. A hit requires the current inputs
    /// to equal the most recent recompute's inputs; anything else misses.
    /// Cheap and predictable for memos with a single "live" input state.
    Single,
    /// `#[drv::memo(lru = N)]` — N slots evicted least-recently-used. Lets
    /// recurring input states (ping-pong, undo/redo) stay cached.
    Lru(usize),
}

#[derive(Clone)]
pub enum MemoParam {
    /// A lens parameter — either the literal `&LensName[<'_>]` form
    /// (body uses it directly) or the `impl Into<LensName<'..>>` sugar
    /// (body calls `.into()` once before use). Both contribute to the
    /// cache key via the lens's field-by-field comparison.
    Lens {
        param_name: String,
        lens_name: String,
        /// The user's original parameter type, emitted verbatim in the
        /// memo's outer fn signature.
        ty_tokens: String,
        /// `true` if `ty_tokens` is `impl Into<Lens<'..>>`; `false` if it
        /// is `&Lens`. Determines whether the body needs `.into()`.
        is_impl_into: bool,
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
}

pub fn type_to_tokens(ty: &syn::Type) -> String {
    quote::quote!(#ty).to_string()
}

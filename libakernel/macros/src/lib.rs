use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Error, FnArg, GenericParam, Generics, ItemTrait, Lifetime, ReceiverKind, ReturnType, TraitItem,
    Type, parse_macro_input, spanned::Spanned,
};

#[proc_macro_attribute]
/// This macro was AI generated and is currently  a proof of concept
pub fn vtable(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return Error::new(Span::call_site(), "#[vtable] takes no arguments")
            .into_compile_error()
            .into();
    }
    let tr = parse_macro_input!(item as ItemTrait);
    expand(&tr).unwrap_or_else(Error::into_compile_error).into()
}

enum ArgKind {
    Plain,
    Slice,
    SliceMut,
}

struct Arg {
    name: syn::Ident,
    abi_ty: TokenStream2,
    kind: ArgKind,
}

struct Method {
    name: syn::Ident,
    /// Lifetime generics of the method, e.g. `<'a>` (empty if none).
    generics: Generics,
    /// Lifetime of the `&self` receiver: the named one, or `'_`.
    self_lt: TokenStream2,
    args: Vec<Arg>,
    ret: ReturnType,
}

impl Method {
    /// `for<'a, ..>` prefix for the fn pointer type, or nothing.
    fn hr(&self) -> TokenStream2 {
        let params = &self.generics.params;
        if params.is_empty() {
            quote!()
        } else {
            quote!(for<#params>)
        }
    }
}

fn lt_or_elided(lt: &Option<Lifetime>) -> TokenStream2 {
    match lt {
        Some(l) => quote!(#l),
        None => quote!('_),
    }
}

/// Whether the signature has an `async` qualifier. Looks at the tokens in front of
/// `fn` so it doesn't depend on where syn stores the fn qualifiers.
fn is_async(sig: &syn::Signature) -> bool {
    sig.to_token_stream()
        .into_iter()
        .take_while(|t| !matches!(t, TokenTree::Ident(i) if i == "fn"))
        .any(|t| matches!(t, TokenTree::Ident(i) if i == "async"))
}

fn expand(tr: &ItemTrait) -> syn::Result<TokenStream2> {
    if !tr.generics.params.is_empty() {
        return Err(Error::new(
            tr.generics.span(),
            "generic traits are not supported",
        ));
    }

    let mut methods = Vec::new();
    for item in &tr.items {
        let TraitItem::Fn(f) = item else {
            continue;
        };
        let sig = &f.sig;

        if sig.ident == "clone" || sig.ident == "drop" {
            return Err(Error::new(
                sig.ident.span(),
                "`clone` and `drop` are reserved: the vtable provides them itself",
            ));
        }
        if is_async(sig) {
            return Err(Error::new(sig.span(), "async methods are not supported"));
        }
        // Only lifetime parameters are allowed; they become higher-ranked (`for<'a>`).
        for p in &sig.generics.params {
            if !matches!(p, GenericParam::Lifetime(_)) {
                return Err(Error::new(
                    p.span(),
                    "only lifetime generics are supported on vtable methods",
                ));
            }
        }
        if sig.generics.where_clause.is_some() {
            return Err(Error::new(
                sig.generics.span(),
                "where clauses are not supported on vtable methods",
            ));
        }
        let rcv = sig
            .receiver()
            .ok_or_else(|| Error::new(sig.span(), "method needs a `&self` receiver"))?;
        // `&self` / `&'a self` only (OpaqueSelf is a shared reference).
        let ReceiverKind::Reference(_, rcv_lt, None) = &rcv.kind else {
            return Err(Error::new(
                rcv.span(),
                "only `&self` receivers are supported (OpaqueSelf is a shared reference)",
            ));
        };
        let self_lt = lt_or_elided(rcv_lt);

        let mut args = Vec::new();
        for (i, arg) in sig.inputs.iter().enumerate() {
            let FnArg::Typed(pt) = arg else { continue };
            let name = format_ident!("a{}", i);
            let (abi_ty, kind) = match &*pt.ty {
                Type::Reference(r) => match &*r.elem {
                    Type::Slice(s) => {
                        let elem = &s.elem;
                        let lt = lt_or_elided(&r.lifetime);
                        if r.mutability.is_some() {
                            (quote!(FFISliceMut<#lt, #elem>), ArgKind::SliceMut)
                        } else {
                            (quote!(FFISlice<#lt, #elem>), ArgKind::Slice)
                        }
                    }
                    _ => {
                        let t = &pt.ty;
                        (quote!(#t), ArgKind::Plain)
                    }
                },
                t => (quote!(#t), ArgKind::Plain),
            };
            args.push(Arg { name, abi_ty, kind });
        }

        methods.push(Method {
            name: sig.ident.clone(),
            generics: sig.generics.clone(),
            self_lt,
            args,
            ret: sig.output.clone(),
        });
    }

    let vis = &tr.vis;
    let tr_ident = &tr.ident;
    let vt_ident = format_ident!("{}VTable", tr_ident);
    let shims_ident = format_ident!("__{}VTableShims", tr_ident);

    // --- struct fields ---
    let fields = methods.iter().map(|m| {
        let name = &m.name;
        let hr = m.hr();
        let sl = &m.self_lt;
        let tys = m.args.iter().map(|a| &a.abi_ty);
        let ret = &m.ret;
        quote! {
            pub #name: #hr unsafe extern "C" fn(OpaqueSelf<#sl> #(, #tys)*) #ret
        }
    });

    // --- shims (monomorphised per implementing type) ---
    let shims = methods.iter().map(|m| {
        let name = &m.name;
        let generics = &m.generics;
        let sl = &m.self_lt;
        let ret = &m.ret;
        let params = m.args.iter().map(|a| {
            let n = &a.name;
            let t = &a.abi_ty;
            quote!(#n: #t)
        });
        // The slices are converted *by value* so the resulting reference carries the
        // original lifetime (needed when the return value borrows from them).
        let call_args = m.args.iter().map(|a| {
            let n = &a.name;
            match a.kind {
                ArgKind::Plain => quote!(#n),
                ArgKind::Slice => quote!(#n.into_slice()),
                ArgKind::SliceMut => quote!(#n.into_slice_mut()),
            }
        });
        quote! {
            unsafe extern "C" fn #name #generics (this: OpaqueSelf<#sl> #(, #params)*) #ret {
                let this = unsafe { this.into_ref::<T>() };
                <T as #tr_ident>::#name(this #(, #call_args)*)
            }
        }
    });

    let field_inits = methods.iter().map(|m| {
        let name = &m.name;
        quote!(#name: #shims_ident::<T>::#name)
    });

    // FIXME: Below clone and drop as generated by AI boxes and leaks the value because otherwise we cannot return an OpaqueSelf
    Ok(quote! {
        #tr

        #[repr(C)]
        #[derive(Clone, Copy)]
        #vis struct #vt_ident {
            #(#fields,)*
            /// Clone the erased value into a fresh heap allocation and return an
            /// owning erased pointer to it. Release it with `drop`.
            pub clone: unsafe extern "C" fn(OpaqueSelf<'_>) -> OpaqueSelf<'static>,
            /// Drop and deallocate a pointer previously returned by `clone`.
            pub drop: unsafe extern "C" fn(OpaqueSelf<'_>),
        }

        #[doc(hidden)]
        struct #shims_ident<T>(::core::marker::PhantomData<T>);

        #[allow(clippy::missing_safety_doc)]
        impl<T: #tr_ident + 'static> #shims_ident<T> {
            #(#shims)*
        }

        impl #vt_ident {
            /// Build the vtable for a concrete implementor, using the given `drop` and
            /// `clone` functions.
            pub const fn new_with<T: #tr_ident + 'static>(
                drop: unsafe extern "C" fn(OpaqueSelf<'_>),
                clone: unsafe extern "C" fn(OpaqueSelf<'_>) -> OpaqueSelf<'static>,
            ) -> Self {
                Self {
                    #(#field_inits,)*
                    clone,
                    drop,
                }
            }
        }
    })
}

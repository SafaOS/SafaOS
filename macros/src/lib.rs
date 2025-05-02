use core::panic;

use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, DeriveInput, ImplItem, ItemImpl};

/// used by the kernel [keyboard driver](file://kernel/src/drivers/keyboard.rs)
/// impl EncodeKey for key set enum
/// each `Self` variant will encode as a `KeyCode` variant with the same name
// TODO: replace IntEnum maybe add a `try_from` function in EncodeKey trait?
#[proc_macro_derive(EncodeKey)]
pub fn derive_encode_key(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as DeriveInput);
    let name = item.ident;

    let data = match item.data {
        syn::Data::Enum(data) => data,
        _ => panic!("expected an enum"),
    };

    let arms: Vec<_> = data
        .variants
        .iter()
        .map(|variant| {
            let ident = &variant.ident;
            quote! { Self::#ident => KeyCode::#ident, }
        })
        .collect();

    TokenStream::from(quote! {
        impl EncodeKey for #name {
            fn encode(self) -> KeyCode {
                match self {
                    #(#arms)*
                }
            }
        }
    })
}

/// Impl Display and Debug for `Self` based on an impl block, put on an impl block that contains the consts you want to display `Self` as
/// example:
/// ```rust
/// #[derive(Clone, Copy, PartialEq, Eq)]
/// pub struct ElfClass(u8);
/// #[display_consts]
/// impl ElfClass {
///    pub const ELF32: Self = Self(1);
///    pub const ELF64: Self = Self(2);
/// }
/// ```
/// `Self(1)` will display as `ElfClass::ELF32` in both debug and normal display contexts
/// in case of unknown value such as `Self(3)` it will display as `ElfClass::3`
#[proc_macro_attribute]
pub fn display_consts(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let block = parse_macro_input!(item as ItemImpl);
    let ty = block.self_ty.clone().into_token_stream();
    let consts = block.items.iter().filter_map(|x| {
        if let ImplItem::Const(con) = x {
            Some(con)
        } else {
            None
        }
    });

    let arms: Vec<proc_macro2::TokenStream> = consts
        .map(|con| {
            let ident = &con.ident;
            quote! { &Self::#ident => write!(f, "{}::{}", stringify!(#ty), stringify!(#ident)), }
        })
        .collect();

    let results = quote! {
        #block

        impl core::fmt::Display for #ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    #(#arms)*
                    Self(x) => write!(f, "{}::{}", stringify!(#ty), x),
                }
            }
        }

        impl core::fmt::Debug for #ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    #(#arms)*
                    Self(x) => write!(f, "{}::{}", stringify!(#ty), x),
                }
            }
        }
    };

    results.into()
}

// it is simple, it just takes a module and takes all of its functions!

use core::panic;

use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, parse_quote, DeriveInput, ImplItem, Item, ItemImpl, ItemMod};

#[proc_macro_attribute]
pub fn test_module(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut module = parse_macro_input!(item as ItemMod);

    let mut content = module.content.take().unwrap();

    let func_names: Vec<_> = content
        .1
        .iter()
        .filter_map(|x| {
            if let Item::Fn(func) = x {
                Some(func.sig.ident.clone())
            } else {
                None
            }
        })
        .collect();
    let len = func_names.len();
    let test_main: Item = parse_quote! {
        pub fn test_main() {
            cross_println!("running {} tests...", #len);
            #(
                cross_println!("running {} test...", stringify!(#func_names));
                #func_names();
                cross_println!("[ok]");
            )*
        }
    };

    content.1.push(test_main);

    module.content = Some(content);
    TokenStream::from(quote! {#module})
}

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
                    x => write!(f, "{}::{}", stringify!(#ty), x),
                }
            }
        }

        impl core::fmt::Debug for #ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    #(#arms)*
                    x => write!(f, "{}::{}", stringify!(#ty), x),
                }
            }
        }
    };

    results.into()
}

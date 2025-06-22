use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::ItemFn;

/// Given a syscall handler argument name and type, generate the raw argument, and the conversion code.
pub fn convert_input_to_raw(
    name: &str,
    ty: &syn::Type,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    // the SyscallFFI trait defines the interface for converting between raw and high-level types.
    // SyscallFFI::Args is the type of the arguments, either a single argument or a tuple of arguments.
    // assumes every argument implements the SyscallFFI trait.
    // due to proc macro limitations, we cannot inspect the SyscallFFI so...
    let name = syn::Ident::new(name, Span::call_site());
    let raw_arg = quote! { #name: <#ty as SyscallFFI>::Args };
    let conversion_code = quote! {
        let #name: #ty = <#ty as SyscallFFI>::make(#name)?;
    };

    (raw_arg, conversion_code)
}

/// Given a function definition, generate a syscall handler function, that calls the original function.
/// a syscall handler is a function that only takes SyscallFFI::Args arguments, converts them to the appropriate types, and calls the original function.
pub fn syscall_handler(func: ItemFn) -> TokenStream {
    let inputs = func.sig.inputs.clone();

    let func_name = func.sig.ident.to_string();

    let generated_name = format!("{}_raw", func_name);
    let generated_name = syn::Ident::new(&generated_name, Span::mixed_site());
    let func_name = syn::Ident::new(&func_name, Span::mixed_site());

    let mut generated_inputs = Vec::new();
    let mut generated_body = Vec::new();
    let mut input_idents = Vec::new();

    for input in inputs {
        match input {
            syn::FnArg::Typed(pat_type) => {
                let ty = &pat_type.ty;
                let syn::Pat::Ident(ref ident) = &*pat_type.pat else {
                    panic!("Unsupported pattern type for input argument");
                };

                let ident = ident.ident.to_string();

                let (generated_input, generated_conversion) = convert_input_to_raw(&ident, ty);

                generated_inputs.push(generated_input);
                generated_body.push(generated_conversion);
                input_idents.push(syn::Ident::new(&ident, Span::call_site()));
            }
            syn::FnArg::Receiver(_) => panic!("Cannot use receiver arguments in syscall handlers"),
        }
    }

    quote! {
        #func

        pub fn #generated_name(#(#generated_inputs),*) -> Result<(), ErrorStatus> {
            #(#generated_body)*
            #func_name(#(#input_idents),*).map_err(|err| err.into())
        }
    }
    .into()
}

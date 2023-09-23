use quote::quote_spanned;
use syn::{parse_macro_input, spanned::Spanned, DeriveInput};

#[proc_macro_derive(BinarySerde)]
pub fn derive_binary_serde(input_tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_tokens as DeriveInput);
    match &input.data {
        syn::Data::Struct(data_struct) => todo!(),
        syn::Data::Enum(data_enum) => todo!(),
        syn::Data::Union(data_union) => {
            return quote_spanned! {
                data_union.union_token.span() => compile_error!("binary serialication does not support unions");
            }.into();
        }
    }
}

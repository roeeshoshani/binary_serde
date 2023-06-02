mod serialize;

use quote::{format_ident, quote, quote_spanned, ToTokens};
use serialize::do_derive_binary_serialize;
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, spanned::Spanned, AttrStyle, DataEnum,
    DataStruct, DeriveInput, Field, Fields, GenericParam, Generics, Meta, Token, Type, Variant,
    WhereClause,
};

#[proc_macro_derive(BinarySerialize)]
pub fn derive_binary_serialize(input_tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_tokens as DeriveInput);
    do_derive_binary_serialize(&input)
}

fn build_mock_enum_with_empty_variants(
    mock_enum_name_ident: &proc_macro2::Ident,
    data_enum: &DataEnum,
    repr_type: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let empty_variants = data_enum.variants.iter().map(|variant| Variant {
        fields: Fields::Unit,
        ..variant.clone()
    });
    quote! {
        #[repr(#repr_type)]
        enum #mock_enum_name_ident {
            #(#empty_variants),*
        }
    }
}

fn build_enum_largest_variant_serialized_size_expr(
    data_enum: &DataEnum,
) -> proc_macro2::TokenStream {
    let mut cur_expr = quote! { 0 };
    for variant in &data_enum.variants {
        let cur_variant_serialized_size_expr =
            build_serialized_size_expr_for_fields(&variant.fields);
        cur_expr = quote! {
            const_max(#cur_expr, #cur_variant_serialized_size_expr)
        }
    }
    quote! {
        ({
            const fn const_max(a: usize, b: usize) -> usize {
                if a >= b { a } else { b }
            }
            #cur_expr
        })
    }
}

fn enum_get_repr_type<'a>(
    derive_input: &'a DeriveInput,
    data_enum: &DataEnum,
) -> Result<&'a proc_macro2::TokenStream, proc_macro2::TokenStream> {
    let Some(repr_type) = derive_input.attrs.iter().find_map(|attr| {
        let AttrStyle::Outer = &attr.style else {
            return None;
        };
        let Meta::List(meta_list) = &attr.meta else {
            return None
        };
        if !meta_list.path.is_ident("repr") {
            return None;
        }
        Some(&meta_list.tokens)
    }) else {
        return Err(quote_spanned! {
            data_enum.enum_token.span() => compile_error!("binary serialization for enums requires a #[repr(...)] attribute on the enum");
        })
    };

    if !is_enum_repr_type_sized_primitive_int(&repr_type) {
        return Err(quote_spanned! {
            repr_type.span() => compile_error!("binary serialization for enums requires the enum's #[repr(...)] attribute to contain an explicitly sized primitive integer type (e.g., u32)");
        });
    }

    Ok(repr_type)
}

fn is_enum_repr_type_sized_primitive_int(repr_type: &proc_macro2::TokenStream) -> bool {
    let repr_type_str = repr_type.to_string();
    repr_type_str.as_bytes()[0].is_ascii_alphabetic() && repr_type_str[1..].parse::<u32>().is_ok()
}

fn fields_to_iter<'a>(fields: &'a Fields) -> Box<dyn Iterator<Item = &'a Field> + 'a> {
    match fields {
        Fields::Named(named_fields) => Box::new(named_fields.named.iter()),
        Fields::Unnamed(unnamed_fields) => Box::new(unnamed_fields.unnamed.iter()),
        Fields::Unit => Box::new(core::iter::empty()),
    }
}

fn build_serialized_size_expr_for_fields(fields: &Fields) -> proc_macro2::TokenStream {
    let field_type_serialized_sizes =
        fields_to_iter(fields).map(|field| serialized_size_of_ty(&field.ty));
    quote! {
        (0 #(
            + #field_type_serialized_sizes
        )*)
    }
}

fn collect_all_sub_types<'a>(
    derive_input: &'a DeriveInput,
) -> Box<dyn Iterator<Item = &'a Type> + 'a> {
    match &derive_input.data {
        syn::Data::Struct(data_struct) => {
            Box::new(fields_to_iter(&data_struct.fields).map(|field| &field.ty))
        }
        syn::Data::Enum(data_enum) => Box::new(
            data_enum
                .variants
                .iter()
                .map(|variant| fields_to_iter(&variant.fields).map(|field| &field.ty))
                .flatten(),
        ),
        syn::Data::Union(_) => unreachable!(),
    }
}

fn serialized_size_of_ty<T: ToTokens>(ty: &T) -> proc_macro2::TokenStream {
    quote! {
        <#ty as ::binary_serde::BinarySerialize>::SERIALIZED_SIZE
    }
}

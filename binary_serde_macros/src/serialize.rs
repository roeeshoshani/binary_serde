use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, spanned::Spanned, AttrStyle, DataEnum,
    DataStruct, DeriveInput, Field, Fields, GenericParam, Generics, Meta, Token, Type, Variant,
    WhereClause,
};

use crate::{
    build_enum_largest_variant_serialized_size_expr, build_mock_enum_with_empty_variants,
    build_serialized_size_expr_for_fields, collect_all_sub_types, enum_get_repr_type,
    fields_to_iter, serialized_size_of_ty,
};

pub fn do_derive_binary_serialize(input: &DeriveInput) -> proc_macro::TokenStream {
    match &input.data {
        syn::Data::Struct(data_struct) => derive_binary_serialize_for_struct(&input, data_struct),
        syn::Data::Enum(data_enum) => derive_binary_serialize_for_enum(&input, data_enum),
        syn::Data::Union(data_union) => {
            return quote_spanned! {
                data_union.union_token.span() => compile_error!("binary serialication does not support unions");
            }.into();
        }
    }
}

fn derive_binary_serialize_for_enum(
    derive_input: &DeriveInput,
    data_enum: &DataEnum,
) -> proc_macro::TokenStream {
    let repr_type = match enum_get_repr_type(derive_input, data_enum) {
        Ok(repr_type) => repr_type,
        Err(err) => return err.into(),
    };
    let largest_variant_serialized_size_expr =
        build_enum_largest_variant_serialized_size_expr(data_enum);
    let tag_serialized_size_expr = serialized_size_of_ty(repr_type);
    let serialized_size_expr =
        quote! { (#tag_serialized_size_expr + #largest_variant_serialized_size_expr) };

    let mock_enum_name_ident = format_ident!("Empty{}", derive_input.ident);
    let mock_enum =
        build_mock_enum_with_empty_variants(&mock_enum_name_ident, data_enum, repr_type);
    let variant_branches = data_enum.variants.iter().map(|variant| {
        let variant_ident = &variant.ident;
        let fields_destructure = match &variant.fields {
            Fields::Named(named_fields) => {
                let field_idents = named_fields
                    .named
                    .iter()
                    .map(|field| field.ident.as_ref().unwrap());
                quote! {
                    {
                        #(#field_idents),*
                    }
                }
            }
            Fields::Unnamed(unnamed_fields) => {
                let field_idents = (0..unnamed_fields.unnamed.len()).map(|i| format_ident!("field{}", i));
                quote! {
                    (
                        #(#field_idents),*
                    )
                }
            }
            Fields::Unit => quote! {},
        };

        let initial_index_in_buf = tag_serialized_size_expr.clone();
        let field_serializations = fields_to_iter(&variant.fields).enumerate().scan(initial_index_in_buf, |cur_index_in_buf, (field_index, field)| {
            let field_var_name = match &field.ident {
                Some(ident) => ident.to_token_stream(),
                None => format_ident!("field{}", field_index).to_token_stream(),
            };
            let field_serialized_size = serialized_size_of_ty(&field.ty);
            let end_index_in_buf = quote! { #cur_index_in_buf + #field_serialized_size };
            let field_serialization_call = quote! {
                ::binary_serde::BinarySerialize::binary_serialize(#field_var_name, &mut buf[#cur_index_in_buf .. #end_index_in_buf], endianness);
            };

            *cur_index_in_buf = end_index_in_buf;

            Some(field_serialization_call)
        });

        let variant_fields_serialized_size = build_serialized_size_expr_for_fields(&variant.fields);
        let cur_variant_serialized_size = quote! { (#tag_serialized_size_expr + #variant_fields_serialized_size) };
        let fill_rest_of_buffer = quote! {
            if #cur_variant_serialized_size < #serialized_size_expr {
                buf[#cur_variant_serialized_size..#serialized_size_expr].fill(0);
            }
        };

        quote! {
            Self::#variant_ident #fields_destructure => {
                let tag_value = #mock_enum_name_ident::#variant_ident as #repr_type;
                ::binary_serde::BinarySerialize::binary_serialize(&tag_value, &mut buf[..#tag_serialized_size_expr], endianness);
                #(
                    #field_serializations
                )*
                #fill_rest_of_buffer
            }
        }
    });

    let serialize_fn_body = quote! {
        #mock_enum
        match self {
            #(#variant_branches),*
        }
    };

    build_binary_serialize_impl(derive_input, serialized_size_expr, serialize_fn_body).into()
}

fn derive_binary_serialize_for_struct(
    derive_input: &DeriveInput,
    data_struct: &DataStruct,
) -> proc_macro::TokenStream {
    build_binary_serialize_impl_with_fields(derive_input, &data_struct.fields).into()
}

fn build_binary_serialize_impl(
    derive_input: &DeriveInput,
    serialized_size_expr: proc_macro2::TokenStream,
    serialize_fn_body: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let ty_ident = &derive_input.ident;
    let (impl_generics, type_generics, maybe_where_clause) = derive_input.generics.split_for_impl();
    let mut where_clause = maybe_where_clause.cloned().unwrap_or_else(|| WhereClause {
        where_token: Token![where](proc_macro2::Span::call_site()),
        predicates: Punctuated::new(),
    });
    let subtypes = collect_all_sub_types(derive_input);
    for subtype in subtypes {
        where_clause.predicates.push(parse_quote! {
            #subtype: ::binary_serde::BinarySerialize
        })
    }
    quote! {
        #[automatically_derived]
        impl #impl_generics ::binary_serde::BinarySerialize for #ty_ident #type_generics #where_clause {
            const SERIALIZED_SIZE: usize = { #serialized_size_expr };

            fn binary_serialize(&self, buf: &mut [u8], endianness: ::binary_serde::Endianness) {
                #serialize_fn_body
            }
        }
    }
}

fn build_binary_serialize_impl_with_fields(
    derive_input: &DeriveInput,
    fields: &Fields,
) -> proc_macro2::TokenStream {
    let serialized_size_expr = build_serialized_size_expr_for_fields(fields);

    let field_serializations = fields_to_iter(fields).enumerate().scan(quote! { 0 }, |cur_index_in_buf, (field_index, field)| {
        let access_name = match &field.ident {
            Some(ident) => quote! { #ident },
            None => quote! { #field_index },
        };
        let field_serialized_size = serialized_size_of_ty(&field.ty);
        let end_index_in_buf = quote! { #cur_index_in_buf + #field_serialized_size};
        let field_serialization_call = quote! {
            ::binary_serde::BinarySerialize::binary_serialize(&self.#access_name, &mut buf[#cur_index_in_buf .. #end_index_in_buf], endianness);
        };

        *cur_index_in_buf = end_index_in_buf;

        Some(field_serialization_call)
    });

    let serialize_fn_body = quote! { #(#field_serializations)* };

    build_binary_serialize_impl(derive_input, serialized_size_expr, serialize_fn_body)
}

use quote::{quote, quote_spanned, ToTokens};
use syn::{parse_macro_input, spanned::Spanned, DeriveInput};

use crate::{
    gen_impl, gen_predicates_for_field_types, GenImplParams, SerializedSizeExpr, TypeExpr,
};

/// the max bit length of a single field.
const MAX_FIELD_BIT_LENGTH: usize = 32;

/// extracts the bit length of an unsigned integer type.
/// if the given string does not represent an explicitly sized unsigned integer type, returns `None`.
/// for example, for an input of `"u32"`, the value `32` will be returned.
fn extract_unsigned_int_type_bit_length(s: &str) -> Option<usize> {
    if !s.starts_with('u') {
        return None;
    }
    s[1..].parse().ok()
}

/// the arguments to the bitfield macro
struct BitfieldArguments {
    repr_type: syn::Type,
    repr_type_bit_length: usize,
    bit_order: syn::Expr,
}
impl syn::parse::Parse for BitfieldArguments {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let repr_type = input.parse::<syn::Type>()?;
        let repr_type_str = repr_type.to_token_stream().to_string();
        let repr_type_bit_length = extract_unsigned_int_type_bit_length(&repr_type_str)
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &repr_type,
                    "expected an explicitly sized unsigned integer type (e.g. `u32`)",
                )
            })?;
        let _ = input.parse::<syn::Token![,]>()?;
        let ident = input.parse::<syn::Ident>()?;
        if ident.to_string() != "order" {
            return Err(syn::Error::new_spanned(
                ident,
                "expected an \"order\" argument",
            ));
        }
        let _ = input.parse::<syn::Token![=]>()?;
        Ok(Self {
            repr_type,
            repr_type_bit_length,
            bit_order: input.parse()?,
        })
    }
}

pub fn binary_serde_bitfield(
    args_tokens: proc_macro::TokenStream,
    input_tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let args = parse_macro_input!(args_tokens as BitfieldArguments);
    let mut input = parse_macro_input!(input_tokens as DeriveInput);

    // save a copy of the original input before messing with it
    let original_input = input.clone();

    match &mut input.data {
        syn::Data::Struct(data_struct) => {
            match &mut data_struct.fields {
                syn::Fields::Named(named_fields) => {
                    let field_bit_lengths = match extract_field_bit_lengths(named_fields) {
                        Ok(v) => v,
                        Err(err) => return err.into(),
                    };
                    let total_bit_length: usize = field_bit_lengths.iter().sum();
                    if total_bit_length != args.repr_type_bit_length {
                        let error_msg = format!("total bit length of fields is {}, but the expected total bit length is {}", total_bit_length, args.repr_type_bit_length);
                        return quote_spanned!{
                            proc_macro2::Span::call_site() => compile_error!(#error_msg);
                        }.into()
                        
                    }
                    let field_types = named_fields.named.iter()
                        .map(|field| TypeExpr::from_type(&field.ty));
                    let field_idents =
                            named_fields.named.iter().map(|field| field.ident.as_ref().cloned().unwrap());
                    let repr_type_expr = TypeExpr(args.repr_type.to_token_stream());
                    let trait_impl = gen_impl(GenImplParams {
                        additional_where_predicates: gen_predicates_for_field_types(field_types.clone()).collect(),
                        serialized_size: repr_type_expr.serialized_size(),
                        recursive_array_type: repr_type_expr.serialized_recursive_array_type(),
                        serialization_code: gen_bitfield_serialization_code(
                            &field_bit_lengths,
                            field_idents.clone(),
                            &args.bit_order
                        ),
                        deserialization_code: gen_bitfield_deserialization_code(&field_bit_lengths, field_idents, field_types, &args.bit_order),
                        type_ident: original_input.ident,
                        generics: original_input.generics,
                    });
                    quote! {
                        #input
                        #trait_impl
                    }.into()
                },
                syn::Fields::Unnamed(_) => quote_spanned! {
                    proc_macro2::Span::call_site() => compile_error!("bitfields can not have unnamed fields, only named fields are supported");
                }.into(),
                syn::Fields::Unit => quote_spanned! {
                    proc_macro2::Span::call_site() => compile_error!("bitfield structs must not be empty");
                }.into(),
            }
        },
        syn::Data::Enum(_) => quote_spanned! {
            proc_macro2::Span::call_site() => compile_error!("bitfields can not be enums, only structs can be bitfields");
        }.into(),
        syn::Data::Union(_) => quote_spanned! {
            proc_macro2::Span::call_site() => compile_error!("bitfields can not be unions, only structs can be bitfields");
        }.into(),
    }
}

/// extracts the bit length of the given fields according to the `#[bits(...)]` attribute on each field, and removes that attribute
/// from each of the fields.
fn extract_field_bit_lengths(
    fields: &mut syn::FieldsNamed,
) -> Result<Vec<usize>, proc_macro2::TokenStream> {
    let mut bit_lengths = Vec::with_capacity(fields.named.len());
    let mut bit_length_attr_indexes = Vec::with_capacity(fields.named.len());
    for field in &mut fields.named {
        let (bit_length_attr_index, bit_length_attr_value) = field
            .attrs
            .iter()
            .enumerate()
            .find_map(|(attr_index, attr)| {
                let syn::Attribute {
                    pound_token: _,
                    style: syn::AttrStyle::Outer,
                    bracket_token: _,
                    meta:
                        syn::Meta::List(syn::MetaList {
                            path,
                            delimiter: _,
                            tokens: attr_value,
                        }),
                } = attr
                else {
                    return None;
                };
                if path.segments.len() != 1 {
                    return None;
                }
                let path_segment = &path.segments[0];
                if !matches!(path_segment.arguments, syn::PathArguments::None) {
                    return None;
                }
                let path_segment_ident = &path_segment.ident;
                if path_segment_ident.to_string() != "bits" {
                    return None;
                }
                Some((attr_index, attr_value))
            })
            .ok_or_else(|| {
                quote_spanned! {
                    field.span() => compile_error!("missing #[bits(...)] attribute on field");
                }
            })?;
        let bit_length = bit_length_attr_value.to_string().parse().map_err(|_| {
            quote_spanned! {
                bit_length_attr_value.span() => compile_error!("expected an unsigned integer for the bit length of a field");
            }
        })?;
        if bit_length > MAX_FIELD_BIT_LENGTH {
            let error_msg = format!(
                "the maximum length of a bit field is {}",
                MAX_FIELD_BIT_LENGTH
            );
            return Err(quote_spanned! {
                bit_length_attr_value.span() => compile_error!(#error_msg);
            });
        }
        bit_lengths.push(bit_length);
        bit_length_attr_indexes.push(bit_length_attr_index);
    }
    for (field, bit_length_attr_index) in fields.named.iter_mut().zip(bit_length_attr_indexes) {
        field.attrs.remove(bit_length_attr_index);
    }
    Ok(bit_lengths)
}

/// generates code for serializing a bitfield struct.
fn gen_bitfield_serialization_code(
    field_bit_lengths: &[usize],
    field_idents: impl Iterator<Item = syn::Ident>,
    bit_order: &syn::Expr,
    repr_type: &syn::Type,
) -> proc_macro2::TokenStream {
    let x = quote! {
        let mut result: #repr_type = 0;
    };
    let field_serializations: Vec<proc_macro2::TokenStream> = field_idents
        .zip(field_bit_lengths)
        .map(|(field_ident, bit_length)| {
            quote! {
                {
                    let serialized = ::binary_serde::BinarySerde::binary_serialize_to_array(
                        &self.#field_ident,
                        endianness
                    );
                    let mut reader = ::binary_serde::LsbBitReader::new(
                        ::binary_serde::recursive_array::RecursiveArray::as_slice(&serialized),
                        endianness,
                    );
                    ::binary_serde::_copy_bits(
                        &mut reader, &mut writer, #bit_length
                    );
                }
            }
        })
        .collect();
    let field_serializations_reversed = {
        let mut reversed = field_serializations.clone();
        reversed.reverse();
        reversed
    };
    quote! {
        let mut writer = ::binary_serde::LsbBitWriter::new(
            buf,
            endianness,
        );
        let bit_order: ::binary_serde::BitfieldBitOrder = #bit_order;
        match bit_order {
            ::binary_serde::BitfieldBitOrder::LsbFirst => {
                #(#field_serializations)*
            },
            ::binary_serde::BitfieldBitOrder::MsbFirst => {
                #(#field_serializations_reversed)*
            },
        }
    }
}

/// generates code for deserializing a bitfield struct.
fn gen_bitfield_deserialization_code(
    field_bit_lengths: &[usize],
    field_idents: impl Iterator<Item = syn::Ident>,
    field_types: impl Iterator<Item = TypeExpr>,
    bit_order: &syn::Expr,
) -> proc_macro2::TokenStream {
    let field_initializers: Vec<proc_macro2::TokenStream> = field_idents
        .zip(field_types)
        .zip(field_bit_lengths)
        .map(|((field_ident, field_type), bit_length)| {
            let recursive_array_type = field_type.serialized_recursive_array_type();
            quote! {
                #field_ident: {
                    let mut array: #recursive_array_type = unsafe { core::mem::zeroed() };
                    let mut writer = ::binary_serde::LsbBitWriter::new(
                        ::binary_serde::recursive_array::RecursiveArray::as_mut_slice(&mut array),
                        endianness,
                    );
                    ::binary_serde::_copy_bits(
                        &mut reader,
                        &mut writer,
                        #bit_length
                    );
                    <#field_type as ::binary_serde::BinarySerde>::binary_deserialize(
                        ::binary_serde::recursive_array::RecursiveArray::as_slice(&array),
                        endianness
                    )?
                }
            }
        })
        .collect();
    let field_initializers_reversed = {
        let mut reversed = field_initializers.clone();
        reversed.reverse();
        reversed
    };
    quote! {
        let mut reader = ::binary_serde::LsbBitReader::new(
            buf,
            endianness,
        );
        let bit_order: ::binary_serde::BitfieldBitOrder = #bit_order;
        match bit_order {
            ::binary_serde::BitfieldBitOrder::LsbFirst => {
                Ok(Self {
                    #(#field_initializers,)*
                })
            },
            ::binary_serde::BitfieldBitOrder::MsbFirst => {
                Ok(Self {
                    #(#field_initializers_reversed,)*
                })
            },
        }
    }
}

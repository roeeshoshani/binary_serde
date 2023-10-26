use quote::{quote, quote_spanned};
use syn::{parse_macro_input, DeriveInput};

use crate::{
    gen_impl, gen_predicates_for_field_types, FieldOffsetExpr, GenImplParams, SerializedSizeExpr,
    TypeExpr,
};

pub fn derive_binary_serde(input_tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_tokens as DeriveInput);
    match &input.data {
        syn::Data::Struct(data_struct) => {
            let field_types = data_struct
                .fields
                .iter()
                .map(|field| TypeExpr::from_type(&field.ty));

            gen_impl(GenImplParams {
                type_ident: input.ident,
                generics: input.generics,
                additional_where_predicates: gen_predicates_for_field_types(field_types.clone())
                    .collect(),
                serialized_size: field_types
                    .clone()
                    .map(|field_type| field_type.serialized_size())
                    .sum(),
                recursive_array_type: gen_recursive_array_type_for_field_types(field_types),
                serialization_code: gen_struct_serialization_code(&data_struct.fields),
                deserialization_code: gen_struct_deserialization_code(&data_struct.fields),
            })
            .into()
        }
        syn::Data::Enum(data_enum) => {
            // make sure that none of the enum's variants hold data in them
            for variant in &data_enum.variants {
                if !matches!(&variant.fields, syn::Fields::Unit) {
                    return quote_spanned! {
                        proc_macro2::Span::call_site() => compile_error!("enum variants which contain data are not supported");
                    }.into();
                }
            }
            let repr_type = match enum_get_repr_type(&input) {
                Ok(repr_type) => TypeExpr(repr_type.clone()),
                Err(err) => return err.into(),
            };
            gen_impl(GenImplParams {
                additional_where_predicates: Vec::new(),
                serialized_size: repr_type.serialized_size(),
                recursive_array_type: repr_type.serialized_recursive_array_type(),
                serialization_code: quote! {
                    let as_primitive: &#repr_type = unsafe { ::core::mem::transmute(self) };
                    ::binary_serde::BinarySerde::binary_serialize(as_primitive, buf, endianness)
                },
                deserialization_code: gen_enum_deserialization_code(
                    &input.ident,
                    repr_type,
                    data_enum,
                ),
                type_ident: input.ident,
                generics: input.generics,
            })
            .into()
        }
        syn::Data::Union(_) => {
            return quote_spanned! {
                proc_macro2::Span::call_site() => compile_error!("unions are not supported, only structs are supported");
            }
            .into();
        }
    }
}

/// attempts to extract the enum's underlying representation type.
fn enum_get_repr_type<'a>(
    derive_input: &'a DeriveInput,
) -> Result<&'a proc_macro2::TokenStream, proc_macro2::TokenStream> {
    let Some(repr_type) = derive_input.attrs.iter().find_map(|attr| {
        let syn::AttrStyle::Outer = &attr.style else {
            return None;
        };
        let syn::Meta::List(meta_list) = &attr.meta else {
            return None;
        };
        if !meta_list.path.is_ident("repr") {
            return None;
        }
        Some(&meta_list.tokens)
    }) else {
        return Err(quote_spanned! {
            proc_macro2::Span::call_site() => compile_error!("a #[repr(...)] attribute is required on the enum to specify the size of the enum's tag");
        });
    };

    if !is_enum_repr_type_sized_primitive_int(&repr_type) {
        return Err(quote_spanned! {
            proc_macro2::Span::call_site() => compile_error!("the enum's #[repr(...)] attribute must contain an explicitly sized primitive integer type (e.g., u32)");
        });
    }

    Ok(repr_type)
}

/// checks if the provided expression inside a `#[repr(...)]` attribute on an enum represents an explicitly sized primitive int.
fn is_enum_repr_type_sized_primitive_int(repr_type: &proc_macro2::TokenStream) -> bool {
    let repr_type_str = repr_type.to_string();
    repr_type_str.as_bytes()[0].is_ascii_alphabetic() && repr_type_str[1..].parse::<u32>().is_ok()
}

/// returns an iterator over the offset and size of each field.
fn fields_offsets_and_sizes<'a, I: Iterator<Item = &'a syn::Field> + 'a>(
    fields: I,
) -> impl Iterator<Item = FieldOffsetAndSize> + 'a {
    fields.scan(SerializedSizeExpr::zero(), |prev_fields_size, cur_field| {
        let cur_field_size = TypeExpr::from_type(&cur_field.ty).serialized_size();
        let new_size = &*prev_fields_size + &cur_field_size;

        // the offset of this field is the size of all previous fields, and update the prev size to the new size.
        let offset = core::mem::replace(prev_fields_size, new_size);

        Some(FieldOffsetAndSize {
            size: cur_field_size,
            offset: FieldOffsetExpr(offset.0),
        })
    })
}

/// information about the offset and size of a field.
struct FieldOffsetAndSize {
    size: SerializedSizeExpr,
    offset: FieldOffsetExpr,
}

/// generates accessors for the given fields. each accessor can be prepended with `self.` to access the value of the field.
fn field_accessors<'a, I: Iterator<Item = &'a syn::Field> + 'a>(
    fields: I,
) -> impl Iterator<Item = syn::Member> + 'a {
    fields
        .enumerate()
        .map(|(field_index, field)| match &field.ident {
            Some(ident) => syn::Member::Named(ident.clone()),
            None => syn::Member::Unnamed(syn::Index::from(field_index)),
        })
}

/// generates code for serializing a struct with the given fields.
fn gen_struct_serialization_code(fields: &syn::Fields) -> proc_macro2::TokenStream {
    let statements = field_accessors(fields.iter())
        .zip(fields_offsets_and_sizes(fields.iter()))
        .map(|(field_accessor, offset_and_size)| {
            let FieldOffsetAndSize { size, offset } = offset_and_size;
            quote! {
                ::binary_serde::BinarySerde::binary_serialize(
                    &self.#field_accessor,
                    &mut buf[#offset..][..#size],
                    endianness
                );
            }
        });
    quote! {
        #(#statements)*
    }
}

/// generates code for deserializing a struct with the given fields.
fn gen_enum_deserialization_code(
    enum_ident: &syn::Ident,
    enum_repr_type: TypeExpr,
    data_enum: &syn::DataEnum,
) -> proc_macro2::TokenStream {
    let variant_consts_idents = data_enum.variants.iter().map(|variant| {
        let const_name = format!(
            "_BINARY_SERDE_CONST_{}",
            pascal_to_capital_snake_case(&variant.ident.to_string())
        );
        syn::Ident::new(&const_name, proc_macro2::Span::mixed_site())
    });
    let variant_consts_definitions = data_enum
        .variants
        .iter()
        .zip(variant_consts_idents.clone())
        .map(|(variant, const_ident)| {
            let variant_ident = &variant.ident;
            quote! {
                #[allow(non_upper_case_globals)]
                const #const_ident: #enum_repr_type = (#enum_ident::#variant_ident) as #enum_repr_type;
            }
        });
    let match_cases =
        data_enum
            .variants
            .iter()
            .zip(variant_consts_idents)
            .map(|(variant, const_ident)| {
                let variant_ident = &variant.ident;
                quote! {
                    #const_ident => Ok(#enum_ident::#variant_ident)
                }
            });
    let enum_name = enum_ident.to_string();
    quote! {
        #(#variant_consts_definitions)*
        let primitive_value = <#enum_repr_type as ::binary_serde::BinarySerde>::binary_deserialize(buf, endianness)?;
        match primitive_value {
            #(#match_cases,)*
            _ => ::core::result::Result::Err(::binary_serde::DeserializeError::InvalidEnumValue {
                enum_name: #enum_name,
            })
        }
    }
}

/// convets a PascalCase string to a CAPITAL_SNAKE_CASE string.
fn pascal_to_capital_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i != 0 {
            result.push('_');
        }
        result.extend(c.to_uppercase())
    }
    result
}

/// generates code for deserializing a struct with the given fields.
fn gen_struct_deserialization_code(fields: &syn::Fields) -> proc_macro2::TokenStream {
    let field_values = fields_offsets_and_sizes(fields.iter()).map(|offset_and_size| {
        let FieldOffsetAndSize { size, offset } = offset_and_size;
        quote! {
            ::binary_serde::BinarySerde::binary_deserialize(
                &buf[#offset..][..#size],
                endianness
            )?
        }
    });
    match fields {
        syn::Fields::Named(_) => {
            let field_specifications =
                fields.iter().zip(field_values).map(|(field, field_value)| {
                    let field_ident = field.ident.as_ref().unwrap();
                    quote! {
                        #field_ident: #field_value
                    }
                });
            quote! {
                Ok(Self {
                    #(#field_specifications),*
                })
            }
        }
        syn::Fields::Unnamed(_) => {
            quote! {
                Ok(Self (
                    #(#field_values),*
                ))
            }
        }
        syn::Fields::Unit => quote! { Ok(Self) },
    }
}
/// generates a recursive array type for a struct made of the given field types.
fn gen_recursive_array_type_for_field_types<I: Iterator<Item = TypeExpr>>(
    field_types: I,
) -> TypeExpr {
    let empty_array = quote! {::binary_serde::recursive_array::EmptyRecursiveArray};
    let final_array_expr = field_types.fold(empty_array, |cur_array_type, cur_field_type| {
        let cur_field_recursive_array_type = cur_field_type.serialized_recursive_array_type();
        quote! {
            ::binary_serde::recursive_array::RecursiveArrayConcatenation<u8, #cur_array_type, #cur_field_recursive_array_type>
        }
    });
    TypeExpr(final_array_expr)
}

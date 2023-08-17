use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, spanned::Spanned, AttrStyle, DataEnum,
    DataStruct, DeriveInput, Field, Fields, Meta, Token, Type, Variant, WhereClause,
};

#[proc_macro_derive(BinarySerde)]
pub fn derive_binary_serde(input_tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_tokens as DeriveInput);
    match &input.data {
        syn::Data::Struct(data_struct) => derive_binary_serde_for_struct(&input, data_struct),
        syn::Data::Enum(data_enum) => derive_binary_serde_for_enum(&input, data_enum),
        syn::Data::Union(data_union) => {
            return quote_spanned! {
                data_union.union_token.span() => compile_error!("binary serialication does not support unions");
            }.into();
        }
    }
}

/// derives the core trait for an enum
fn derive_binary_serde_for_enum(
    derive_input: &DeriveInput,
    data_enum: &DataEnum,
) -> proc_macro::TokenStream {
    let repr_type = match enum_get_repr_type(derive_input, data_enum) {
        Ok(repr_type) => repr_type,
        Err(err) => return err.into(),
    };
    let largest_variant_serialized_size_expr =
        build_enum_largest_variant_serialized_size_expr(data_enum);
    let tag_serialized_size_expr = max_serialized_size_of_ty(repr_type);
    let max_serialized_size_expr =
        quote! { (#tag_serialized_size_expr + #largest_variant_serialized_size_expr) };

    let enum_name_str = derive_input.ident.to_string();
    let mock_enum_name_ident = format_ident!("Empty{}", derive_input.ident);
    let mock_enum =
        build_mock_enum_with_empty_variants(&mock_enum_name_ident, data_enum, repr_type);
    let serialize_fn_body = {
        let serialization_variant_branches = data_enum.variants.iter().map(|variant| {
            let variant_ident = &variant.ident;
            let fields_destructure = build_enum_variant_fields_destructure(variant);
            let initial_index_in_buf = tag_serialized_size_expr.clone();
            let field_serializations = variant.fields.iter().enumerate().scan(
                initial_index_in_buf,
                |cur_index_in_buf, (field_index, field)| {
                    let field_var_name = get_enum_destructured_field_name(field, field_index);
                    let field_max_serialized_size = max_serialized_size_of_ty(&field.ty);
                    let end_index_in_buf =
                        quote! { #cur_index_in_buf + #field_max_serialized_size };
                    let field_serialization_call = quote! {
                        ::binary_serde::BinarySerdeInternal::binary_serialize_internal(
                            #field_var_name,
                            &mut buf[#cur_index_in_buf .. #end_index_in_buf],
                            endianness
                        );
                    };

                    *cur_index_in_buf = end_index_in_buf;

                    Some(field_serialization_call)
                },
            );

            let variant_fields_max_serialized_size =
                build_max_serialized_size_expr_for_fields(&variant.fields);
            let cur_variant_max_serialized_size =
                quote! { (#tag_serialized_size_expr + #variant_fields_max_serialized_size) };
            let fill_rest_of_buffer = quote! {
                if #cur_variant_max_serialized_size < #max_serialized_size_expr {
                    buf[#cur_variant_max_serialized_size..#max_serialized_size_expr].fill(0);
                }
            };

            quote! {
                Self::#variant_ident #fields_destructure => {
                    let tag_value = #mock_enum_name_ident::#variant_ident as #repr_type;
                    ::binary_serde::BinarySerdeInternal::binary_serialize_internal(
                        &tag_value,
                        &mut buf[..#tag_serialized_size_expr], endianness
                    );
                    #(
                        #field_serializations
                    )*
                    #fill_rest_of_buffer
                }
            }
        });
        quote! {
            #mock_enum
            match self {
                #(#serialization_variant_branches,)*
            }
        }
    };

    let serialize_min_fn_body = {
        let serialization_variant_branches = data_enum.variants.iter().map(|variant| {
            let variant_ident = &variant.ident;
            let fields_destructure = build_enum_variant_fields_destructure(variant);
            let field_serializations = variant.fields.iter().enumerate().map(|(field_index, field)| {
                let field_var_name = get_enum_destructured_field_name(field, field_index);
                let field_max_serialized_size = max_serialized_size_of_ty(&field.ty);
                let field_serialization_call = quote! {
                    cur_index_in_buf += ::binary_serde::BinarySerdeInternal::binary_serialize_min_internal(
                        #field_var_name,
                        &mut buf[cur_index_in_buf .. cur_index_in_buf + #field_max_serialized_size],
                        endianness
                    );
                };

                Some(field_serialization_call)
            });

            quote! {
                Self::#variant_ident #fields_destructure => {
                    let tag_value = #mock_enum_name_ident::#variant_ident as #repr_type;
                    let mut cur_index_in_buf = 0;
                    cur_index_in_buf += ::binary_serde::BinarySerdeInternal::binary_serialize_min_internal(
                        &tag_value,
                        &mut buf[..#tag_serialized_size_expr], endianness
                    );
                    #(
                        #field_serializations
                    )*
                    cur_index_in_buf
                }
            }
        });
        quote! {
            #mock_enum
            match self {
                #(#serialization_variant_branches,)*
            }
        }
    };

    let mock_enum_const_variants_iter = data_enum.variants.iter().map(|variant| {
        let const_ident = format_ident!("{}{}", mock_enum_name_ident, variant.ident);
        let variant_ident = &variant.ident;
        quote! {
            #[allow(non_upper_case_globals)]
            const #const_ident: #repr_type = #mock_enum_name_ident::#variant_ident as #repr_type;
        }
    });
    let mock_enum_const_variants = quote! { #(#mock_enum_const_variants_iter)* };
    let deserialize_fn_body = {
        let deserialization_variant_branches = data_enum.variants.iter().map(|variant| {
            let variant_ident = &variant.ident;
            let mock_enum_const_variant_ident =
                format_ident!("{}{}", mock_enum_name_ident, variant_ident);
            let fields_initializer = build_deserialization_fields_initializer(
                &variant.fields,
                tag_serialized_size_expr.clone(),
            );
            quote! {
                #[allow(non_upper_case_globals)]
                #mock_enum_const_variant_ident => {
                    ::core::result::Result::Ok(Self::#variant_ident #fields_initializer)
                }
            }
        });
        quote! {
            #mock_enum
            #mock_enum_const_variants
            let tag = <#repr_type as ::binary_serde::BinarySerdeInternal>::binary_deserialize_internal(
                &buf[..#tag_serialized_size_expr],
                endianness,
                index_in_buf,
            )?;
            match tag {
                #(#deserialization_variant_branches,)*
                _ => {
                    ::core::result::Result::Err(::binary_serde::DeserializeError {
                        kind: ::binary_serde::DeserializeErrorKind::InvalidEnumTag { enum_name: #enum_name_str },
                        span: ::binary_serde::DeserializeErrorSpan {
                            start: index_in_buf,
                            end: index_in_buf + #tag_serialized_size_expr,
                        }
                    })
                }
            }
        }
    };

    let deserialize_min_fn_body = {
        let deserialization_variant_branches = data_enum.variants.iter().map(|variant| {
            let variant_ident = &variant.ident;
            let mock_enum_const_variant_ident =
                format_ident!("{}{}", mock_enum_name_ident, variant_ident);
            let fields_initializer = build_deserialization_min_fields_initializer(&variant.fields);
            quote! {
                #[allow(non_upper_case_globals)]
                #mock_enum_const_variant_ident => {
                    let res = Self::#variant_ident #fields_initializer;
                    ::core::result::Result::Ok((res, cur_index_in_buf))
                }
            }
        });
        quote! {
            #mock_enum
            #mock_enum_const_variants
            let mut cur_index_in_buf = 0;
            let (tag, tag_size) = <#repr_type as ::binary_serde::BinarySerdeInternal>::binary_deserialize_min_internal(
                &buf[..#tag_serialized_size_expr],
                endianness,
                index_in_buf,
            )?;
            cur_index_in_buf += tag_size;
            match tag {
                #(#deserialization_variant_branches,)*
                _ => {
                    ::core::result::Result::Err(::binary_serde::DeserializeError {
                        kind: ::binary_serde::DeserializeErrorKind::InvalidEnumTag { enum_name: #enum_name_str },
                        span: ::binary_serde::DeserializeErrorSpan {
                            start: index_in_buf,
                            end: index_in_buf + #tag_serialized_size_expr,
                        }
                    })
                }
            }
        }
    };

    let serialized_size_fn_body = {
        let serialization_variant_branches = data_enum.variants.iter().map(|variant| {
            let variant_ident = &variant.ident;
            let fields_destructure = build_enum_variant_fields_destructure(variant);
            let fields_size_expr = build_variant_fields_binary_serialized_size(&variant.fields);
            quote! {
                Self::#variant_ident #fields_destructure => {
                    let tag_value = #mock_enum_name_ident::#variant_ident as #repr_type;
                    ::binary_serde::BinarySerdeInternal::binary_serialized_size_internal(&tag_value) + #fields_size_expr
                }
            }
        });
        quote! {
            #mock_enum
            match self {
                #(#serialization_variant_branches,)*
            }
        }
    };

    build_binary_serde_impl(
        derive_input,
        max_serialized_size_expr,
        serialize_fn_body,
        serialize_min_fn_body,
        serialized_size_fn_body,
        deserialize_fn_body,
        deserialize_min_fn_body,
    )
    .into()
}

/// get the name of a destructured enum variant field that was destructured using [`build_enum_variant_fields_destructure`].
fn get_enum_destructured_field_name(field: &Field, field_index: usize) -> proc_macro2::TokenStream {
    match &field.ident {
        Some(ident) => ident.to_token_stream(),
        None => format_ident!("field{}", field_index).to_token_stream(),
    }
}

/// builds a destructuring of the fields of an enum variant.
///
/// to get the name of a destructured field, use the [`get_enum_destructured_field_name`] function.
///
/// # Example
///
/// for example, for the following enum:
/// ```
/// enum Animal {
///     Person { age: i32, name: String },
///     Dog(i32, u32),
/// }
/// ```
/// for the variant `Animal::Person`, this function would return:
/// ```
/// { age, name }
/// ```
/// and for `Animal::Dog`, this function would return:
/// ```
/// (field0, field1)
/// ```
fn build_enum_variant_fields_destructure(variant: &Variant) -> proc_macro2::TokenStream {
    match &variant.fields {
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
            let field_idents =
                (0..unnamed_fields.unnamed.len()).map(|i| format_ident!("field{}", i));
            quote! {
                (
                    #(#field_idents),*
                )
            }
        }
        Fields::Unit => quote! {},
    }
}

/// derives the core trait for a struct
fn derive_binary_serde_for_struct(
    derive_input: &DeriveInput,
    data_struct: &DataStruct,
) -> proc_macro::TokenStream {
    let fields = &data_struct.fields;
    let max_serialized_size_expr = build_max_serialized_size_expr_for_fields(fields);

    let serialize_fn_body = {
        let field_serializations = fields.iter().enumerate().scan(quote! { 0 }, |cur_index_in_buf, (field_index, field)| {
                let access_name = build_field_self_access_name(field, field_index);
                let field_max_serialized_size = max_serialized_size_of_ty(&field.ty);
                let end_index_in_buf = quote! { #cur_index_in_buf + #field_max_serialized_size};
                let field_serialization_call = quote! {
                    ::binary_serde::BinarySerdeInternal::binary_serialize_internal(&self.#access_name, &mut buf[#cur_index_in_buf .. #end_index_in_buf], endianness);
                };

                *cur_index_in_buf = end_index_in_buf;

                Some(field_serialization_call)
            });
        quote! { #(#field_serializations)* }
    };

    let serialize_min_fn_body = {
        let field_serializations = fields.iter().enumerate().map(|(field_index, field)| {
                let access_name = build_field_self_access_name(field, field_index);
                let field_max_serialized_size = max_serialized_size_of_ty(&field.ty);
                let field_serialization_call = quote! {
                    cur_index_in_buf += ::binary_serde::BinarySerdeInternal::binary_serialize_min_internal(
                        &self.#access_name,
                        &mut buf[cur_index_in_buf .. cur_index_in_buf + #field_max_serialized_size],
                        endianness
                    );
                };

                Some(field_serialization_call)
            });
        quote! {
            let mut cur_index_in_buf = 0;
            #(#field_serializations)*
            cur_index_in_buf
        }
    };

    let deserialize_fn_body = {
        let fields_initializer = build_deserialization_fields_initializer(fields, quote! { 0 });
        quote! {
            ::core::result::Result::Ok(Self #fields_initializer)
        }
    };

    let deserialize_min_fn_body = {
        let fields_initializer = build_deserialization_min_fields_initializer(fields);
        quote! {
            let mut cur_index_in_buf = 0;
            let res = Self #fields_initializer;
            ::core::result::Result::Ok((res, cur_index_in_buf))
        }
    };

    let serialized_size_fn_body = build_struct_fields_binary_serialized_size(fields);

    build_binary_serde_impl(
        derive_input,
        max_serialized_size_expr,
        serialize_fn_body,
        serialize_min_fn_body,
        serialized_size_fn_body,
        deserialize_fn_body,
        deserialize_min_fn_body,
    )
    .into()
}

/// builds an implementation of the core trait provided the required items.
fn build_binary_serde_impl(
    derive_input: &DeriveInput,
    max_serialized_size_expr: proc_macro2::TokenStream,
    serialize_fn_body: proc_macro2::TokenStream,
    serialize_min_fn_body: proc_macro2::TokenStream,
    serialized_size_fn_body: proc_macro2::TokenStream,
    deserialize_fn_body: proc_macro2::TokenStream,
    deserialize_min_fn_body: proc_macro2::TokenStream,
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
            #subtype: ::binary_serde::BinarySerde
        })
    }
    quote! {
        #[automatically_derived]
        impl #impl_generics ::binary_serde::BinarySerdeInternal for #ty_ident #type_generics #where_clause {
            const MAX_SERIALIZED_SIZE_INTERNAL: usize = { #max_serialized_size_expr };

            fn binary_serialize_internal(&self, buf: &mut [u8], endianness: ::binary_serde::Endianness) {
                #serialize_fn_body
            }

            fn binary_serialize_min_internal(&self, buf: &mut [u8], endianness: ::binary_serde::Endianness) -> usize {
                #serialize_min_fn_body
            }

            fn binary_serialized_size_internal(&self) -> usize {
                #serialized_size_fn_body
            }

            fn binary_deserialize_internal(buf: &[u8], endianness: ::binary_serde::Endianness, index_in_buf: usize) -> ::core::result::Result<Self, ::binary_serde::DeserializeError> {
                #deserialize_fn_body
            }

            fn binary_deserialize_min_internal(buf: &[u8], endianness: ::binary_serde::Endianness, index_in_buf: usize) -> ::core::result::Result<(Self, usize), ::binary_serde::DeserializeError> {
                #deserialize_min_fn_body
            }
        }
    }
}

/// build an expression for the binary serialized size of the given fields
fn build_struct_fields_binary_serialized_size(fields: &Fields) -> proc_macro2::TokenStream {
    let field_sizes = fields.iter().enumerate().map(|(field_index, field)| {
        let access_name = build_field_self_access_name(field, field_index);
        quote! {
            ::binary_serde::BinarySerdeInternal::binary_serialized_size_internal(
                &self.#access_name
            )
        }
    });
    quote! {
        ( 0 #( + #field_sizes)* )
    }
}

/// builds an expression for the binary serialized size of the given enum variant fields.
///
/// this function assumes that the returned expression would be placed in a context where the variant's fields
/// have been destructured using [`build_enum_variant_fields_destructure`].
fn build_variant_fields_binary_serialized_size(fields: &Fields) -> proc_macro2::TokenStream {
    let field_sizes = fields.iter().enumerate().map(|(field_index, field)| {
        let field_name = get_enum_destructured_field_name(field, field_index);
        quote! {
            ::binary_serde::BinarySerdeInternal::binary_serialized_size_internal(#field_name)
        }
    });
    quote! {
        ( 0 #( + #field_sizes)* )
    }
}

/// builds an expression for accessing the provided field using a `self.` prefix.
fn build_field_self_access_name(field: &Field, field_index: usize) -> proc_macro2::TokenStream {
    match &field.ident {
        Some(ident) => quote! { #ident },
        None => syn::Index::from(field_index).to_token_stream(),
    }
}

/// builds an expression for initializing the provided fields by deserializing the input buffer into the corresponding field types.
///
/// the result of this function can be used after the name of a struct or an enum variant to generate a value of that type.
fn build_deserialization_fields_initializer(
    fields: &Fields,
    initial_index_in_buf: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let field_initializers = fields
        .iter()
        .scan(initial_index_in_buf, |cur_index_in_buf, field| {
            let initialization_name = match &field.ident {
                Some(ident) => quote! { #ident : },
                None => quote! {},
            };
            let field_max_serialized_size = max_serialized_size_of_ty(&field.ty);
            let end_index_in_buf = quote! { #cur_index_in_buf + #field_max_serialized_size};
            let field_ty = &field.ty;
            let initialization_value = quote! {
                <#field_ty as ::binary_serde::BinarySerdeInternal>::binary_deserialize_internal(
                    &buf[#cur_index_in_buf .. #end_index_in_buf],
                    endianness, index_in_buf + #cur_index_in_buf
                )?
            };

            *cur_index_in_buf = end_index_in_buf;

            Some(quote! {
                #initialization_name #initialization_value
            })
        });

    build_field_initializers_to_type_initializer(fields, field_initializers)
}

/// builds an expression for initializing the provided fields by minimally deserializing the input buffer into the corresponding field types.
///
/// the result of this function can be used after the name of a struct or an enum variant to generate a value of that type.
fn build_deserialization_min_fields_initializer(fields: &Fields) -> proc_macro2::TokenStream {
    let field_initializers =
        fields.iter().map(|field| {
            let initialization_name = match &field.ident {
                Some(ident) => quote! { #ident : },
                None => quote! {},
            };
            let field_max_serialized_size = max_serialized_size_of_ty(&field.ty);
            let field_ty = &field.ty;
            let initialization_value = quote! {
                {
                    let (res, size) = <#field_ty as ::binary_serde::BinarySerdeInternal>::binary_deserialize_min_internal(
                        &buf[cur_index_in_buf .. cur_index_in_buf + #field_max_serialized_size],
                        endianness, index_in_buf + cur_index_in_buf
                    )?;
                    cur_index_in_buf += size;
                    res
                }
            };


            Some(quote! {
                #initialization_name #initialization_value
            })
        });

    build_field_initializers_to_type_initializer(fields, field_initializers)
}

/// given a list of field initializer expressions, generates a type initializer which can be used after the name of a struct
/// or an enum variant to generate a value of that type.
fn build_field_initializers_to_type_initializer<I: ToTokens>(
    fields: &Fields,
    field_initializers: impl Iterator<Item = I>,
) -> proc_macro2::TokenStream {
    match fields {
        Fields::Named(_) => quote! {
            {
                #(#field_initializers,)*
            }
        },
        Fields::Unnamed(_) => quote! {
            (
                #(#field_initializers),*
            )
        },
        Fields::Unit => quote! {},
    }
}

/// builds a mock enum which is a copy of the provided `data_enum`, but with all variant data removed, such that all variants are empty.
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

/// builds an expression for the serialized size of the largest enum variant of the provided enum.
fn build_enum_largest_variant_serialized_size_expr(
    data_enum: &DataEnum,
) -> proc_macro2::TokenStream {
    let mut cur_expr = quote! { 0 };
    for variant in &data_enum.variants {
        let cur_variant_max_serialized_size_expr =
            build_max_serialized_size_expr_for_fields(&variant.fields);
        cur_expr = quote! {
            const_max(#cur_expr, #cur_variant_max_serialized_size_expr)
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

/// attempts to extract the enum's underlying representation type.
fn enum_get_repr_type<'a>(
    derive_input: &'a DeriveInput,
    data_enum: &DataEnum,
) -> Result<&'a proc_macro2::TokenStream, proc_macro2::TokenStream> {
    let Some(repr_type) = derive_input.attrs.iter().find_map(|attr| {
        let AttrStyle::Outer = &attr.style else {
            return None;
        };
        let Meta::List(meta_list) = &attr.meta else {
            return None;
        };
        if !meta_list.path.is_ident("repr") {
            return None;
        }
        Some(&meta_list.tokens)
    }) else {
        return Err(quote_spanned! {
            data_enum.enum_token.span() => compile_error!("binary serialization for enums requires a #[repr(...)] attribute on the enum to specify the size of the enum's tag");
        });
    };

    if !is_enum_repr_type_sized_primitive_int(&repr_type) {
        return Err(quote_spanned! {
            repr_type.span() => compile_error!("binary serialization for enums requires the enum's #[repr(...)] attribute to contain an explicitly sized primitive integer type (e.g., u32)");
        });
    }

    Ok(repr_type)
}

/// checks if the provided expression inside a `#[repr(...)]` attribute on an enum represents an explicitly sized primitive int.
fn is_enum_repr_type_sized_primitive_int(repr_type: &proc_macro2::TokenStream) -> bool {
    let repr_type_str = repr_type.to_string();
    repr_type_str.as_bytes()[0].is_ascii_alphabetic() && repr_type_str[1..].parse::<u32>().is_ok()
}

/// builds an expression for the maximum serialized size of the given fields.
fn build_max_serialized_size_expr_for_fields(fields: &Fields) -> proc_macro2::TokenStream {
    let field_type_max_serialized_sizes = fields
        .iter()
        .map(|field| max_serialized_size_of_ty(&field.ty));
    quote! {
        (0 #(
            + #field_type_max_serialized_sizes
        )*)
    }
}

/// returns an iterator over all subtypes of all fields of the provided struct or enum.
fn collect_all_sub_types<'a>(
    derive_input: &'a DeriveInput,
) -> Box<dyn Iterator<Item = &'a Type> + 'a> {
    match &derive_input.data {
        syn::Data::Struct(data_struct) => {
            Box::new(data_struct.fields.iter().map(|field| &field.ty))
        }
        syn::Data::Enum(data_enum) => Box::new(
            data_enum
                .variants
                .iter()
                .map(|variant| variant.fields.iter().map(|field| &field.ty))
                .flatten(),
        ),
        syn::Data::Union(_) => unreachable!(),
    }
}

/// builds an expression for the maximum serialized size of the given type.
fn max_serialized_size_of_ty<T: ToTokens>(ty: &T) -> proc_macro2::TokenStream {
    quote! {
        <#ty as ::binary_serde::BinarySerdeInternal>::MAX_SERIALIZED_SIZE_INTERNAL
    }
}

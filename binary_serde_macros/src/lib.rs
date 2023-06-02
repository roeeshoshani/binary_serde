use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, spanned::Spanned, AttrStyle, DataEnum,
    DataStruct, DeriveInput, Field, Fields, Meta, Token, Type, Variant,
    WhereClause,
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
    let tag_serialized_size_expr = serialized_size_of_ty(repr_type);
    let serialized_size_expr =
        quote! { (#tag_serialized_size_expr + #largest_variant_serialized_size_expr) };

    let enum_name_str = derive_input.ident.to_string();
    let mock_enum_name_ident = format_ident!("Empty{}", derive_input.ident);
    let mock_enum =
        build_mock_enum_with_empty_variants(&mock_enum_name_ident, data_enum, repr_type);
    let serialization_variant_branches = data_enum.variants.iter().map(|variant| {
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
                ::binary_serde::BinarySerde::binary_serialize(#field_var_name, &mut buf[#cur_index_in_buf .. #end_index_in_buf], endianness);
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
                ::binary_serde::BinarySerde::binary_serialize(&tag_value, &mut buf[..#tag_serialized_size_expr], endianness);
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
            #(#serialization_variant_branches,)*
        }
    };

    let deserialization_variant_branches = data_enum.variants.iter().map(|variant| {
        let variant_ident = &variant.ident;
        let mock_enum_const_variant_ident =
            format_ident!("{}{}", mock_enum_name_ident, variant_ident);
        let fields_initializer = build_deserialization_fields_initializer(
            &variant.fields,
            tag_serialized_size_expr.clone(),
        );
        quote! {
            #mock_enum_const_variant_ident => {
                ::core::result::Result::Ok(Self::#variant_ident #fields_initializer)
            }
        }
    });
    let mock_enum_const_variants = data_enum.variants.iter().map(|variant| {
        let const_ident = format_ident!("{}{}", mock_enum_name_ident, variant.ident);
        let variant_ident = &variant.ident;
        quote! {
            const #const_ident: #repr_type = #mock_enum_name_ident::#variant_ident as #repr_type;
        }
    });
    let deserialize_fn_body = quote! {
        #mock_enum
        #(#mock_enum_const_variants)*
        let tag = <#repr_type as ::binary_serde::BinarySerde>::binary_deserialize_with_ctx(
            &buf[..#tag_serialized_size_expr],
            endianness,
            index_in_buf,
        )?;
        match tag {
            #(#deserialization_variant_branches,)*
            _ => {
                ::core::result::Result::Err(::binary_serde::Error {
                    kind: ::binary_serde::ErrorKind::InvalidEnumTag { enum_name: #enum_name_str },
                    span: ::binary_serde::ErrorSpan {
                        start: index_in_buf,
                        end: index_in_buf + #tag_serialized_size_expr,
                    }
                })
            }
        }
    };

    build_binary_serde_impl(
        derive_input,
        serialized_size_expr,
        serialize_fn_body,
        deserialize_fn_body,
    )
    .into()
}

fn derive_binary_serde_for_struct(
    derive_input: &DeriveInput,
    data_struct: &DataStruct,
) -> proc_macro::TokenStream {
    build_binary_serde_impl_with_fields(derive_input, &data_struct.fields).into()
}

fn build_binary_serde_impl(
    derive_input: &DeriveInput,
    serialized_size_expr: proc_macro2::TokenStream,
    serialize_fn_body: proc_macro2::TokenStream,
    deserialize_fn_body: proc_macro2::TokenStream,
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
        impl #impl_generics ::binary_serde::BinarySerde for #ty_ident #type_generics #where_clause {
            const SERIALIZED_SIZE: usize = { #serialized_size_expr };

            fn binary_serialize(&self, buf: &mut [u8], endianness: ::binary_serde::Endianness) {
                #serialize_fn_body
            }

            fn binary_deserialize_with_ctx(buf: &[u8], endianness: ::binary_serde::Endianness, index_in_buf: usize) -> ::core::result::Result<Self, ::binary_serde::Error> {
                #deserialize_fn_body
            }
        }
    }
}

fn build_binary_serde_impl_with_fields(
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
            ::binary_serde::BinarySerde::binary_serialize(&self.#access_name, &mut buf[#cur_index_in_buf .. #end_index_in_buf], endianness);
        };

        *cur_index_in_buf = end_index_in_buf;

        Some(field_serialization_call)
    });
    let serialize_fn_body = quote! { #(#field_serializations)* };

    let fields_initializer = build_deserialization_fields_initializer(fields, quote! { 0 });
    let deserialize_fn_body = quote! {
        ::core::result::Result::Ok(Self #fields_initializer)
    };

    build_binary_serde_impl(
        derive_input,
        serialized_size_expr,
        serialize_fn_body,
        deserialize_fn_body,
    )
}

fn build_deserialization_fields_initializer(
    fields: &Fields,
    initial_index_in_buf: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let field_initializers =
        fields_to_iter(fields).scan(initial_index_in_buf, |cur_index_in_buf, field| {
            let initialization_name = match &field.ident {
                Some(ident) => quote! { #ident : },
                None => quote! {},
            };
            let field_serialized_size = serialized_size_of_ty(&field.ty);
            let end_index_in_buf = quote! { #cur_index_in_buf + #field_serialized_size};
            let field_ty = &field.ty;
            let initialization_value = quote! {
                <#field_ty as ::binary_serde::BinarySerde>::binary_deserialize_with_ctx(
                    &buf[#cur_index_in_buf .. #end_index_in_buf],
                    endianness, index_in_buf + #cur_index_in_buf
                )?
            };

            *cur_index_in_buf = end_index_in_buf;

            Some(quote! {
                #initialization_name #initialization_value
            })
        });

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
        <#ty as ::binary_serde::BinarySerde>::SERIALIZED_SIZE
    }
}

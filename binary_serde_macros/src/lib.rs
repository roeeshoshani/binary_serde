use quote::{quote, quote_spanned};
use syn::{parse_macro_input, parse_quote, DeriveInput};

#[proc_macro_derive(BinarySerde)]
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
                serialization_code: gen_serialization_code(&data_struct.fields),
                deserialization_code: gen_deserialization_code(&data_struct.fields),
            })
            .into()
        }
        syn::Data::Enum(_) => {
            return quote_spanned! {
                proc_macro2::Span::call_site() => compile_error!("enums are not supported, only structs are supported");
            }
            .into();
        }
        syn::Data::Union(_) => {
            return quote_spanned! {
                proc_macro2::Span::call_site() => compile_error!("unions are not supported, only structs are supported");
            }
            .into();
        }
    }
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
fn gen_serialization_code(fields: &syn::Fields) -> proc_macro2::TokenStream {
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
fn gen_deserialization_code(fields: &syn::Fields) -> proc_macro2::TokenStream {
    let field_values = fields_offsets_and_sizes(fields.iter()).map(|offset_and_size| {
        let FieldOffsetAndSize { size, offset } = offset_and_size;
        quote! {
            ::binary_serde::BinarySerde::binary_deserialize(
                &buf[#offset..][..#size],
                endianness
            )
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
                Self {
                    #(#field_specifications),*
                }
            }
        }
        syn::Fields::Unnamed(_) => {
            quote! {
                Self (
                    #(#field_values),*
                )
            }
        }
        syn::Fields::Unit => quote! { Self },
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

/// generates a list of where predicates which make sure that all the given fields types implement the `BinarySerde` trait.
fn gen_predicates_for_field_types<I: Iterator<Item = TypeExpr>>(
    field_types: I,
) -> impl Iterator<Item = syn::WherePredicate> {
    field_types.map(|field_type| {
        parse_quote! {
            #field_type: ::binary_serde::BinarySerde
        }
    })
}

struct GenImplParams {
    /// the identifier of the type for which the trait is to be implemented.
    type_ident: syn::Ident,

    /// the generics of the type for which the trait is implemented.
    generics: syn::Generics,

    /// additional where predicates for the trait implementation.
    additional_where_predicates: Vec<syn::WherePredicate>,

    /// the serailized size of the type.
    serialized_size: SerializedSizeExpr,

    /// the recursive array type of this type.
    recursive_array_type: TypeExpr,

    /// code for serializing this type.
    /// this will be used as the body of the `binary_serialize` method.
    serialization_code: proc_macro2::TokenStream,

    /// code for deserializing this type.
    /// this will be used as the body of the `binary_deserialize` method.
    deserialization_code: proc_macro2::TokenStream,
}

/// generates the final implementation of the `BinarySerde` trait given the implementation details.
fn gen_impl(params: GenImplParams) -> proc_macro2::TokenStream {
    let GenImplParams {
        type_ident,
        generics,
        additional_where_predicates,
        serialized_size,
        recursive_array_type,
        serialization_code,
        deserialization_code,
    } = params;
    let (impl_generics, type_generics, maybe_where_clause) = generics.split_for_impl();
    let mut where_clause = maybe_where_clause.cloned().unwrap_or_else(|| {
        parse_quote! {
            where
        }
    });
    where_clause
        .predicates
        .extend(additional_where_predicates.into_iter());
    quote! {
        #[automatically_derived]
        impl #impl_generics ::binary_serde::BinarySerde for #type_ident #type_generics #where_clause {
            const SERIALIZED_SIZE: usize = (#serialized_size);
            type RecursiveArray = (#recursive_array_type);

            fn binary_serialize(&self, buf: &mut [u8], endianness: ::binary_serde::Endianness) {
                #serialization_code
            }
            fn binary_deserialize(buf: &[u8], endianness: ::binary_serde::Endianness) -> Self {
                #deserialization_code
            }
        }
    }
}

/// implements the `ToTokens` trait for a newtype which is just a wrapper something else which implements `ToTokens`.
macro_rules! impl_to_tokens_for_newtype {
    {$t: ty} => {
        impl quote::ToTokens for $t {
            fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
                self.0.to_tokens(tokens)
            }
        }
    };
}

/// an expression representing a type.
struct TypeExpr(proc_macro2::TokenStream);
impl_to_tokens_for_newtype! {TypeExpr}
impl TypeExpr {
    /// creates a new type expression from the given type value.
    fn from_type(ty: &syn::Type) -> Self {
        Self(quote! {
            #ty
        })
    }

    /// returns the serialized size of this type.
    /// this is only valid if the type implements the `BinarySerde` trait.
    fn serialized_size(&self) -> SerializedSizeExpr {
        SerializedSizeExpr(quote! {
            <#self as ::binary_serde::BinarySerde>::SERIALIZED_SIZE
        })
    }

    /// returns the recursive array type which this type is serialized into.
    /// this is only valid if the type implements the `BinarySerde` trait.
    fn serialized_recursive_array_type(&self) -> TypeExpr {
        TypeExpr(quote! {
            <#self as ::binary_serde::BinarySerde>::RecursiveArray
        })
    }
}

/// an expression for the serialized size of some type.
struct SerializedSizeExpr(proc_macro2::TokenStream);
impl_to_tokens_for_newtype! {SerializedSizeExpr}
impl SerializedSizeExpr {
    /// returns a serialized size expression for a size of zero
    fn zero() -> Self {
        Self(quote! {0})
    }
}
impl core::ops::Add for SerializedSizeExpr {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(quote! {
            (#self) + (#rhs)
        })
    }
}
impl<'a> core::ops::Add for &'a SerializedSizeExpr {
    type Output = SerializedSizeExpr;

    fn add(self, rhs: Self) -> Self::Output {
        SerializedSizeExpr(quote! {
            (#self) + (#rhs)
        })
    }
}
impl std::iter::Sum for SerializedSizeExpr {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.reduce(|a, b| a + b).unwrap_or_else(Self::zero)
    }
}

/// an expression for the offset of a field in the binary serialization of some type.
struct FieldOffsetExpr(proc_macro2::TokenStream);
impl_to_tokens_for_newtype! {FieldOffsetExpr}

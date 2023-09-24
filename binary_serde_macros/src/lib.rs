use quote::{quote, quote_spanned};
use syn::{parse_macro_input, parse_quote, DeriveInput};

mod bitfield;
mod simple;

/// a derive macro for automatically implementing the `BinarySerde` trait.
#[proc_macro_derive(BinarySerde)]
pub fn derive_binary_serde(input_tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    simple::derive_binary_serde(input_tokens)
}

/// an attribute for defining bitfields and implementing the `BinarySerde` trait for them.
///
/// you should add the `#[bits(...)]` attribute on each of the fields to specify their bit length.
///
/// the fields are treated as lsb first.
///
/// you can use any type that implements the `BinarySerde` trait as the type of a field in the bitfield.
///
/// # Example
///
/// ```
/// #[derive(Debug, Default, PartialEq, Eq)]
/// #[binary_serde_bitfield]
/// struct Elf32RelocationInfo {
///     #[bits(8)]
///     ty: u8,
///
///     #[bits(24)]
///     symbol_index: u32,
/// }
/// ```
#[proc_macro_attribute]
pub fn binary_serde_bitfield(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    bitfield::binary_serde_bitfield(args, input)
}

/// parameters for generating an implementation of the `BinarySerde` trait.
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
            fn binary_deserialize(
                buf: &[u8],
                endianness: ::binary_serde::Endianness
            ) -> ::core::result::Result<Self, ::binary_serde::DeserializeError> {
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

//! this is a crate that allows serializing and deserializing rust types into a simple binary format.
//!
//! please note that the format only support fixed size data types. dynamically sized types like `&[T]`, `Vec` and `String` are not supported.
//!
//! the serialization and deserialization have 2 modes of operation:
//! - the first mode provides operations on fixed size buffers. serialization of a some value of type `T` will always generate an output
//! of the same size, no matter what value is provided. deserialization requires a buffer of a specific size.
//! - the second mode provides more compact serialization and deserialization, but different values of the same type will generate
//! differently sized outputs.
//!
//! the format is very `no_std` friendly, since it allows for knowing the maximum serialized size of a type as a compile time constant,
//! which means that the type can be serialized into a buffer on the stack whose size is known at compile time, requiring no heap allocations.
//!
//! the format also allows very easy parsing of common binary format which often just represent fields in a packed binary representation.
//!
//! # Example
//! ```
//! use binary_serde::{BinarySerde, Endianness};
//!
//! #[derive(BinarySerde, Debug)]
//! #[repr(u8)]
//! enum Message {
//!     Number { number: i32 },
//!     Buffer([u8; 1024]),
//!     Empty,
//! }
//!
//! fn main() {
//!     let mut buffer = [0u8; <Message as BinarySerde>::MAX_SERIALIZED_SIZE];
//!     let msg = Message::Buffer([1; 1024]);
//!     msg.binary_serialize(&mut buffer, Endianness::Big);
//!     println!("{:?}", buffer);
//!     let recreated_msg = Message::binary_deserialize(&buffer, Endianness::Big);
//!     println!("{:?}", recreated_msg);
//! }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;

pub use binary_serde_macros::BinarySerde;
use thiserror_no_std::Error;

/// endianness.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Endianness {
    Big,
    Little,
}

/// the different kinds of errors that can occur when deserializing.
#[derive(Debug, Error)]
pub enum DeserializeErrorKind {
    #[error("invalid enum tag for enum {enum_name}")]
    InvalidEnumTag { enum_name: &'static str },
}

/// a span where an error originates in the input buffer.
#[derive(Debug)]
pub struct DeserializeErrorSpan {
    pub start: usize,
    pub end: usize,
}
impl std::fmt::Display for DeserializeErrorSpan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// an error which occured while deserializing.
#[derive(Debug, Error)]
#[error("{kind} at {span}")]
pub struct DeserializeError {
    /// the kind of error that occured.
    pub kind: DeserializeErrorKind,

    /// the span in the input where the error originates.
    pub span: DeserializeErrorSpan,
}

/// an internal trait for binary serialization and deserialization.
pub trait BinarySerdeInternal: Sized {
    /// the maximum size of this type when serialized to binary.
    const MAX_SERIALIZED_SIZE_INTERNAL: usize;

    /// serializes this type to binary.
    /// the length of `buf` must be exactly equal to `Self::MAX_SERIALIZED_SIZE_INTERNAL`.
    fn binary_serialize_internal(&self, buf: &mut [u8], endianness: Endianness);

    /// serializes this type to a minimal binary representation.
    /// `buf` must be large enough to hold the serialized content of `self`.
    /// it is recommended to use `Self::MAX_SERIALIZED_SIZE_INTERNAL` for the size of the buffer.
    /// returns the serialized size.
    fn binary_serialize_min_internal(&self, buf: &mut [u8], endianness: Endianness) -> usize;

    /// calculates the size of this value when serialized to its minimal binary representation
    fn binary_serialized_size_internal(&self) -> usize;

    /// deserializes a binary representation of this type.
    /// the length of `buf` must be exactly equal to `Self::MAX_SERIALIZED_SIZE_INTERNAL`.
    /// the `index_in_buf` parameter is the current index of the passed buffer inside of the larger buffer that is being parsed
    /// using `binary_deserialize`
    fn binary_deserialize_internal(
        buf: &[u8],
        endianness: Endianness,
        index_in_buf: usize,
    ) -> Result<Self, DeserializeError>;

    /// deserializes a minimal binary representation of this type.
    /// the `index_in_buf` parameter is the current index of the passed buffer inside of the larger buffer that is being parsed
    /// using `binary_deserialize`
    /// returns the deserialized content and the size of its binary representation.
    fn binary_deserialize_min_internal(
        buf: &[u8],
        endianness: Endianness,
        index_in_buf: usize,
    ) -> Result<(Self, usize), DeserializeError>;
}

/// a trait which allows serializing and deserializing a type in the binary format.
pub trait BinarySerde: BinarySerdeInternal {
    /// the maximum size of this type when serialized to binary.
    const MAX_SERIALIZED_SIZE: usize = <Self as BinarySerdeInternal>::MAX_SERIALIZED_SIZE_INTERNAL;

    /// serializes this type to binary.
    /// the length of `buf` must be exactly equal to `Self::MAX_SERIALIZED_SIZE`.
    /// this function is guaranteed to fill the entire buffer.
    fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness) {
        self.binary_serialize_internal(buf, endianness)
    }

    /// serializes this type to binary.
    /// `buf` must be large enough to hold the serialized content of `self`.
    /// it is recommended to use `Self::MAX_SERIALIZED_SIZE` for the size of the buffer.
    /// returns the serialized content.
    fn binary_serialize_min<'a>(&self, buf: &'a mut [u8], endianness: Endianness) -> &'a [u8] {
        let serialized_size = self.binary_serialize_min_internal(buf, endianness);
        &buf[..serialized_size]
    }

    /// calculates the size of this value when serialized to its minimal binary representation
    fn binary_serialized_size(&self) -> usize {
        self.binary_serialized_size_internal()
    }

    /// deserializes a binary representation of this type.
    /// the length of `buf` must be exactly equal to `Self::MAX_SERIALIZED_SIZE`.
    fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Result<Self, DeserializeError> {
        <Self as BinarySerdeInternal>::binary_deserialize_internal(buf, endianness, 0)
    }

    /// deserializes a minimal binary representation of this type.
    /// returns the deserialized content and the binary size of it.
    /// leftover data in the buffer is ignored.
    fn binary_deserialize_min(
        buf: &[u8],
        endianness: Endianness,
    ) -> Result<(Self, usize), DeserializeError> {
        <Self as BinarySerdeInternal>::binary_deserialize_min_internal(buf, endianness, 0)
    }
}
impl<T: BinarySerdeInternal> BinarySerde for T {}

macro_rules! impl_serialize_for_primitive_int_and_float_types {
    {$($int: ty),+} => {
        $(
            impl BinarySerdeInternal for $int {
                const MAX_SERIALIZED_SIZE_INTERNAL: usize = core::mem::size_of::<Self>();

                fn binary_serialize_internal(&self, buf: &mut [u8], endianness: Endianness){
                    buf.copy_from_slice(&match endianness {
                        Endianness::Big => self.to_be_bytes(),
                        Endianness::Little => self.to_le_bytes(),
                    });
                }

                fn binary_serialize_min_internal(&self, buf: &mut [u8], endianness: Endianness) -> usize {
                    self.binary_serialize_internal(buf, endianness);
                    Self::MAX_SERIALIZED_SIZE_INTERNAL
                }

                fn binary_serialized_size_internal(&self) -> usize {
                    Self::MAX_SERIALIZED_SIZE_INTERNAL
                }

                fn binary_deserialize_internal(buf: &[u8], endianness: Endianness, _index_in_buf: usize) -> Result<Self, DeserializeError> {
                    let bytes_array: [u8; core::mem::size_of::<Self>()] = buf.try_into().unwrap();
                    Ok(match endianness {
                        Endianness::Big => Self::from_be_bytes(bytes_array),
                        Endianness::Little => Self::from_le_bytes(bytes_array),
                    })
                }

                fn binary_deserialize_min_internal(
                    buf: &[u8],
                    endianness: Endianness,
                    index_in_buf: usize,
                ) -> Result<(Self, usize), DeserializeError> {
                    Self::binary_deserialize_internal(buf, endianness, index_in_buf)
                        .map(|res| (res, Self::MAX_SERIALIZED_SIZE_INTERNAL))
                }
            }
        )+
    };
}

impl_serialize_for_primitive_int_and_float_types! {u8,u16,u32,u64,u128,i8,i16,i32,i64,i128,f32,f64}

impl BinarySerdeInternal for bool {
    const MAX_SERIALIZED_SIZE_INTERNAL: usize = 1;

    fn binary_serialize_internal(&self, buf: &mut [u8], _endianness: Endianness) {
        buf[0] = if *self { 1 } else { 0 }
    }

    fn binary_serialize_min_internal(&self, buf: &mut [u8], endianness: Endianness) -> usize {
        self.binary_serialize_internal(buf, endianness);
        Self::MAX_SERIALIZED_SIZE_INTERNAL
    }

    fn binary_serialized_size_internal(&self) -> usize {
        Self::MAX_SERIALIZED_SIZE_INTERNAL
    }

    fn binary_deserialize_internal(
        buf: &[u8],
        _endianness: Endianness,
        _index_in_buf: usize,
    ) -> Result<Self, DeserializeError> {
        Ok(buf[0] != 0)
    }

    fn binary_deserialize_min_internal(
        buf: &[u8],
        _endianness: Endianness,
        _index_in_buf: usize,
    ) -> Result<(Self, usize), DeserializeError> {
        Ok((buf[0] != 0, Self::MAX_SERIALIZED_SIZE_INTERNAL))
    }
}

impl<T: BinarySerdeInternal, const SIZE: usize> BinarySerdeInternal for [T; SIZE] {
    const MAX_SERIALIZED_SIZE_INTERNAL: usize = SIZE * T::MAX_SERIALIZED_SIZE_INTERNAL;

    fn binary_serialize_internal(&self, buf: &mut [u8], endianness: Endianness) {
        for (item, buf_chunk) in self
            .iter()
            .zip(buf.chunks_mut(T::MAX_SERIALIZED_SIZE_INTERNAL))
        {
            let _ = item.binary_serialize_internal(buf_chunk, endianness);
        }
    }

    fn binary_serialize_min_internal(&self, buf: &mut [u8], endianness: Endianness) -> usize {
        let mut cur_index_in_buf = 0;
        for item in self {
            let item_serialized_size = item.binary_serialize_min_internal(
                &mut buf[cur_index_in_buf..cur_index_in_buf + T::MAX_SERIALIZED_SIZE_INTERNAL],
                endianness,
            );
            cur_index_in_buf += item_serialized_size;
        }
        cur_index_in_buf
    }

    fn binary_serialized_size_internal(&self) -> usize {
        self.iter()
            .map(|item| item.binary_serialized_size_internal())
            .sum()
    }

    fn binary_deserialize_internal(
        buf: &[u8],
        endianness: Endianness,
        index_in_buf: usize,
    ) -> Result<Self, DeserializeError> {
        array_init::try_array_init(|i| {
            let buf_chunk = &buf
                [i * T::MAX_SERIALIZED_SIZE_INTERNAL..(i + 1) * T::MAX_SERIALIZED_SIZE_INTERNAL];
            T::binary_deserialize_internal(
                buf_chunk,
                endianness,
                index_in_buf + i * T::MAX_SERIALIZED_SIZE_INTERNAL,
            )
        })
    }

    fn binary_deserialize_min_internal(
        buf: &[u8],
        endianness: Endianness,
        index_in_buf: usize,
    ) -> Result<(Self, usize), DeserializeError> {
        let mut cur_index_in_buf = 0;
        let array = array_init::try_array_init(|_| {
            let buf_chunk =
                &buf[cur_index_in_buf..cur_index_in_buf + T::MAX_SERIALIZED_SIZE_INTERNAL];
            let (item, item_size) = T::binary_deserialize_min_internal(
                buf_chunk,
                endianness,
                index_in_buf + cur_index_in_buf,
            )?;
            cur_index_in_buf += item_size;
            Ok(item)
        })?;
        Ok((array, cur_index_in_buf))
    }
}

impl<T> BinarySerdeInternal for PhantomData<T> {
    const MAX_SERIALIZED_SIZE_INTERNAL: usize = 0;

    fn binary_serialize_internal(&self, _buf: &mut [u8], _endianness: Endianness) {}

    fn binary_serialize_min_internal(&self, _buf: &mut [u8], _endianness: Endianness) -> usize {
        0
    }

    fn binary_serialized_size_internal(&self) -> usize {
        0
    }

    fn binary_deserialize_internal(
        _buf: &[u8],
        _endianness: Endianness,
        _index_in_buf: usize,
    ) -> Result<Self, DeserializeError> {
        Ok(Self)
    }

    fn binary_deserialize_min_internal(
        _buf: &[u8],
        _endianness: Endianness,
        _index_in_buf: usize,
    ) -> Result<(Self, usize), DeserializeError> {
        Ok((Self, 0))
    }
}

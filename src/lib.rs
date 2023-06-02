#![no_std]

use core::marker::PhantomData;

pub use binary_serde_macros::BinarySerde;
use thiserror_no_std::Error;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Endianness {
    Big,
    Little,
}

#[derive(Debug, Error)]
pub enum ErrorKind {
    #[error("invalid enum tag for enum {enum_name}")]
    InvalidEnumTag { enum_name: &'static str },
}

#[derive(Debug)]
pub struct ErrorSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Error)]
pub struct Error {
    pub kind: ErrorKind,
    pub span: ErrorSpan,
}

pub trait BinarySerde: Sized {
    /// the size of this type when serialized to binary.
    const SERIALIZED_SIZE: usize;

    /// serializes this type to binary.
    /// the length of `buf` must be exactly equal to `Self::SERIALIZED_SIZE`.
    fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness);

    /// deserializes a binary representation of this type.
    /// the length of `buf` must be exactly equal to `Self::SERIALIZED_SIZE`.
    /// the `index_in_buf` parameter is the current index of the passed buffer inside of the larger buffer that is being parsed
    /// using `binary_deserialize`
    fn binary_deserialize_with_ctx(
        buf: &[u8],
        endianness: Endianness,
        index_in_buf: usize,
    ) -> Result<Self, Error>;

    /// deserializes a binary representation of this type.
    /// the length of `buf` must be exactly equal to `Self::SERIALIZED_SIZE`.
    fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Result<Self, Error> {
        Self::binary_deserialize_with_ctx(buf, endianness, 0)
    }
}

macro_rules! impl_serialize_for_primitive_int_types {
    {$($int: ty),+} => {
        $(
            impl BinarySerde for $int {
                const SERIALIZED_SIZE: usize = core::mem::size_of::<Self>();

                fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness) {
                    buf.copy_from_slice(&match endianness {
                        Endianness::Big => self.to_be_bytes(),
                        Endianness::Little => self.to_le_bytes(),
                    });
                }

                fn binary_deserialize_with_ctx(buf: &[u8], endianness: Endianness, _index_in_buf: usize) -> Result<Self, Error> {
                    let bytes_array: [u8; core::mem::size_of::<Self>()] = buf.try_into().unwrap();
                    Ok(match endianness {
                        Endianness::Big => Self::from_be_bytes(bytes_array),
                        Endianness::Little => Self::from_le_bytes(bytes_array),
                    })
                }
            }
        )+
    };
}

impl_serialize_for_primitive_int_types! {u8,u16,u32,u64,u128,i8,i16,i32,i64,i128}

impl<T: BinarySerde, const SIZE: usize> BinarySerde for [T; SIZE] {
    const SERIALIZED_SIZE: usize = SIZE * T::SERIALIZED_SIZE;

    fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness) {
        for (item, buf_chunk) in self.iter().zip(buf.chunks_mut(T::SERIALIZED_SIZE)) {
            item.binary_serialize(buf_chunk, endianness)
        }
    }

    fn binary_deserialize_with_ctx(
        buf: &[u8],
        endianness: Endianness,
        index_in_buf: usize,
    ) -> Result<Self, Error> {
        array_init::try_array_init(|i| {
            let buf_chunk = &buf[i * T::SERIALIZED_SIZE..(i + 1) * T::SERIALIZED_SIZE];
            T::binary_deserialize_with_ctx(
                buf_chunk,
                endianness,
                index_in_buf + i * T::SERIALIZED_SIZE,
            )
        })
    }
}

impl<T> BinarySerde for PhantomData<T> {
    const SERIALIZED_SIZE: usize = 0;

    fn binary_serialize(&self, _buf: &mut [u8], _endianness: Endianness) {}

    fn binary_deserialize_with_ctx(
        _buf: &[u8],
        _endianness: Endianness,
        _index_in_buf: usize,
    ) -> Result<Self, Error> {
        Ok(Self)
    }
}

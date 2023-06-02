#![no_std]

use core::marker::PhantomData;

pub use binary_serde_macros::BinarySerialize;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Endianness {
    Big,
    Little,
}

pub trait BinarySerialize {
    /// the size of this type when serialized to binary.
    const SERIALIZED_SIZE: usize;

    /// serializes this type to binary.
    /// the length of `buf` must be exactly equal to `Self::SERIALIZED_SIZE`.
    fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness);
}

macro_rules! impl_serialize_for_primitive_int_types {
    {$($int: ty),+} => {
        $(
            impl BinarySerialize for $int {
                const SERIALIZED_SIZE: usize = core::mem::size_of::<$int>();

                fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness) {
                    buf.copy_from_slice(&match endianness {
                        Endianness::Big => self.to_be_bytes(),
                        Endianness::Little => self.to_le_bytes(),
                    });
                }
            }
        )+
    };
}

impl_serialize_for_primitive_int_types! {u8,u16,u32,u64,u128,i8,i16,i32,i64,i128}

impl<T: BinarySerialize, const SIZE: usize> BinarySerialize for [T; SIZE] {
    const SERIALIZED_SIZE: usize = SIZE * T::SERIALIZED_SIZE;

    fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness) {
        for (item, buf_chunk) in self.iter().zip(buf.chunks_mut(T::SERIALIZED_SIZE)) {
            item.binary_serialize(buf_chunk, endianness)
        }
    }
}

impl<T> BinarySerialize for PhantomData<T> {
    const SERIALIZED_SIZE: usize = 0;

    fn binary_serialize(&self, _buf: &mut [u8], _endianness: Endianness) {}
}

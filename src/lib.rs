// TODO: uncomment this
// #![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;

pub use binary_serde_macros::BinarySerde;
use recursive_array::{
    recursive_array_type_of_size, EmptyRecursiveArray, RecursiveArray, RecursiveArrayMultiplier,
    RecursiveArraySingleItem,
};

pub use recursive_array;

/// endianness.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Endianness {
    Big,
    Little,
}

/// a trait for serializing and deserializing a type into a packed binary format.
pub trait BinarySerde: Sized {
    /// the size of this type when serialized to a packed binary format.
    const SERIALIZED_SIZE: usize;

    /// the fixed size recursive array type that is returned when serializing this type to an array.
    /// the length of this array is guaranteed to be equal to [`Self::SERIALIZED_SIZE`].
    type RecursiveArray: RecursiveArray<u8>;

    /// serialize this value into the given buffer using the given endianness.
    ///
    /// # Panics
    ///
    /// this function panics if the length of `buf` is not exactly equal to [`Self::SERIALIZED_SIZE`].
    fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness);

    /// serialize this value to a fixed size array using the given endianness.
    fn binary_serialize_to_array(&self, endianness: Endianness) -> Self::RecursiveArray {
        let mut array: core::mem::MaybeUninit<Self::RecursiveArray> =
            core::mem::MaybeUninit::uninit();
        self.binary_serialize(
            unsafe {
                core::slice::from_raw_parts_mut(
                    array.as_mut_ptr().cast::<u8>(),
                    Self::SERIALIZED_SIZE,
                )
            },
            endianness,
        );
        unsafe { array.assume_init() }
    }

    /// serialize this value into the given stream using the given endianness.
    fn binary_serialize_into<W: std::io::Write>(
        &self,
        stream: &mut W,
        endianness: Endianness,
    ) -> std::io::Result<()> {
        let serialized = self.binary_serialize_to_array(endianness);
        stream.write_all(serialized.as_slice())?;
        Ok(())
    }

    /// deserializes the given buffer using the given endianness into a value of this type.
    ///
    /// # Panics
    ///
    /// this function panics if the length of `buf` is not exactly equal to [`Self::SERIALIZED_SIZE`].
    fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Self;

    /// deserializes the data from the given stream using the given endianness into a value of this type.
    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        endianness: Endianness,
    ) -> std::io::Result<Self> {
        let mut uninit_array: core::mem::MaybeUninit<Self::RecursiveArray> =
            core::mem::MaybeUninit::uninit();
        stream.read_exact(unsafe {
            core::slice::from_raw_parts_mut(
                uninit_array.as_mut_ptr().cast::<u8>(),
                Self::SERIALIZED_SIZE,
            )
        })?;
        let array = unsafe { uninit_array.assume_init() };
        Ok(Self::binary_deserialize(array.as_slice(), endianness))
    }
}

impl BinarySerde for u8 {
    const SERIALIZED_SIZE: usize = 1;

    type RecursiveArray = RecursiveArraySingleItem<u8>;

    fn binary_serialize(&self, buf: &mut [u8], _endianness: Endianness) {
        buf[0] = *self;
    }

    fn binary_serialize_to_array(&self, _endianness: Endianness) -> Self::RecursiveArray {
        RecursiveArraySingleItem::new(*self)
    }

    fn binary_serialize_into<W: std::io::Write>(
        &self,
        stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        stream.write_all(&[*self])
    }

    fn binary_deserialize(buf: &[u8], _endianness: Endianness) -> Self {
        buf[0]
    }

    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        _endianness: Endianness,
    ) -> std::io::Result<Self> {
        let mut result: core::mem::MaybeUninit<Self> = core::mem::MaybeUninit::uninit();
        stream.read_exact(unsafe { core::slice::from_raw_parts_mut(result.as_mut_ptr(), 1) })?;
        Ok(unsafe { result.assume_init() })
    }
}

impl BinarySerde for i8 {
    const SERIALIZED_SIZE: usize = 1;

    type RecursiveArray = RecursiveArraySingleItem<u8>;

    fn binary_serialize(&self, buf: &mut [u8], _endianness: Endianness) {
        buf[0] = *self as u8;
    }

    fn binary_serialize_to_array(&self, _endianness: Endianness) -> Self::RecursiveArray {
        RecursiveArraySingleItem::new(*self as u8)
    }

    fn binary_serialize_into<W: std::io::Write>(
        &self,
        stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        stream.write_all(&[*self as u8])
    }

    fn binary_deserialize(buf: &[u8], _endianness: Endianness) -> Self {
        buf[0] as i8
    }

    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        _endianness: Endianness,
    ) -> std::io::Result<Self> {
        let mut result: core::mem::MaybeUninit<u8> = core::mem::MaybeUninit::uninit();
        stream.read_exact(unsafe { core::slice::from_raw_parts_mut(result.as_mut_ptr(), 1) })?;
        let byte = unsafe { result.assume_init() };
        Ok(byte as i8)
    }
}

impl BinarySerde for bool {
    const SERIALIZED_SIZE: usize = 1;

    type RecursiveArray = RecursiveArraySingleItem<u8>;

    fn binary_serialize(&self, buf: &mut [u8], _endianness: Endianness) {
        buf[0] = *self as u8;
    }

    fn binary_serialize_to_array(&self, _endianness: Endianness) -> Self::RecursiveArray {
        RecursiveArraySingleItem::new(*self as u8)
    }

    fn binary_serialize_into<W: std::io::Write>(
        &self,
        stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        stream.write_all(&[*self as u8])
    }

    fn binary_deserialize(buf: &[u8], _endianness: Endianness) -> Self {
        buf[0] != 0
    }

    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        _endianness: Endianness,
    ) -> std::io::Result<Self> {
        let mut result: core::mem::MaybeUninit<u8> = core::mem::MaybeUninit::uninit();
        stream.read_exact(unsafe { core::slice::from_raw_parts_mut(result.as_mut_ptr(), 1) })?;
        let byte = unsafe { result.assume_init() };
        Ok(byte != 0)
    }
}

impl<T> BinarySerde for PhantomData<T> {
    const SERIALIZED_SIZE: usize = 0;

    type RecursiveArray = EmptyRecursiveArray;

    fn binary_serialize(&self, _buf: &mut [u8], _endianness: Endianness) {}

    fn binary_serialize_to_array(&self, _endianness: Endianness) -> Self::RecursiveArray {
        EmptyRecursiveArray
    }

    fn binary_serialize_into<W: std::io::Write>(
        &self,
        _stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        Ok(())
    }

    fn binary_deserialize(_buf: &[u8], _endianness: Endianness) -> Self {
        Self
    }

    fn binary_deserialize_from<R: std::io::Read>(
        _stream: &mut R,
        _endianness: Endianness,
    ) -> std::io::Result<Self> {
        Ok(Self)
    }
}

impl BinarySerde for () {
    const SERIALIZED_SIZE: usize = 0;

    type RecursiveArray = EmptyRecursiveArray;

    fn binary_serialize(&self, _buf: &mut [u8], _endianness: Endianness) {}

    fn binary_serialize_to_array(&self, _endianness: Endianness) -> Self::RecursiveArray {
        EmptyRecursiveArray
    }

    fn binary_serialize_into<W: std::io::Write>(
        &self,
        _stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        Ok(())
    }

    fn binary_deserialize(_buf: &[u8], _endianness: Endianness) -> Self {
        ()
    }

    fn binary_deserialize_from<R: std::io::Read>(
        _stream: &mut R,
        _endianness: Endianness,
    ) -> std::io::Result<Self> {
        Ok(())
    }
}

impl<const N: usize, T: BinarySerde> BinarySerde for [T; N] {
    const SERIALIZED_SIZE: usize = T::SERIALIZED_SIZE * N;

    type RecursiveArray = RecursiveArrayMultiplier<N, u8, T::RecursiveArray>;

    fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness) {
        /// an iterator which zips 2 iterators and makes sure that they are of the same length, and panics if they are not.
        struct ZipExact<A: Iterator, B: Iterator> {
            a: A,
            b: B,
        }
        impl<A: Iterator, B: Iterator> Iterator for ZipExact<A, B> {
            type Item = (A::Item, B::Item);

            fn next(&mut self) -> Option<Self::Item> {
                match (self.a.next(), self.b.next()) {
                    (Some(a), Some(b)) => Some((a, b)),
                    (None, None) => None,
                    _ => panic!("zipped iterators are of different lengths"),
                }
            }
        }
        /// zip 2 iterators into an iterator which yields a single item at a time from both iterators, and panics if the iterators
        /// are not of the same length.
        fn zip_exact<A: Iterator, B: Iterator>(a: A, b: B) -> ZipExact<A, B> {
            ZipExact { a, b }
        }

        for (item, item_buf) in zip_exact(self.iter(), buf.chunks_mut(T::SERIALIZED_SIZE)) {
            item.binary_serialize(item_buf, endianness)
        }
    }

    fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Self {
        core::array::from_fn(|i| {
            T::binary_deserialize(
                &buf[i * T::SERIALIZED_SIZE..][T::SERIALIZED_SIZE..],
                endianness,
            )
        })
    }
}

macro_rules! impl_for_primitive_types {
    {$($type: ty),+} => {
        $(
            impl BinarySerde for $type {
                const SERIALIZED_SIZE: usize = core::mem::size_of::<Self>();
                type RecursiveArray = recursive_array_type_of_size!(u8, core::mem::size_of::<Self>());

                fn binary_serialize(&self, buf: &mut [u8], endianness: Endianness) {
                    let bytes = match endianness {
                        Endianness::Big => self.to_be_bytes(),
                        Endianness::Little => self.to_le_bytes(),
                    };
                    buf.copy_from_slice(bytes.as_slice());
                }
                fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Self {
                    let array = buf.try_into().unwrap();
                    match endianness {
                        Endianness::Big => Self::from_be_bytes(array),
                        Endianness::Little => Self::from_le_bytes(array),
                    }
                }
            }
        )+
    };
}

impl_for_primitive_types! {u16,u32,u64,u128,i16,i32,i64,i128,f32,f64}

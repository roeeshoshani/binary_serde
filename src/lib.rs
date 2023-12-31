//! this is a crate which allows serializing and deserializing rust structs into a packed binary format.
//!
//! the format does exactly what you expect it to do, it just serializes all fields in order,
//! according to their representation in memory, and according to the chosen endianness.
//!
//! this is very useful for parsing many common binary formats which often just represent fields in a packed binary representation,
//! just like the format used by this crate.
//!
//! additionally, this crate is very `no_std` friendly and allows writing highly performant code because it allows for knowing
//! the serialized size of a type as a compile time constant, which means that the type can be serialized into a buffer on
//! the stack whose size is known at compile time, requiring no heap allocations.
//!
//! please note that this means that dynamically sized types like `&[T]`, `Vec<T>` and `String` are not supported.
//!
//! ### bitfields
//!
//! this crate also supports defining bitfields since those seem to be quite common in a lot of binary formats.
//! the bitfield definition allows the user to specify the bit length of each field of the struct.
//! the bitfields are defined using the `binary_serde_bitfield` attribute.
//! the order of the fields in a bitfield is treated as lsb first.
//! an example of a bitfield can be seen in the example below.
//!
//! ### std support
//!
//! this crate provides a feature flag called `std` which enables a bunch of std related features:
//! - the error types implement `std::error::Error`
//! - adds the `binary_serialize_into` and the `binary_deserialize_from` functions to the `BinarySerde` trait which allow
//!   serializing/deserializing to/from data streams (`std::io::Read`/`std::io::Write`).
//! - adds a bunch of convenience functions and structs which require `std` support.
//!
//! # Example
//! a simple example of serializing and deserializing an elf 32 bit relocation entry:
//! ```
//! use binary_serde::{binary_serde_bitfield, BinarySerde, Endianness};
//!
//! #[derive(Debug, BinarySerde, Default, PartialEq, Eq)]
//! #[repr(u8)]
//! enum Elf32RelocationType {
//!     #[default]
//!     Direct = 1,
//!     PcRelative = 2,
//!     GotEntry = 3,
//!     // ...
//! }
//!
//! #[derive(Debug, Default, PartialEq, Eq)]
//! #[binary_serde_bitfield(order = BitfieldBitOrder::LsbFirst)]
//! struct Elf32RelocationInfo {
//!     #[bits(8)]
//!     ty: Elf32RelocationType,
//!
//!     #[bits(24)]
//!     symbol_index: u32,
//! }
//!
//! #[derive(Debug, BinarySerde, Default, PartialEq, Eq)]
//! struct Elf32RelocationWithAddend {
//!     offset: u32,
//!     info: Elf32RelocationInfo,
//!     addend: u32,
//! }
//!
//! fn main() {
//!     let rel = Elf32RelocationWithAddend::default();
//!
//!     // serialize the relocation to a statically allocated array on the stack
//!     let bytes = rel.binary_serialize_to_array(Endianness::Little);
//!
//!     // deserialize the relocation from its bytes to get back the original value
//!     let reconstructed_rel =
//!         Elf32RelocationWithAddend::binary_deserialize(bytes.as_ref(), Endianness::Little).unwrap();
//!
//!     assert_eq!(rel, reconstructed_rel)
//! }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;

pub use binary_serde_macros::{binary_serde_bitfield, BinarySerde};
use recursive_array::{
    recursive_array_type_of_size, EmptyRecursiveArray, RecursiveArray, RecursiveArrayMultiplier,
    RecursiveArraySingleItem,
};

pub use recursive_array;
use thiserror_no_std::Error;

/// endianness.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Endianness {
    Big,
    Little,
}
impl Endianness {
    #[cfg(target_endian = "little")]
    pub const NATIVE: Self = Endianness::Little;
    #[cfg(target_endian = "big")]
    pub const NATIVE: Self = Endianness::Big;
}

/// the bit order of a bitfield.
pub enum BitfieldBitOrder {
    /// the bitfield is msb first, which means that the bits of the first field will be placed at the msb.
    MsbFirst,

    /// the bitfield is lsb first, which means that the bits of the first field will be placed at the lsb.
    LsbFirst,
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

    #[cfg(feature = "std")]
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
    /// # Errors
    ///
    /// this function return an error if the given bytes do not represent a valid value of this type.
    /// this can only ever happen if during deserialization we got an enum value that does not match any of the enum's variants.
    ///
    /// # Panics
    ///
    /// this function panics if the length of `buf` is not exactly equal to [`Self::SERIALIZED_SIZE`].
    fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Result<Self, DeserializeError>;

    #[cfg(feature = "std")]
    /// deserializes the data from the given stream using the given endianness into a value of this type.
    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        endianness: Endianness,
    ) -> Result<Self, DeserializeFromError> {
        let mut uninit_array: core::mem::MaybeUninit<Self::RecursiveArray> =
            core::mem::MaybeUninit::uninit();
        stream.read_exact(unsafe {
            core::slice::from_raw_parts_mut(
                uninit_array.as_mut_ptr().cast::<u8>(),
                Self::SERIALIZED_SIZE,
            )
        })?;
        let array = unsafe { uninit_array.assume_init() };
        Ok(Self::binary_deserialize(array.as_slice(), endianness)?)
    }
}

/// an error which can occur while deserializing.
#[derive(Debug, Error)]
pub enum DeserializeError {
    #[error("invalid value for enum {enum_name}")]
    InvalidEnumValue { enum_name: &'static str },
}

#[cfg(feature = "std")]
/// an error which can occur while deserializing from a data stream.
#[derive(Debug, Error)]
pub enum DeserializeFromError {
    /// an io error has occured while reading from the stream.
    #[error("io error while reading from stream")]
    IoError(
        #[from]
        #[source]
        std::io::Error,
    ),

    /// a deserialization error occured while trying to deserialize the bytes that were read from the stream.
    #[error("deserialization error")]
    DeserializeError(
        #[from]
        #[source]
        DeserializeError,
    ),
}

/// an error which can occur while serializing/deserializing to/from a safe buffer serializer/deserializer.
#[derive(Debug, Error)]
pub enum BinarySerdeBufSafeError {
    /// a deserialization error occured while trying to deserialize the bytes that were read from the buffer.
    #[error("deserialization error")]
    DeserializeError(
        #[from]
        #[source]
        DeserializeError,
    ),

    /// the index is out of the bounds of the buffer
    #[error("index {index} is out of bounds of buffer of len {buf_len}")]
    OutOfBounds { index: usize, buf_len: usize },
}
/// binary serializes the given value using the given endianness into the given vector.
#[cfg(feature = "std")]
pub fn binary_serialize_into_vec<T: BinarySerde>(
    value: &T,
    endianness: Endianness,
    vec: &mut Vec<u8>,
) {
    vec.reserve(T::SERIALIZED_SIZE);
    value.binary_serialize(
        unsafe {
            core::slice::from_raw_parts_mut(vec.as_mut_ptr().add(vec.len()), T::SERIALIZED_SIZE)
        },
        endianness,
    );
    unsafe { vec.set_len(vec.len() + T::SERIALIZED_SIZE) }
}

/// a serializer which serializes value into a vector of bytes.
#[cfg(feature = "std")]
#[derive(Clone)]
pub struct BinarySerializerToVec {
    buf: Vec<u8>,
    endianness: Endianness,
}
#[cfg(feature = "std")]
impl BinarySerializerToVec {
    /// creates a new serializer with an empty buffer and with the given endianness.
    pub fn new(endianness: Endianness) -> Self {
        Self {
            buf: Vec::new(),
            endianness,
        }
    }

    /// creates a new serializer with the given initial buffer and endianness.
    pub fn new_with_buffer(initial_buffer: Vec<u8>, endianness: Endianness) -> Self {
        Self {
            buf: initial_buffer,
            endianness,
        }
    }

    /// serializes the given value into the buffer.
    pub fn serialize<T: BinarySerde>(&mut self, value: &T) {
        binary_serialize_into_vec(value, self.endianness, &mut self.buf)
    }

    /// returns a reference to the serialized data in the buffer.
    pub fn data(&self) -> &[u8] {
        &self.buf
    }

    /// returns a refernece to the underlying buffer of the serializer.
    pub fn buffer(&self) -> &Vec<u8> {
        &self.buf
    }

    /// returns a mutable refernece to the underlying buffer of the serializer.
    pub fn buffer_mut(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }

    /// consumes this serializer and returns its internal buffer.
    pub fn into_buffer(self) -> Vec<u8> {
        self.buf
    }

    /// sets the endianness of this serializer
    pub fn set_endianness(&mut self, new_endianness: Endianness) {
        self.endianness = new_endianness
    }

    /// returns the endianness of this serializer.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }
}

/// a serializer which serializes value into a data stream.
#[cfg(feature = "std")]
#[derive(Clone)]
pub struct BinarySerializerToStream<W: std::io::Write> {
    stream: W,
    endianness: Endianness,
}
#[cfg(feature = "std")]
impl<W: std::io::Write> BinarySerializerToStream<W> {
    /// creates a new serializer which serializes into the given stream using the given endianness.
    pub fn new(stream: W, endianness: Endianness) -> Self {
        Self { stream, endianness }
    }

    /// serializes the given value into the stream.
    pub fn serialize<T: BinarySerde>(&mut self, value: &T) -> std::io::Result<()> {
        value.binary_serialize_into(&mut self.stream, self.endianness)
    }

    /// consumes this serializer and returns its internal stream.
    pub fn into_stream(self) -> W {
        self.stream
    }

    /// returns a refernece to the underlying stream of this serializer
    pub fn stream(&self) -> &W {
        &self.stream
    }

    /// returns a mutable refernece to the underlying stream of this serializer
    pub fn stream_mut(&mut self) -> &mut W {
        &mut self.stream
    }

    /// sets the endianness of this serializer
    pub fn set_endianness(&mut self, new_endianness: Endianness) {
        self.endianness = new_endianness
    }

    /// returns the endianness of this serializer.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }
}

/// a deserializer which deserializes values from a buffer.
#[derive(Clone)]
pub struct BinaryDeserializerFromBuf<'a> {
    buf: &'a [u8],
    endianness: Endianness,
    position: usize,
}
impl<'a> BinaryDeserializerFromBuf<'a> {
    /// creates a new deserializer which deserializes values from the given buffer using the given endianness.
    pub fn new(buf: &'a [u8], endianness: Endianness) -> Self {
        Self {
            buf,
            endianness,
            position: 0,
        }
    }

    /// deserializes a value of type `T` from the current position in the buffer, and advances the position accordingly.
    ///
    /// # Panics
    ///
    /// this function panics if the deserialization exceeds the bounds of the buffer.
    pub fn deserialize<T: BinarySerde>(&mut self) -> Result<T, DeserializeError> {
        let result = T::binary_deserialize(
            &self.buf[self.position..][..T::SERIALIZED_SIZE],
            self.endianness,
        )?;
        self.position += T::SERIALIZED_SIZE;
        Ok(result)
    }

    /// returns the current position of this deserializer in the buffer.
    pub fn position(&self) -> usize {
        self.position
    }

    /// sets the position of this deserializer in the buffer.
    pub fn set_position(&mut self, new_position: usize) {
        self.position = new_position;
    }

    /// moves this deserializer's position forwards according to the given amount.
    pub fn move_forwards(&mut self, amount: usize) {
        self.position += amount;
    }

    /// moves this deserializer's position backwards according to the given amount.
    pub fn move_backwards(&mut self, amount: usize) {
        self.position -= amount;
    }

    /// sets the endianness of this deserializer
    pub fn set_endianness(&mut self, new_endianness: Endianness) {
        self.endianness = new_endianness
    }

    /// returns the endianness of this deserializer.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// returns a reference to the underlying buffer of this deserializer
    pub fn buf(&self) -> &'a [u8] {
        self.buf
    }
}

/// a binary serializer and deserializes which serializes and deserializes values to or from a buffer.
pub struct BinarySerdeBuf<'a> {
    buf: &'a mut [u8],
    endianness: Endianness,
    position: usize,
}
impl<'a> BinarySerdeBuf<'a> {
    /// creates a new binary serializer/deserializer which serializes/deserializer values to/from the given buffer using
    /// the given endianness.
    pub fn new(buf: &'a mut [u8], endianness: Endianness) -> Self {
        Self {
            buf,
            endianness,
            position: 0,
        }
    }

    /// serializes a value of type `T` into the current position in the buffer, and advances the position accordingly.
    ///
    /// # Panics
    ///
    /// this function panics if the serialization exceeds the bounds of the buffer.
    pub fn serialize<T: BinarySerde>(&mut self, value: &T) {
        value.binary_serialize(
            &mut self.buf[self.position..][..T::SERIALIZED_SIZE],
            self.endianness,
        );
        self.position += T::SERIALIZED_SIZE;
    }

    /// deserializes a value of type `T` from the current position in the buffer, and advances the position accordingly.
    ///
    /// # Panics
    ///
    /// this function panics if the deserialization exceeds the bounds of the buffer.
    pub fn deserialize<T: BinarySerde>(&mut self) -> Result<T, DeserializeError> {
        let result = T::binary_deserialize(
            &self.buf[self.position..][..T::SERIALIZED_SIZE],
            self.endianness,
        )?;
        self.position += T::SERIALIZED_SIZE;
        Ok(result)
    }
    /// returns the current position of this deserializer in the buffer.
    pub fn position(&self) -> usize {
        self.position
    }

    /// sets the position of this deserializer in the buffer.
    pub fn set_position(&mut self, new_position: usize) {
        self.position = new_position;
    }

    /// moves this deserializer's position forwards according to the given amount.
    pub fn move_forwards(&mut self, amount: usize) {
        self.position += amount;
    }

    /// moves this deserializer's position backwards according to the given amount.
    pub fn move_backwards(&mut self, amount: usize) {
        self.position -= amount;
    }

    /// sets the endianness of this serializer/deserializer
    pub fn set_endianness(&mut self, new_endianness: Endianness) {
        self.endianness = new_endianness
    }

    /// returns the endianness of this serializer/deserializer.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// returns a reference to the underlying buffer of this serializer/deserializer.
    pub fn buf(&mut self) -> &mut [u8] {
        self.buf
    }
}

/// a binary serializer/deserializer which serializes/deserializer values to a buffer and performs bounds checks on the buffer
/// when serializing/deserializing to avoid panics.
pub struct BinarySerdeBufSafe<'a> {
    buf: &'a mut [u8],
    endianness: Endianness,
    position: usize,
}
impl<'a> BinarySerdeBufSafe<'a> {
    /// creates a new binary serializer/deserializer which serializes/deserializer values to/from the given buffer using
    /// the given endianness.
    pub fn new(buf: &'a mut [u8], endianness: Endianness) -> Self {
        Self {
            buf,
            endianness,
            position: 0,
        }
    }

    /// checks that the given index is in range of the buffer, and if it is not, returns an error.
    fn check_index(&self, index: usize) -> Result<(), BinarySerdeBufSafeError> {
        if index < self.buf.len() {
            Ok(())
        } else {
            Err(BinarySerdeBufSafeError::OutOfBounds {
                index,
                buf_len: self.buf.len(),
            })
        }
    }

    /// serializes a value of type `T` into the current position in the buffer, and advances the position accordingly.
    pub fn serialize<T: BinarySerde>(&mut self, value: &T) -> Result<(), BinarySerdeBufSafeError> {
        // make sure that the start index is in range
        self.check_index(self.position)?;

        // if the end index is not the same as the start index, make sure that it is also in range
        if T::SERIALIZED_SIZE > 1 {
            self.check_index(self.position + T::SERIALIZED_SIZE - 1)?;
        }

        value.binary_serialize(
            &mut self.buf[self.position..][..T::SERIALIZED_SIZE],
            self.endianness,
        );
        self.position += T::SERIALIZED_SIZE;

        Ok(())
    }

    /// deserializes a value of type `T` from the current position in the buffer, and advances the position accordingly.
    pub fn deserialize<T: BinarySerde>(&mut self) -> Result<T, BinarySerdeBufSafeError> {
        // make sure that the start index is in range
        self.check_index(self.position)?;

        // if the end index is not the same as the start index, make sure that it is also in range
        if T::SERIALIZED_SIZE > 1 {
            self.check_index(self.position + T::SERIALIZED_SIZE - 1)?;
        }

        let result = T::binary_deserialize(
            &self.buf[self.position..][..T::SERIALIZED_SIZE],
            self.endianness,
        )?;
        self.position += T::SERIALIZED_SIZE;
        Ok(result)
    }
    /// returns the current position of this deserializer in the buffer.
    pub fn position(&self) -> usize {
        self.position
    }

    /// sets the position of this deserializer in the buffer.
    pub fn set_position(&mut self, new_position: usize) {
        self.position = new_position;
    }

    /// moves this deserializer's position forwards according to the given amount.
    pub fn move_forwards(&mut self, amount: usize) {
        self.position += amount;
    }

    /// moves this deserializer's position backwards according to the given amount.
    pub fn move_backwards(&mut self, amount: usize) {
        self.position -= amount;
    }

    /// sets the endianness of this serializer/deserializer
    pub fn set_endianness(&mut self, new_endianness: Endianness) {
        self.endianness = new_endianness
    }

    /// returns the endianness of this serializer/deserializer.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// returns a reference to the underlying buffer of this serializer/deserializer.
    pub fn buf(&mut self) -> &mut [u8] {
        self.buf
    }
}

/// a deserializer which deserializes values from a buffer and performs bounds checks on the buffer when deserializing
/// to avoid panics.
#[derive(Clone)]
pub struct BinaryDeserializerFromBufSafe<'a> {
    buf: &'a [u8],
    endianness: Endianness,
    position: usize,
}
impl<'a> BinaryDeserializerFromBufSafe<'a> {
    /// creates a new deserializer which deserializes values from the given buffer using the given endianness.
    pub fn new(buf: &'a [u8], endianness: Endianness) -> Self {
        Self {
            buf,
            endianness,
            position: 0,
        }
    }

    /// checks that the given index is in range of the buffer, and if it is not, returns an error.
    fn check_index(&self, index: usize) -> Result<(), BinarySerdeBufSafeError> {
        if index < self.buf.len() {
            Ok(())
        } else {
            Err(BinarySerdeBufSafeError::OutOfBounds {
                index,
                buf_len: self.buf.len(),
            })
        }
    }

    /// deserializes a value of type `T` from the current position in the buffer, and advances the position accordingly.
    pub fn deserialize<T: BinarySerde>(&mut self) -> Result<T, BinarySerdeBufSafeError> {
        // make sure that the start index is in range
        self.check_index(self.position)?;

        // if the end index is not the same as the start index, make sure that it is also in range
        if T::SERIALIZED_SIZE > 1 {
            self.check_index(self.position + T::SERIALIZED_SIZE - 1)?;
        }

        let result = T::binary_deserialize(
            &self.buf[self.position..][..T::SERIALIZED_SIZE],
            self.endianness,
        )?;
        self.position += T::SERIALIZED_SIZE;
        Ok(result)
    }

    /// returns the current position of this deserializer in the buffer.
    pub fn position(&self) -> usize {
        self.position
    }

    /// sets the position of this deserializer in the buffer.
    pub fn set_position(&mut self, new_position: usize) {
        self.position = new_position;
    }

    /// moves this deserializer's position forwards according to the given amount.
    pub fn move_forwards(&mut self, amount: usize) {
        self.position += amount;
    }

    /// moves this deserializer's position backwards according to the given amount.
    pub fn move_backwards(&mut self, amount: usize) {
        self.position -= amount;
    }

    /// sets the endianness of this deserializer
    pub fn set_endianness(&mut self, new_endianness: Endianness) {
        self.endianness = new_endianness
    }

    /// returns the endianness of this deserializer.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// returns a reference to the underlying buffer of this deserializer.
    pub fn buf(&self) -> &'a [u8] {
        self.buf
    }
}

/// a deserializer which deserializes values from a data stream.
#[cfg(feature = "std")]
#[derive(Clone)]
pub struct BinaryDeserializerFromStream<R: std::io::Read> {
    stream: R,
    endianness: Endianness,
}
#[cfg(feature = "std")]
impl<R: std::io::Read> BinaryDeserializerFromStream<R> {
    /// creates a new deserializer which deserializes values from the given data stream using the given endianness.
    pub fn new(stream: R, endianness: Endianness) -> Self {
        Self { stream, endianness }
    }

    /// deserializes a value of type `T` from the data stream.
    pub fn deserialize<T: BinarySerde>(&mut self) -> Result<T, DeserializeFromError> {
        T::binary_deserialize_from(&mut self.stream, self.endianness)
    }

    /// consumes this deserializer and returns its internal stream.
    pub fn into_stream(self) -> R {
        self.stream
    }

    /// returns a refernece to the underlying stream of this deserializer
    pub fn stream(&self) -> &R {
        &self.stream
    }

    /// returns a mutable refernece to the underlying stream of this deserializer
    pub fn stream_mut(&mut self) -> &mut R {
        &mut self.stream
    }

    /// sets the endianness of this deserializer
    pub fn set_endianness(&mut self, new_endianness: Endianness) {
        self.endianness = new_endianness
    }

    /// returns the endianness of this deserializer.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }
}

/// extracts the bits at the given range from the given byte.
fn get_bits_of_byte(byte: u8, start_bit_index: usize, bits_amount: usize) -> u8 {
    let mask = if bits_amount == 8 {
        u8::MAX
    } else {
        (1 << bits_amount) - 1
    };
    (byte >> start_bit_index) & mask
}

/// returns a bitmask which extracts the bits at the given range from a byte.
fn get_bits_mask(start_bit_index: usize, bits_amount: usize) -> u8 {
    if bits_amount == 8 {
        u8::MAX
    } else {
        let unshifted = (1 << bits_amount) - 1;
        unshifted << start_bit_index
    }
}

/// a bit reader which allows reading a byte slice as a sequence of bits in an lsb first format.
#[doc(hidden)]
#[derive(Clone)]
pub struct LsbBitReader<'a> {
    bytes: &'a [u8],
    bit_index_in_cur_byte: usize,
    endianness: Endianness,
    endianness_neutral_byte_index: usize,
}
impl<'a> LsbBitReader<'a> {
    /// creates a new bit reader which reads from the given byte array using the given endianness.
    pub fn new(bytes: &'a [u8], endianness: Endianness) -> Self {
        Self {
            bytes,
            bit_index_in_cur_byte: 0,
            endianness,
            endianness_neutral_byte_index: 0,
        }
    }

    /// returns the current byte index while taking into account the endianness.
    pub fn cur_byte_index(&self) -> usize {
        match self.endianness {
            Endianness::Big => self.bytes.len() - 1 - self.endianness_neutral_byte_index,
            Endianness::Little => self.endianness_neutral_byte_index,
        }
    }

    /// returns the amount of bits left to read from the current byte. this is a value between 1-8 (including both ends).
    pub fn bits_left_in_cur_byte(&self) -> usize {
        8 - self.bit_index_in_cur_byte
    }

    /// reads the given amount of bits from this bit reader and advances the reader by the given amount.
    /// the provided amount must be lower than or equal to the amount of bits left to read from the current byte, such
    /// that the read doesn't require crossing a byte boundary.
    pub fn read_bits(&mut self, bits_amount: usize) -> u8 {
        assert!(bits_amount <= self.bits_left_in_cur_byte());
        let cur_byte_index = self.cur_byte_index();
        let result = get_bits_of_byte(
            self.bytes[cur_byte_index],
            self.bit_index_in_cur_byte,
            bits_amount,
        );

        self.bit_index_in_cur_byte += bits_amount;
        if self.bit_index_in_cur_byte == 8 {
            self.endianness_neutral_byte_index += 1;
            self.bit_index_in_cur_byte = 0;
        }

        result
    }
}

/// a bit writer which allows writing bit sequences to a byte slice in an lsb first format.
#[doc(hidden)]
pub struct LsbBitWriter<'a> {
    bytes: &'a mut [u8],
    bit_index_in_cur_byte: usize,
    endianness: Endianness,
    endianness_neutral_byte_index: usize,
}
impl<'a> LsbBitWriter<'a> {
    /// creates a new bit writer which writes to the given byte array using the given endianness, starting at the given bit offset.
    pub fn new(bytes: &'a mut [u8], endianness: Endianness) -> Self {
        Self {
            bit_index_in_cur_byte: 0,
            endianness,
            endianness_neutral_byte_index: 0,
            bytes,
        }
    }

    /// returns the current byte index while taking into account the endianness.
    pub fn cur_byte_index(&self) -> usize {
        match self.endianness {
            Endianness::Big => self.bytes.len() - 1 - self.endianness_neutral_byte_index,
            Endianness::Little => self.endianness_neutral_byte_index,
        }
    }

    /// returns the amount of bits left to write to the current byte. this is a value between 1-8 (including both ends).
    pub fn bits_left_in_cur_byte(&self) -> usize {
        8 - self.bit_index_in_cur_byte
    }

    /// writes the given amount of bits from the given bits and advances the bit writer by the given amount.
    /// the provided amount must be lower than or equal to the amount of bits left to write from the current byte, such
    /// that the write doesn't require crossing a byte boundary.
    pub fn write_bits(&mut self, bits: u8, bits_amount: usize) {
        let cur_byte_index = self.cur_byte_index();
        let mask = get_bits_mask(self.bit_index_in_cur_byte, bits_amount);
        self.bytes[cur_byte_index] =
            (self.bytes[cur_byte_index] & !mask) | (bits << self.bit_index_in_cur_byte);

        self.bit_index_in_cur_byte += bits_amount;
        if self.bit_index_in_cur_byte == 8 {
            self.endianness_neutral_byte_index += 1;
            self.bit_index_in_cur_byte = 0;
        }
    }
}

/// copies the given amount of bits from the `from` reader to the `to` writer.
#[doc(hidden)]
pub fn _copy_bits<'a, 'b>(
    from: &mut LsbBitReader<'a>,
    to: &mut LsbBitWriter<'b>,
    bits_amount: usize,
) {
    let mut bits_left = bits_amount;
    while bits_left > 0 {
        // calculate the amount of bits to copy in the current iteration such that we don't cross byte boundaries both in the
        // reader and writer, and also take into account the amount of bits left to copy.
        let cur_amount = core::cmp::min(
            core::cmp::min(from.bits_left_in_cur_byte(), to.bits_left_in_cur_byte()),
            bits_left,
        );

        to.write_bits(from.read_bits(cur_amount), cur_amount);
        bits_left -= cur_amount;
    }
}

/// a struct used for adding padding in the middle of your serializable structs.
/// when serializing, it will write the `PADDING_VALUE` to the buffer `PADDING_LENGTH` times.
/// deserializing, it will do nothing.
#[derive(Debug, Clone, Copy, Hash)]
pub struct BinarySerdePadding<const PADDING_LENGTH: usize, const PADDING_VALUE: u8>;
impl<const PADDING_LENGTH: usize, const PADDING_VALUE: u8> BinarySerde
    for BinarySerdePadding<PADDING_LENGTH, PADDING_VALUE>
{
    const SERIALIZED_SIZE: usize = PADDING_LENGTH;

    type RecursiveArray = recursive_array_type_of_size!(u8, PADDING_LENGTH);

    fn binary_serialize(&self, buf: &mut [u8], _endianness: Endianness) {
        buf[..PADDING_LENGTH].fill(PADDING_VALUE);
    }

    fn binary_deserialize(_buf: &[u8], _endianness: Endianness) -> Result<Self, DeserializeError> {
        Ok(Self)
    }
}

/// implements the [`BinarySerde`] trait for a type generated using the `bitflags!` macro from the `bitflags` crate.
#[macro_export]
macro_rules! impl_binary_serde_for_bitflags_ty {
    {$bitflags_ty: ty} => {
        impl ::binary_serde::BinarySerde for $bitflags_ty
        where
            <$bitflags_ty as ::bitflags::Flags>::Bits: ::binary_serde::BinarySerde,
        {
            const SERIALIZED_SIZE: usize =
                <<$bitflags_ty as ::bitflags::Flags>::Bits as ::binary_serde::BinarySerde>::SERIALIZED_SIZE;

            type RecursiveArray =
                <<$bitflags_ty as ::bitflags::Flags>::Bits as ::binary_serde::BinarySerde>::RecursiveArray;

            fn binary_serialize(&self, buf: &mut [u8], endianness: ::binary_serde::Endianness) {
                let bits = ::bitflags::Flags::bits(self);
                ::binary_serde::BinarySerde::binary_serialize(&bits, buf, endianness)
            }

            fn binary_deserialize(
                buf: &[u8],
                endianness: ::binary_serde::Endianness,
            ) -> Result<Self, ::binary_serde::DeserializeError> {
                let bits = ::binary_serde::BinarySerde::binary_deserialize(buf, endianness)?;
                Ok(::bitflags::Flags::from_bits_retain(bits))
            }

            fn binary_serialize_to_array(
                &self,
                endianness: ::binary_serde::Endianness,
            ) -> Self::RecursiveArray {
                let bits = ::bitflags::Flags::bits(self);
                ::binary_serde::BinarySerde::binary_serialize_to_array(&bits, endianness)
            }
        }
    };
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

    #[cfg(feature = "std")]
    fn binary_serialize_into<W: std::io::Write>(
        &self,
        stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        stream.write_all(&[*self])
    }

    fn binary_deserialize(buf: &[u8], _endianness: Endianness) -> Result<Self, DeserializeError> {
        Ok(buf[0])
    }

    #[cfg(feature = "std")]
    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        _endianness: Endianness,
    ) -> Result<Self, DeserializeFromError> {
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

    #[cfg(feature = "std")]
    fn binary_serialize_into<W: std::io::Write>(
        &self,
        stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        stream.write_all(&[*self as u8])
    }

    fn binary_deserialize(buf: &[u8], _endianness: Endianness) -> Result<Self, DeserializeError> {
        Ok(buf[0] as i8)
    }

    #[cfg(feature = "std")]
    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        _endianness: Endianness,
    ) -> Result<Self, DeserializeFromError> {
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

    #[cfg(feature = "std")]
    fn binary_serialize_into<W: std::io::Write>(
        &self,
        stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        stream.write_all(&[*self as u8])
    }

    fn binary_deserialize(buf: &[u8], _endianness: Endianness) -> Result<Self, DeserializeError> {
        Ok(buf[0] != 0)
    }

    #[cfg(feature = "std")]
    fn binary_deserialize_from<R: std::io::Read>(
        stream: &mut R,
        _endianness: Endianness,
    ) -> Result<Self, DeserializeFromError> {
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

    #[cfg(feature = "std")]
    fn binary_serialize_into<W: std::io::Write>(
        &self,
        _stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        Ok(())
    }

    fn binary_deserialize(_buf: &[u8], _endianness: Endianness) -> Result<Self, DeserializeError> {
        Ok(Self)
    }

    #[cfg(feature = "std")]
    fn binary_deserialize_from<R: std::io::Read>(
        _stream: &mut R,
        _endianness: Endianness,
    ) -> Result<Self, DeserializeFromError> {
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

    #[cfg(feature = "std")]
    fn binary_serialize_into<W: std::io::Write>(
        &self,
        _stream: &mut W,
        _endianness: Endianness,
    ) -> std::io::Result<()> {
        Ok(())
    }

    fn binary_deserialize(_buf: &[u8], _endianness: Endianness) -> Result<Self, DeserializeError> {
        Ok(())
    }

    #[cfg(feature = "std")]
    fn binary_deserialize_from<R: std::io::Read>(
        _stream: &mut R,
        _endianness: Endianness,
    ) -> Result<Self, DeserializeFromError> {
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

    fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Result<Self, DeserializeError> {
        array_init::try_array_init(|i| {
            T::binary_deserialize(
                &buf[i * T::SERIALIZED_SIZE..][..T::SERIALIZED_SIZE],
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
                fn binary_deserialize(buf: &[u8], endianness: Endianness) -> Result<Self, DeserializeError> {
                    let array = buf.try_into().unwrap();
                    Ok(match endianness {
                        Endianness::Big => Self::from_be_bytes(array),
                        Endianness::Little => Self::from_le_bytes(array),
                    })
                }
            }
        )+
    };
}

impl_for_primitive_types! {u16,u32,u64,u128,i16,i32,i64,i128,f32,f64}

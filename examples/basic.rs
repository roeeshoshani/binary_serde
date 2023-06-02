use std::marker::PhantomData;

use binary_serde::{BinarySerialize, Endianness};

#[derive(BinarySerialize)]
#[repr(u32)]
enum Bla<T> {
    A,
    B { x: u8, y: u16 } = 7,
    C(u8),
    D { phantom: PhantomData<T> },
}

fn main() {
    let mut buf = [1u8; <Bla<String> as BinarySerialize>::SERIALIZED_SIZE];
    let b = Bla::<u8>::C(5);
    b.binary_serialize(&mut buf, Endianness::Big);
    println!("{:?}", buf.as_slice())
}

# binary_serde

this is a crate that allows serializing and deserializing rust types into a simple binary format.

please note that the format only support fixed size data types. dynamically sized types like `&[T]`, `Vec` and `String` are not supported.

the serialization and deserialization have 2 modes of operation:
- the first mode provides operations on fixed size buffers. serialization of a some value of type `T` will always generate an output
of the same size, no matter what value is provided. deserialization requires a buffer of a specific size.
- the second mode provides more compact serialization and deserialization, but different values of the same type will generate
differently sized outputs.

the format is very `no_std` friendly, since it allows for knowing the maximum serialized size of a type as a compile time constant,
which means that the type can be serialized into a buffer on the stack whose size is known at compile time, requiring no heap allocations.

the format also allows very easy parsing of common binary format which often just represent fields in a packed binary representation.

## Example
```rust
use binary_serde::{BinarySerde, Endianness};

#[derive(BinarySerde, Debug)]
#[repr(u8)]
enum Message {
    Number { number: i32 },
    Buffer([u8; 1024]),
    Empty,
}

fn main() {
    let mut buffer = [0u8; <Message as BinarySerde>::MAX_SERIALIZED_SIZE];
    let msg = Message::Buffer([1; 1024]);
    msg.binary_serialize(&mut buffer, Endianness::Big);
    println!("{:?}", buffer);
    let recreated_msg = Message::binary_deserialize(&buffer, Endianness::Big);
    println!("{:?}", recreated_msg);
}
```

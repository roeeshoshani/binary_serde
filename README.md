# binary_serde

this is a crate which allows serializing and deserializing rust structs into a packed binary format.

the format does exactly what you expect it to do, it just serializes all fields in order,
according to their representation in memory, and according to the chosen endianness.

this is very useful for parsing many common binary formats which often just represent fields in a packed binary representation,
just like the format used by this crate.

additionally, this crate is very `no_std` friendly and allows writing highly performant code because it allows for knowing
the maximum serialized size of a type as a compile time constant, which means that the type can be serialized into a buffer on
the stack whose size is known at compile time, requiring no heap allocations.

please note that this means that dynamically sized types like `&[T]`, `Vec<T>` and `String` are not supported.

this crate also supports defining bitfields since those seem to be quite common in a lot of binary formats.
the bitfield definition allows the user to specify the bit length of each field struct.
the bitfields are defined using the [`binary_serde_bitfield`] attribute.
the order of the fields in a bitfield is treated as lsb first.
an example of a bitfield can be seen in the example below.

## Example
a simple example of serializing and deserializing an elf 32 bit relocation entry:
```rust
use binary_serde::{binary_serde_bitfield, BinarySerde, Endianness};

#[derive(Debug, BinarySerde, Default, PartialEq, Eq)]
#[repr(u8)]
enum Elf32RelocationType {
    #[default]
    Direct = 1,
    PcRelative = 2,
    GotEntry = 3,
    // ...
}

#[derive(Debug, Default, PartialEq, Eq)]
#[binary_serde_bitfield]
struct Elf32RelocationInfo {
    #[bits(8)]
    ty: Elf32RelocationType,

    #[bits(24)]
    symbol_index: u32,
}

#[derive(Debug, BinarySerde, Default, PartialEq, Eq)]
struct Elf32RelocationWithAddend {
    offset: u32,
    info: Elf32RelocationInfo,
    addend: u32,
}

fn main() {
    let rel = Elf32RelocationWithAddend::default();

    // serialize the relocation to a statically allocated array on the stack
    let bytes = rel.binary_serialize_to_array(Endianness::Little);

    // deserialize the relocation from its bytes to get back the original value
    let reconstructed_rel =
        Elf32RelocationWithAddend::binary_deserialize(bytes.as_ref(), Endianness::Little).unwrap();

    assert_eq!(rel, reconstructed_rel)
}
```

License: MIT

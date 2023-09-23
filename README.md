# binary_serde

this is a crate which allows serializing and deserializing rust structs into a packed binary format.

the format does exactly what you expect it to do, it just serializes all fields in order,
according to their representation in memory.

this is very useful for parsing many common binary formats which often just represent fields in a packed binary representation,
just like the format used by this crate.

additionally, this crate is very `no_std` friendly and allows writing highly performant code because it it allows for knowing
the maximum serialized size of a type as a compile time constant, which means that the type can be serialized into a buffer on
the stack whose size is known at compile time, requiring no heap allocations.

please note that this means that dynamically sized types like `&[T]`, `Vec<T>` and `String` are not supported.

## Example
```rust
use binary_serde::{BinarySerde, Endianness};

#[derive(Debug, BinarySerde, Default)]
#[repr(u32)]
enum ElfSectionHeaderType {
    #[default]
    ProgBits = 1,
    SymbolTable = 2,
    StringTable = 3,
    // TODO: add the rest of the types...
}

#[derive(Debug, BinarySerde, Default)]
struct Elf32SectionHeader {
    sh_name: u32,
    sh_type: ElfSectionHeaderType,
    sh_flags: u32,
    sh_addr: u32,
    sh_offset: u32,
    sh_size: u32,
    sh_link: u32,
    sh_info: u32,
    sh_addralign: u32,
    sh_entsize: u32,
}

fn main() {
    let shdr = Elf32SectionHeader::default();
    let bytes = shdr.binary_serialize_to_array(Endianness::Big);
    let reconstructed_shdr =
        Elf32SectionHeader::binary_deserialize(bytes.as_ref(), Endianness::Big);
}
```

License: MIT

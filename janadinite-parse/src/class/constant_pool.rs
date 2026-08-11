// NOTE: type definition and encoding was AI assisted (decoding was done by me).

use simd_cesu8::mutf8;
extern crate alloc;
use alloc::borrow::Cow;
use alloc::sync::Arc;

use crate::io::{self, ClassDecode, ClassEncode, ClassWriter};

/// A constant_pool entry, discriminated by its tag byte, as described in §4.4.
///
/// All the various structures in the constant_pool table begin with a one-byte
/// tag indicating the kind of constant denoted by the entry. The contents of
/// the info array vary with the value of tag. The valid tags and their values
/// are listed in Table 4.4-A. Each tag byte must be followed by two or more
/// bytes giving information about the specific constant.
#[derive(Debug, Clone)]
pub enum ConstantPoolEntry {
    /// CONSTANT_Class_info { u1 tag = 7; u2 name_index; }
    ///
    /// The value of the name_index item must be a valid index into the
    /// constant_pool table. The constant_pool entry at that index must be a
    /// CONSTANT_Utf8_info structure (§4.4.7) representing a valid binary
    /// class or interface name encoded in internal form (§4.2.1).
    ///
    /// Because arrays are objects, the opcodes anewarray and multianewarray
    /// — but not the opcode new — can reference array "classes" via
    /// CONSTANT_Class_info structures in the constant_pool table. For such
    /// array classes, the name of the class is the descriptor of the array
    /// type (§4.3.2).
    Class { name_index: u16 },

    /// CONSTANT_Fieldref_info { u1 tag = 9; u2 class_index; u2 name_and_type_index; }
    ///
    /// The value of the class_index item must be a valid index into the
    /// constant_pool table. The constant_pool entry at that index must be a
    /// CONSTANT_Class_info structure (§4.4.1) representing a class or
    /// interface type that has the field or method as a member.
    ///
    /// The class_index item of a CONSTANT_Fieldref_info structure may be
    /// either a class type or an interface type.
    ///
    /// The value of the name_and_type_index item must be a valid index into
    /// the constant_pool table. The constant_pool entry at that index must
    /// be a CONSTANT_NameAndType_info structure (§4.4.6). This constant_pool
    /// entry indicates the name and descriptor of the field or method.
    Fieldref {
        class_index: u16,
        name_and_type_index: u16,
    },

    /// CONSTANT_Methodref_info { u1 tag = 10; u2 class_index; u2 name_and_type_index; }
    ///
    /// The class_index item of a CONSTANT_Methodref_info structure must be a
    /// class type, not an interface type.
    ///
    /// If the name of the method of a CONSTANT_Methodref_info structure
    /// begins with a '<' ('\u003c'), then the name must be the special name
    /// <init>, representing an instance initialization method (§2.9.1). The
    /// return type of such a method must be void.
    Methodref {
        class_index: u16,
        name_and_type_index: u16,
    },

    /// CONSTANT_InterfaceMethodref_info { u1 tag = 11; u2 class_index; u2 name_and_type_index; }
    ///
    /// The class_index item of a CONSTANT_InterfaceMethodref_info structure
    /// must be an interface type.
    InterfaceMethodref {
        class_index: u16,
        name_and_type_index: u16,
    },

    /// CONSTANT_String_info { u1 tag = 8; u2 string_index; }
    ///
    /// The value of the string_index item must be a valid index into the
    /// constant_pool table. The constant_pool entry at that index must be a
    /// CONSTANT_Utf8_info structure (§4.4.7) representing the sequence of
    /// Unicode code points to which the String object is to be initialized.
    String { string_index: u16 },

    /// CONSTANT_Integer_info { u1 tag = 3; u4 bytes; }
    ///
    /// The bytes item of the CONSTANT_Integer_info structure represents the
    /// value of the int constant. The bytes of the value are stored in
    /// big-endian (high byte first) order.
    Integer { bytes: u32 },

    /// CONSTANT_Float_info { u1 tag = 4; u4 bytes; }
    ///
    /// The bytes item of the CONSTANT_Float_info structure represents the
    /// value of the float constant in IEEE 754 binary32 floating-point
    /// single format. The bytes of the single format representation are
    /// stored in big-endian (high byte first) order.
    Float { value: f32 },

    /// CONSTANT_Long_info { u1 tag = 5; u4 high_bytes; u4 low_bytes; }
    ///
    /// The unsigned high_bytes and low_bytes items of the CONSTANT_Long_info
    /// structure together represent the value of the long constant
    /// ((long) high_bytes << 32) + low_bytes, where the bytes of each of
    /// high_bytes and low_bytes are stored in big-endian (high byte first)
    /// order.
    ///
    /// All 8-byte constants take up two entries in the constant_pool table
    /// of the class file. If a CONSTANT_Long_info or CONSTANT_Double_info
    /// structure is the entry at index n in the constant_pool table, then
    /// the next usable entry in the table is located at index n+2. The
    /// constant_pool index n+1 must be valid but is considered unusable.
    Long { bytes: u64 },

    /// CONSTANT_Double_info { u1 tag = 6; u4 high_bytes; u4 low_bytes; }
    ///
    /// The high_bytes and low_bytes items of the CONSTANT_Double_info
    /// structure together represent the double value in IEEE 754 binary64
    /// floating-point double format
    /// ((long) high_bytes << 32) + low_bytes.
    ///
    /// See the note under CONSTANT_Long_info regarding the constant_pool
    /// index immediately following a CONSTANT_Double_info entry.
    Double { value: f64 },

    /// CONSTANT_NameAndType_info { u1 tag = 12; u2 name_index; u2 descriptor_index; }
    ///
    /// The value of the name_index item must be a valid index into the
    /// constant_pool table. The constant_pool entry at that index must be a
    /// CONSTANT_Utf8_info structure (§4.4.7) representing either the
    /// special method name <init> (§2.9.1) or a valid unqualified name
    /// denoting a field or method (§4.2.2).
    ///
    /// The value of the descriptor_index item must be a valid index into
    /// the constant_pool table. The constant_pool entry at that index must
    /// be a CONSTANT_Utf8_info structure (§4.4.7) representing a valid
    /// field descriptor or method descriptor (§4.3.2, §4.3.3).
    NameAndType {
        name_index: u16,
        descriptor_index: u16,
    },

    /// CONSTANT_Utf8_info { u1 tag = 1; u2 length; u1 bytes[length]; }
    ///
    /// The value of the length item gives the number of bytes in the bytes
    /// array (not the length of the resulting string). The bytes array
    /// contains the bytes of the string, which are modified UTF-8 (§4.4.7)
    /// encoded.
    ///
    /// The bytes array must not contain supplementary characters directly,
    /// but must instead represent them via surrogate pairs, and no byte in
    /// the array may have the value zero (each modified UTF-8 encoded
    /// character occupies between one and six bytes).
    Utf8 { string: Arc<str> },

    /// CONSTANT_MethodHandle_info { u1 tag = 15; u1 reference_kind; u2 reference_index; }
    ///
    /// The value of the reference_kind item must be in the range 1 to 9. It
    /// denotes the kind of this method handle, which characterizes its
    /// bytecode behavior (Table 4.4.8-A: REF_getField=1, REF_getStatic=2,
    /// REF_putField=3, REF_putStatic=4, REF_invokeVirtual=5,
    /// REF_invokeStatic=6, REF_invokeSpecial=7, REF_newInvokeSpecial=8,
    /// REF_invokeInterface=9).
    ///
    /// The value of the reference_index item must be a valid index into the
    /// constant_pool table, and the constant_pool entry at that index
    /// depends on the value of the reference_kind item, per the rules laid
    /// out in §4.4.8.
    MethodHandle {
        reference_kind: u8,
        reference_index: u16,
    },

    /// CONSTANT_MethodType_info { u1 tag = 16; u2 descriptor_index; }
    ///
    /// The value of the descriptor_index item must be a valid index into
    /// the constant_pool table. The constant_pool entry at that index must
    /// be a CONSTANT_Utf8_info structure (§4.4.7) representing a method
    /// descriptor (§4.3.3).
    MethodType { descriptor_index: u16 },

    /// CONSTANT_InvokeDynamic_info { u1 tag = 18; u2 bootstrap_method_attr_index; u2 name_and_type_index; }
    ///
    /// The value of the bootstrap_method_attr_index item must be a valid
    /// index into the bootstrap_methods array of the bootstrap method table
    /// (§4.7.23) of this class file.
    ///
    /// The value of the name_and_type_index item must be a valid index into
    /// the constant_pool table. The constant_pool entry at that index must
    /// be a CONSTANT_NameAndType_info structure (§4.4.6) representing a
    /// method name and method descriptor (§4.3.3).
    InvokeDynamic {
        bootstrap_method_attr_index: u16,
        name_and_type_index: u16,
    },

    /**
    All 8-byte constants take up two entries in the constant_pool table of the class file. If a CONSTANT_Long_info or CONSTANT_Double_info structure is the item in the constant_pool table at index n, then the next usable item in the pool is located at index n+2. The constant_pool index n+1 must be valid but is considered unusable.

    In retrospect, making 8-byte constants take two constant pool entries was a poor choice.
    */
    Unusable,
}

impl ClassDecode for ConstantPoolEntry {
    fn decode_from<R: crate::io::ClassReader + ?Sized>(reader: &mut R) -> crate::io::Result<Self> {
        let tag = reader.read_u8()?;

        match tag {
            1 => {
                let len = reader.read_u16()?;
                let bytes: Vec<u8> = reader.decode_n(len as usize)?;

                let string = match mutf8::decode_strict(&*bytes).map_err(|_| {
                    io::Error::new_static(io::ErrorKind::InvalidData, "Invalid MUTF8")
                })? {
                    Cow::Owned(owned) => owned.into(),
                    Cow::Borrowed(_) => {
                        // Safety: Data was verified by the function above.
                        unsafe { String::from_utf8_unchecked(bytes) }.into()
                    }
                };

                Ok(Self::Utf8 { string })
            }
            3 => Ok(Self::Integer {
                bytes: reader.read_u32()?,
            }),
            4 => Ok(Self::Float {
                value: reader.decode()?,
            }),
            5 => Ok(Self::Long {
                bytes: reader.read_u64()?,
            }),
            6 => Ok(Self::Double {
                value: reader.decode()?,
            }),
            7 => {
                let name_index = reader.read_u16()?;
                Ok(Self::Class { name_index })
            }
            8 => {
                let string_index = reader.read_u16()?;
                Ok(Self::String { string_index })
            }
            9 | 10 | 11 => {
                let class_index = reader.read_u16()?;
                let name_and_type_index = reader.read_u16()?;
                Ok(match tag {
                    9 => Self::Fieldref {
                        class_index,
                        name_and_type_index,
                    },
                    10 => Self::Methodref {
                        class_index,
                        name_and_type_index,
                    },
                    11 => Self::InterfaceMethodref {
                        class_index,
                        name_and_type_index,
                    },
                    _ => unreachable!(),
                })
            }
            12 => {
                let name_index = reader.read_u16()?;
                let descriptor_index = reader.read_u16()?;
                Ok(Self::NameAndType {
                    name_index,
                    descriptor_index,
                })
            }
            15 => {
                let reference_kind = reader.read_u8()?;
                let reference_index = reader.read_u16()?;

                Ok(Self::MethodHandle {
                    reference_kind,
                    reference_index,
                })
            }
            16 => {
                let descriptor_index = reader.read_u16()?;
                Ok(Self::MethodType { descriptor_index })
            }
            18 => {
                let bootstrap_method_attr_index = reader.read_u16()?;
                let name_and_type_index = reader.read_u16()?;
                Ok(Self::InvokeDynamic {
                    bootstrap_method_attr_index,
                    name_and_type_index,
                })
            }
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid Constant Pool tag ({other})"),
            )),
        }
    }
}

// AI generated encode logic mirroring ClassDecode.
impl ClassEncode for ConstantPoolEntry {
    fn encode_into<W: ClassWriter + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Self::Utf8 { string } => {
                writer.write_u8(1)?;

                let bytes = mutf8::encode(string);
                let len: u16 = bytes.len().try_into().map_err(|_| {
                    io::Error::new_static(
                        io::ErrorKind::InvalidData,
                        "UTF8 constant exceeds 65535 bytes",
                    )
                })?;

                writer.write_u16(len)?;
                writer.write_next_exact(&bytes)?;
                Ok(())
            }
            Self::Integer { bytes } => {
                writer.write_u8(3)?;
                writer.write_u32(*bytes)
            }
            Self::Float { value } => {
                writer.write_u8(4)?;
                writer.encode(value)
            }
            Self::Long { bytes } => {
                writer.write_u8(5)?;
                writer.write_u64(*bytes)
            }
            Self::Double { value } => {
                writer.write_u8(6)?;
                writer.encode(value)
            }
            Self::Class { name_index } => {
                writer.write_u8(7)?;
                writer.write_u16(*name_index)
            }
            Self::String { string_index } => {
                writer.write_u8(8)?;
                writer.write_u16(*string_index)
            }
            Self::Fieldref {
                class_index,
                name_and_type_index,
            } => {
                writer.write_u8(9)?;
                writer.write_u16(*class_index)?;
                writer.write_u16(*name_and_type_index)
            }
            Self::Methodref {
                class_index,
                name_and_type_index,
            } => {
                writer.write_u8(10)?;
                writer.write_u16(*class_index)?;
                writer.write_u16(*name_and_type_index)
            }
            Self::InterfaceMethodref {
                class_index,
                name_and_type_index,
            } => {
                writer.write_u8(11)?;
                writer.write_u16(*class_index)?;
                writer.write_u16(*name_and_type_index)
            }
            Self::NameAndType {
                name_index,
                descriptor_index,
            } => {
                writer.write_u8(12)?;
                writer.write_u16(*name_index)?;
                writer.write_u16(*descriptor_index)
            }
            Self::MethodHandle {
                reference_kind,
                reference_index,
            } => {
                writer.write_u8(15)?;
                writer.write_u8(*reference_kind)?;
                writer.write_u16(*reference_index)
            }
            Self::MethodType { descriptor_index } => {
                writer.write_u8(16)?;
                writer.write_u16(*descriptor_index)
            }
            Self::InvokeDynamic {
                bootstrap_method_attr_index,
                name_and_type_index,
            } => {
                writer.write_u8(18)?;
                writer.write_u16(*bootstrap_method_attr_index)?;
                writer.write_u16(*name_and_type_index)
            }
            // do nothing
            Self::Unusable => Ok(()),
        }
    }
}

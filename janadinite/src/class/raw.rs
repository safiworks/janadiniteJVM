//! Represents raw JVM types that provide direct encoding and decoding methods.

// NOTE: type definition was AI assisted.

use crate::{
    class::ConstantPoolEntry,
    io::{self, ClassDecode, ClassEncode},
};

/// The field_info structure, described in §4.5.
///
/// Each field is described by a field_info structure. No two fields in one
/// class file may have the same name and descriptor (§4.3.2).
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// The value of the access_flags item is a mask of flags used to denote
    /// access permissions to and properties of this field. The
    /// interpretation of each flag, when set, is as specified in Table
    /// 4.5-A.
    pub access_flags: u16,
    /// The value of the name_index item must be a valid index into the
    /// constant_pool table. The constant_pool entry at that index must be a
    /// CONSTANT_Utf8_info structure (§4.4.7) representing a valid
    /// unqualified name denoting a field (§4.2.2).
    pub name_index: u16,
    /// The value of the descriptor_index item must be a valid index into
    /// the constant_pool table. The constant_pool entry at that index must
    /// be a CONSTANT_Utf8_info structure (§4.4.7) representing a valid
    /// field descriptor (§4.3.2).
    pub descriptor_index: u16,
    /// Each value of the attributes table must be an attribute_info
    /// structure (§4.7).
    ///
    /// A field can have any number of optional attributes associated with
    /// it. The attributes defined by this specification as appearing in the
    /// attributes table of a field_info structure are listed in Table
    /// 4.7-C.
    ///
    /// A Java Virtual Machine implementation must silently ignore any or
    /// all attributes in the attributes table of a field_info structure
    /// that it does not recognize. Attributes not defined in this
    /// specification are not allowed to affect the semantics of the class
    /// file, but only to provide additional descriptive information
    /// (§4.7.1).
    pub attributes: Box<[AttributeInfo]>,
}

/// The method_info structure, described in §4.6.
///
/// Each method, including each instance initialization method (§2.9.1) and
/// the class or interface initialization method (§2.9.2), is described by a
/// method_info structure. No two methods in one class file may have the
/// same name and descriptor (§4.3.3).
#[derive(Debug, Clone)]
pub struct MethodInfo {
    /// The value of the access_flags item is a mask of flags used to denote
    /// access permission to and properties of this method. The
    /// interpretation of each flag, when set, is as specified in Table
    /// 4.6-A.
    pub access_flags: u16,
    /// The value of the name_index item must be a valid index into the
    /// constant_pool table. The constant_pool entry at that index must be a
    /// CONSTANT_Utf8_info structure (§4.4.7) representing either one of the
    /// special method names <init> or <clinit> (§2.9.1, §2.9.2), or a valid
    /// unqualified name denoting a method (§4.2.2).
    pub name_index: u16,
    /// The value of the descriptor_index item must be a valid index into
    /// the constant_pool table. The constant_pool entry at that index must
    /// be a CONSTANT_Utf8_info structure (§4.4.7) representing a valid
    /// method descriptor (§4.3.3).
    pub descriptor_index: u16,
    /// Each value of the attributes table must be an attribute_info
    /// structure (§4.7).
    ///
    /// A method can have any number of optional attributes associated with
    /// it. The attributes defined by this specification as appearing in the
    /// attributes table of a method_info structure are listed in Table
    /// 4.7-C.
    ///
    /// A Java Virtual Machine implementation must silently ignore any or
    /// all attributes in the attributes table of a method_info structure
    /// that it does not recognize. Attributes not defined in this
    /// specification are not allowed to affect the semantics of the class
    /// file, but only to provide additional descriptive information
    /// (§4.7.1).
    pub attributes: Box<[AttributeInfo]>,
}

/// The generic attribute_info structure, described in §4.7.
///
/// Attributes are used in the ClassFile (§4.1), field_info (§4.5),
/// method_info (§4.6), and Code_attribute (§4.7.3) structures of the class
/// file format.
///
/// All attributes have the following general format, regardless of their
/// specific kind (e.g. ConstantValue, Code, Exceptions, SourceFile,
/// LineNumberTable, and so on, as enumerated in Table 4.7-C).
#[derive(Debug, Clone)]
pub struct AttributeInfo {
    /// The value of the attribute_name_index item must be a valid index
    /// into the constant_pool table. The constant_pool entry at that index
    /// must be a CONSTANT_Utf8_info structure (§4.4.7) representing the
    /// name of the attribute.
    pub attribute_name_index: u16,
    /// The info array contains attribute_length bytes of additional
    /// information about this attribute. The interpretation of this
    /// information is entirely determined by the value of the
    /// attribute_name_index item — different attribute kinds (ConstantValue,
    /// Code, StackMapTable, Exceptions, InnerClasses, EnclosingMethod,
    /// Synthetic, Signature, SourceFile, SourceDebugExtension,
    /// LineNumberTable, LocalVariableTable, LocalVariableTypeTable,
    /// Deprecated, RuntimeVisibleAnnotations, and so on) parse this array
    /// differently, per their own dedicated structure definitions
    /// elsewhere in §4.7.
    pub info: Box<[u8]>,
}

#[derive(Debug, Clone)]
pub struct RawClassFile {
    /// The magic item supplies the magic number identifying the class file format; it has the value 0xCAFEBABE.
    pub magic: u32,
    /**
     The values of the minor_version and major_version items are the minor and major version numbers of this class file. Together, a major and a minor version number determine the version of the class file format. If a class file has major version number M and minor version number m, we denote the version of its class file format as M.m. Thus, class file format versions may be ordered lexicographically, for example, 1.5 < 2.0 < 2.1.

    A Java Virtual Machine implementation can support a class file format of version v if and only if v lies in some contiguous range Mi.0 ≤ v ≤ Mj.m. The release level of the Java SE platform to which a Java Virtual Machine implementation conforms is responsible for determining the range.

    Oracle's Java Virtual Machine implementation in JDK release 1.0.2 supports class file format versions 45.0 through 45.3 inclusive. JDK releases 1.1.* support class file format versions in the range 45.0 through 45.65535 inclusive. For k ≥ 2, JDK release 1.k supports class file format versions in the range 45.0 through 44+k.0 inclusive
    */
    pub minor_version: u16,
    pub major_version: u16,
    /// The value of the constant_pool_count (self.len()) item is equal to the number of entries in the constant_pool table plus one. A constant_pool index is considered valid if it is greater than zero and less than constant_pool_count, with the exception for constants of type long and double noted in §4.4.5.
    ///
    ///
    /// The constant_pool is a table of structures (§4.4) representing various string constants, class and interface names, field names, and other constants that are referred to within the ClassFile structure and its substructures. The format of each constant_pool table entry is indicated by its first "tag" byte.
    ///
    /// The constant_pool table is indexed from 1 to constant_pool_count - 1.
    pub constant_pool: Box<[ConstantPoolEntry]>,
    /// The value of the access_flags item is a mask of flags used to denote access permissions to and properties of this class or interface. The interpretation of each flag, when set, is specified in Table 4.1-A.
    pub access_flags: u16,
    /// The value of the this_class item must be a valid index into the constant_pool table. The constant_pool entry at that index must be a CONSTANT_Class_info structure (§4.4.1) representing the class or interface defined by this class file.
    pub this_class: u16,
    /// For a class, the value of the super_class item either must be zero or must be a valid index into the constant_pool table. If the value of the super_class item is nonzero, the constant_pool entry at that index must be a CONSTANT_Class_info structure representing the direct superclass of the class defined by this class file. Neither the direct superclass nor any of its superclasses may have the ACC_FINAL flag set in the access_flags item of its ClassFile structure.
    /// If the value of the super_class item is zero, then this class file must represent the class Object, the only class or interface without a direct superclass.
    /// For an interface, the value of the super_class item must always be a valid index into the constant_pool table. The constant_pool entry at that index must be a CONSTANT_Class_info structure representing the class Object
    pub super_class: u16,
    /// The value of the interfaces_count item gives the number of direct superinterfaces of this class or interface type.
    ///
    /// Each value in the interfaces array must be a valid index into the constant_pool table. The constant_pool entry at each value of interfaces[i], where 0 ≤ i < interfaces_count, must be a CONSTANT_Class_info structure representing an interface that is a direct superinterface of this class or interface type, in the left-to-right order given in the source for the type.
    pub interfaces: Box<[u16]>,
    /// Each value in the fields table must be a field_info structure (§4.5) giving a complete description of a field in this class or interface. The fields table includes only those fields that are declared by this class or interface. It does not include items representing fields that are inherited from superclasses or superinterfaces.
    pub fields: Box<[FieldInfo]>,
    /// Each value in the methods table must be a method_info structure (§4.6) giving a complete description of a method in this class or interface. If neither of the ACC_NATIVE and ACC_ABSTRACT flags are set in the access_flags item of a method_info structure, the Java Virtual Machine instructions implementing the method are also supplied.
    ///
    /// The method_info structures represent all methods declared by this class or interface type, including instance methods, class methods, instance initialization methods (§2.9.1), and any class or interface initialization method (§2.9.2). The methods table does not include items representing methods that are inherited from superclasses or superinterfaces.
    pub methods: Box<[MethodInfo]>,
    /// Each value of the attributes table must be an attribute_info structure (§4.7).
    ///
    /// The attributes defined by this specification as appearing in the attributes table of a ClassFile structure are listed in Table 4.7-C.
    ///
    /// The rules concerning attributes defined to appear in the attributes table of a ClassFile structure are given in §4.7.
    ///
    /// The rules concerning non-predefined attributes in the attributes table of a ClassFile structure are given in §4.7.1.
    pub attributes: Vec<AttributeInfo>,
}

impl ClassDecode for AttributeInfo {
    fn decode_from<R: crate::io::ClassReader + ?Sized>(reader: &mut R) -> crate::io::Result<Self> {
        let attribute_name_index = reader.read_u16()?;
        let attribute_length = reader.read_u32()?;

        let info = reader
            .decode_n(attribute_length as usize)?
            .into_boxed_slice();

        Ok(Self {
            attribute_name_index,
            info,
        })
    }
}

impl ClassEncode for AttributeInfo {
    fn encode_into<W: crate::io::ClassWriter + ?Sized>(
        &self,
        writer: &mut W,
    ) -> crate::io::Result<()> {
        writer.write_u16(self.attribute_name_index)?;
        writer.write_u32(self.info.len() as u32)?;
        writer.write_next_exact(&*self.info)?;
        Ok(())
    }
}

impl ClassDecode for FieldInfo {
    fn decode_from<R: crate::io::ClassReader + ?Sized>(reader: &mut R) -> crate::io::Result<Self> {
        let access_flags = reader.read_u16()?;
        let name_index = reader.read_u16()?;
        let descriptor_index = reader.read_u16()?;
        let attributes_count = reader.read_u16()?;

        let attributes = reader
            .decode_n(attributes_count as usize)?
            .into_boxed_slice();
        Ok(Self {
            access_flags,
            name_index,
            descriptor_index,
            attributes,
        })
    }
}

impl ClassEncode for FieldInfo {
    fn encode_into<W: crate::io::ClassWriter + ?Sized>(
        &self,
        writer: &mut W,
    ) -> crate::io::Result<()> {
        writer.write_u16(self.access_flags)?;
        writer.write_u16(self.name_index)?;
        writer.write_u16(self.descriptor_index)?;
        writer.write_u16(self.attributes.len() as u16)?;
        writer.encode_items(&self.attributes)?;

        Ok(())
    }
}

impl ClassDecode for MethodInfo {
    fn decode_from<R: crate::io::ClassReader + ?Sized>(reader: &mut R) -> crate::io::Result<Self> {
        let access_flags = reader.read_u16()?;
        let name_index = reader.read_u16()?;
        let descriptor_index = reader.read_u16()?;
        let attributes_count = reader.read_u16()?;

        let attributes = reader
            .decode_n(attributes_count as usize)?
            .into_boxed_slice();
        Ok(Self {
            access_flags,
            name_index,
            descriptor_index,
            attributes,
        })
    }
}

impl ClassEncode for MethodInfo {
    fn encode_into<W: crate::io::ClassWriter + ?Sized>(
        &self,
        writer: &mut W,
    ) -> crate::io::Result<()> {
        writer.write_u16(self.access_flags)?;
        writer.write_u16(self.name_index)?;
        writer.write_u16(self.descriptor_index)?;
        writer.write_u16(self.attributes.len() as u16)?;
        writer.encode_items(&self.attributes)?;

        Ok(())
    }
}

impl ClassDecode for RawClassFile {
    fn decode_from<R: crate::io::ClassReader + ?Sized>(reader: &mut R) -> crate::io::Result<Self> {
        let magic = reader.read_u32()?;

        if magic != 0xCAFE_BABE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid magic number ({magic:#010X})"),
            ));
        }

        let minor_version = reader.read_u16()?;
        let major_version = reader.read_u16()?;

        // constant_pool_count is one greater than the number of *logical*
        // entries — see §4.4.5 for why Long/Double consume two logical
        // slots (index n and the unusable index n+1) while occupying only
        // one physical entry in the vec here.
        let constant_pool_count = reader.read_u16()?;
        let mut constant_pool = Vec::with_capacity(constant_pool_count as usize);
        constant_pool.push(ConstantPoolEntry::Unusable);

        let mut index = 1u16;
        while index < constant_pool_count {
            let entry: ConstantPoolEntry = reader.decode()?;

            // 4.4.5: All 8-byte constants take up two entries in the constant_pool table of the class file. If a CONSTANT_Long_info or CONSTANT_Double_info structure is the item in the constant_pool table at index n, then the next usable item in the pool is located at index n+2. The constant_pool index n+1 must be valid but is considered unusable.
            index += match entry {
                ConstantPoolEntry::Long { .. } | ConstantPoolEntry::Double { .. } => {
                    constant_pool.push(entry);
                    constant_pool.push(ConstantPoolEntry::Unusable);
                    2
                }
                _ => {
                    constant_pool.push(entry);
                    1
                }
            };
        }

        let access_flags = reader.read_u16()?;
        let this_class = reader.read_u16()?;
        let super_class = reader.read_u16()?;

        let interfaces_count = reader.read_u16()?;
        let interfaces = reader
            .decode_n(interfaces_count as usize)?
            .into_boxed_slice();

        let fields_count = reader.read_u16()?;
        let fields = reader.decode_n(fields_count as usize)?.into_boxed_slice();

        let methods_count = reader.read_u16()?;
        let methods = reader.decode_n(methods_count as usize)?.into_boxed_slice();

        let attributes_count = reader.read_u16()?;
        let attributes = reader.decode_n(attributes_count as usize)?;

        Ok(Self {
            magic,
            minor_version,
            major_version,
            constant_pool: constant_pool.into_boxed_slice(),
            access_flags,
            this_class,
            super_class,
            interfaces,
            fields,
            methods,
            attributes,
        })
    }
}

impl ClassEncode for RawClassFile {
    fn encode_into<W: crate::io::ClassWriter + ?Sized>(
        &self,
        writer: &mut W,
    ) -> crate::io::Result<()> {
        writer.write_u32(self.magic)?;
        writer.write_u16(self.minor_version)?;
        writer.write_u16(self.major_version)?;

        writer.write_u16(self.constant_pool.len().saturating_sub(1 /* null */) as u16)?;
        writer.encode_items(&self.constant_pool)?;

        writer.write_u16(self.access_flags)?;
        writer.write_u16(self.this_class)?;
        writer.write_u16(self.super_class)?;

        writer.write_u16(self.interfaces.len() as u16)?;
        writer.encode_items(&self.interfaces)?;

        writer.write_u16(self.fields.len() as u16)?;
        writer.encode_items(&self.fields)?;

        writer.write_u16(self.methods.len() as u16)?;
        writer.encode_items(&self.methods)?;

        writer.write_u16(self.attributes.len() as u16)?;
        writer.encode_items(&self.attributes)?;

        Ok(())
    }
}

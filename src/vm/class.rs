use std::{
    fmt::Debug,
    ops::Deref,
    sync::{Arc, OnceLock},
};

use janadinite_parse::{
    self as raw,
    class::{self, Class, ConstantPoolEntry, JVMMethod},
    io::ClassReader,
};

pub enum VMConstantPoolEntry {
    UTF8(Arc<str>),
    MethodRef {
        unresolved_name: Arc<str>,
        unresolved_class: Arc<str>,
        unresolved_descriptor: Arc<str>,

        resolved: OnceLock<(Arc<VMClass>, Arc<VMMethod>)>,
    },
    // Methods within the same class
    ResolvedMethod(Arc<VMMethod>),
    ResolvedField,
    FiledRef {
        unresolved_name: Arc<str>,
        unresolved_class: Arc<str>,
        unresolved_descriptor: Arc<str>,
    },
    Int(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    Unusable,
}

impl Debug for VMConstantPoolEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VMConstantPoolEntry::UTF8(s) => write!(f, "UTF8({})", s),
            VMConstantPoolEntry::MethodRef {
                unresolved_name,
                unresolved_class,
                unresolved_descriptor,
                ..
            } => write!(
                f,
                "MethodRef({}/{}/{})",
                unresolved_name, unresolved_class, unresolved_descriptor
            ),
            VMConstantPoolEntry::FiledRef {
                unresolved_name,
                unresolved_class,
                unresolved_descriptor,
                ..
            } => write!(
                f,
                "FiledRef({}/{}/{})",
                unresolved_name, unresolved_class, unresolved_descriptor
            ),
            VMConstantPoolEntry::ResolvedMethod(m) => write!(f, "ResolvedMethod({})", m.name()),
            VMConstantPoolEntry::ResolvedField => write!(f, "ResolvedField"),

            VMConstantPoolEntry::Int(i) => write!(f, "Int({})", i),
            VMConstantPoolEntry::Float(fl) => write!(f, "Float({})", fl),
            VMConstantPoolEntry::Long(l) => write!(f, "Long({})", l),
            VMConstantPoolEntry::Double(d) => write!(f, "Double({})", d),
            VMConstantPoolEntry::Unusable => write!(f, "Unusable"),
        }
    }
}

#[derive(Debug)]
pub struct VMConstantPool {
    entries: Box<[VMConstantPoolEntry]>,
}

impl VMConstantPool {
    pub fn get_entry(&self, index: u16) -> Option<&VMConstantPoolEntry> {
        self.entries.get(index as usize)
    }

    pub fn from_unresolved(
        entries: &[ConstantPoolEntry],
        this_name: &str,
        methods: &[Arc<VMMethod>],
    ) -> raw::io::Result<Self> {
        let mut resolved_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let resolved = match entry {
                ConstantPoolEntry::Utf8 { string } => VMConstantPoolEntry::UTF8(string.clone()),
                ConstantPoolEntry::Integer { bytes } => VMConstantPoolEntry::Int(*bytes as i32),
                ConstantPoolEntry::Float { value } => VMConstantPoolEntry::Float(*value),
                ConstantPoolEntry::Long { bytes } => VMConstantPoolEntry::Long(*bytes as i64),
                ConstantPoolEntry::Double { value } => VMConstantPoolEntry::Double(*value),
                ConstantPoolEntry::Methodref {
                    class_index,
                    name_and_type_index,
                    ..
                }
                | ConstantPoolEntry::Fieldref {
                    class_index,
                    name_and_type_index,
                } => {
                    let class_name_index = class::get_const!(
                        Class { name_index },
                        entries,
                        *class_index,
                        "Fieldref/Methodref class index"
                    );

                    let (name_index, descriptor_index) = class::get_const!(
                        NameAndType {
                            name_index,
                            descriptor_index
                        },
                        entries,
                        *name_and_type_index,
                        "Fieldref/Methodref name and type index"
                    );

                    let class_name = class::get_const!(UTF8 entries, *class_name_index, "Fieldref/Methodref class name");
                    let name =
                        class::get_const!(UTF8 entries, *name_index, "Fieldref/Methodref name");

                    let descriptor = class::get_const!(UTF8 entries, *descriptor_index, "Fieldref/Methodref descriptor");

                    if matches!(entry, ConstantPoolEntry::Methodref { .. }) {
                        if &**class_name == this_name
                            && let Some(meth) = methods.iter().find(|m| {
                                m.name() == &**name && m.raw_descriptor() == &**descriptor
                            })
                        {
                            VMConstantPoolEntry::ResolvedMethod(meth.clone())
                        } else {
                            VMConstantPoolEntry::MethodRef {
                                unresolved_name: name.clone(),
                                unresolved_class: class_name.clone(),
                                unresolved_descriptor: descriptor.clone(),
                                resolved: OnceLock::new(),
                            }
                        }
                    } else if matches!(entry, ConstantPoolEntry::Fieldref { .. }) {
                        VMConstantPoolEntry::FiledRef {
                            unresolved_name: name.clone(),
                            unresolved_class: class_name.clone(),
                            unresolved_descriptor: descriptor.clone(),
                        }
                    } else {
                        unreachable!()
                    }
                }
                _ => VMConstantPoolEntry::Unusable,
            };

            resolved_entries.push(resolved);
        }
        Ok(Self {
            entries: resolved_entries.into_boxed_slice(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct VMMethod {
    method: JVMMethod,
}

impl Deref for VMMethod {
    type Target = JVMMethod;

    fn deref(&self) -> &Self::Target {
        &self.method
    }
}

#[derive(Debug)]
pub struct VMClass {
    name: Arc<str>,
    super_name: Arc<str>,
    methods: Box<[Arc<VMMethod>]>,
    constant_pool: VMConstantPool,
}

impl VMClass {
    pub fn name(&self) -> &Arc<str> {
        &self.name
    }

    pub fn super_name(&self) -> &Arc<str> {
        &self.super_name
    }

    pub fn methods(&self) -> &[Arc<VMMethod>] {
        &self.methods
    }

    pub fn constant_pool(&self) -> &VMConstantPool {
        &self.constant_pool
    }

    pub fn parse(reader: &mut impl ClassReader) -> raw::io::Result<Self> {
        let class = Class::decode(reader)?;
        let methods: Box<[Arc<VMMethod>]> = class
            .methods
            .into_iter()
            .map(|m| Arc::new(VMMethod { method: m }))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let constant_pool =
            VMConstantPool::from_unresolved(&class.constant_pool, &class.this_name, &methods)?;
        Ok(Self {
            name: class.this_name,
            super_name: class.super_name,
            methods,
            constant_pool,
        })
    }
}

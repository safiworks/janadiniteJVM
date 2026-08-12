use std::{
    cell::UnsafeCell,
    fmt::Debug,
    ops::Deref,
    sync::{Arc, OnceLock},
};

use janadinite_parse::{
    self as raw,
    class::{self, Class, ConstantPoolEntry, JVMAccessFlag, JVMField, JVMMethod},
    io::ClassReader,
};

use crate::vm::JVMSlot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldID {
    Static(u16),
    Normal(u32),
}

pub type MethodID = usize;

pub enum VMConstantPoolEntry {
    UTF8(Arc<str>),
    MethodRef {
        unresolved_name: Arc<str>,
        unresolved_class: Arc<str>,
        unresolved_descriptor: Arc<str>,

        resolved: OnceLock<(Arc<VMClass>, MethodID)>,
    },
    // Methods within the same class
    ResolvedMethod(MethodID),
    ResolvedField(FieldID),
    FiledRef {
        unresolved_name: Arc<str>,
        unresolved_class: Arc<str>,
        unresolved_descriptor: Arc<str>,
        resolved: OnceLock<(Arc<VMClass>, FieldID)>,
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
            VMConstantPoolEntry::ResolvedMethod(m) => write!(f, "ResolvedMethod({})", m),
            VMConstantPoolEntry::ResolvedField(fi) => write!(f, "ResolvedField({:?})", fi),

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
        methods: &[VMMethod],
        fields: &[VMField],
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
                            VMConstantPoolEntry::ResolvedMethod(meth.id())
                        } else {
                            VMConstantPoolEntry::MethodRef {
                                unresolved_name: name.clone(),
                                unresolved_class: class_name.clone(),
                                unresolved_descriptor: descriptor.clone(),
                                resolved: OnceLock::new(),
                            }
                        }
                    } else if matches!(entry, ConstantPoolEntry::Fieldref { .. }) {
                        if &**class_name == this_name
                            && let Some(field) = fields.iter().find(|m| {
                                m.name() == &**name && m.raw_descriptor() == &**descriptor
                            })
                        {
                            VMConstantPoolEntry::ResolvedField(field.data.as_id())
                        } else {
                            VMConstantPoolEntry::FiledRef {
                                unresolved_name: name.clone(),
                                unresolved_class: class_name.clone(),
                                unresolved_descriptor: descriptor.clone(),
                                resolved: OnceLock::new(),
                            }
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

#[derive(Debug)]
pub enum FieldData {
    Normal(u32),
    Static(u16, [UnsafeCell<JVMSlot>; 2]),
}
impl FieldData {
    const fn as_id(&self) -> FieldID {
        match self {
            Self::Normal(n) => FieldID::Normal(*n),
            Self::Static(idx, _) => FieldID::Static(*idx),
        }
    }
}

unsafe impl Send for FieldData {}
unsafe impl Sync for FieldData {}

#[derive(Debug)]
pub struct VMField {
    data: FieldData,
    field: JVMField,
}

impl VMField {
    #[inline]
    pub fn as_static(&self) -> Option<&[UnsafeCell<JVMSlot>]> {
        let FieldData::Static(_, ref array) = self.data else {
            return None;
        };

        Some(&array[..self.slot_count()])
    }

    pub const fn id(&self) -> FieldID {
        self.data.as_id()
    }
}

impl Deref for VMField {
    type Target = JVMField;

    fn deref(&self) -> &Self::Target {
        &self.field
    }
}

#[derive(Debug, Clone)]
pub struct VMMethod {
    id: MethodID,
    method: JVMMethod,
}

impl Deref for VMMethod {
    type Target = JVMMethod;

    fn deref(&self) -> &Self::Target {
        &self.method
    }
}

impl VMMethod {
    pub fn id(&self) -> MethodID {
        self.id
    }
}

#[derive(Debug)]
pub struct VMClass {
    name: Arc<str>,
    super_name: Arc<str>,
    methods: Box<[VMMethod]>,
    fields: Box<[VMField]>,
    constant_pool: VMConstantPool,
}

impl VMClass {
    pub fn name(&self) -> &Arc<str> {
        &self.name
    }

    pub fn super_name(&self) -> &Arc<str> {
        &self.super_name
    }

    pub fn methods(&self) -> &[VMMethod] {
        &self.methods
    }

    #[inline]
    pub fn method_by_id(&self, id: MethodID) -> Option<&VMMethod> {
        self.methods().get(id)
    }

    #[inline]
    pub fn method_by_name(&self, name: &str, desc: Option<&str>) -> Option<&VMMethod> {
        self.methods()
            .iter()
            .find(|meth| meth.name() == name && desc.is_none_or(|d| d == meth.raw_descriptor()))
    }

    #[inline]
    pub fn field_by_id(&self, id: FieldID) -> Option<&VMField> {
        match id {
            FieldID::Static(idx) => self.fields.get(idx as usize),
            FieldID::Normal(n) => self.fields.iter().find(|f| match f.data {
                FieldData::Normal(f_n) if f_n == n => true,
                _ => false,
            }),
        }
    }

    #[inline]
    pub fn field_by_name(&self, name: &str) -> Option<&VMField> {
        self.fields.iter().find(|f| f.name() == name)
    }

    pub fn fields_slots(&self) -> usize {
        self.fields.iter().map(|f| f.field.slot_count()).sum()
    }

    pub fn constant_pool(&self) -> &VMConstantPool {
        &self.constant_pool
    }

    pub fn parse(reader: &mut impl ClassReader) -> raw::io::Result<Self> {
        let class = Class::decode(reader)?;
        let methods: Box<[VMMethod]> = class
            .methods
            .into_iter()
            .enumerate()
            .map(|(id, m)| VMMethod {
                id: id as MethodID,
                method: m,
            })
            .collect::<Box<[_]>>();

        let mut curr_off = 0;
        let fields: Box<[VMField]> = class
            .fields
            .into_iter()
            .enumerate()
            .map(|(idx, f)| {
                let is_static = f.access_flags().contains(JVMAccessFlag::ACC_STATIC);
                if !is_static {
                    let f = VMField {
                        data: FieldData::Normal(curr_off),
                        field: f,
                    };

                    curr_off += f.slot_count() as u32;
                    f
                } else {
                    let f = VMField {
                        data: FieldData::Static(
                            idx as u16,
                            [const { UnsafeCell::new(JVMSlot::null()) }; 2],
                        ),
                        field: f,
                    };

                    f
                }
            })
            .collect::<Box<[_]>>();

        let constant_pool = VMConstantPool::from_unresolved(
            &class.constant_pool,
            &class.this_name,
            &methods,
            &fields,
        )?;
        Ok(Self {
            name: class.this_name,
            super_name: class.super_name,
            fields,
            methods,
            constant_pool,
        })
    }
}

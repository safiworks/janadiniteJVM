use std::{
    cell::UnsafeCell,
    fmt::Debug,
    ops::Deref,
    sync::{Arc, OnceLock},
};

use janadinite_parse::{
    self as raw,
    class::{self, Class, ConstantPoolEntry, JVMAccessFlag, JVMCode, JVMField, JVMMethod},
};

use crate::vm::JVMSlot;

pub type FieldOff = u16;

#[derive(Debug, Clone, Copy)]
pub enum MethodID {
    Static(u16),
    Vtable(u16),
}

pub enum VMConstantPoolEntry {
    UTF8(Arc<str>),
    Class(Arc<str>),
    MethodRef {
        unresolved_name: Arc<str>,
        unresolved_class: Arc<str>,
        unresolved_descriptor: Arc<str>,

        resolved: OnceLock<(Arc<VMClass>, MethodID)>,
    },
    // Methods within the same class
    ResolvedMethod(MethodID),
    ResolvedField(FieldOff),
    FiledRef {
        unresolved_name: Arc<str>,
        unresolved_class: Arc<str>,
        unresolved_descriptor: Arc<str>,
        resolved: OnceLock<(Arc<VMClass>, FieldOff)>,
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
            VMConstantPoolEntry::Class(s) => write!(f, "Class({})", s),
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
            VMConstantPoolEntry::ResolvedMethod(m) => write!(f, "ResolvedMethod({:?})", m),
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

    #[inline]
    pub fn from_unresolved<'a>(
        entries: &[ConstantPoolEntry],
        this_name: &str,
        methods: impl Iterator<Item = &'a VMMethod> + Clone,
        fields: &[VMField],
    ) -> raw::io::Result<Self> {
        let mut resolved_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let resolved = match entry {
                ConstantPoolEntry::Utf8 { string } => VMConstantPoolEntry::UTF8(string.clone()),
                ConstantPoolEntry::Class { name_index } => {
                    let class_name =
                        class::get_const!(UTF8 entries, *name_index, "Class class name");

                    VMConstantPoolEntry::Class(class_name.clone())
                }
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
                            && let Some(meth) = methods.clone().find(|m| {
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
                            VMConstantPoolEntry::ResolvedField(field.idx)
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
    Static([UnsafeCell<JVMSlot>; 2]),
}

unsafe impl Send for FieldData {}
unsafe impl Sync for FieldData {}

#[derive(Debug)]
pub struct VMField {
    idx: FieldOff,
    data: FieldData,
    field: JVMField,
}

impl VMField {
    #[inline(always)]
    /// Returns the offset within the object of a non-static field.
    pub fn obj_off(&self) -> Option<u32> {
        match self.data {
            FieldData::Normal(n) => Some(n),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_static(&self) -> Option<&[UnsafeCell<JVMSlot>]> {
        let FieldData::Static(ref array) = self.data else {
            return None;
        };

        Some(&array[..self.slot_count()])
    }
    #[inline(always)]
    pub const fn off(&self) -> FieldOff {
        self.idx
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
    idx: u16,
    method: JVMMethod,
}

impl Deref for VMMethod {
    type Target = JVMMethod;

    fn deref(&self) -> &Self::Target {
        &self.method
    }
}

impl VMMethod {
    pub fn is_static(&self) -> bool {
        self.access_flags.contains(JVMAccessFlag::ACC_STATIC)
    }

    pub fn id(&self) -> MethodID {
        if self.is_static() {
            MethodID::Static(self.idx)
        } else {
            MethodID::Vtable(self.idx)
        }
    }
}

#[derive(Debug)]
pub struct VMClass {
    name: Arc<str>,
    super_class: Option<Arc<VMClass>>,
    vtable: Box<[VMMethod]>,
    static_vtable: Box<[VMMethod]>,
    fields: Box<[VMField]>,
    constant_pool: VMConstantPool,
}

impl VMClass {
    pub fn name(&self) -> &Arc<str> {
        &self.name
    }

    pub fn super_class(&self) -> Option<&Arc<VMClass>> {
        self.super_class.as_ref()
    }

    pub fn vtable(&self) -> &[VMMethod] {
        &self.vtable
    }

    #[inline(always)]
    pub fn method_by_id(&self, id: MethodID) -> Option<&VMMethod> {
        match id {
            MethodID::Vtable(v) => self.vtable().get(v as usize),
            MethodID::Static(v) => self.static_vtable.get(v as usize),
        }
    }

    #[inline]
    pub fn method_by_name(&self, name: &str, desc: Option<&str>) -> Option<&VMMethod> {
        self.vtable()
            .iter()
            .chain(self.static_vtable.iter())
            .find(|meth| meth.name() == name && desc.is_none_or(|d| d == meth.raw_descriptor()))
    }

    #[inline]
    pub fn field_by_idx(&self, off: FieldOff) -> Option<&VMField> {
        self.fields.get(off as usize)
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

    pub fn java_lang_object() -> Self {
        Self {
            name: "java/lang/Object".into(),
            super_class: None,
            vtable: Box::new([VMMethod {
                idx: 0,
                method: JVMMethod {
                    access_flags: JVMAccessFlag::ACC_PUBLIC,
                    attributes: Box::new([]),
                    name: "<init>".into(),
                    descriptor: "()V".into(),
                    args_size: 0,
                    code: Some(JVMCode {
                        max_stack: 1,
                        max_locals: 1,
                        code: Box::new([0xb1]),
                        exception_table: Box::new([]),
                        attributes: Box::new([]),
                    }),
                },
            }]),
            static_vtable: Box::new([]),
            fields: Box::new([]),
            constant_pool: VMConstantPool {
                entries: Box::new([]),
            },
        }
    }

    pub fn create(class: Class, super_class: Option<Arc<VMClass>>) -> raw::io::Result<Self> {
        let mut vtable = super_class
            .as_ref()
            .map(|supe| Vec::from(supe.vtable.clone()))
            .unwrap_or(Vec::new());
        let mut static_vtable = Vec::new();

        for meth in class.methods {
            let is_static = meth.access_flags.contains(JVMAccessFlag::ACC_STATIC);
            if is_static {
                static_vtable.push(VMMethod {
                    idx: static_vtable.len() as u16,
                    method: meth,
                });
                continue;
            }

            if let Some(i) = vtable.iter().position(|m| {
                m.name() == meth.name() && m.raw_descriptor() == meth.raw_descriptor()
            }) {
                vtable[i] = VMMethod {
                    idx: i as u16,
                    method: meth,
                }
            } else {
                vtable.push(VMMethod {
                    idx: vtable.len() as u16,
                    method: meth,
                });
            }
        }

        let mut curr_off = 0;
        let fields: Box<[VMField]> = class
            .fields
            .into_iter()
            .enumerate()
            .map(|(idx, f)| {
                let is_static = f.access_flags().contains(JVMAccessFlag::ACC_STATIC);
                if !is_static {
                    let f = VMField {
                        idx: idx as u16,
                        data: FieldData::Normal(curr_off),
                        field: f,
                    };

                    curr_off += f.slot_count() as u32;
                    f
                } else {
                    let f = VMField {
                        idx: idx as u16,
                        data: FieldData::Static([const { UnsafeCell::new(JVMSlot::null()) }; 2]),
                        field: f,
                    };

                    f
                }
            })
            .collect::<Box<[_]>>();

        let constant_pool = VMConstantPool::from_unresolved(
            &class.constant_pool,
            &class.this_name,
            vtable.iter().chain(static_vtable.iter()),
            &fields,
        )?;
        Ok(Self {
            name: class.this_name,
            super_class,
            fields,
            vtable: vtable.into_boxed_slice(),
            static_vtable: static_vtable.into_boxed_slice(),
            constant_pool,
        })
    }
}

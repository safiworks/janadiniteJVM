use std::{
    collections::HashMap,
    fs::File,
    mem::ManuallyDrop,
    ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Neg, Rem, Sub},
    path::PathBuf,
    sync::{Arc, RwLock},
};

mod class;
pub(crate) mod heap;
pub use class::*;
use janadinite_parse::class::{Class, JVMAccessFlag, JVMCode, OpCode};

use crate::vm::heap::{ObjectKind, ObjectRef};

#[derive(Debug, PartialEq, Eq)]
#[repr(transparent)]
/// A single JVM Slot
/// Currently I store a usize for a slot unfourtantely the JVM spec says that a long value must occupy 2 slots which means a single one is 32bit
/// however objects also use a single slot and this was the only way i figured to store them...
pub struct JVMSlot(isize);

impl JVMSlot {
    pub const fn null() -> Self {
        Self(0)
    }

    pub fn from_object(obj_ref: ObjectRef) -> Self {
        let obj = ((obj_ref.into_ptr() as usize) as isize).wrapping_neg();
        Self(obj)
    }

    pub fn into_object(self) -> Option<ObjectRef> {
        let this = ManuallyDrop::new(self);

        if !this.0.is_negative() {
            // !!!!!
            return None;
        }

        let as_usize = this.0.wrapping_neg() as usize;

        // Safety: self is consumed returning the inner object.
        Some(unsafe { ObjectRef::from_ptr(as_usize as *const _) })
    }

    fn object_clone(&self) -> Option<ObjectRef> {
        // Safety: the object is cloned first
        self.with_object(|r| r.clone())
    }

    #[inline(always)]
    pub fn with_object<R>(&self, f: impl FnOnce(&ObjectRef) -> R) -> Option<R> {
        if !self.0.is_negative() {
            // !!!!!
            return None;
        }

        let as_usize = (-self.0) as usize;
        let obj = ManuallyDrop::new(unsafe { ObjectRef::from_ptr(as_usize as *const _) });
        Some(f(&*obj))
    }

    pub const fn as_i32(&self) -> i32 {
        self.0.cast_unsigned() as i32
    }

    pub const fn from_i32(i: i32) -> Self {
        Self((i as u32 as usize).cast_signed())
    }

    pub const fn from_i64(i: i64) -> [Self; 2] {
        let i = i as u64;

        let low = i as u32;
        let high = (i >> 32) as u32;
        [Self::from_i32(low as i32), Self::from_i32(high as i32)]
    }
}

impl Drop for JVMSlot {
    fn drop(&mut self) {
        if let Some(obj) = core::mem::replace(self, JVMSlot::null()).into_object() {
            drop(obj);
        }
    }
}

impl Clone for JVMSlot {
    fn clone(&self) -> Self {
        if let Some(obj) = self.object_clone() {
            Self::from_object(obj)
        } else {
            Self(self.0)
        }
    }
}

#[derive(Debug, Clone)]
pub enum JVMValue {
    Object(ObjectRef),
    Byte(i8),
    Short(i16),
    Int(i32),
    Float(f32),
    Long(i64),
    Double(f64),
}

impl JVMValue {
    pub const fn slot_count(&self) -> usize {
        match self {
            Self::Long(_) | Self::Double(_) => 2,
            _ => 1,
        }
    }

    /// Converts value to slots, slots must be bigger than or equal to [`Self::slot_count`].
    pub fn to_slots(self, slots: &mut [JVMSlot]) -> &mut [JVMSlot] {
        assert!(slots.len() >= self.slot_count());
        let slots = &mut slots[..self.slot_count()];
        match self {
            Self::Object(obj) => {
                slots[0] = JVMSlot::from_object(obj);
            }
            Self::Byte(b) => {
                slots[0] = JVMSlot::from_i32(b as i32);
            }
            Self::Short(s) => {
                slots[0] = JVMSlot::from_i32(s as i32);
            }
            Self::Int(i) => {
                slots[0] = JVMSlot::from_i32(i);
            }
            Self::Float(f) => {
                slots[0] = JVMSlot::from_i32(f.to_bits() as i32);
            }
            Self::Long(l) => {
                slots.clone_from_slice(&JVMSlot::from_i64(l));
            }
            Self::Double(d) => {
                slots.clone_from_slice(&JVMSlot::from_i64(d.to_bits() as i64));
            }
        }

        slots
    }
}

#[derive(Debug, Clone)]
pub enum VMError {
    StackUnderflow,
    InvalidLocal(u16),
    InvalidMain(&'static str),
    InvalidConstantPoolEntry(u16),
    NoSuchMethodInClass(String),
    NoSuchFieldInClass(String),
    NotStatic(String),
    IsAStatic(String),
    NoSuchClass(String),
    Corrupted(String),
    NotAnObject,
}

/// Describes the beginning of a stack frame in a method.
#[derive(Debug, Clone, Copy, Default)]
struct StackFrame {
    #[allow(unused)]
    meth_ref: u16,
    stack_start: usize,
    #[allow(unused)]
    max_stack: u16,
    locals_start: usize,
}

/// Describes the storage state of the VM thread, such as the stack, heap, and locals.
#[derive(Debug)]
struct ThreadContext {
    stack: Vec<JVMSlot>,
    locals: Vec<JVMSlot>,
    prev_frames: Vec<StackFrame>,
    current_frame: StackFrame,
}

impl ThreadContext {
    /// Creates a new, initial thread context.
    pub fn new(max_stack: u16, max_locals: u16) -> Self {
        Self {
            stack: Vec::with_capacity(max_stack as usize),
            locals: vec![JVMSlot::null(); max_locals as usize],
            prev_frames: Vec::new(),
            current_frame: StackFrame {
                meth_ref: 0,
                stack_start: 0,
                max_stack,
                locals_start: 0,
            },
        }
    }

    /// Must call [`Self::finish_push_frame`] first.
    pub fn pop_create_args(&mut self, args_size: u16) -> Result<&mut [JVMSlot], VMError> {
        let locals_start = self.locals.len();
        self.locals
            .resize(locals_start + args_size as usize, JVMSlot::null());

        for i in (0..args_size).rev() {
            self.locals[locals_start + i as usize] =
                self.stack.pop().ok_or(VMError::StackUnderflow)?;
        }

        Ok(&mut self.locals[locals_start..])
    }

    pub fn finish_push_frame(
        &mut self,
        meth_ref: u16,
        max_locals: u16,
        max_stack: u16,
        args_size: u16,
    ) {
        assert!(max_locals >= args_size);
        let locals_start = self.locals.len() - args_size as usize;
        self.locals
            .resize(locals_start + max_locals as usize, JVMSlot::null());

        self.stack.reserve(max_stack as usize);
        self.prev_frames.push(self.current_frame);
        self.current_frame = StackFrame {
            meth_ref,
            max_stack,
            locals_start,
            stack_start: self.stack.len(),
        };
    }

    /// Pushes a new stack frame onto the frame stack.
    ///
    /// Same as calling [`Self::pop_create_args`] and then [`Self::finish_push_frame`] immediately.
    pub fn push_frame(
        &mut self,
        meth_ref: u16,
        max_stack: u16,
        max_locals: u16,
        args_size: u16,
    ) -> Result<(), VMError> {
        self.pop_create_args(args_size)?;
        self.finish_push_frame(meth_ref, max_locals, max_stack, args_size);
        Ok(())
    }

    /// Pops the current stack frame from the frame stack.
    pub fn pop_frame(&mut self) {
        let popped_frame = self.current_frame;
        self.current_frame = self.prev_frames.pop().unwrap_or_default();

        self.locals.truncate(popped_frame.locals_start);
        self.stack.truncate(popped_frame.stack_start);
    }

    #[inline(always)]
    pub fn push_value(&mut self, v: JVMValue) {
        let idx = self.stack.len();
        self.stack.resize(idx + v.slot_count(), JVMSlot::null());
        let slots = &mut self.stack[idx..];
        v.to_slots(slots);
    }

    /// Pushes an integer value onto the stack.
    #[inline(always)]
    pub fn ipush(&mut self, v: i32) {
        self.stack.push(JVMSlot::from_i32(v));
    }

    /// Pushes a float value onto the stack.
    #[inline(always)]
    pub fn fpush(&mut self, v: f32) {
        self.ipush(v.to_bits() as i32);
    }

    #[inline(always)]
    pub fn lpush(&mut self, v: i64) {
        self.push_value(JVMValue::Long(v));
    }

    #[inline(always)]
    pub fn dpush(&mut self, v: f64) {
        self.push_value(JVMValue::Double(v));
    }

    #[inline(always)]
    pub fn push_slot(&mut self, slot: JVMSlot) {
        self.stack.push(slot);
    }

    #[inline(always)]
    pub fn apush(&mut self, object: ObjectRef) {
        self.push_slot(JVMSlot::from_object(object));
    }

    #[inline(always)]
    pub fn pop_slot(&mut self) -> Option<JVMSlot> {
        if self.stack.len() <= self.current_frame.stack_start {
            return None;
        }
        self.stack.pop()
    }

    /// Pops an integer value from the stack.
    #[inline(always)]
    pub fn ipop(&mut self) -> Result<i32, VMError> {
        if self.stack.len() <= self.current_frame.stack_start {
            return Err(VMError::StackUnderflow);
        }
        self.stack
            .pop()
            .map(|v| v.as_i32())
            .ok_or(VMError::StackUnderflow)
    }

    #[inline(always)]
    pub fn fpop(&mut self) -> Result<f32, VMError> {
        self.ipop().map(|i| f32::from_bits(i as u32))
    }

    #[inline(always)]
    pub fn apop(&mut self) -> Result<ObjectRef, VMError> {
        self.pop_slot()
            .ok_or(VMError::StackUnderflow)?
            .into_object()
            .ok_or(VMError::NotAnObject)
    }

    #[inline(always)]
    pub fn lpop(&mut self) -> Result<i64, VMError> {
        let high = self.ipop()? as u32;
        let low = self.ipop()? as u32;

        Ok(((low as u64) | (high as u64) << 32) as i64)
    }

    #[inline(always)]
    pub fn dpop(&mut self) -> Result<f64, VMError> {
        self.lpop().map(|l| f64::from_bits(l as u64))
    }

    /// Loads an integer value from the local variable at the given index.
    #[inline(always)]
    pub fn iload(&mut self, index: u16) -> Result<i32, VMError> {
        self.locals
            .get(index as usize + self.current_frame.locals_start)
            .map(|v| v.as_i32())
            .ok_or(VMError::InvalidLocal(index))
    }

    /// Loads a float value from the local variable at the given index.
    #[inline(always)]
    pub fn fload(&mut self, index: u16) -> Result<f32, VMError> {
        self.iload(index).map(|i| f32::from_bits(i as u32))
    }

    /// Loads a long value from the local variable at the given index.
    #[inline(always)]
    pub fn lload(&mut self, index: u16) -> Result<i64, VMError> {
        let low = self.iload(index)? as u32;
        let high = self.iload(index + 1)? as u32;

        Ok((low as u64 | ((high as u64) << 32)) as i64)
    }

    /// Loads a double value from the local variable at the given index.
    #[inline(always)]
    pub fn dload(&mut self, index: u16) -> Result<f64, VMError> {
        self.lload(index).map(|l| f64::from_bits(l as u64))
    }

    #[inline(always)]
    pub fn aload(&mut self, index: u16) -> Result<ObjectRef, VMError> {
        self.locals
            .get(index as usize + self.current_frame.locals_start)
            .ok_or(VMError::InvalidLocal(index))?
            .object_clone()
            .ok_or(VMError::NotAnObject)
    }

    /// Stores an integer value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn astore(&mut self, index: u16, value: ObjectRef) -> bool {
        self.vstore(index, JVMValue::Object(value))
    }

    /// Stores an integer value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn istore(&mut self, index: u16, value: i32) -> bool {
        self.vstore(index, JVMValue::Int(value))
    }

    /// Stores a float value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn fstore(&mut self, index: u16, value: f32) -> bool {
        self.vstore(index, JVMValue::Float(value))
    }

    /// Stores a long value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn lstore(&mut self, index: u16, value: i64) -> bool {
        self.vstore(index, JVMValue::Long(value))
    }

    /// Stores a double value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn dstore(&mut self, index: u16, value: f64) -> bool {
        self.vstore(index, JVMValue::Double(value))
    }

    /// Stores a value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn vstore(&mut self, index: u16, value: JVMValue) -> bool {
        let idx = index as usize + self.current_frame.locals_start;
        if let Some(locals) = self.locals.get_mut(idx..idx + value.slot_count()) {
            value.to_slots(locals);
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn iget(&mut self, index: u16) -> Option<&mut JVMSlot> {
        self.locals
            .get_mut(index as usize + self.current_frame.locals_start)
    }

    #[inline(always)]
    pub fn dup(&mut self) {
        let slot = self.pop_slot();
        if let Some(slot) = slot {
            self.push_slot(slot.clone());
            self.push_slot(slot);
        }
    }

    #[inline(always)]
    pub fn dup2(&mut self) {
        let slot1 = self.pop_slot();
        let slot2 = self.pop_slot();

        if let Some(slot2) = slot2.clone() {
            self.push_slot(slot2);
        }

        if let Some(slot1) = slot1.clone() {
            self.push_slot(slot1);
        }

        if let Some(slot2) = slot2 {
            self.push_slot(slot2);
        }

        if let Some(slot1) = slot1 {
            self.push_slot(slot1);
        }
    }
}

#[derive(Debug)]
pub struct VM {
    main_class: Arc<VMClass>,
    loaded_classes: RwLock<HashMap<Arc<str>, Arc<VMClass>>>,
    classpath: PathBuf,
}

impl VM {
    fn from_class_path(classpath: PathBuf) -> Self {
        let mut loaded_classes = HashMap::with_capacity(1);

        let java = Arc::new(VMClass::java_lang_object());
        loaded_classes.insert(java.name().clone(), java.clone());
        Self {
            classpath,
            main_class: java,
            loaded_classes: RwLock::new(loaded_classes),
        }
    }
    pub fn open(classpath: PathBuf, main_class: &str) -> Result<Self, VMError> {
        let mut this = Self::from_class_path(classpath);
        let main_class = this.with_class_or_load_by_name(main_class, |c| Ok(c.clone()))?;

        println!("{main_class:#?}");

        this.main_class = main_class;
        Ok(this)
    }

    #[inline]
    fn load_class_by_name<'s>(
        &'s self,
        mut write_guard: std::sync::RwLockWriteGuard<'s, HashMap<Arc<str>, Arc<VMClass>>>,
        name: &str,
    ) -> Result<Arc<VMClass>, VMError> {
        if let Some(class) = write_guard.get(name) {
            return Ok(class.clone());
        }

        let path = self.classpath.join(name).with_extension("class");
        let mut file = File::open(path).map_err(|_| VMError::NoSuchClass(name.into()))?;
        let raw_class =
            Class::decode(&mut file).map_err(|e| VMError::Corrupted(e.message().into()))?;

        let super_class = if raw_class.super_name.is_empty() {
            None
        } else {
            let sup = self.load_class_by_name(write_guard, &raw_class.super_name)?;
            write_guard = self
                .loaded_classes
                .write()
                .expect("Failed to reacquire lock on loaded classes");

            if let Some(class) = write_guard.get(name) {
                return Ok(class.clone());
            }
            Some(sup)
        };

        let class = VMClass::create(raw_class, super_class)
            .map_err(|e| VMError::Corrupted(e.message().into()))?;

        let class = Arc::new(class);
        write_guard.insert(name.into(), class.clone());
        drop(write_guard);

        if let Some(clinit) = class.method_by_name("<clinit>", Some("()V")) {
            let code = clinit.code().expect("<clinit> no code");
            self.run_code(
                &class,
                &mut ThreadContext::new(code.max_stack(), code.max_locals()),
                code,
            )
            .expect("<clinit> run failed");
        }

        Ok(class)
    }

    fn with_class_or_load_by_name<R>(
        &self,
        name: &str,
        f: impl FnOnce(&Arc<VMClass>) -> Result<R, VMError>,
    ) -> Result<R, VMError> {
        let loaded_classes = self
            .loaded_classes
            .read()
            .expect("failed to lock loaded_classes");
        if let Some(class) = loaded_classes.get(name) {
            return f(class);
        } else {
            drop(loaded_classes);
            let write_guard = self
                .loaded_classes
                .write()
                .expect("failed to lock loaded_classes");
            if let Some(class) = write_guard.get(name) {
                return f(class);
            }
            f(&self.load_class_by_name(write_guard, name)?)
        }
    }

    fn with_class_or_load_by_ref<R>(
        &self,
        this_class: &VMClass,
        ref_idx: u16,
        f: impl FnOnce(&Arc<VMClass>) -> Result<R, VMError>,
    ) -> Result<R, VMError> {
        let name = this_class
            .constant_pool()
            .get_class(ref_idx)
            .ok_or(VMError::InvalidConstantPoolEntry(ref_idx))?;

        self.with_class_or_load_by_name(&*name, f)
    }

    fn get_method_or_resolve<'c>(
        &self,
        this_class: &'c VMClass,
        ref_idx: u16,
    ) -> Result<(&'c VMClass, MethodID), VMError> {
        let entry = this_class
            .constant_pool()
            .get_entry(ref_idx)
            .ok_or(VMError::InvalidConstantPoolEntry(ref_idx))?;

        match entry {
            VMConstantPoolEntry::MethodRef {
                class,
                name_and_desc,
                resolved,
                ..
            } => {
                if let Some((class, method)) = resolved.get() {
                    Ok((class, *method))
                } else {
                    let unresolved_class = this_class
                        .constant_pool()
                        .get_class(*class)
                        .expect("MethodRef points to invalid class");
                    let (unresolved_name, unresolved_descriptor) = this_class
                        .constant_pool()
                        .get_name_and_type(*name_and_desc)
                        .expect("MethodRef points to invalid NameAndType");

                    let (class, method) =
                        self.with_class_or_load_by_name(&*unresolved_class, |class| {
                            class
                                .method_by_name(&*unresolved_name, Some(&*unresolved_descriptor))
                                .map(|m| (class.clone(), m.id()))
                                .ok_or_else(|| {
                                    VMError::NoSuchMethodInClass(String::from(&**unresolved_name))
                                })
                        })?;

                    let (class, method) = resolved.get_or_init(|| (class, method));
                    Ok((class, *method))
                }
            }
            VMConstantPoolEntry::ResolvedMethod(m) => Ok((this_class, *m)),
            _ => Err(VMError::InvalidConstantPoolEntry(ref_idx)),
        }
    }

    fn get_field_or_resolve<'c>(
        &self,
        this_class: &'c VMClass,
        ref_idx: u16,
    ) -> Result<(&'c VMClass, FieldOff), VMError> {
        let entry = this_class
            .constant_pool()
            .get_entry(ref_idx)
            .ok_or(VMError::InvalidConstantPoolEntry(ref_idx))?;

        match entry {
            VMConstantPoolEntry::FiledRef {
                class,
                name_and_desc,
                resolved,
                ..
            } => {
                if let Some((class, method)) = resolved.get() {
                    Ok((class, *method))
                } else {
                    let unresolved_class = this_class
                        .constant_pool()
                        .get_class(*class)
                        .expect("MethodRef points to invalid class");
                    let (unresolved_name, _) = this_class
                        .constant_pool()
                        .get_name_and_type(*name_and_desc)
                        .expect("MethodRef points to invalid NameAndType");

                    let (class, field) =
                        self.with_class_or_load_by_name(&*unresolved_class, |class| {
                            class
                                .field_by_name(&*unresolved_name)
                                .map(|f| (class.clone(), f.off()))
                                .ok_or_else(|| {
                                    VMError::NoSuchFieldInClass(String::from(&**unresolved_name))
                                })
                        })?;

                    let (class, field) = resolved.get_or_init(|| (class, field));
                    Ok((class, *field))
                }
            }
            VMConstantPoolEntry::ResolvedField(f) => Ok((this_class, *f)),
            _ => Err(VMError::InvalidConstantPoolEntry(ref_idx)),
        }
    }

    fn run_code(
        &self,
        class: &VMClass,
        context: &mut ThreadContext,
        code: &JVMCode,
    ) -> Result<Option<JVMValue>, VMError> {
        let mut last_pc: u16 = 0;
        let mut instr = code.instructions();

        macro_rules! return_ {
            ($var:ident, $pop_method:ident) => {
                return Ok(Some(JVMValue::$var(context.$pop_method()?)))
            };
        }

        /// OpCodes that do x OP y => stack
        macro_rules! op2_1 {
            ($pop:ident, $push:ident, $m:ident) => {{
                let v2 = context.$pop()?;
                let v1 = context.$pop()?;
                context.$push(v1.$m(v2));
            }};
            (i $m: ident) => {
                op2_1!(ipop, ipush, $m)
            };
            (f $m: ident) => {
                op2_1!(fpop, fpush, $m)
            };
            (l $m: ident) => {
                op2_1!(lpop, lpush, $m)
            };
            (d $m: ident) => {
                op2_1!(dpop, dpush, $m)
            };
        }

        /// OpCodes that do OP x => stack
        macro_rules! op1_1 {
            ($pop:ident, $push:ident, $m:ident) => {{
                let v1 = context.$pop()?;
                context.$push(v1.$m());
            }};
            (i $m: ident) => {
                op1_1!(ipop, ipush, $m)
            };
            (f $m: ident) => {
                op1_1!(fpop, fpush, $m)
            };
            (l $m: ident) => {
                op1_1!(lpop, lpush, $m)
            };
            (d $m: ident) => {
                op1_1!(dpop, dpush, $m)
            };
        }

        /// OpCodes that do locals => stack
        macro_rules! load {
            ($load: ident, $push: ident, $idx: expr) => {{
                let v = context.$load($idx as u16)?;
                context.$push(v);
            }};
        }

        /// Opcodes that do stack => locals
        macro_rules! store {
            ($store: ident, $pop: ident, $idx:expr) => {{
                let v = context.$pop()?;
                if !context.$store($idx as u16, v) {
                    return Err(VMError::InvalidLocal($idx as u16));
                }
            }};
        }

        #[inline(always)]
        fn ldc(
            pool: &VMConstantPool,
            idx: u16,
            context: &mut ThreadContext,
        ) -> Result<(), VMError> {
            match pool.get_entry(idx) {
                Some(VMConstantPoolEntry::Float(f)) => {
                    context.fpush(*f);
                    Ok(())
                }
                Some(VMConstantPoolEntry::Int(i)) => {
                    context.ipush(*i);
                    Ok(())
                }
                _ => Err(VMError::InvalidConstantPoolEntry(idx)),
            }
        }
        while let Some(opcode) = instr.next_op() {
            match opcode {
                OpCode::Lconst0 => context.lpush(0),
                OpCode::Lconst1 => context.lpush(1),
                OpCode::Fconst0 => context.fpush(0.),
                OpCode::Fconst1 => context.fpush(1.),
                OpCode::Fconst2 => context.fpush(2.),
                OpCode::Dconst0 => context.dpush(0.),
                OpCode::Dconst1 => context.dpush(1.),

                OpCode::Ldc(idx) => ldc(class.constant_pool(), idx as u16, context)?,
                OpCode::LdcW(idx) => ldc(class.constant_pool(), idx, context)?,
                OpCode::Ldc2W(ref_idx) => match class.constant_pool().get_entry(ref_idx) {
                    Some(VMConstantPoolEntry::Double(d)) => {
                        context.dpush(*d);
                    }
                    Some(VMConstantPoolEntry::Long(l)) => {
                        context.lpush(*l);
                    }
                    _ => return Err(VMError::InvalidConstantPoolEntry(ref_idx)),
                },
                OpCode::InvokeStatic(ref_idx) | OpCode::InvokeSpecial(ref_idx) => {
                    heap::safepoint();

                    let (meth_class, method_id) = self.get_method_or_resolve(class, ref_idx)?;
                    let method = meth_class.method_by_id(method_id).unwrap();
                    let code = method.code().expect("TODO: methods without code");

                    context.push_frame(
                        ref_idx,
                        code.max_stack(),
                        code.max_locals(),
                        method.args_size() as u16
                            + if matches!(opcode, OpCode::InvokeSpecial(_)) {
                                1
                            } else {
                                0
                            },
                    )?;

                    let result = self.run_code(meth_class, context, &code)?;
                    context.pop_frame();
                    if let Some(result) = result {
                        context.push_value(result);
                    }
                }
                OpCode::InvokeVirtual(ref_idx) => {
                    heap::safepoint();

                    let (meth_class, method_id) = self.get_method_or_resolve(class, ref_idx)?;

                    let method = meth_class.method_by_id(method_id).unwrap();
                    let args =
                        context.pop_create_args(method.args_size() as u16 + 1 /* object */)?;
                    let class = args[0]
                        .with_object(|object| {
                            let ObjectKind::Instance { ref class } = object.kind else {
                                return Err(VMError::NotAnObject);
                            };

                            Ok(class.clone())
                        })
                        .ok_or(VMError::NotAnObject)??;

                    let obj_meth = class
                        .method_by_name(method.name(), Some(method.raw_descriptor()))
                        .expect("Couldn't search for virtual method");

                    let code = obj_meth.code().expect("FIXME: Handle methods without code");
                    context.finish_push_frame(
                        ref_idx,
                        code.max_locals(),
                        code.max_stack(),
                        method.args_size() as u16 + 1,
                    );

                    let result = self.run_code(&class, context, &code)?;
                    context.pop_frame();
                    if let Some(result) = result {
                        context.push_value(result);
                    }
                }
                OpCode::Getstatic(ref_idx) => {
                    let (field_class, field_idx) = self.get_field_or_resolve(class, ref_idx)?;
                    let field = field_class.field_by_idx(field_idx).unwrap();
                    let static_data = field
                        .as_static()
                        .ok_or_else(|| VMError::NotStatic(field.name().into()))?;

                    for slot in static_data.iter().rev() {
                        // Safety: object access is not sync by default, statics are initialized once (?)
                        context.push_slot(unsafe { &*slot.get() }.clone());
                    }
                }
                OpCode::Putstatic(ref_idx) => {
                    let (field_class, field_idx) = self.get_field_or_resolve(class, ref_idx)?;
                    let field = field_class.field_by_idx(field_idx).unwrap();
                    let static_data = field
                        .as_static()
                        .ok_or_else(|| VMError::NotStatic(field.name().into()))?;

                    for slot in static_data {
                        unsafe { *slot.get() = context.pop_slot().ok_or(VMError::StackUnderflow)? };
                    }
                }
                OpCode::GetField(ref_idx) | OpCode::PutField(ref_idx) => {
                    let (field_class, field_idx) = self.get_field_or_resolve(class, ref_idx)?;
                    let field = field_class.field_by_idx(field_idx).unwrap();
                    let slots = field.slot_count();
                    let off = field
                        .obj_off()
                        .ok_or_else(|| VMError::IsAStatic(field.name().into()))?;

                    // Safety: object isn't expected static...
                    match opcode {
                        OpCode::GetField(_) => {
                            let object = context
                                .pop_slot()
                                .ok_or(VMError::StackUnderflow)?
                                .into_object()
                                .ok_or(VMError::NotAnObject)?;

                            let slots = &object.data[off as usize..off as usize + slots];
                            for slot in slots {
                                context.push_slot(unsafe { &*slot.get() }.clone());
                            }
                        }
                        OpCode::PutField(_) => {
                            let got_slot = context.pop_slot().ok_or(VMError::StackUnderflow)?;
                            let object = context
                                .pop_slot()
                                .ok_or(VMError::StackUnderflow)?
                                .into_object()
                                .ok_or(VMError::NotAnObject)?;

                            let slots = &object.data[off as usize..off as usize + slots];
                            unsafe { *slots[0].get() = got_slot };
                        }
                        _ => unreachable!(),
                    }
                }
                OpCode::Bipush(b) => context.ipush(b as i32),

                OpCode::Iload(idx) => load!(iload, ipush, idx),
                OpCode::Fload(idx) => load!(fload, fpush, idx),
                OpCode::Lload(idx) => load!(lload, lpush, idx),
                OpCode::Dload(idx) => load!(dload, dpush, idx),
                OpCode::Aload(idx) => load!(aload, apush, idx),

                OpCode::WideIload(idx) => load!(iload, ipush, idx),
                OpCode::WideFload(idx) => load!(fload, fpush, idx),
                OpCode::WideLload(idx) => load!(lload, lpush, idx),
                OpCode::WideDload(idx) => load!(dload, dpush, idx),
                OpCode::WideAload(idx) => load!(aload, apush, idx),

                OpCode::AStore(idx) => store!(astore, apop, idx),
                OpCode::IStore(idx) => store!(istore, ipop, idx),
                OpCode::FStore(idx) => store!(fstore, fpop, idx),
                OpCode::LStore(idx) => store!(lstore, lpop, idx),
                OpCode::DStore(idx) => store!(dstore, dpop, idx),

                OpCode::WideAStore(idx) => store!(astore, apop, idx),
                OpCode::WideIStore(idx) => store!(istore, ipop, idx),
                OpCode::WideFStore(idx) => store!(fstore, fpop, idx),
                OpCode::WideLStore(idx) => store!(lstore, lpop, idx),
                OpCode::WideDStore(idx) => store!(dstore, dpop, idx),

                OpCode::Iinc(idx, imm) => {
                    context
                        .iget(idx as u16)
                        .map(|l| {
                            let v = l.as_i32();
                            *l = JVMSlot::from_i32(v + imm as i32)
                        })
                        .ok_or(VMError::InvalidLocal(idx as u16))?;
                }
                OpCode::WideIinc(idx, imm) => {
                    context
                        .iget(idx as u16)
                        .map(|l| {
                            let v = l.as_i32();
                            *l = JVMSlot::from_i32(v + imm as i32)
                        })
                        .ok_or(VMError::InvalidLocal(idx as u16))?;
                }

                OpCode::Imul => op2_1!(i wrapping_mul),
                OpCode::Lmul => op2_1!(l wrapping_mul),
                OpCode::Fmul => op2_1!(f mul),
                OpCode::Dmul => op2_1!(d mul),

                OpCode::Idiv => op2_1!(i wrapping_div),
                OpCode::Ldiv => op2_1!(l wrapping_div),
                OpCode::Fdiv => op2_1!(f div),
                OpCode::Ddiv => op2_1!(d div),

                OpCode::Isub => op2_1!(i wrapping_sub),
                OpCode::Lsub => op2_1!(l wrapping_sub),
                OpCode::Fsub => op2_1!(f sub),
                OpCode::Dsub => op2_1!(d sub),

                OpCode::Iadd => op2_1!(i wrapping_add),
                OpCode::Ladd => op2_1!(l wrapping_add),
                OpCode::Fadd => op2_1!(f add),
                OpCode::Dadd => op2_1!(d add),

                OpCode::Ineg => op1_1!(i wrapping_neg),
                OpCode::Lneg => op1_1!(l wrapping_neg),
                OpCode::Fneg => op1_1!(f neg),
                OpCode::Dneg => op1_1!(d neg),

                OpCode::Irem => op2_1!(i wrapping_rem),
                OpCode::Lrem => op2_1!(l wrapping_rem),
                OpCode::Frem => op2_1!(f rem),
                OpCode::Drem => op2_1!(d rem),

                OpCode::Iand => op2_1!(i bitand),
                OpCode::Land => op2_1!(l bitand),
                OpCode::Ior => op2_1!(i bitor),
                OpCode::Lor => op2_1!(l bitor),
                OpCode::Ixor => op2_1!(i bitxor),
                OpCode::Lxor => op2_1!(l bitxor),
                OpCode::Ishl => {
                    let v2 = context.ipop()?;
                    let v1 = context.ipop()?;
                    context.ipush(v1 << (v2 as u32 & 0b11111));
                }
                OpCode::Lshl => {
                    let v2 = context.lpop()?;
                    let v1 = context.lpop()?;
                    context.lpush(v1 << (v2 as u32 & 0b111111));
                }

                OpCode::Ishr => {
                    let v2 = context.ipop()?;
                    let v1 = context.ipop()?;
                    context.ipush(v1 >> (v2 as u32 & 0b11111));
                }
                OpCode::Lshr => {
                    let v2 = context.lpop()?;
                    let v1 = context.lpop()?;
                    context.lpush(v1 >> (v2 as u32 & 0b111111));
                }

                OpCode::Iushr => {
                    let v2 = context.ipop()?;
                    let v1 = context.ipop()?;
                    context.ipush((v1 as u32 >> (v2 as u32 & 0b11111)) as i32);
                }
                OpCode::Lushr => {
                    let v2 = context.lpop()?;
                    let v1 = context.lpop()?;
                    context.lpush((v1 as u64 >> (v2 as u32 & 0b111111)) as i64);
                }
                OpCode::I2l => {
                    let v = context.ipop()?;
                    context.lpush(v as i64);
                }
                OpCode::I2f => {
                    let v = context.ipop()?;
                    context.fpush(v as f32);
                }
                OpCode::I2d => {
                    let v = context.ipop()?;
                    context.dpush(v as f64);
                }
                OpCode::Lcmp => {
                    let v2 = context.lpop()?;
                    let v1 = context.lpop()?;

                    let res = match v1.cmp(&v2) {
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                        std::cmp::Ordering::Less => -1,
                    };

                    context.ipush(res);
                }
                OpCode::Fcmpg | OpCode::Fcmpl => {
                    let v2 = context.fpop()?;
                    let v1 = context.fpop()?;

                    let res = match v1.partial_cmp(&v2) {
                        Some(std::cmp::Ordering::Equal) => 0,
                        Some(std::cmp::Ordering::Greater) => 1,
                        Some(std::cmp::Ordering::Less) => -1,
                        None => {
                            if matches!(opcode, OpCode::Fcmpl) {
                                -1
                            } else {
                                1
                            }
                        }
                    };

                    context.ipush(res);
                }
                OpCode::Dcmpg | OpCode::Dcmpl => {
                    let v2 = context.dpop()?;
                    let v1 = context.dpop()?;

                    let res = match v1.partial_cmp(&v2) {
                        Some(std::cmp::Ordering::Equal) => 0,
                        Some(std::cmp::Ordering::Greater) => 1,
                        Some(std::cmp::Ordering::Less) => -1,
                        None => {
                            if matches!(opcode, OpCode::Dcmpl) {
                                -1
                            } else {
                                1
                            }
                        }
                    };

                    context.ipush(res);
                }
                OpCode::Goto(off) => {
                    instr.set_pc(last_pc.saturating_add_signed(off));
                }
                OpCode::IfIcmpEq(off) | OpCode::IfIcmpNe(off) => {
                    let v2 = context.ipop()?;
                    let v1 = context.ipop()?;

                    match (v1.cmp(&v2), opcode) {
                        (std::cmp::Ordering::Equal, OpCode::IfIcmpEq(_))
                        | (
                            std::cmp::Ordering::Greater | std::cmp::Ordering::Less,
                            OpCode::IfIcmpNe(_),
                        ) => {
                            instr.set_pc(last_pc.saturating_add_signed(off));
                        }
                        _ => {}
                    }
                }
                OpCode::IfNe(off) | OpCode::IfEq(off) => {
                    let v1 = context.ipop()?;
                    let v2 = 0;

                    match (v1.cmp(&v2), opcode) {
                        (std::cmp::Ordering::Equal, OpCode::IfEq(_))
                        | (
                            std::cmp::Ordering::Greater | std::cmp::Ordering::Less,
                            OpCode::IfNe(_),
                        ) => {
                            instr.set_pc(last_pc.saturating_add_signed(off));
                        }
                        _ => {}
                    }
                }
                OpCode::Return => return Ok(None),
                OpCode::IReturn => return_!(Int, ipop),
                OpCode::AReturn => return_!(Object, apop),
                OpCode::LReturn => return_!(Long, lpop),
                OpCode::FReturn => return_!(Float, fpop),
                OpCode::DReturn => return_!(Double, dpop),
                OpCode::New(class_ref) => {
                    let new_class =
                        self.with_class_or_load_by_ref(class, class_ref, |c| Ok(c.clone()))?;
                    // Safety: Thread was already registered before this call.
                    let instance = unsafe { heap::allocate(new_class) };
                    context.push_slot(JVMSlot::from_object(instance));
                }
                OpCode::Dup => context.dup(),
                OpCode::Dup2 => context.dup2(),
                OpCode::Dup2X1 | OpCode::Dup2X2 | OpCode::DupX1 | OpCode::DupX2 => todo!("dupN_xY"),
                OpCode::Invalid(i) => {
                    eprintln!("Invalid OpCode: {i}");
                }
            }

            last_pc = instr.pc();
        }

        Ok(None)
    }
    pub fn run_main(&self) -> Result<Option<JVMValue>, VMError> {
        let main_class = &self.main_class;

        let meth = main_class
            .method_by_name("main", None)
            .ok_or_else(|| VMError::NoSuchMethodInClass("main".to_string()))?;

        if !meth.access_flags().contains(JVMAccessFlag::ACC_STATIC) {
            return Err(VMError::InvalidMain("not static"));
        }

        let Some(code) = meth.code() else {
            return Err(VMError::InvalidMain("no code"));
        };

        let mut context = ThreadContext::new(code.max_stack(), code.max_locals());
        self.run_code(&*main_class, &mut context, &code)
    }
}

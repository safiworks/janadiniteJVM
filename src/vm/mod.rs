use std::{
    collections::HashMap,
    fs::File,
    mem::ManuallyDrop,
    path::PathBuf,
    sync::{Arc, RwLock},
};

mod class;
pub(crate) mod heap;
pub use class::*;
use janadinite_parse::class::{JVMAccessFlag, JVMCode, OpCode};

use crate::vm::heap::ObjectRef;

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

    /// Pushes an integer value onto the stack.
    #[inline(always)]
    pub fn ipush(&mut self, v: i32) {
        self.stack.push(JVMSlot::from_i32(v));
    }

    #[inline(always)]
    pub fn push_slot(&mut self, slot: JVMSlot) {
        self.stack.push(slot);
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
    pub fn ipop(&mut self) -> Option<i32> {
        if self.stack.len() <= self.current_frame.stack_start {
            return None;
        }
        self.stack.pop().map(|v| v.as_i32())
    }

    #[inline(always)]
    pub fn apop(&mut self) -> Option<ObjectRef> {
        self.pop_slot().and_then(|a| a.into_object())
    }

    /// Loads an integer value from the local variable at the given index.
    #[inline(always)]
    pub fn iload(&mut self, index: u16) -> Option<i32> {
        self.locals
            .get(index as usize + self.current_frame.locals_start)
            .map(|v| v.as_i32())
    }

    #[inline(always)]
    pub fn aload(&mut self, index: u16) -> Option<ObjectRef> {
        self.locals
            .get(index as usize + self.current_frame.locals_start)
            .and_then(|v| v.object_clone())
    }

    /// Stores an integer value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn astore(&mut self, index: u16, value: ObjectRef) -> bool {
        if let Some(local) = self
            .locals
            .get_mut(index as usize + self.current_frame.locals_start)
        {
            *local = JVMSlot::from_object(value);
            true
        } else {
            false
        }
    }

    /// Stores an integer value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn istore(&mut self, index: u16, value: i32) -> bool {
        if let Some(local) = self
            .locals
            .get_mut(index as usize + self.current_frame.locals_start)
        {
            *local = JVMSlot::from_i32(value);
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
    pub fn open(classpath: PathBuf, main_class: &str) -> std::io::Result<Self> {
        let main_class_path = classpath.join(main_class).with_extension("class");
        let mut main_class_file = File::open(main_class_path)?;
        let main_class = Arc::new(VMClass::parse(&mut main_class_file)?);

        println!("{main_class:#?}");
        let mut loaded_classes = HashMap::with_capacity(1);
        loaded_classes.insert(main_class.name().clone(), main_class.clone());

        let java = Arc::new(VMClass::java_lang_object());
        loaded_classes.insert(java.name().clone(), java);
        Ok(Self {
            classpath,
            main_class,
            loaded_classes: RwLock::new(loaded_classes),
        })
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
            let mut write_guard = self
                .loaded_classes
                .write()
                .expect("failed to lock loaded_classes");
            if let Some(class) = write_guard.get(name) {
                return f(class);
            }

            let path = self.classpath.join(name).with_extension("class");
            let mut file = File::open(path).map_err(|_| VMError::NoSuchClass(name.into()))?;
            let class =
                VMClass::parse(&mut file).map_err(|e| VMError::Corrupted(e.message().into()))?;

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
            f(&class)
        }
    }

    fn with_class_or_load_by_ref<R>(
        &self,
        this_class: &VMClass,
        ref_idx: u16,
        f: impl FnOnce(&Arc<VMClass>) -> Result<R, VMError>,
    ) -> Result<R, VMError> {
        let entry = this_class
            .constant_pool()
            .get_entry(ref_idx)
            .ok_or(VMError::InvalidConstantPoolEntry(ref_idx))?;

        match entry {
            VMConstantPoolEntry::Class(name) => self.with_class_or_load_by_name(&*name, f),
            _ => Err(VMError::InvalidConstantPoolEntry(ref_idx)),
        }
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
                resolved,
                unresolved_class,
                unresolved_name,
                unresolved_descriptor,
                ..
            } => {
                if let Some((class, method)) = resolved.get() {
                    Ok((class, *method))
                } else {
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
    ) -> Result<(&'c VMClass, FieldID), VMError> {
        let entry = this_class
            .constant_pool()
            .get_entry(ref_idx)
            .ok_or(VMError::InvalidConstantPoolEntry(ref_idx))?;

        match entry {
            VMConstantPoolEntry::FiledRef {
                resolved,
                unresolved_class,
                unresolved_name,
                ..
            } => {
                if let Some((class, method)) = resolved.get() {
                    Ok((class, *method))
                } else {
                    let (class, field) =
                        self.with_class_or_load_by_name(&*unresolved_class, |class| {
                            class
                                .field_by_name(&*unresolved_name)
                                .map(|f| (class.clone(), f.id()))
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
    ) -> Result<Option<i32>, VMError> {
        let mut last_pc: u16 = 0;
        let mut instr = code.instructions();

        while let Some(opcode) = instr.next_op() {
            match opcode {
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
                        context.ipush(result);
                    }
                }
                OpCode::InvokeVirtual(ref_idx) => {
                    heap::safepoint();

                    let (meth_class, method_id) = self.get_method_or_resolve(class, ref_idx)?;

                    let method = meth_class.method_by_id(method_id).unwrap();
                    let args =
                        context.pop_create_args(method.args_size() as u16 + 1 /* object */)?;

                    if args[0]
                        .with_object(|obj| !core::ptr::eq(meth_class, Arc::as_ptr(&obj.class)))
                        .ok_or(VMError::NotAnObject)?
                    {
                        let object = args[0].object_clone().unwrap();
                        // FIXME: we should build a vtable
                        let obj_meth = object
                            .class
                            .method_by_name(method.name(), Some(method.raw_descriptor()))
                            .expect("Couldn't search for virtual method");

                        let code = obj_meth.code().expect("FIXME: Handle methods without code");
                        context.finish_push_frame(
                            ref_idx,
                            code.max_stack(),
                            code.max_locals(),
                            method.args_size() as u16 + 1,
                        );

                        let result = self.run_code(&object.class, context, &code)?;
                        context.pop_frame();
                        if let Some(result) = result {
                            context.ipush(result);
                        }
                    } else {
                        /* normal invokespecial */
                        let code = method.code().expect("TODO: methods without code");

                        context.finish_push_frame(
                            ref_idx,
                            code.max_stack(),
                            code.max_locals(),
                            method.args_size() as u16 + 1,
                        );

                        let result = self.run_code(meth_class, context, &code)?;
                        context.pop_frame();
                        if let Some(result) = result {
                            context.ipush(result);
                        }
                    }
                }
                OpCode::Getstatic(ref_idx) => {
                    let (field_class, field_id) = self.get_field_or_resolve(class, ref_idx)?;
                    let field = field_class.field_by_id(field_id).unwrap();
                    let static_data = field
                        .as_static()
                        .ok_or_else(|| VMError::NotStatic(field.name().into()))?;

                    for slot in static_data.iter().rev() {
                        // Safety: object access is not sync by default, statics are initialized once (?)
                        context.push_slot(unsafe { &*slot.get() }.clone());
                    }
                }
                OpCode::Putstatic(ref_idx) => {
                    let (field_class, field_id) = self.get_field_or_resolve(class, ref_idx)?;
                    let field = field_class.field_by_id(field_id).unwrap();
                    let static_data = field
                        .as_static()
                        .ok_or_else(|| VMError::NotStatic(field.name().into()))?;

                    for slot in static_data {
                        unsafe { *slot.get() = context.pop_slot().ok_or(VMError::StackUnderflow)? };
                    }
                }
                OpCode::GetField(ref_idx) | OpCode::PutField(ref_idx) => {
                    let (field_class, field_id) = self.get_field_or_resolve(class, ref_idx)?;
                    let field = field_class.field_by_id(field_id).unwrap();
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
                OpCode::Iload(idx) => {
                    let v = context.iload(idx as u16).ok_or(VMError::StackUnderflow)?;
                    context.ipush(v);
                }
                OpCode::IStore(idx) => {
                    let v = context.ipop().ok_or(VMError::StackUnderflow)?;
                    if !context.istore(idx as u16, v) {
                        return Err(VMError::InvalidLocal(idx as u16));
                    }
                }

                OpCode::Aload(idx) => {
                    let v = context
                        .aload(idx as u16)
                        .ok_or(VMError::InvalidLocal(idx.into()))?;
                    context.push_slot(JVMSlot::from_object(v));
                }
                OpCode::AStore(idx) => {
                    let v = context.apop().ok_or(VMError::StackUnderflow)?;
                    if !context.astore(idx as u16, v) {
                        return Err(VMError::InvalidLocal(idx as u16));
                    }
                }
                OpCode::Iinc(idx, imm) => {
                    context
                        .iget(idx as u16)
                        .map(|l| {
                            let v = l.as_i32();
                            *l = JVMSlot::from_i32(v + imm as i32)
                        })
                        .ok_or(VMError::InvalidLocal(idx as u16))?;
                }
                OpCode::Imul => {
                    let v2 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    context.ipush(v1.wrapping_mul(v2));
                }
                OpCode::Idiv => {
                    let v2 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    context.ipush(v1.wrapping_div(v2));
                }
                OpCode::Ineg => {
                    let v = context.ipop().ok_or(VMError::StackUnderflow)?;
                    context.ipush(-v);
                }
                OpCode::Isub => {
                    let v2 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = context.ipop().ok_or(VMError::StackUnderflow)?;

                    context.ipush(v1.wrapping_sub(v2));
                }
                OpCode::Iadd => {
                    let v2 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    context.ipush(v1.wrapping_add(v2));
                }
                OpCode::Goto(off) => {
                    instr.set_pc(last_pc.saturating_add_signed(off));
                }
                OpCode::IfIcmpEq(off) | OpCode::IfIcmpNe(off) => {
                    let v2 = context.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = context.ipop().ok_or(VMError::StackUnderflow)?;

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
                    let v1 = context.ipop().ok_or(VMError::StackUnderflow)?;
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
                OpCode::IReturn => return Ok(Some(context.ipop().ok_or(VMError::StackUnderflow)?)),
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
    pub fn run_main(&self) -> Result<Option<i32>, VMError> {
        let main_class = self.main_class.clone();

        if let Some(clinit) = main_class.method_by_name("<clinit>", Some("()V")) {
            let code = clinit.code().expect("<clinit> no code");
            self.run_code(
                &main_class,
                &mut ThreadContext::new(code.max_stack(), code.max_locals()),
                code,
            )
            .expect("<clinit> run failed");
        }

        let meth = main_class
            .methods()
            .iter()
            .find(|m| m.name() == "main")
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

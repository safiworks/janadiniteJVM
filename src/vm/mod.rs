use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{Arc, RwLock},
};

mod class;
pub use class::*;
use janadinite_parse::class::{JVMAccessFlag, JVMCode, OpCode};

#[derive(Debug, Clone)]
pub enum VMError {
    StackUnderflow,
    InvalidLocal(u16),
    InvalidMain(&'static str),
    InvalidConstantPoolEntry(u16),
    NoSuchMethodInClass(String),
    NoSuchClass,
    Corrupted(String),
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
    stack: Vec<i32>,
    locals: Vec<i32>,
    prev_frames: Vec<StackFrame>,
    current_frame: StackFrame,
}

impl ThreadContext {
    /// Creates a new, initial thread context.
    pub fn new(max_stack: u16, max_locals: u16) -> Self {
        Self {
            stack: Vec::with_capacity(max_stack as usize),
            locals: vec![0; max_locals as usize],
            prev_frames: Vec::new(),
            current_frame: StackFrame {
                meth_ref: 0,
                stack_start: 0,
                max_stack,
                locals_start: 0,
            },
        }
    }

    /// Pushes a new stack frame onto the frame stack.
    pub fn push_frame(
        &mut self,
        meth_ref: u16,
        max_stack: u16,
        max_locals: u16,
        args_size: u16,
    ) -> Result<(), VMError> {
        let locals_start = self.locals.len();
        self.locals.resize(locals_start + max_locals as usize, 0);
        self.stack.reserve(max_stack as usize);

        for i in 0..args_size {
            self.locals[locals_start + i as usize] =
                self.stack.pop().ok_or(VMError::StackUnderflow)?;
        }

        self.prev_frames.push(self.current_frame);
        self.current_frame = StackFrame {
            meth_ref,
            max_stack,
            locals_start,
            stack_start: self.stack.len(),
        };

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
        self.stack.push(v);
    }

    /// Pops an integer value from the stack.
    #[inline(always)]
    pub fn ipop(&mut self) -> Option<i32> {
        if self.stack.len() <= self.current_frame.stack_start {
            return None;
        }
        self.stack.pop()
    }

    /// Loads an integer value from the local variable at the given index.
    #[inline(always)]
    pub fn iload(&mut self, index: u16) -> Option<i32> {
        self.locals
            .get(index as usize + self.current_frame.locals_start)
            .copied()
    }

    /// Stores an integer value into the local variable at the given index.
    #[must_use = "Returns whether the store was successful"]
    #[inline(always)]
    pub fn istore(&mut self, index: u16, value: i32) -> bool {
        if let Some(local) = self
            .locals
            .get_mut(index as usize + self.current_frame.locals_start)
        {
            *local = value;
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn iget(&mut self, index: u16) -> Option<&mut i32> {
        self.locals
            .get_mut(index as usize + self.current_frame.locals_start)
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
            let mut file = File::open(path).map_err(|_| VMError::NoSuchClass)?;
            let class =
                VMClass::parse(&mut file).map_err(|e| VMError::Corrupted(e.message().into()))?;
            let class = Arc::new(class);
            write_guard.insert(name.into(), class.clone());
            f(&class)
        }
    }

    fn get_method_or_resolve<'c>(
        &self,
        this_class: &'c VMClass,
        ref_idx: u16,
    ) -> Result<(&'c VMClass, &'c VMMethod), VMError> {
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
                    Ok((class, method))
                } else {
                    let (class, method) =
                        self.with_class_or_load_by_name(&*unresolved_class, |class| {
                            class
                                .methods()
                                .iter()
                                .find(|m| {
                                    (**m).name() == &**unresolved_name
                                        && (**m).raw_descriptor() == &**unresolved_descriptor
                                })
                                .map(|m| (class.clone(), m.clone()))
                                .ok_or_else(|| {
                                    VMError::NoSuchMethodInClass(String::from(&**unresolved_name))
                                })
                        })?;

                    let (class, method) = resolved.get_or_init(|| (class, method));
                    Ok((class, method))
                }
            }
            VMConstantPoolEntry::ResolvedMethod(m) => Ok((this_class, m)),
            _ => Err(VMError::InvalidConstantPoolEntry(ref_idx)),
        }
    }

    fn run_code(
        &self,
        class: &VMClass,
        context: &mut ThreadContext,
        code: &JVMCode,
    ) -> Result<i32, VMError> {
        let mut last_pc: u16 = 0;
        let mut instr = code.instructions();

        while let Some(opcode) = instr.next_op() {
            match opcode {
                OpCode::InvokeStatic(ref_idx) => {
                    let (meth_class, method) = self.get_method_or_resolve(class, ref_idx)?;
                    let code = method.code().expect("TODO: methods without code");

                    context.push_frame(
                        ref_idx,
                        code.max_stack(),
                        code.max_locals(),
                        method.args_size() as u16,
                    )?;

                    let result = self.run_code(meth_class, context, &code)?;
                    context.pop_frame();
                    context.ipush(result);
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
                OpCode::Iinc(idx, imm) => {
                    context
                        .iget(idx as u16)
                        .map(|l| *l += imm as i32)
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
                OpCode::IReturn => return Ok(context.ipop().ok_or(VMError::StackUnderflow)?),
                OpCode::Invalid(i) => {
                    eprintln!("Invalid OpCode: {i}");
                }
            }

            last_pc = instr.pc();
        }

        Ok(0)
    }
    pub fn run_main(&self) -> Result<i32, VMError> {
        let main_class = self.main_class.clone();
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

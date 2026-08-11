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
    InvalidMain(&'static str),
    InvalidConstantPoolEntry(u16),
    NoSuchMethodInClass(String),
    NoSuchClass,
    Corrupted(String),
}
#[derive(Debug)]
struct Stack {
    data: Vec<u32>,
}

impl Stack {
    pub fn new(cap: usize) -> Self {
        Self {
            data: Vec::with_capacity(cap),
        }
    }

    pub fn ipush(&mut self, v: i32) {
        self.data.push(v.cast_unsigned());
    }

    pub fn ipop(&mut self) -> Option<i32> {
        self.data.pop().map(|v| v.cast_signed())
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
        mut locals: Box<[i32]>,
        code: &JVMCode,
    ) -> Result<i32, VMError> {
        let mut stack = Stack::new(code.max_stack() as usize);

        let mut last_pc: u16 = 0;
        let mut instr = code.instructions();

        while let Some(opcode) = instr.next_op() {
            match opcode {
                OpCode::InvokeStatic(ref_idx) => {
                    let (meth_class, method) = self.get_method_or_resolve(class, ref_idx)?;
                    let code = method.code().expect("TODO: methods without code");
                    let mut locals = vec![0; code.max_locals() as usize].into_boxed_slice();

                    for i in 0..method.args_size() {
                        let arg = stack.ipop().ok_or(VMError::StackUnderflow)?;
                        locals[i] = arg;
                    }

                    let result = self.run_code(meth_class, locals, &code)?;
                    stack.ipush(result);
                }
                OpCode::Bipush(b) => stack.ipush(b as i32),
                OpCode::Iload(idx) => {
                    stack.ipush(locals[idx as usize]);
                }
                OpCode::IStore(idx) => {
                    locals[idx as usize] = stack.ipop().ok_or(VMError::StackUnderflow)?;
                }
                OpCode::Iinc(idx, imm) => {
                    locals[idx as usize] += imm as i32;
                }
                OpCode::Imul => {
                    let v2 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    stack.ipush(v1.wrapping_mul(v2));
                }
                OpCode::Idiv => {
                    let v2 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    stack.ipush(v1.wrapping_div(v2));
                }
                OpCode::Ineg => {
                    let v = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    stack.ipush(-v);
                }
                OpCode::Isub => {
                    let v2 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = stack.ipop().ok_or(VMError::StackUnderflow)?;

                    stack.ipush(v1.wrapping_sub(v2));
                }
                OpCode::Iadd => {
                    let v2 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    stack.ipush(v1.wrapping_add(v2));
                }
                OpCode::Goto(off) => {
                    instr.set_pc(last_pc.saturating_add_signed(off));
                }
                OpCode::IfIcmpEq(off) | OpCode::IfIcmpNe(off) => {
                    let v2 = stack.ipop().ok_or(VMError::StackUnderflow)?;
                    let v1 = stack.ipop().ok_or(VMError::StackUnderflow)?;

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
                    let v1 = stack.ipop().ok_or(VMError::StackUnderflow)?;
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
                OpCode::IReturn => return Ok(stack.ipop().ok_or(VMError::StackUnderflow)?),
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

        self.run_code(
            &*main_class,
            vec![0; code.max_locals() as usize].into_boxed_slice(),
            &code,
        )
    }
}

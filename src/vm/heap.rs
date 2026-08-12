use std::{
    cell::UnsafeCell,
    ops::Deref,
    sync::{Arc, RwLock, atomic::AtomicBool},
};

use crate::vm::{JVMSlot, ThreadContext, VMClass};

pub static HEAP: RwLock<Heap> = RwLock::new(Heap::new());

#[derive(Debug)]
pub struct Heap {
    objects: Vec<Arc<Object>>,
}

impl Heap {
    pub const fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }
    pub fn allocate(&mut self, class: Arc<VMClass>) -> ObjectRef {
        let data: Box<[UnsafeCell<JVMSlot>]> = (0..class.fields_slots())
            .map(|_| UnsafeCell::new(JVMSlot::null()))
            .collect();

        let obj = self
            .objects
            .push_mut(Arc::new(Object {
                alive: AtomicBool::new(false),
                data,
                class,
            }))
            .clone();

        unsafe { ObjectRef::from_ptr(Arc::into_raw(obj)) }
    }
}

#[derive(Debug)]
pub struct Object {
    class: Arc<VMClass>,
    alive: AtomicBool,
    pub data: Box<[UnsafeCell<JVMSlot>]>,
}

unsafe impl Send for Object {}
unsafe impl Sync for Object {}

/// A pointer to an [`Object`].
///
/// As long as this reference is accessible the object should be available and safe to access.
/// An object should only be freed if no references to it exist.
#[derive(Debug, Clone)]
pub struct ObjectRef(Arc<Object>);

impl ObjectRef {
    pub unsafe fn from_ptr(ptr: *const Object) -> Self {
        unsafe { Self(Arc::from_raw(ptr)) }
    }
    pub fn into_ptr(self) -> *const Object {
        Arc::into_raw(self.0)
    }
}

impl Deref for ObjectRef {
    type Target = Object;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

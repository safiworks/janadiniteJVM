use std::{
    cell::UnsafeCell,
    collections::HashSet,
    hash::Hash,
    mem::ManuallyDrop,
    num::NonZero,
    ops::Deref,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use rustc_hash::FxBuildHasher;

use crate::vm::{JVMSlot, VMClass, VMError};

const GC_THRESHOLD: usize = 20;

static SHOULD_STOP_THE_WORLD: AtomicBool = AtomicBool::new(false);

static HEAP: Mutex<Heap> = Mutex::new(Heap::new());

static PARKED_THREADS: Condvar = Condvar::new();
static GC_THREAD: Condvar = Condvar::new();

pub(crate) fn gc_thread() -> ! {
    let mut heap = HEAP.lock().expect("Failed to acquire lock on heap");

    loop {
        heap = GC_THREAD
            .wait_while(heap, |_| !SHOULD_STOP_THE_WORLD.load(Ordering::Relaxed))
            .expect("Failed to wait for GC Thread wake signal");

        heap.try_gc();
        SHOULD_STOP_THE_WORLD.store(false, Ordering::SeqCst);
        PARKED_THREADS.notify_all();
    }
}

pub fn register_thread() {
    let mut heap = HEAP
        .lock()
        .expect("Failed to acquire lock on heap during register");
    heap.heap_threads += 1;

    if SHOULD_STOP_THE_WORLD.load(Ordering::Acquire) {
        drop(stop_the_world_inner(heap));
    }
}

pub fn unregister_thread() {
    let mut heap = HEAP
        .lock()
        .expect("Failed to acquire lock on heap during register");
    heap.heap_threads -= 1;

    if SHOULD_STOP_THE_WORLD.load(Ordering::Acquire) && heap.parked_threads >= heap.heap_threads {
        GC_THREAD.notify_all();
    }
}

#[inline(always)]
unsafe fn allocate_inner(kind: ObjectKind, slots: usize) -> ObjectRef {
    let mut heap = HEAP.lock().expect("Failed to acquire lock on heap");

    if SHOULD_STOP_THE_WORLD.load(Ordering::Relaxed) {
        heap = stop_the_world_inner(heap);
    }
    heap.allocate(kind, slots)
}

/// Safety: thread has to be registered with [`register_thread`] first.
pub unsafe fn allocate(class: Arc<VMClass>) -> ObjectRef {
    let slots = class.fields_slots();
    unsafe { allocate_inner(ObjectKind::Instance { class }, slots) }
}

/// Safety: thread has to be registered with [`register_thread`] first.
pub unsafe fn allocate_array(count: usize, size: NonZero<u16>) -> ObjectRef {
    unsafe {
        allocate_inner(
            ObjectKind::Array { size: size },
            count * size.get() as usize,
        )
    }
}

pub unsafe fn allocate_multiarray(
    dimensions: usize,
    mut count: impl Iterator<Item = Result<i32, VMError>>,
    last_dim_o_size: NonZero<u16>,
) -> Result<ObjectRef, VMError> {
    let mut heap = HEAP.lock().expect("Failed to acquire lock on heap");

    if SHOULD_STOP_THE_WORLD.load(Ordering::Relaxed) {
        heap = stop_the_world_inner(heap);
    }

    let mut last_object: Option<Object> = None;
    for i in 0..dimensions {
        let len = count.next().unwrap()?;

        let size = if i == 0 {
            last_dim_o_size
        } else {
            NonZero::<u16>::MIN
        };
        if len == 0 {
            last_object = Some(Object::new_array(0, size));
            continue;
        }

        let obj = Object::new_array(len as usize, size);
        if let Some(child) = last_object {
            for i in 0..(obj.data.len().saturating_sub(1)) {
                // Safety: no other thread is manipulating this object
                unsafe {
                    *obj.data[i].get() =
                        JVMSlot::from_object(heap.allocate_from_object(child.clone_unchecked()));
                }
            }

            if let Some(last) = obj.data.last() {
                unsafe { *last.get() = JVMSlot::from_object(heap.allocate_from_object(child)) };
            }
        }
        last_object = Some(obj);
    }

    last_object
        .map(|obj| heap.allocate_from_object(obj))
        .ok_or_else(|| VMError::Corrupted("allocate array with dimensions=0".into()))
}

fn stop_the_world_inner(mut heap: MutexGuard<Heap>) -> MutexGuard<Heap> {
    heap.parked_threads += 1;
    if heap.parked_threads >= heap.heap_threads {
        GC_THREAD.notify_all();
    }

    let mut heap = PARKED_THREADS
        .wait_while(heap, |_| SHOULD_STOP_THE_WORLD.load(Ordering::Relaxed))
        .expect("Failed to wait for stop the world");
    heap.parked_threads -= 1;
    heap
}

/// Needed for hot loops that don't allocate to handle Stop-The-World events...
pub fn safepoint() {
    if !SHOULD_STOP_THE_WORLD.load(Ordering::Relaxed) {
        return;
    }

    drop(stop_the_world_inner(
        HEAP.lock().expect("Failed to acquire lock on heap"),
    ));
}

#[derive(Debug)]
pub struct Heap {
    // NOTE: dropping plain ObjectRef could result on a deadlock, convert into Arc first...
    objects: HashSet<ObjectRef, FxBuildHasher>,
    alloc_count: usize,
    unreachable: Vec<ObjectRef>,
    heap_threads: usize,
    parked_threads: usize,
}

impl Heap {
    pub const fn new() -> Self {
        Self {
            objects: const { HashSet::with_hasher(rustc_hash::FxBuildHasher) },
            unreachable: Vec::new(),
            heap_threads: 0,
            alloc_count: 0,
            parked_threads: 0,
        }
    }

    pub fn try_gc(&mut self) {
        assert_eq!(
            self.heap_threads, self.parked_threads,
            "Called without stop-the-world happening first"
        );
        let all_objects = &mut self.objects;
        let threshold = GC_THRESHOLD;

        if self.alloc_count >= threshold {
            for obj in all_objects.iter() {
                unsafe { *obj.gc_refs.get() = Arc::strong_count(&obj.0) - 1 };
            }

            for obj in all_objects.iter() {
                for data in &obj.data {
                    // Safety: world should be stopped first...
                    unsafe {
                        (*data.get()).with_object(|child| *child.gc_refs.get() -= 1);
                    }
                }
            }

            for obj in all_objects.extract_if(|obj| unsafe { *obj.gc_refs.get() } == 0) {
                unsafe { *obj.gc_refs.get() = usize::MAX };
                self.unreachable.push(obj);
            }

            let mut found_refs = false;
            for obj in all_objects.iter() {
                for data in &obj.data {
                    // Safety: world should be stopped first...
                    unsafe {
                        (*data.get()).with_object(|child| {
                            if *child.gc_refs.get() == usize::MAX {
                                found_refs = true;
                                *child.gc_refs.get() = 1;
                            }
                        });
                    }
                }
            }

            while found_refs {
                found_refs = false;
                for obj in self.unreachable.iter() {
                    if unsafe { *obj.gc_refs.get() != usize::MAX } {
                        for data in &obj.data {
                            unsafe {
                                (*data.get()).with_object(|child| {
                                    if *child.gc_refs.get() == usize::MAX {
                                        found_refs = true;
                                        *child.gc_refs.get() = 1;
                                    }
                                });
                            }
                        }
                    }
                }
            }

            for obj in self.unreachable.drain(..) {
                if unsafe { *obj.gc_refs.get() } != usize::MAX {
                    all_objects.insert(obj);
                } else {
                    let arc = obj.into_arc();

                    for data in &arc.data {
                        if let Some(obj) =
                            core::mem::replace(unsafe { &mut *data.get() }, JVMSlot::null())
                                .into_object()
                        {
                            drop(obj.into_arc());
                        }
                    }

                    drop(arc);
                }
            }

            self.alloc_count = 0;
        }
    }

    pub fn set_gc_flags(&self) {
        if self.alloc_count >= GC_THRESHOLD {
            SHOULD_STOP_THE_WORLD.store(true, Ordering::Release);
        }
    }

    pub fn allocate(&mut self, kind: ObjectKind, slots: usize) -> ObjectRef {
        let data: Box<[UnsafeCell<JVMSlot>]> = (0..slots)
            .map(|_| UnsafeCell::new(JVMSlot::null()))
            .collect();

        self.allocate_from_object(Object {
            gc_refs: UnsafeCell::new(0),
            data,
            kind,
        })
    }

    pub fn allocate_from_object(&mut self, object: Object) -> ObjectRef {
        let arc_obj = Arc::new(object);
        self.objects.insert(ObjectRef(arc_obj.clone()));
        self.alloc_count += 1;

        self.set_gc_flags();
        unsafe { ObjectRef::from_ptr(Arc::into_raw(arc_obj)) }
    }
}

#[derive(Debug, Clone)]
pub enum ObjectKind {
    Instance { class: Arc<VMClass> },
    Array { size: NonZero<u16> },
}
#[derive(Debug)]
pub struct Object {
    gc_refs: UnsafeCell<usize>,
    pub kind: ObjectKind,
    pub data: Box<[UnsafeCell<JVMSlot>]>,
}

impl Object {
    pub fn new_class(class: Arc<VMClass>) -> Self {
        let data: Box<[UnsafeCell<JVMSlot>]> = (0..class.fields_slots())
            .map(|_| UnsafeCell::new(JVMSlot::null()))
            .collect();
        Self {
            gc_refs: UnsafeCell::new(0),
            kind: ObjectKind::Instance { class },
            data,
        }
    }

    pub fn new_array(count: usize, size: NonZero<u16>) -> Self {
        let len = count * size.get() as usize;
        let data: Box<[UnsafeCell<JVMSlot>]> =
            (0..len).map(|_| UnsafeCell::new(JVMSlot::null())).collect();

        Self {
            gc_refs: UnsafeCell::new(0),
            kind: ObjectKind::Array { size },
            data,
        }
    }

    /// Clone an Object.
    ///
    /// User must be allowed to read data with race conditions in mind.
    pub unsafe fn clone_unchecked(&self) -> Self {
        Self {
            gc_refs: UnsafeCell::new(0),
            data: self
                .data
                .iter()
                .map(|j| UnsafeCell::new(unsafe { &*j.get() }.clone()))
                .collect(),
            kind: self.kind.clone(),
        }
    }
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

    pub fn into_arc(self) -> Arc<Object> {
        unsafe { Arc::from_raw(self.into_ptr()) }
    }

    pub fn into_ptr(self) -> *const Object {
        let this = ManuallyDrop::new(self);
        Arc::as_ptr(&this.0)
    }
}

impl Deref for ObjectRef {
    type Target = Object;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Hash for ObjectRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Hash::hash(&(Arc::as_ptr(&self.0) as usize), state)
    }
}

impl PartialEq for ObjectRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ObjectRef {}

impl Drop for ObjectRef {
    fn drop(&mut self) {
        // This reference and the HEAP's reference meaning we can try to deallocate
        if Arc::strong_count(&self.0) == 2 {
            let mut heap = HEAP
                .lock()
                .expect("Failed to acquire lock on heap while dealllocting object");

            if Arc::strong_count(&self.0) == 2 {
                let Some(removed) = heap.objects.take(self) else {
                    panic!("Failed to deallocate an object")
                };
                drop(removed.into_arc());
                heap.alloc_count -= 1;
            }
        }
    }
}

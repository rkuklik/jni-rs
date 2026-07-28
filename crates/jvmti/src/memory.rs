use core::alloc::GlobalAlloc;
use core::alloc::Layout;
use core::ffi::c_char;
use core::mem::transmute;
use core::ops::Deref;
use core::ops::DerefMut;
use core::ptr;
use core::ptr::NonNull;

use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;
use jni::strings::JNIStr;
use jni_sys::jlong;

use crate::env::EnvUntyped;
use crate::errors::Result;
use crate::macros::invoke;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EnvAlloc {
    pub(crate) env: EnvUntyped,
}

// Type bounds do nothing currently apart form showing explicitly in the docs.
// https://github.com/rust-lang/rust/issues/112792
#[allow(type_alias_bounds)]
/// Memory allocated via [`EnvUntyped`].
pub type JBox<T: ?Sized> = allocator_api2::boxed::Box<T, EnvAlloc>;

/// Unmanaged JVMTI allocation
#[derive(Debug)]
#[repr(transparent)]
#[must_use]
pub struct JPtr<T: ?Sized>(NonNull<T>);

impl<T: ?Sized> JPtr<T> {
    /// # Safety
    ///
    /// Pointer must point to initialized memory allocated by JVMTI environment.
    ///
    /// Ownership of the allocation is transferred after this call.
    pub unsafe fn new(ptr: NonNull<T>) -> Self {
        Self(ptr)
    }

    pub fn raw(self) -> NonNull<T> {
        self.0
    }

    pub fn manage(self, alloc: EnvAlloc) -> JBox<T> {
        // SAFETY: see constructor
        unsafe { alloc.boxed(self.0) }
    }
}

impl<T: ?Sized> Deref for JPtr<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: see constructor
        unsafe { self.0.as_ref() }
    }
}

impl<T: ?Sized> DerefMut for JPtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: see constructor
        unsafe { self.0.as_mut() }
    }
}

pub(crate) unsafe fn cast_box_slice<S, D>(boxed: JBox<[S]>) -> JBox<[D]> {
    const {
        assert!(size_of::<S>() == size_of::<D>());
        assert!(align_of::<S>() == align_of::<D>());
    };
    let (slice, alloc) = JBox::into_non_null_with_allocator(boxed);
    unsafe { alloc.boxed_slice(slice.cast::<D>().as_ptr(), slice.len()) }
}

/// Allocation and deallocation of memory used by JVM TI functionality. Can be
/// used to provide working memory for agents, but Memory managed by JVM TI is
/// not compatible with other memory allocation libraries and mechanisms.
impl EnvUntyped {
    /// Allocate an area of memory through the JVM TI allocator.
    ///
    /// The allocated memory should be freed with [`deallocate`].
    /// You shouldn't need to use this directly as any API that deals
    /// with JVM TI allocations will use [`JBox`].
    ///
    /// [`deallocate`]: EnvUntyped::deallocate
    pub fn allocate(&self, size: usize) -> Result<*mut u8> {
        let mut ptr: *mut u8 = ptr::null_mut();
        // zero gives out null anyway
        if size != 0 {
            // SAFETY: no requirement
            const _: () = assert!(size_of::<jlong>() >= size_of::<usize>());
            unsafe { invoke!(self, v1, Allocate, size as jlong, &mut ptr) }?;
        }
        Ok(ptr)
    }

    /// Deallocate memory using the JVM TI allocator.
    ///
    /// This function should be used to deallocate any memory allocated and
    /// returned by a JVM TI function (including memory allocated with [`allocate`]).
    /// All allocated memory must be deallocated or the memory cannot be reclaimed.
    /// You shouldn't need to use this directly as any API that deals
    /// with JVM TI allocations will use [`JBox`].
    ///
    /// # Safety
    ///
    /// Allocation pointed to by `ptr` must be [currently allocated by JVM TI][link]
    /// and it's lifetime ends with this call. Only the environment which allocated the
    /// may deallocate it (see [`JBox`] if you want to correctly handle this automatically).
    ///
    /// [`allocate`]: EnvUntyped::allocate
    /// [link]: https://doc.rust-lang.org/core/alloc/trait.Allocator.html#currently-allocated-memory
    pub unsafe fn deallocate(&self, ptr: *mut u8) -> Result<()> {
        // for null does nothing anyway
        if !ptr.is_null() {
            // SAFETY: caller ensures valid pointer
            unsafe { invoke!(self, v1, Deallocate, ptr) }
        } else {
            Ok(())
        }
    }

    pub fn allocator(&self) -> &EnvAlloc {
        // SAFETY: `EnvAlloc` is `repr(transparent)`
        unsafe { transmute::<&Self, &EnvAlloc>(self) }
    }
}

impl EnvAlloc {
    /// Assume that `ptr` was allocated via this environment.
    ///
    /// Converts a raw unmanaged pointer to RAII managed with this environment.
    /// This is useful when calling raw API which returns newly allocated memory.
    ///
    /// # Safety
    ///
    /// Ownership of memory pointed to by `ptr` is transferred to [`JBox`] and may
    /// not be used anymore. The memory must have been allocated by this environment,
    /// with the exception that zero-sized allocation may be dangling (but well aligned).
    pub unsafe fn boxed<T: ?Sized>(&self, ptr: NonNull<T>) -> JBox<T> {
        unsafe { JBox::from_non_null_in(ptr, self.clone()) }
    }

    /// Assume that `len` elements were allocated at `ptr` via this environment.
    ///
    /// Converts a raw unmanaged pointer to RAII managed with this environment.
    /// This is useful when calling raw API which returns newly allocated memory.
    ///
    /// # Safety
    ///
    /// As with [`Self::boxed_non_null`], with allocation valid for `len` elements.
    ///
    /// Pointer may be `null` if `len` is zero.
    pub unsafe fn boxed_slice<T>(&self, ptr: *mut T, len: usize) -> JBox<[T]> {
        let ptr = NonNull::new(ptr).unwrap_or_else(NonNull::dangling);
        let arr = NonNull::slice_from_raw_parts(ptr, len);
        unsafe { self.boxed(arr) }
    }

    /// Assume that `ptr` was allocated via this environment and points to JNIStr.
    ///
    /// # Safety
    ///
    /// As with [`Self::boxed_non_null`] and [`JNIStr::from_ptr`].
    pub unsafe fn boxed_jnistr(&self, ptr: NonNull<c_char>) -> JBox<JNIStr> {
        // SAFETY: caller ensures valid pointer
        let ptr: *const JNIStr = unsafe { JNIStr::from_ptr(ptr.as_ptr()) };
        // TODO: use `NonNull::from_ref` after MSRV at 1.89.0
        let str = unsafe { NonNull::new_unchecked(ptr.cast_mut()) };
        // SAFETY: caller ensures valid pointer
        unsafe { self.boxed(str) }
    }
}

unsafe impl Allocator for EnvAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        if size == 0 {
            return Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0));
        }
        // SAFETY: size != 0 and `allocate` succeeded implies non-null pointer
        let ptr = self.env.allocate(size).map_err(|_| AllocError)?;
        let ptr = unsafe { NonNull::new_unchecked(ptr) };
        Ok(NonNull::slice_from_raw_parts(ptr, size))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: caller ensures valid pointer and ZST are guarded against
        if layout.size() != 0 {
            _ = unsafe { self.env.deallocate(ptr.as_ptr()) };
        }
    }
}

unsafe impl GlobalAlloc for EnvAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.env
            .allocate(layout.size())
            .ok()
            .unwrap_or_else(ptr::null_mut)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _: Layout) {
        // SAFETY: `GlobalAlloc` requires valid pointer with non-zero size
        _ = unsafe { self.env.deallocate(ptr) };
    }
}

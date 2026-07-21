use core::ffi::c_void;
use core::marker::PhantomData;
use core::ops::Deref;
use core::ops::DerefMut;
use core::ptr;
use core::ptr::NonNull;

use alloc::boxed::Box;

use jni::errors::Error as JNIError;
use jni::errors::jni_error_code_to_result;
use jni::vm::JavaVM;
use jvmti_sys::jvmtiEnv;

use crate::errors::ContextError;
use crate::errors::Error;
use crate::errors::Result;
use crate::errors::SupportError;
use crate::macros::invoke;
use crate::version::JVMTIVersion;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EnvUntyped {
    ptr: NonNull<jvmtiEnv>,
}

// SAFETY: docs state that JVMTI is thread-safe
unsafe impl Send for EnvUntyped {}
unsafe impl Sync for EnvUntyped {}

impl EnvUntyped {
    pub fn create(jvm: &JavaVM, version: JVMTIVersion) -> Result<EnvUntyped, JNIError> {
        let mut jvmti: *mut jvmtiEnv = ptr::null_mut();
        let raw: *mut jni_sys::JavaVM = jvm.get_raw();
        let ver = version.into();
        // SAFETY: JavaVM is guaranteed to be valid pointer, JNI crate mantates version
        // 1.4 or greater and JVMTI is valid on return without error.
        let code = unsafe {
            let ptr: *mut *mut jvmtiEnv = &mut jvmti;
            let ptr: *mut *mut c_void = ptr.cast();
            ((**raw).v1_2.GetEnv)(raw, ptr, ver)
        };
        jni_error_code_to_result(code)?;
        // SAFETY: guaranteed to be valid when no error is returned
        let ptr = unsafe { NonNull::new_unchecked(jvmti) };
        Ok(Self { ptr })
    }

    /// # Safety
    ///
    /// The environment may not be used after this call. This includes currently
    /// running event callbacks, destructors using [`EnvAlloc`] and others, which
    /// may be still run automatically even after this call.
    ///
    /// [`EnvAlloc`]: crate::memory::EnvAlloc
    pub unsafe fn dispose(self) -> Result<()> {
        unsafe { invoke!(self, v1, DisposeEnvironment) }
    }

    /// # Safety
    ///
    /// Pointer must be non-null pointing to a valid JVMTI environment.
    pub unsafe fn from_raw(ptr: *mut jvmtiEnv) -> Self {
        let ptr = unsafe { NonNull::new_unchecked(ptr) };
        Self { ptr }
    }

    pub fn as_raw(&self) -> *mut jvmtiEnv {
        self.ptr.as_ptr()
    }

    pub fn into_raw(self) -> *mut jvmtiEnv {
        self.ptr.as_ptr()
    }

    pub fn with_data<T>(self, data: T) -> Result<Env<T>, EnvError<T>> {
        Env::new(self, data)
    }

    pub fn ensure_version(&self, version: JVMTIVersion) -> Result<()> {
        if self.version()? < version {
            Err(SupportError::OperationUnsupported.into())
        } else {
            Ok(())
        }
    }

    pub fn version(&self) -> Result<JVMTIVersion> {
        let mut raw = 0;
        // SAFETY: `GetVersionNumber` will return a JVMTI version number
        unsafe {
            invoke!(self, v1, GetVersionNumber, &mut raw)?;
            Ok(JVMTIVersion::new_unchecked(raw as u32))
        }
    }

    /// # Safety
    ///
    /// While not unsafe by itself, [`Env<T>`] may make assumptions about what data
    /// is present in the environment. You must ensure that you have the unique ownership
    /// of this environment.
    pub unsafe fn set_environment_local_storage_raw(&self, data: *mut ()) -> Result<()> {
        unsafe { invoke!(self, v1, SetEnvironmentLocalStorage, data.cast()) }
    }

    pub fn environment_local_storage_raw(&self) -> Result<*mut ()> {
        let mut data = ptr::null_mut();
        unsafe { invoke!(self, v1, GetEnvironmentLocalStorage, &mut data) }?;
        Ok(data.cast())
    }
}

pub type EnvError<T> = ContextError<(EnvUntyped, T), Error>;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Env<T> {
    untyped: EnvUntyped,
    phantom: PhantomData<T>,
}

impl Env<()> {
    pub fn dataless(env: EnvUntyped) -> Self {
        Self {
            untyped: env,
            phantom: PhantomData,
        }
    }

    pub fn untyped(self) -> EnvUntyped {
        self.leak()
    }
}

impl<T> Env<T> {
    pub fn new(env: EnvUntyped, data: T) -> Result<Env<T>, EnvError<T>> {
        let ptr = Box::into_raw(Box::new(data));

        // No need to write data for ZST. However, the destructor is still
        // correctly prevented from running by converting box to pointer.
        if const { size_of::<T>() == 0 } {
            return Ok(Self {
                untyped: env,
                phantom: PhantomData,
            });
        }

        // SAFETY: takes env by value, so nobody else can see the local storage.
        let res = unsafe { env.set_environment_local_storage_raw(ptr.cast()) };
        if let Err(error) = res {
            // SAFETY: if error is returned, ownership wasn't trasfered
            let boxed = unsafe { Box::<T>::from_raw(ptr) };
            return Err(EnvError {
                data: (env, *boxed),
                error,
            });
        }

        Ok(Self {
            untyped: env,
            phantom: PhantomData,
        })
    }

    pub fn leak(self) -> EnvUntyped {
        self.untyped
    }

    /// # Safety
    ///
    /// As if calling [`EnvUntyped::from_raw`] and passing the result to [`Env::from_untyped`].
    pub unsafe fn from_raw(ptr: *mut jvmtiEnv) -> Self {
        unsafe { Self::from_untyped(EnvUntyped::from_raw(ptr)) }
    }

    /// # Safety
    ///
    /// Environment must be a result of [`Self::leak`].
    pub unsafe fn from_untyped(untyped: EnvUntyped) -> Self {
        Self {
            untyped,
            phantom: PhantomData,
        }
    }

    pub fn data(&self) -> Result<&T> {
        if const { size_of::<T>() == 0 } {
            // SAFETY: `T` is zero-sized and we were provided with one on construction,
            // so this doesn't violate invariants of inconstructible ZST's
            return Ok(unsafe { NonNull::<T>::dangling().as_ref() });
        }

        // SAFETY: `Env<T>` is only constructible by initializing the local
        // storage and changing local storage is documented unsafe operation.
        let ptr = self.environment_local_storage_raw()?.cast::<T>();
        Ok(unsafe { NonNull::new_unchecked(ptr).as_ref() })
    }
}

impl<T> Deref for Env<T> {
    type Target = EnvUntyped;

    fn deref(&self) -> &Self::Target {
        &self.untyped
    }
}

impl<T> DerefMut for Env<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.untyped
    }
}

// SAFETY: `T` is only accessed through reference
unsafe impl<T: Send + Sync> Send for Env<T> {}
unsafe impl<T: Send + Sync> Sync for Env<T> {}

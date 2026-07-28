use core::ffi::c_char;
use core::ptr;
use core::ptr::NonNull;

use allocator_api2::vec::IntoIter as VecIt;
use jni::strings::JNIStr;
use jni_sys::jint;

use crate::env::EnvUntyped;
use crate::errors::Error;
use crate::errors::Result;
use crate::errors::SupportError;
use crate::macros::invoke;
use crate::memory::EnvAlloc;
use crate::memory::JBox;
use crate::memory::JPtr;

struct PropertyIterator {
    iter: VecIt<JPtr<c_char>, EnvAlloc>,
}

impl Iterator for PropertyIterator {
    type Item = JBox<JNIStr>;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = self.iter.next()?.raw();
        let env = self.iter.allocator();
        Some(unsafe { env.boxed_jnistr(ptr) })
    }
}

impl Drop for PropertyIterator {
    fn drop(&mut self) {
        for _ in self {}
    }
}

impl EnvUntyped {
    pub fn system_properties_raw(&self) -> Result<JBox<[JPtr<c_char>]>> {
        let mut count: jint = 0;
        let mut names: *mut *mut c_char = ptr::null_mut();
        unsafe {
            invoke!(self, v1, GetSystemProperties, &mut count, &mut names)?;
            Ok(self
                .allocator()
                .boxed_slice(names.cast::<JPtr<c_char>>(), count as usize))
        }
    }

    pub fn system_properties(&self) -> Result<impl Iterator<Item = JBox<JNIStr>>> {
        let iter = self.system_properties_raw()?.into_vec().into_iter();
        Ok(PropertyIterator { iter })
    }

    pub fn system_property_raw(&self, property: &JNIStr) -> Result<JBox<JNIStr>> {
        let mut value: *mut c_char = ptr::null_mut();
        unsafe {
            invoke!(self, v1, GetSystemProperty, property.as_ptr(), &mut value)?;
            Ok(self.allocator().boxed_jnistr(NonNull::new_unchecked(value)))
        }
    }

    pub fn system_property(&self, name: impl AsRef<JNIStr>) -> Result<JBox<JNIStr>> {
        self.system_property_raw(name.as_ref())
    }

    pub fn set_system_property_raw(&self, name: &JNIStr, value: Option<&JNIStr>) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v1,
                SetSystemProperty,
                name.as_ptr(),
                value.map_or_else(ptr::null, JNIStr::as_ptr)
            )
        }
    }

    pub fn system_property_writable(&self, name: impl AsRef<JNIStr>) -> Result<bool> {
        match self.set_system_property_raw(name.as_ref(), None) {
            Ok(()) => Ok(true),
            Err(Error::Support(SupportError::NotAvailable)) => Ok(false),
            Err(err) => Err(err),
        }
    }

    pub fn set_system_property(
        &self,
        name: impl AsRef<JNIStr>,
        value: impl AsRef<JNIStr>,
    ) -> Result<()> {
        self.set_system_property_raw(name.as_ref(), Some(value.as_ref()))
    }
}

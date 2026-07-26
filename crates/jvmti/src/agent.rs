use core::fmt;

use jni_sys::JNI_ERR;
use jni_sys::JNI_OK;
use jni_sys::jint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResultCode(jint);

impl ResultCode {
    pub const SUCCESS: Self = Self(JNI_OK);
    pub const FAILURE: Self = Self(JNI_ERR);

    pub const fn new(value: jint) -> Self {
        Self(value)
    }

    #[doc(hidden)]
    pub const fn raw(self) -> jint {
        self.0
    }
}

impl From<()> for ResultCode {
    fn from((): ()) -> Self {
        Self::SUCCESS
    }
}

impl From<jint> for ResultCode {
    fn from(value: jint) -> Self {
        Self(value)
    }
}

impl<T, E> From<Result<T, E>> for ResultCode
where
    T: Into<ResultCode>,
    E: fmt::Debug,
{
    fn from(value: Result<T, E>) -> Self {
        match value {
            Ok(res) => res.into(),
            Err(err) => panic!("error starting agent {err:?}"),
        }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __agent_start {
    ($name:ident, $func:expr) => {
        #[unsafe(no_mangle)]
        unsafe extern "C" fn $name(
            vm: *mut $crate::jni::sys::JavaVM,
            options: *mut ::core::ffi::c_char,
            reserved: *mut ::core::ffi::c_void,
        ) -> $crate::jni::sys::jint {
            use ::core::convert::Into;
            use ::core::ops::FnOnce;

            use $crate::agent::ResultCode;
            use $crate::jni::strings::JNIStr;
            use $crate::jni::vm::JavaVM;

            // inference sometimes breaks on closures with higher-kinded types but nudge via identity
            // helps circumvent error: implementation of `core::ops::FnOnce` is not general enough
            // see: https://users.rust-lang.org/t/i-dont-understand-this-lifetime-error/137426
            const fn infer<T, F>(f: F) -> F
            where
                T: Into<ResultCode>,
                F: FnOnce(JavaVM, &JNIStr) -> T,
            {
                f
            }

            _ = reserved;
            let jvm = unsafe { JavaVM::from_raw(vm) };
            let arg = unsafe { JNIStr::from_ptr(options) };
            let fun = infer($func);
            let res = fun(jvm, arg);
            Into::<ResultCode>::into(res).raw()
        }

        const _: $crate::sys::$name = $name;
    };
}

#[macro_export]
macro_rules! onload {
    ($func:expr) => {
        $crate::__agent_start!(Agent_OnLoad, $func);
    };
}

#[macro_export]
macro_rules! onattach {
    ($func:expr) => {
        $crate::__agent_start!(Agent_OnAttach, $func);
    };
}

#[macro_export]
macro_rules! onunload {
    ($func:expr) => {
        #[unsafe(no_mangle)]
        unsafe extern "C" fn Agent_OnUnload(vm: *mut $crate::jni::sys::JavaVM) {
            use $crate::jni::vm::JavaVM;

            const fn infer<F>(f: F) -> F
            where
                F: ::core::ops::FnOnce(JavaVM),
            {
                f
            }

            let jvm = unsafe { JavaVM::from_raw(vm) };
            let fun = infer($func);
            fun(jvm)
        }

        const _: $crate::sys::Agent_OnUnload = Agent_OnUnload;
    };
}

pub use onattach;
pub use onload;
pub use onunload;

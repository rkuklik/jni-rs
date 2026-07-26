use core::num::NonZero;

use jni::strings::JNIStr;
use jni::vm::JavaVM;
use jni_sys::jint;

type StartResult = Result<(), NonZero<jint>>;

pub trait AgentLoad {
    fn load(self, vm: JavaVM, options: &JNIStr) -> StartResult;
}

pub trait AgentAttach {
    fn attach(self, vm: JavaVM, options: &JNIStr) -> StartResult;
}

pub trait AgentUnload {
    fn unload(self, vm: JavaVM);
}

pub trait IntoStartResult {
    #[doc(hidden)]
    fn resolve(self) -> StartResult;
}

impl IntoStartResult for () {
    fn resolve(self) -> StartResult {
        Ok(())
    }
}

impl IntoStartResult for jint {
    fn resolve(self) -> StartResult {
        match NonZero::new(self) {
            Some(code) => Err(code),
            None => Ok(()),
        }
    }
}

impl IntoStartResult for StartResult {
    fn resolve(self) -> StartResult {
        self
    }
}

impl<T: IntoStartResult, F: FnOnce(JavaVM, &JNIStr) -> T> AgentLoad for F {
    fn load(self, vm: JavaVM, options: &JNIStr) -> Result<(), NonZero<jint>> {
        self(vm, options).resolve()
    }
}

impl<T: IntoStartResult, F: FnOnce(JavaVM, &JNIStr) -> T> AgentAttach for F {
    fn attach(self, vm: JavaVM, options: &JNIStr) -> Result<(), NonZero<jint>> {
        self(vm, options).resolve()
    }
}

impl<F: FnOnce(JavaVM)> AgentUnload for F {
    fn unload(self, vm: JavaVM) {
        self(vm)
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __agent_start {
    // this blight works around: implementation of `core::ops::FnOnce` is not general enough
    // See: https://users.rust-lang.org/t/i-dont-understand-this-lifetime-error/137426
    ($name:ident, $call:path, $($move:ident)? |$($arg:tt $(: $type:ty)?),+ $(,)?| $($body:tt)+) => {
        $crate::__agent_start!($name, $call, {
            // inference breaks on closures with higher-kinded types but nudge via identity function helps
            const fn infer<T, F>(f: F) -> F
            where
                T: $crate::agent::StartResult,
                F: ::core::ops::FnOnce($crate::jni::vm::JavaVM, &$crate::jni::strings::JNIStr) -> T,
            {
                f
            }
            infer($($move)? |$($arg $(:$type)?,)+| $($body)+)
        });
    };
    ($name:ident, $call:path, $agent:expr) => {
        #[unsafe(no_mangle)]
        unsafe extern "C" fn $name(
            vm: *mut $crate::jni::sys::JavaVM,
            options: *mut ::core::ffi::c_char,
            reserved: *mut ::core::ffi::c_void,
        ) -> $crate::jni::sys::jint {
            _ = reserved;
            let jvm = unsafe { $crate::jni::vm::JavaVM::from_raw(vm) };
            let arg = unsafe { $crate::jni::strings::JNIStr::from_ptr(options) };
            let res = $call($agent, jvm, arg);
            match res {
                Ok(()) => 0,
                Err(code) => code.get(),
            }
        }

        const _: $crate::sys::$name = $name;
    };
}

#[macro_export]
macro_rules! onload {
    ($($agent:tt)*) => {
        $crate::__agent_start!(Agent_OnLoad, $crate::agent::AgentLoad::load, $($agent)*);
    };
}

#[macro_export]
macro_rules! onattach {
    ($($agent:tt)*) => {
        $crate::__agent_start!(Agent_OnAttach, $crate::agent::AgentAttach::attach, $($agent)*);
    };
}

#[macro_export]
macro_rules! onunload {
    ($agent:expr) => {
        #[unsafe(no_mangle)]
        unsafe extern "C" fn Agent_OnUnload(vm: *mut $crate::jni::sys::JavaVM) {
            let jvm = unsafe { $crate::jni::vm::JavaVM::from_raw(vm) };
            $crate::agent::AgentUnload::unload($agent, jvm)
        }

        const _: $crate::sys::Agent_OnUnload = Agent_OnUnload;
    };
}

pub use onattach;
pub use onload;
pub use onunload;

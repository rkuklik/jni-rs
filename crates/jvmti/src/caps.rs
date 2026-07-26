use core::mem::transmute;

use bitflags::bitflags;
use jvmti_sys::jvmtiCapabilities;

use crate::env::EnvUntyped;
use crate::errors::Result;
use crate::macros::invoke;

macro_rules! caps {
    ($($name:ident => $bit:ident,)*) => {
    bitflags! {
        #[repr(transparent)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct Capabilities: u128 {
            $(
            const $name = 1 << jvmtiCapabilities::$bit;
            )*

            const _ = !0;
        }
    }
    }
}

caps! {
    TAG_OBJECTS => CAN_TAG_OBJECTS_BIT,
    GENERATE_FIELD_MODIFICATION_EVENTS => CAN_GENERATE_FIELD_MODIFICATION_EVENTS_BIT,
    GENERATE_FIELD_ACCESS_EVENTS => CAN_GENERATE_FIELD_ACCESS_EVENTS_BIT,
    GET_BYTECODES => CAN_GET_BYTECODES_BIT,
    GET_SYNTHETIC_ATTRIBUTE => CAN_GET_SYNTHETIC_ATTRIBUTE_BIT,
    GET_OWNED_MONITOR_INFO => CAN_GET_OWNED_MONITOR_INFO_BIT,
    GET_CURRENT_CONTENDED_MONITOR => CAN_GET_CURRENT_CONTENDED_MONITOR_BIT,
    GET_MONITOR_INFO => CAN_GET_MONITOR_INFO_BIT,
    POP_FRAME => CAN_POP_FRAME_BIT,
    REDEFINE_CLASSES => CAN_REDEFINE_CLASSES_BIT,
    SIGNAL_THREAD => CAN_SIGNAL_THREAD_BIT,
    GET_SOURCE_FILE_NAME => CAN_GET_SOURCE_FILE_NAME_BIT,
    GET_LINE_NUMBERS => CAN_GET_LINE_NUMBERS_BIT,
    GET_SOURCE_DEBUG_EXTENSION => CAN_GET_SOURCE_DEBUG_EXTENSION_BIT,
    ACCESS_LOCAL_VARIABLES => CAN_ACCESS_LOCAL_VARIABLES_BIT,
    MAINTAIN_ORIGINAL_METHOD_ORDER => CAN_MAINTAIN_ORIGINAL_METHOD_ORDER_BIT,
    GENERATE_SINGLE_STEP_EVENTS => CAN_GENERATE_SINGLE_STEP_EVENTS_BIT,
    GENERATE_EXCEPTION_EVENTS => CAN_GENERATE_EXCEPTION_EVENTS_BIT,
    GENERATE_FRAME_POP_EVENTS => CAN_GENERATE_FRAME_POP_EVENTS_BIT,
    GENERATE_BREAKPOINT_EVENTS => CAN_GENERATE_BREAKPOINT_EVENTS_BIT,
    SUSPEND => CAN_SUSPEND_BIT,
    REDEFINE_ANY_CLASS => CAN_REDEFINE_ANY_CLASS_BIT,
    GET_CURRENT_THREAD_CPU_TIME => CAN_GET_CURRENT_THREAD_CPU_TIME_BIT,
    GET_THREAD_CPU_TIME => CAN_GET_THREAD_CPU_TIME_BIT,
    GENERATE_METHOD_ENTRY_EVENTS => CAN_GENERATE_METHOD_ENTRY_EVENTS_BIT,
    GENERATE_METHOD_EXIT_EVENTS => CAN_GENERATE_METHOD_EXIT_EVENTS_BIT,
    GENERATE_ALL_CLASS_HOOK_EVENTS => CAN_GENERATE_ALL_CLASS_HOOK_EVENTS_BIT,
    GENERATE_COMPILED_METHOD_LOAD_EVENTS => CAN_GENERATE_COMPILED_METHOD_LOAD_EVENTS_BIT,
    GENERATE_MONITOR_EVENTS => CAN_GENERATE_MONITOR_EVENTS_BIT,
    GENERATE_VM_OBJECT_ALLOC_EVENTS => CAN_GENERATE_VM_OBJECT_ALLOC_EVENTS_BIT,
    GENERATE_NATIVE_METHOD_BIND_EVENTS => CAN_GENERATE_NATIVE_METHOD_BIND_EVENTS_BIT,
    GENERATE_GARBAGE_COLLECTION_EVENTS => CAN_GENERATE_GARBAGE_COLLECTION_EVENTS_BIT,
    GENERATE_OBJECT_FREE_EVENTS => CAN_GENERATE_OBJECT_FREE_EVENTS_BIT,
    FORCE_EARLY_RETURN => CAN_FORCE_EARLY_RETURN_BIT,
    GET_OWNED_MONITOR_STACK_DEPTH_INFO => CAN_GET_OWNED_MONITOR_STACK_DEPTH_INFO_BIT,
    GET_CONSTANT_POOL => CAN_GET_CONSTANT_POOL_BIT,
    SET_NATIVE_METHOD_PREFIX => CAN_SET_NATIVE_METHOD_PREFIX_BIT,
    RETRANSFORM_CLASSES => CAN_RETRANSFORM_CLASSES_BIT,
    RETRANSFORM_ANY_CLASS => CAN_RETRANSFORM_ANY_CLASS_BIT,
    GENERATE_RESOURCE_EXHAUSTION_HEAP_EVENTS => CAN_GENERATE_RESOURCE_EXHAUSTION_HEAP_EVENTS_BIT,
    GENERATE_RESOURCE_EXHAUSTION_THREADS_EVENTS => CAN_GENERATE_RESOURCE_EXHAUSTION_THREADS_EVENTS_BIT,
    GENERATE_EARLY_VMSTART => CAN_GENERATE_EARLY_VMSTART_BIT,
    GENERATE_EARLY_CLASS_HOOK_EVENTS => CAN_GENERATE_EARLY_CLASS_HOOK_EVENTS_BIT,
    GENERATE_SAMPLED_OBJECT_ALLOC_EVENTS => CAN_GENERATE_SAMPLED_OBJECT_ALLOC_EVENTS_BIT,
    SUPPORT_VIRTUAL_THREADS => CAN_SUPPORT_VIRTUAL_THREADS_BIT,
}

const _: () = assert!(size_of::<jvmtiCapabilities>() == size_of::<Capabilities>());

impl Capabilities {
    /// Constructs capabilities from raw capabilities.
    ///
    /// Consider using [`Self::empty`] or associated constants if you don't already
    /// have a [`jvmtiCapabilities`] instance.
    pub const fn new(raw: jvmtiCapabilities) -> Self {
        Self::from_bits_retain(u128::from_ne_bytes(raw.inner))
    }

    pub const fn raw(self) -> jvmtiCapabilities {
        jvmtiCapabilities {
            inner: self.bits().to_ne_bytes(),
        }
    }

    const fn as_raw(&self) -> &jvmtiCapabilities {
        // SAFETY: all values are valid for both types
        // SAFETY: `repr(transparent)` and going from higher to lower alignment
        const _: () = assert!(align_of::<jvmtiCapabilities>() <= align_of::<Capabilities>());
        unsafe { transmute::<&Self, &jvmtiCapabilities>(self) }
    }
}

impl From<jvmtiCapabilities> for Capabilities {
    fn from(value: jvmtiCapabilities) -> Self {
        Self::new(value)
    }
}

impl From<Capabilities> for jvmtiCapabilities {
    fn from(value: Capabilities) -> Self {
        value.raw()
    }
}

impl EnvUntyped {
    pub fn potential_capabilities_raw(&self) -> Result<jvmtiCapabilities> {
        let mut caps = jvmtiCapabilities::EMPTY;
        unsafe { invoke!(self, v1, GetPotentialCapabilities, &mut caps) }?;
        Ok(caps)
    }

    pub fn potential_capabilities(&self) -> Result<Capabilities> {
        self.potential_capabilities_raw().map(Capabilities::new)
    }

    pub fn capabilities_raw(&self) -> Result<jvmtiCapabilities> {
        let mut caps = jvmtiCapabilities::EMPTY;
        unsafe { invoke!(self, v1, GetCapabilities, &mut caps) }?;
        Ok(caps)
    }

    pub fn capabilities(&self) -> Result<Capabilities> {
        self.capabilities_raw().map(Capabilities::new)
    }

    pub fn add_capabilities_raw(&self, caps: &jvmtiCapabilities) -> Result<()> {
        unsafe { invoke!(self, v1, AddCapabilities, caps) }
    }

    pub fn add_capabilities(&self, caps: &Capabilities) -> Result<()> {
        self.add_capabilities_raw(caps.as_raw())
    }

    pub fn relinquish_capabilities_raw(&self, caps: &jvmtiCapabilities) -> Result<()> {
        unsafe { invoke!(self, v1, RelinquishCapabilities, caps) }
    }

    pub fn relinquish_capabilities(&self, caps: &Capabilities) -> Result<()> {
        self.relinquish_capabilities_raw(caps.as_raw())
    }
}

use core::ffi::c_char;
use core::ffi::c_uchar;
use core::ffi::c_void;
use core::marker::PhantomData;
use core::mem::transmute;
use core::ptr;
use core::slice::from_raw_parts;
use core::time::Duration;

use bitflags::bitflags;
use jni::Env as JNIEnv;
use jni::JValueOwned;
use jni::ids::JFieldID;
use jni::ids::JMethodID;
use jni::objects::JClass;
use jni::objects::JObject;
use jni::objects::JThread;
use jni::strings::JNIStr;
use jni::vm::AttachGuard;
use jni_sys::jboolean;
use jni_sys::jclass;
use jni_sys::jfieldID;
use jni_sys::jint;
use jni_sys::jlong;
use jni_sys::jmethodID;
use jni_sys::jobject;
use jni_sys::jvalue;
use jvmti_sys::JVMTI_RESOURCE_EXHAUSTED_JAVA_HEAP;
use jvmti_sys::JVMTI_RESOURCE_EXHAUSTED_OOM_ERROR;
use jvmti_sys::JVMTI_RESOURCE_EXHAUSTED_THREADS;
use jvmti_sys::jlocation;
use jvmti_sys::jthread;
use jvmti_sys::jvmtiAddrLocationMap;
use jvmti_sys::jvmtiEvent;
use jvmti_sys::jvmtiEventCallbacks;
use jvmti_sys::jvmtiEventMode;

use crate::env::Env;
use crate::env::EnvUntyped;
use crate::errors::Result;
use crate::macros::invoke;
use crate::macros::jenum;
use crate::memory::JBox;

impl EnvUntyped {
    /// # Safety
    ///
    /// The callbacks must either be [`ptr::null`] or point to initialized instance where
    /// all non-null functions must be safe to call with arguments described by JVMTI docs.
    pub unsafe fn set_event_callbacks_raw(
        &self,
        callbacks: *const jvmtiEventCallbacks,
    ) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v1,
                SetEventCallbacks,
                callbacks,
                const { size_of::<jvmtiEventCallbacks>() as jint },
            )
        }
    }

    pub fn remove_event_callbacks(&self) -> Result<()> {
        // SAFETY: `null` explicitly removes existing callbacks
        unsafe { self.set_event_callbacks_raw(ptr::null()) }
    }

    pub fn set_event_notification_mode_raw(
        &self,
        mode: jvmtiEventMode,
        event: jvmtiEvent,
        thread: jthread,
    ) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v1,
                SetEventNotificationMode,
                mode,
                event,
                thread,
                ptr::null::<()>(),
            )
        }
    }

    pub fn set_event_notification_mode_global(&self, event: Event, enable: bool) -> Result<()> {
        self.set_event_notification_mode(JThread::null(), event, enable)
    }

    pub fn set_event_notification_mode<'any>(
        &self,
        thread: impl AsRef<JThread<'any>>,
        event: Event,
        enable: bool,
    ) -> Result<()> {
        let mode = if enable {
            jvmtiEventMode::JVMTI_ENABLE
        } else {
            jvmtiEventMode::JVMTI_DISABLE
        };
        self.set_event_notification_mode_raw(mode, event.into(), thread.as_ref().as_raw())
    }

    pub fn generate_events_raw(&self, event: jvmtiEvent) -> Result<()> {
        unsafe { invoke!(self, v1, GenerateEvents, event) }
    }

    pub fn generate_events(&self, event: Event) -> Result<()> {
        self.generate_events_raw(event.into())
    }
}

impl<T> Env<T> {
    pub fn set_event_callbacks<E>(&self) -> Result<()>
    where
        E: EventCallbacks<Data = T>,
    {
        let vtable = CallbackBuilder::<E>::event();
        unsafe { self.set_event_callbacks_raw(&vtable) }
    }
}

jenum! {
    Event : jvmtiEvent {
        Breakpoint = JVMTI_EVENT_BREAKPOINT,
        ClassFileLoadHook = JVMTI_EVENT_CLASS_FILE_LOAD_HOOK,
        ClassLoad = JVMTI_EVENT_CLASS_LOAD,
        ClassPrepare = JVMTI_EVENT_CLASS_PREPARE,
        CompiledMethodLoad = JVMTI_EVENT_COMPILED_METHOD_LOAD,
        CompiledMethodUnload = JVMTI_EVENT_COMPILED_METHOD_UNLOAD,
        DataDumpRequest = JVMTI_EVENT_DATA_DUMP_REQUEST,
        DynamicCodeGenerated = JVMTI_EVENT_DYNAMIC_CODE_GENERATED,
        Exception = JVMTI_EVENT_EXCEPTION,
        ExceptionCatch = JVMTI_EVENT_EXCEPTION_CATCH,
        FieldAccess = JVMTI_EVENT_FIELD_ACCESS,
        FieldModification = JVMTI_EVENT_FIELD_MODIFICATION,
        FramePop = JVMTI_EVENT_FRAME_POP,
        GarbageCollectionStart = JVMTI_EVENT_GARBAGE_COLLECTION_START,
        GarbageCollectionFinish = JVMTI_EVENT_GARBAGE_COLLECTION_FINISH,
        MethodEntry = JVMTI_EVENT_METHOD_ENTRY,
        MethodExit = JVMTI_EVENT_METHOD_EXIT,
        MonitorContendedEnter = JVMTI_EVENT_MONITOR_CONTENDED_ENTER,
        MonitorContendedEntered = JVMTI_EVENT_MONITOR_CONTENDED_ENTERED,
        MonitorWait = JVMTI_EVENT_MONITOR_WAIT,
        MonitorWaited = JVMTI_EVENT_MONITOR_WAITED,
        NativeMethodBind = JVMTI_EVENT_NATIVE_METHOD_BIND,
        ObjectFree = JVMTI_EVENT_OBJECT_FREE,
        ResourceExhausted = JVMTI_EVENT_RESOURCE_EXHAUSTED,
        SampledObjectAlloc = JVMTI_EVENT_SAMPLED_OBJECT_ALLOC,
        SingleStep = JVMTI_EVENT_SINGLE_STEP,
        ThreadStart = JVMTI_EVENT_THREAD_START,
        ThreadEnd = JVMTI_EVENT_THREAD_END,
        VirtualThreadStart = JVMTI_EVENT_VIRTUAL_THREAD_START,
        VirtualThreadEnd = JVMTI_EVENT_VIRTUAL_THREAD_END,
        VmObjectAlloc = JVMTI_EVENT_VM_OBJECT_ALLOC,
        VmInit = JVMTI_EVENT_VM_INIT,
        VmStart = JVMTI_EVENT_VM_START,
        VmDeath = JVMTI_EVENT_VM_DEATH,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CodePosition {
    pub method: JMethodID,
    pub location: jlocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(doc), repr(C))] // for transmute, not public
pub struct AddrLocationMap {
    pub address: *const (),
    pub location: jlocation,
}

#[derive(Debug)]
pub struct NativeCode(*const ());

impl NativeCode {
    /// # Safety
    ///
    /// Supplied pointer must point to valid native method implementation, depending
    /// on which method is supposed to be implemented.
    ///
    /// After calling this function, the raw pointer allocation is owned by its result.
    pub const unsafe fn new(addr: *const ()) -> Self {
        Self(addr)
    }

    pub const fn get(self) -> *const () {
        self.0
    }
}

#[derive(Debug)]
pub struct NativeCodeMut(*mut ());

impl NativeCodeMut {
    /// # Safety
    ///
    /// Supplied pointer must point to valid native method implementation, depending
    /// on which method is supposed to be implemented.
    ///
    /// After calling this function, the raw pointer allocation is owned by its result.
    pub const unsafe fn new(addr: *mut ()) -> Self {
        Self(addr)
    }

    pub const fn get(self) -> *mut () {
        self.0
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ResourceExhaustionFlags: u32 {
        const OOM = JVMTI_RESOURCE_EXHAUSTED_OOM_ERROR;
        const HEAP = JVMTI_RESOURCE_EXHAUSTED_JAVA_HEAP;
        const THREADS = JVMTI_RESOURCE_EXHAUSTED_THREADS;

        const _ = !0;
    }
}

#[allow(unused_variables)]
pub trait EventCallbacks {
    type Data: Send + Sync;

    fn breakpoint<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        position: CodePosition,
    ) {
    }

    fn class_file_load_hook<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        class: JClass<'local>,
        loader: JObject<'local>,
        name: Option<&JNIStr>,
        protection_domain: JObject<'local>,
        data: &[u8],
    ) -> Option<JBox<[u8]>> {
        None
    }

    fn class_load<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        class: JClass<'local>,
    ) {
    }

    fn class_prepare<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        class: JClass<'local>,
    ) {
    }

    fn compiled_method_load(
        jvmti: &Env<Self::Data>,
        method: JMethodID,
        code: NativeCode,
        code_bytes: usize,
        map: Option<&[AddrLocationMap]>,
        info: *const (),
    ) {
    }

    fn compiled_method_unload(jvmti: &Env<Self::Data>, method: JMethodID, code: NativeCode) {}

    fn data_dump_request(jvmti: &Env<Self::Data>) {}

    fn dynamic_code_generated(
        jvmti: &Env<Self::Data>,
        name: &JNIStr,
        code: NativeCode,
        code_bytes: usize,
    ) {
    }

    fn exception<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        source: CodePosition,
        exception: JObject<'local>,
        catch: Option<CodePosition>,
    ) {
    }

    fn exception_catch<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        position: CodePosition,
        exception: JObject<'local>,
    ) {
    }

    fn field_access<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        position: CodePosition,
        class: JClass<'local>,
        object: JObject<'local>,
        field: JFieldID,
    ) {
    }

    #[expect(clippy::too_many_arguments)]
    fn field_modification<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        position: CodePosition,
        class: JClass<'local>,
        object: JObject<'local>,
        field: JFieldID,
        value: JValueOwned<'local>,
    ) {
    }

    fn frame_pop<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        method: JMethodID,
        by_exception: bool,
    ) {
    }

    fn garbage_collection_start(jvmti: &Env<Self::Data>) {}
    fn garbage_collection_finish(jvmti: &Env<Self::Data>) {}

    fn method_entry<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        method: JMethodID,
    ) {
    }

    fn method_exit<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        method: JMethodID,
        value: jvalue,
        by_exception: bool,
    ) {
    }

    fn monitor_contended_enter<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        object: JObject<'local>,
    ) {
    }

    fn monitor_contended_entered<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        object: JObject<'local>,
    ) {
    }

    fn monitor_wait<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        object: JObject<'local>,
        timeout: Duration,
    ) {
    }

    fn monitor_waited<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        object: JObject<'local>,
        timed_out: bool,
    ) {
    }

    fn native_method_bind<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        method: JMethodID,
        address: NativeCodeMut,
    ) -> Option<NativeCodeMut> {
        None
    }

    fn object_free(jvmti: &Env<Self::Data>, tag: jlong) {}

    fn resource_exhausted<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        flags: ResourceExhaustionFlags,
        description: &JNIStr,
    ) {
    }

    fn sampled_object_alloc<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        object: JObject<'local>,
        class: JClass<'local>,
        size: usize,
    ) {
    }

    fn single_step<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        position: CodePosition,
    ) {
    }

    fn thread_start<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
    ) {
    }
    fn thread_end<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
    ) {
    }

    fn virtual_thread_start<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
    ) {
    }
    fn virtual_thread_end<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
    ) {
    }

    fn vm_object_alloc<'local>(
        jvmti: &Env<Self::Data>,
        jni: &mut JNIEnv<'local>,
        thread: JThread<'local>,
        object: JObject<'local>,
        class: JClass<'local>,
        size: jlong,
    ) {
    }

    fn vm_init<'a>(jvmti: &Env<Self::Data>, jni: &mut JNIEnv<'a>, thread: JThread<'a>) {}
    fn vm_start(jvmti: &Env<Self::Data>, jni: &mut JNIEnv<'_>) {}
    fn vm_death(jvmti: &Env<Self::Data>, jni: &mut JNIEnv<'_>) {}
}

#[derive(Debug)]
pub struct CallbackBuilder<T: ?Sized>(PhantomData<T>);

macro_rules! b {
    (jvmti, $name:expr) => {
        &Env::<T::Data>::from_raw($name)
    };
    (jni, $name:expr) => {
        AttachGuard::<'_>::from_unowned($name).borrow_env_mut()
    };
    (opt, $cmd:ident, $ptr:expr $(,$t:tt)*) => {
        if $ptr.is_null() { None } else { Some(b!($cmd, $ptr $(,$t)*)) }
    };
    (slice, $ptr:expr, $len:expr) => {
        from_raw_parts($ptr, $len as usize)
    };
    (str, $ptr:expr) => {
        JNIStr::from_ptr($ptr)
    };
    (pos, $method:expr, $location:expr) => {
        CodePosition {
            method: JMethodID::from_raw($method),
            location: $location,
        }
    };
    (thread, $name:expr) => {
        transmute::<jthread, JThread<'_>>($name)
    };
    (class, $name:expr) => {
        transmute::<jclass, JClass<'_>>($name)
    };
    (obj, $name:expr) => {
        transmute::<jobject, JObject<'_>>($name)
    };
}

type JVMTIRaw = *mut jvmti_sys::jvmtiEnv;
type JNIRaw = *mut jni_sys::JNIEnv;

macro_rules! callbacks {
    ($f:ident) => {
        jvmtiEventCallbacks {
            VMInit: $f!(vm_init),
            VMDeath: $f!(vm_death),
            ThreadStart: $f!(thread_start),
            ThreadEnd: $f!(thread_end),
            ClassFileLoadHook: $f!(class_file_load_hook),
            ClassLoad: $f!(class_load),
            ClassPrepare: $f!(class_prepare),
            VMStart: $f!(vm_start),
            Exception: $f!(exception),
            ExceptionCatch: $f!(exception_catch),
            SingleStep: $f!(single_step),
            FramePop: $f!(frame_pop),
            Breakpoint: $f!(breakpoint),
            FieldAccess: $f!(field_access),
            FieldModification: $f!(field_modification),
            MethodEntry: $f!(method_entry),
            MethodExit: $f!(method_exit),
            NativeMethodBind: $f!(native_method_bind),
            CompiledMethodLoad: $f!(compiled_method_load),
            CompiledMethodUnload: $f!(compiled_method_unload),
            DynamicCodeGenerated: $f!(dynamic_code_generated),
            DataDumpRequest: $f!(data_dump_request),
            reserved72: None,
            MonitorWait: $f!(monitor_wait),
            MonitorWaited: $f!(monitor_waited),
            MonitorContendedEnter: $f!(monitor_contended_enter),
            MonitorContendedEntered: $f!(monitor_contended_entered),
            reserved77: None,
            reserved78: None,
            reserved79: None,
            ResourceExhausted: $f!(resource_exhausted),
            GarbageCollectionStart: $f!(garbage_collection_start),
            GarbageCollectionFinish: $f!(garbage_collection_finish),
            ObjectFree: $f!(object_free),
            VMObjectAlloc: $f!(vm_object_alloc),
            reserved85: None,
            SampledObjectAlloc: $f!(sampled_object_alloc),
            VirtualThreadStart: $f!(virtual_thread_start),
            VirtualThreadEnd: $f!(virtual_thread_end),
        }
    };
}

#[expect(unsafe_op_in_unsafe_fn)]
#[expect(missing_docs)]
#[expect(clippy::missing_safety_doc)]
impl<T: EventCallbacks + ?Sized> CallbackBuilder<T> {
    pub const fn event() -> jvmtiEventCallbacks {
        macro_rules! func {
            ($name:ident) => {
                Some(Self::$name)
            };
        }
        callbacks!(func)
    }

    pub fn event_nullable() -> jvmtiEventCallbacks {
        struct Noop;
        impl EventCallbacks for Noop {
            type Data = ();
        }
        macro_rules! func {
            ($name:ident) => {{
                #[expect(function_casts_as_integer)]
                let neq = Noop::$name as usize != T::$name as usize;
                neq.then_some(Self::$name)
            }};
        }
        callbacks!(func)
    }

    pub unsafe extern "C" fn breakpoint(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        location: jlocation,
    ) {
        T::breakpoint(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(pos, method, location),
        );
    }

    pub unsafe extern "C" fn class_file_load_hook(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        class_being_redefined: jclass,
        loader: jobject,
        name: *const c_char,
        protection_domain: jobject,
        class_data_len: jint,
        class_data: *const c_uchar,
        new_class_data_len: *mut jint,
        new_class_data: *mut *mut c_uchar,
    ) {
        let new = T::class_file_load_hook(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(class, class_being_redefined),
            b!(obj, loader),
            b!(opt, str, name),
            b!(obj, protection_domain),
            b!(slice, class_data, class_data_len),
        );
        if let Some::<*mut [u8]>(raw) = new.map(JBox::into_raw) {
            *new_class_data = raw.cast::<u8>();
            *new_class_data_len = raw.len() as jint;
        };
    }

    pub unsafe extern "C" fn class_load(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        klass: jclass,
    ) {
        T::class_load(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(class, klass),
        );
    }

    pub unsafe extern "C" fn class_prepare(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        klass: jclass,
    ) {
        T::class_prepare(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(class, klass),
        );
    }

    pub unsafe extern "C" fn compiled_method_load(
        jvmti_env: JVMTIRaw,
        method: jmethodID,
        code_size: jint,
        code_addr: *const c_void,
        map_length: jint,
        map: *const jvmtiAddrLocationMap,
        compile_info: *const c_void,
    ) {
        T::compiled_method_load(
            b!(jvmti, jvmti_env),
            JMethodID::from_raw(method),
            NativeCode::new(code_addr.cast::<()>()),
            code_size as usize,
            b!(opt, slice, map.cast::<AddrLocationMap>(), map_length),
            compile_info.cast::<()>(),
        );
    }

    pub unsafe extern "C" fn compiled_method_unload(
        jvmti_env: JVMTIRaw,
        method: jmethodID,
        code_addr: *const c_void,
    ) {
        T::compiled_method_unload(
            b!(jvmti, jvmti_env),
            JMethodID::from_raw(method),
            NativeCode::new(code_addr.cast::<()>()),
        );
    }

    pub unsafe extern "C" fn data_dump_request(jvmti_env: JVMTIRaw) {
        T::data_dump_request(b!(jvmti, jvmti_env));
    }

    pub unsafe extern "C" fn dynamic_code_generated(
        jvmti_env: JVMTIRaw,
        name: *const c_char,
        address: *const c_void,
        length: jint,
    ) {
        T::dynamic_code_generated(
            b!(jvmti, jvmti_env),
            b!(str, name),
            NativeCode::new(address.cast::<()>()),
            length as usize,
        );
    }

    pub unsafe extern "C" fn exception(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        location: jlocation,
        exception: jobject,
        catch_method: jmethodID,
        catch_location: jlocation,
    ) {
        T::exception(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(pos, method, location),
            b!(obj, exception),
            b!(opt, pos, catch_method, catch_location),
        );
    }

    pub unsafe extern "C" fn exception_catch(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        location: jlocation,
        exception: jobject,
    ) {
        T::exception_catch(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(pos, method, location),
            b!(obj, exception),
        );
    }

    pub unsafe extern "C" fn field_access(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        location: jlocation,
        field_klass: jclass,
        object: jobject,
        field: jfieldID,
    ) {
        T::field_access(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(pos, method, location),
            b!(class, field_klass),
            b!(obj, object),
            JFieldID::from_raw(field),
        );
    }

    pub unsafe extern "C" fn field_modification(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        location: jlocation,
        field_klass: jclass,
        object: jobject,
        field: jfieldID,
        signature_type: c_char,
        new_value: jvalue,
    ) {
        // `JavaType` parses more, we only get single char
        let value = match signature_type as u8 {
            b'Z' => JValueOwned::Bool(new_value.z),
            b'B' => JValueOwned::Byte(new_value.b),
            b'C' => JValueOwned::Char(new_value.c),
            b'S' => JValueOwned::Short(new_value.s),
            b'I' => JValueOwned::Int(new_value.i),
            b'J' => JValueOwned::Long(new_value.j),
            b'F' => JValueOwned::Float(new_value.f),
            b'D' => JValueOwned::Double(new_value.d),
            b'L' | b'[' => JValueOwned::Object(b!(obj, new_value.l)),
            _ => unreachable!("invalid signature type: {signature_type}"),
        };
        T::field_modification(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(pos, method, location),
            b!(class, field_klass),
            b!(obj, object),
            JFieldID::from_raw(field),
            value,
        );
    }

    pub unsafe extern "C" fn frame_pop(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        was_popped_by_exception: jboolean,
    ) {
        T::frame_pop(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            JMethodID::from_raw(method),
            was_popped_by_exception,
        );
    }

    pub unsafe extern "C" fn garbage_collection_start(jvmti_env: JVMTIRaw) {
        T::garbage_collection_start(b!(jvmti, jvmti_env));
    }

    pub unsafe extern "C" fn garbage_collection_finish(jvmti_env: JVMTIRaw) {
        T::garbage_collection_finish(b!(jvmti, jvmti_env));
    }

    pub unsafe extern "C" fn method_entry(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
    ) {
        T::method_entry(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            JMethodID::from_raw(method),
        );
    }

    pub unsafe extern "C" fn method_exit(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        was_popped_by_exception: jboolean,
        return_value: jvalue,
    ) {
        T::method_exit(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            JMethodID::from_raw(method),
            return_value,
            was_popped_by_exception,
        );
    }

    pub unsafe extern "C" fn monitor_contended_enter(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        object: jobject,
    ) {
        T::monitor_contended_enter(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(obj, object),
        );
    }

    pub unsafe extern "C" fn monitor_contended_entered(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        object: jobject,
    ) {
        T::monitor_contended_entered(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(obj, object),
        );
    }

    pub unsafe extern "C" fn monitor_wait(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        object: jobject,
        timeout: jlong,
    ) {
        T::monitor_wait(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(obj, object),
            Duration::from_millis(timeout as u64),
        );
    }

    pub unsafe extern "C" fn monitor_waited(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        object: jobject,
        timed_out: jboolean,
    ) {
        T::monitor_waited(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(obj, object),
            timed_out,
        );
    }

    pub unsafe extern "C" fn native_method_bind(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        address: *mut c_void,
        new_address_ptr: *mut *mut c_void,
    ) {
        let new = T::native_method_bind(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            JMethodID::from_raw(method),
            NativeCodeMut::new(address.cast()),
        );
        if let Some(ptr) = new {
            *new_address_ptr = ptr.get().cast();
        }
    }

    pub unsafe extern "C" fn object_free(jvmti_env: JVMTIRaw, tag: jlong) {
        T::object_free(b!(jvmti, jvmti_env), tag);
    }

    pub unsafe extern "C" fn resource_exhausted(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        flags: jint,
        reserved: *const c_void,
        description: *const c_char,
    ) {
        _ = reserved;
        T::resource_exhausted(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            ResourceExhaustionFlags::from_bits_retain(flags as u32),
            b!(str, description),
        );
    }

    pub unsafe extern "C" fn sampled_object_alloc(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        object: jobject,
        object_klass: jclass,
        size: jlong,
    ) {
        T::sampled_object_alloc(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(obj, object),
            b!(class, object_klass),
            size as usize,
        );
    }

    pub unsafe extern "C" fn single_step(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        method: jmethodID,
        location: jlocation,
    ) {
        T::single_step(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(pos, method, location),
        );
    }

    pub unsafe extern "C" fn thread_start(jvmti_env: JVMTIRaw, jni_env: JNIRaw, thread: jthread) {
        T::thread_start(b!(jvmti, jvmti_env), b!(jni, jni_env), b!(thread, thread));
    }

    pub unsafe extern "C" fn thread_end(jvmti_env: JVMTIRaw, jni_env: JNIRaw, thread: jthread) {
        T::thread_end(b!(jvmti, jvmti_env), b!(jni, jni_env), b!(thread, thread));
    }

    pub unsafe extern "C" fn virtual_thread_start(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
    ) {
        T::virtual_thread_start(b!(jvmti, jvmti_env), b!(jni, jni_env), b!(thread, thread));
    }
    pub unsafe extern "C" fn virtual_thread_end(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
    ) {
        T::virtual_thread_end(b!(jvmti, jvmti_env), b!(jni, jni_env), b!(thread, thread));
    }

    pub unsafe extern "C" fn vm_object_alloc(
        jvmti_env: JVMTIRaw,
        jni_env: JNIRaw,
        thread: jthread,
        object: jobject,
        object_klass: jclass,
        size: jlong,
    ) {
        T::vm_object_alloc(
            b!(jvmti, jvmti_env),
            b!(jni, jni_env),
            b!(thread, thread),
            b!(obj, object),
            b!(class, object_klass),
            size,
        );
    }

    pub unsafe extern "C" fn vm_init(jvmti_env: JVMTIRaw, jni_env: JNIRaw, thread: jthread) {
        T::vm_init(b!(jvmti, jvmti_env), b!(jni, jni_env), b!(thread, thread));
    }

    pub unsafe extern "C" fn vm_start(jvmti_env: JVMTIRaw, jni_env: JNIRaw) {
        T::vm_start(b!(jvmti, jvmti_env), b!(jni, jni_env));
    }

    pub unsafe extern "C" fn vm_death(jvmti_env: JVMTIRaw, jni_env: JNIRaw) {
        T::vm_death(b!(jvmti, jvmti_env), b!(jni, jni_env));
    }
}

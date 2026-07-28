use core::ffi::c_void;
use core::num::NonZero;
use core::ops::ControlFlow;
use core::ptr;
use core::slice::from_raw_parts;

use bitflags::bitflags;
use jni::Env as JNIEnv;
use jni::JValue;
use jni::ids::JMethodID;
use jni::objects::JClass;
use jni::objects::JObject;
use jni_sys::jboolean;
use jni_sys::jbyte;
use jni_sys::jchar;
use jni_sys::jclass;
use jni_sys::jdouble;
use jni_sys::jfloat;
use jni_sys::jint;
use jni_sys::jlong;
use jni_sys::jobject;
use jni_sys::jshort;
use jni_sys::jvalue;
use jvmti_sys::JVMTI_HEAP_FILTER_CLASS_TAGGED;
use jvmti_sys::JVMTI_HEAP_FILTER_CLASS_UNTAGGED;
use jvmti_sys::JVMTI_HEAP_FILTER_TAGGED;
use jvmti_sys::JVMTI_HEAP_FILTER_UNTAGGED;
use jvmti_sys::JVMTI_VISIT_ABORT;
use jvmti_sys::JVMTI_VISIT_OBJECTS;
use jvmti_sys::enum_t;
use jvmti_sys::jlocation;
use jvmti_sys::jvmtiHeapCallbacks;
use jvmti_sys::jvmtiHeapReferenceInfo;
use jvmti_sys::jvmtiHeapReferenceKind;
use jvmti_sys::jvmtiPrimitiveType;

use crate::env::EnvUntyped;
use crate::errors::Result;
use crate::events::CallbackBuilder;
use crate::macros::invoke;
use crate::macros::jenum;
use crate::memory::JBox;
use crate::memory::cast_box_slice;

#[derive(Debug)]
#[non_exhaustive]
pub struct TaggedObjects<O, T> {
    pub objects: Option<JBox<[O]>>,
    pub tags: Option<JBox<[T]>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagRequest {
    Objects,
    Tags,
    Both,
}

pub type JTag = NonZero<jlong>;

impl EnvUntyped {
    pub fn set_tag_raw(&self, object: jobject, tag: jlong) -> Result<()> {
        unsafe { invoke!(self, v1, SetTag, object, tag) }
    }

    pub fn set_tag<'any>(&self, object: impl AsRef<JObject<'any>>, tag: JTag) -> Result<()> {
        self.set_tag_raw(object.as_ref().as_raw(), tag.get())
    }

    pub fn get_tag_raw(&self, object: jobject) -> Result<jlong> {
        let mut tag = 0;
        unsafe { invoke!(self, v1, GetTag, object, &mut tag) }?;
        Ok(tag)
    }

    pub fn get_tag<'any>(&self, object: impl AsRef<JObject<'any>>) -> Result<Option<JTag>> {
        self.get_tag_raw(object.as_ref().as_raw()).map(JTag::new)
    }

    pub fn objects_with_tags_raw(
        &self,
        targets: &[jlong],
        request: TagRequest,
    ) -> Result<TaggedObjects<jobject, jlong>> {
        let objs = matches!(request, TagRequest::Both | TagRequest::Objects);
        let tags = matches!(request, TagRequest::Both | TagRequest::Tags);

        let mut count: jint = 0;
        let mut pobjs: *mut jobject = ptr::null_mut();
        let mut ptags: *mut jlong = ptr::null_mut();
        unsafe {
            invoke!(
                self,
                v1,
                GetObjectsWithTags,
                targets.len() as jint,
                targets.as_ptr(),
                &mut count,
                if objs { &mut pobjs } else { ptr::null_mut() },
                if tags { &mut ptags } else { ptr::null_mut() },
            )?;
            let alloc = self.allocator();
            let count = count as usize;
            let objs = objs.then(|| alloc.boxed_slice(pobjs, count));
            let tags = tags.then(|| alloc.boxed_slice(ptags, count));
            Ok(TaggedObjects {
                objects: objs,
                tags,
            })
        }
    }

    pub fn objects_with_tags<'local>(
        &self,
        env: &mut JNIEnv<'local>,
        targets: &[JTag],
        request: TagRequest,
    ) -> Result<TaggedObjects<JObject<'local>, JTag>> {
        env.assert_top();
        // SAFETY: `NonZero` is ABI compatible
        let targets = unsafe { from_raw_parts(targets.as_ptr().cast::<jlong>(), targets.len()) };
        let TaggedObjects { objects, tags } = self.objects_with_tags_raw(targets, request)?;
        // SAFETY: J*<'local> and j* are always compatible
        let objects = objects.map(|o| unsafe { cast_box_slice::<jobject, JObject<'local>>(o) });
        // SAFETY: tags must be `NonZero`; zero tags are not searched
        let tags = tags.map(|t| unsafe { cast_box_slice::<jlong, JTag>(t) });
        Ok(TaggedObjects { objects, tags })
    }

    pub fn force_garbage_collection(&self) -> Result<()> {
        unsafe { invoke!(self, v1, ForceGarbageCollection) }
    }

    /// # Safety
    ///
    /// All non-null callbacks must be safe to call with arguments described by JVMTI docs.
    pub unsafe fn follow_references_raw(
        &self,
        heap_filter: jint,
        klass: jclass,
        initial_object: jobject,
        callbacks: &jvmtiHeapCallbacks,
        user_data: *const (),
    ) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v1_1,
                FollowReferences,
                heap_filter,
                klass,
                initial_object,
                callbacks,
                user_data.cast(),
            )
        }
    }

    pub fn follow_references<'c, 'o, T: HeapVisitReference>(
        &self,
        visitor: &mut T,
        filter: HeapFilter,
        class: impl AsRef<JClass<'c>>,
        object: impl AsRef<JObject<'o>>,
    ) -> Result<()> {
        let data = visitor as *mut T;
        let table = CallbackBuilder::<T>::heap_reference();
        unsafe {
            self.follow_references_raw(
                filter.bits() as jint,
                class.as_ref().as_raw(),
                object.as_ref().as_raw(),
                &table,
                data.cast(),
            )
        }
    }

    /// # Safety
    ///
    /// All non-null callbacks must be safe to call with arguments described by JVMTI docs.
    pub unsafe fn iterate_heap_raw(
        &self,
        heap_filter: jint,
        klass: jclass,
        callbacks: &jvmtiHeapCallbacks,
        user_data: *const (),
    ) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v1_1,
                IterateThroughHeap,
                heap_filter,
                klass,
                callbacks,
                user_data.cast(),
            )
        }
    }

    pub fn iterate_heap<'any, T: HeapVisitObject>(
        &self,
        visitor: &mut T,
        filter: HeapFilter,
        class: impl AsRef<JClass<'any>>,
    ) -> Result<()> {
        let data = visitor as *mut T;
        let table = CallbackBuilder::<T>::heap_object();
        unsafe {
            self.iterate_heap_raw(
                filter.bits() as jint,
                class.as_ref().as_raw(),
                &table,
                data.cast(),
            )
        }
    }
}

jenum! {
    ReferenceKind : jvmtiHeapReferenceKind {
        Class = JVMTI_HEAP_REFERENCE_CLASS,
        Field = JVMTI_HEAP_REFERENCE_FIELD,
        ArrayElement = JVMTI_HEAP_REFERENCE_ARRAY_ELEMENT,
        ClassLoader = JVMTI_HEAP_REFERENCE_CLASS_LOADER,
        Signers = JVMTI_HEAP_REFERENCE_SIGNERS,
        ProtectionDomain = JVMTI_HEAP_REFERENCE_PROTECTION_DOMAIN,
        Interface = JVMTI_HEAP_REFERENCE_INTERFACE,
        StaticField = JVMTI_HEAP_REFERENCE_STATIC_FIELD,
        ConstantPool = JVMTI_HEAP_REFERENCE_CONSTANT_POOL,
        Superclass = JVMTI_HEAP_REFERENCE_SUPERCLASS,
        JniGlobal = JVMTI_HEAP_REFERENCE_JNI_GLOBAL,
        SystemClass = JVMTI_HEAP_REFERENCE_SYSTEM_CLASS,
        Monitor = JVMTI_HEAP_REFERENCE_MONITOR,
        StackLocal = JVMTI_HEAP_REFERENCE_STACK_LOCAL,
        JniLocal = JVMTI_HEAP_REFERENCE_JNI_LOCAL,
        Thread = JVMTI_HEAP_REFERENCE_THREAD,
        Other = JVMTI_HEAP_REFERENCE_OTHER,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldKind {
    Instance,
    Static,
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct HeapFilter: enum_t {
        const TAGGED = JVMTI_HEAP_FILTER_TAGGED;
        const UNTAGGED = JVMTI_HEAP_FILTER_UNTAGGED;
        const CLASS_TAGGED = JVMTI_HEAP_FILTER_CLASS_TAGGED;
        const CLASS_UNTAGGED = JVMTI_HEAP_FILTER_CLASS_UNTAGGED;

        const _ = !0;
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct VisitControl: enum_t {
        const OBJECTS = JVMTI_VISIT_OBJECTS;
        const ABORT = JVMTI_VISIT_ABORT;

        const _ = !0;
    }
}

impl From<ControlFlow<()>> for VisitControl {
    fn from(value: ControlFlow<()>) -> Self {
        match value {
            ControlFlow::Continue(()) => Self::empty(),
            ControlFlow::Break(()) => Self::ABORT,
        }
    }
}

impl From<ControlFlow<(), bool>> for VisitControl {
    fn from(value: ControlFlow<(), bool>) -> Self {
        match value {
            ControlFlow::Continue(false) => Self::empty(),
            ControlFlow::Continue(true) => Self::OBJECTS,
            ControlFlow::Break(()) => Self::ABORT,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ReferenceInfo<'a> {
    Index(usize),
    StackLocal(&'a StackLocalInfo),
    JniLocal(&'a JniLocalInfo),
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ThreadInfo {
    pub tag: Option<JTag>,
    pub id: jlong,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct JniLocalInfo {
    pub thread: ThreadInfo,
    pub depth: jint,
    pub method: JMethodID,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StackLocalInfo {
    pub thread: ThreadInfo,
    pub depth: jint,
    pub method: JMethodID,
    pub location: jlocation,
    pub slot: jint,
}

#[derive(Debug, Clone, Copy)]
pub enum ArrayElements<'a> {
    Bool(&'a [jboolean]),
    Byte(&'a [jbyte]),
    Char(&'a [jchar]),
    Short(&'a [jshort]),
    Int(&'a [jint]),
    Long(&'a [jlong]),
    Float(&'a [jfloat]),
    Double(&'a [jdouble]),
}

#[derive(Debug)]
pub struct HeapTags<'a> {
    pub class: Option<JTag>,
    pub object: &'a mut Option<JTag>,
}

#[derive(Debug)]
pub enum Referrer<'a> {
    HeapRoot,
    SelfReference,
    Object(HeapTags<'a>),
}

#[allow(unused_variables)]
pub trait HeapVisitPrimitive: Send {
    fn visit_primitive_field(
        &mut self,
        tags: HeapTags<'_>,
        kind: FieldKind,
        value: JValue<'_>,
        index: usize,
    ) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn visit_primitive_array(
        &mut self,
        tags: HeapTags<'_>,
        elements: ArrayElements<'_>,
        size: usize,
    ) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn visit_primitive_string(
        &mut self,
        tags: HeapTags<'_>,
        content: &[jchar],
        size: usize,
    ) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }
}

#[allow(unused_variables)]
pub trait HeapVisitObject: HeapVisitPrimitive + Send {
    fn visit_object(
        &mut self,
        tags: HeapTags<'_>,
        size: usize,
        array_length: Option<usize>,
    ) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }
}

#[allow(unused_variables)]
pub trait HeapVisitReference: HeapVisitPrimitive + Send {
    fn visit_reference(
        &mut self,
        tags: HeapTags<'_>,
        referrer: Referrer<'_>,
        kind: ReferenceKind,
        info: Option<ReferenceInfo<'_>>,
        size: usize,
        array_length: Option<usize>,
    ) -> ControlFlow<(), bool> {
        ControlFlow::Continue(true)
    }
}

macro_rules! b {
    (self, $name:expr) => {
        $name.cast::<T>().as_mut().unwrap_unchecked()
    };
    (tags, $class:expr, $object:expr) => {
        HeapTags {
            class: JTag::new($class),
            object: $object.cast::<Option<JTag>>().as_mut().unwrap_unchecked(),
        }
    };
    (kind, $kind:expr) => {
        ReferenceKind::try_from($kind).unwrap()
    };
}

fn ctrl(value: impl Into<VisitControl>) -> jint {
    value.into().bits() as jint
}

struct Noop;
impl HeapVisitPrimitive for Noop {}
impl HeapVisitReference for Noop {}
impl HeapVisitObject for Noop {}

macro_rules! func {
    ($table:ident, $name:ident, $wrap:ident) => {{
        #[expect(function_casts_as_integer)]
        if Noop::$name as usize != T::$name as usize {
            $table.$wrap = Some(Self::$wrap);
        }
    }};
}

#[expect(unsafe_op_in_unsafe_fn)]
#[expect(missing_docs)]
#[expect(clippy::missing_safety_doc)]
impl<T: HeapVisitPrimitive> CallbackBuilder<T> {
    pub fn heap_primitive() -> jvmtiHeapCallbacks {
        let mut t = jvmtiHeapCallbacks::new();
        func!(t, visit_primitive_field, primitive_field_callback);
        func!(t, visit_primitive_array, array_primitive_value_callback);
        func!(t, visit_primitive_string, string_primitive_value_callback);
        t
    }

    pub unsafe extern "C" fn primitive_field_callback(
        kind: jvmtiHeapReferenceKind,
        info: *const jvmtiHeapReferenceInfo,
        object_class_tag: jlong,
        object_tag_ptr: *mut jlong,
        value: jvalue,
        value_type: jvmtiPrimitiveType,
        user_data: *mut c_void,
    ) -> jint {
        let cf = T::visit_primitive_field(
            b!(self, user_data),
            b!(tags, object_class_tag, object_tag_ptr),
            match b!(kind, kind) {
                ReferenceKind::Field => FieldKind::Instance,
                ReferenceKind::StaticField => FieldKind::Static,
                _ => unreachable!(),
            },
            match value_type {
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_BOOLEAN => JValue::Bool(value.z),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_BYTE => JValue::Byte(value.b),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_CHAR => JValue::Char(value.c),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_SHORT => JValue::Short(value.s),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_INT => JValue::Int(value.i),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_LONG => JValue::Long(value.j),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_FLOAT => JValue::Float(value.f),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_DOUBLE => JValue::Double(value.d),
                _ => unreachable!(),
            },
            (*info).field.index as usize,
        );
        ctrl(cf)
    }

    pub unsafe extern "C" fn array_primitive_value_callback(
        class_tag: jlong,
        size: jlong,
        tag_ptr: *mut jlong,
        element_count: jint,
        element_type: jvmtiPrimitiveType,
        elements: *const c_void,
        user_data: *mut c_void,
    ) -> jint {
        macro_rules! arr {
            () => {
                from_raw_parts(elements.cast(), element_count as usize)
            };
        }

        let cf = T::visit_primitive_array(
            b!(self, user_data),
            b!(tags, class_tag, tag_ptr),
            match element_type {
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_BOOLEAN => ArrayElements::Bool(arr!()),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_BYTE => ArrayElements::Byte(arr!()),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_CHAR => ArrayElements::Char(arr!()),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_SHORT => ArrayElements::Short(arr!()),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_INT => ArrayElements::Int(arr!()),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_LONG => ArrayElements::Long(arr!()),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_FLOAT => ArrayElements::Float(arr!()),
                jvmtiPrimitiveType::JVMTI_PRIMITIVE_TYPE_DOUBLE => ArrayElements::Double(arr!()),
                _ => unreachable!(),
            },
            size as usize,
        );
        ctrl(cf)
    }

    pub unsafe extern "C" fn string_primitive_value_callback(
        class_tag: jlong,
        size: jlong,
        tag_ptr: *mut jlong,
        value: *const jchar,
        value_length: jint,
        user_data: *mut c_void,
    ) -> jint {
        let cf = T::visit_primitive_string(
            b!(self, user_data),
            b!(tags, class_tag, tag_ptr),
            from_raw_parts(value, value_length as usize),
            size as usize,
        );
        ctrl(cf)
    }
}

#[expect(unsafe_op_in_unsafe_fn)]
#[expect(missing_docs)]
#[expect(clippy::missing_safety_doc)]
impl<T: HeapVisitObject> CallbackBuilder<T> {
    pub fn heap_object() -> jvmtiHeapCallbacks {
        let mut t = Self::heap_primitive();
        func!(t, visit_object, heap_iteration_callback);
        t
    }

    pub unsafe extern "C" fn heap_iteration_callback(
        class_tag: jlong,
        size: jlong,
        tag_ptr: *mut jlong,
        length: jint,
        user_data: *mut c_void,
    ) -> jint {
        let cf = T::visit_object(
            b!(self, user_data),
            b!(tags, class_tag, tag_ptr),
            size as usize,
            usize::try_from(length).ok(),
        );
        ctrl(cf)
    }
}

#[expect(unsafe_op_in_unsafe_fn)]
#[expect(missing_docs)]
#[expect(clippy::missing_safety_doc)]
impl<T: HeapVisitReference> CallbackBuilder<T> {
    pub fn heap_reference() -> jvmtiHeapCallbacks {
        let mut t = Self::heap_primitive();
        func!(t, visit_reference, heap_reference_callback);
        t
    }

    pub unsafe extern "C" fn heap_reference_callback(
        reference_kind: jvmtiHeapReferenceKind,
        reference_info: *const jvmtiHeapReferenceInfo,
        class_tag: jlong,
        referrer_class_tag: jlong,
        size: jlong,
        tag_ptr: *mut jlong,
        referrer_tag_ptr: *mut jlong,
        length: jint,
        user_data: *mut c_void,
    ) -> jint {
        use ReferenceInfo as RI;
        use ReferenceKind as RK;
        let kind = b!(kind, reference_kind);
        let info = &*reference_info;
        let info = match kind {
            RK::Field | RK::StaticField => Some(RI::Index(info.field.index as usize)),
            RK::ArrayElement => Some(RI::Index(info.array.index as usize)),
            RK::ConstantPool => Some(RI::Index(info.constant_pool.index as usize)),
            RK::StackLocal => Some(RI::StackLocal(&*reference_info.cast::<StackLocalInfo>())),
            RK::JniLocal => Some(RI::JniLocal(&*reference_info.cast::<JniLocalInfo>())),
            _ => None,
        };
        let referrer = if referrer_tag_ptr.is_null() {
            Referrer::HeapRoot
        } else if referrer_tag_ptr == tag_ptr {
            Referrer::SelfReference
        } else {
            Referrer::Object(b!(tags, referrer_class_tag, referrer_tag_ptr))
        };
        let cf = T::visit_reference(
            b!(self, user_data),
            b!(tags, class_tag, tag_ptr),
            referrer,
            kind,
            info,
            size as usize,
            usize::try_from(length).ok(),
        );
        ctrl(cf)
    }
}

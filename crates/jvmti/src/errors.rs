use core::error;
use core::fmt;
use core::num::NonZero;

use jni::errors::Error as JNIError;
use jvmti_sys::enum_t;
use jvmti_sys::jvmtiError;

use crate::macros::jenum;

#[derive(Debug)]
#[repr(transparent)]
pub struct InvalidVariant<T>(pub T);

impl<T: fmt::Debug> fmt::Display for InvalidVariant<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("invalid/unknown variant: {:?}", self.0))
    }
}

impl<T: fmt::Debug> error::Error for InvalidVariant<T> {}

/// Trims whitespace and uses at most first sentence (for use in macros for display impl)
const fn errormsg(text: &str) -> &str {
    let text = text.trim_ascii();
    let bytes = text.as_bytes();
    let mut pos = 0;
    loop {
        if pos >= text.len() {
            return text;
        }
        if bytes[pos] == b'.' {
            // skip stuff like e.g.
            if pos < text.len() - 1 && !bytes[pos + 1].is_ascii_whitespace() {
                pos += 2;
                continue;
            }
            // TODO: use `split_at` once MSRV is increased
            match core::str::from_utf8(bytes.split_at(pos).0) {
                Ok(text) => return text,
                Err(_) => panic!("unreachable"),
            };
        }
        pos += 1;
    }
}

macro_rules! error {
    (
        $($variant:ident($type:ident)),*
        $(,)?
    ) => {
        /// Error produced by JVMTI
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Error {
            $(
            $variant($type),
            )*
            /// Unknown error value (likely missing binding from `jvmti.h`)
            Unknown(NonZero<u8>),
        }

        // guard that zero is indeed no error (so `Unknown` can be `NonZero`)
        const _: () = assert!(jvmtiError::JVMTI_ERROR_NONE.0 == 0);

        $(
        impl From<$type> for Error {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }

        impl $type {
            const MSGS: [&str; Self::LEN] = const {
                let mut res = [""; _];
                let mut idx = 0;
                while idx < res.len() {
                    res[idx] = errormsg(Self::DOCS_VAR[idx]);
                    idx += 1;
                }
                res
            };
        }

        // force message evaluation so that it's known to be correct
        #[cfg(debug_assertions)]
        const _: () = _ = $type::MSGS;

        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(Self::MSGS[self.index()])
            }
        }

        impl error::Error for $type {}
        )*

        impl TryFrom<jvmtiError> for Error {
            type Error = InvalidVariant<jvmtiError>;

            fn try_from(value: jvmtiError) -> Result<Self, Self::Error> {
                if value == jvmtiError::JVMTI_ERROR_NONE {
                    return Err(InvalidVariant(value));
                }
                $(
                if let Ok(err) = $type::try_from(value) {
                    return Ok(Self::$variant(err));
                };
                )*
                // SAFETY: const guard above
                let code = unsafe { NonZero::new_unchecked(value.0 as u8) };
                Ok(Self::Unknown(code))
            }
        }

        impl From<Error> for jvmtiError {
            fn from(value: Error) -> Self {
                match value {
                    $(Error::$variant(e) => e.into(),)*
                    Error::Unknown(code) => Self(enum_t::from(code.get())),
                }
            }
        }


        impl fmt::Display for Error {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(Self::$variant(_) => f.write_str(const { errormsg($type::DOCS_TOP) }),)*
                    Self::Unknown(code) => f.write_fmt(format_args!("unknown error code: {code}")),
                }
            }
        }

        impl error::Error for Error {
            fn source(&self) -> Option<&(dyn error::Error + 'static)> {
                match self {
                    $(Self::$variant(e) => Some(e),)*
                    Self::Unknown(_) => None,
                }
            }
        }
    };
}

jenum! {
    /// General JVMTI failure
    UniversalError : jvmtiError {
        /// Pointer is unexpectedly null
        NullPointer = JVMTI_ERROR_NULL_POINTER,
        /// The function attempted to allocate memory and no more memory was available for allocation
        OutOfMemory = JVMTI_ERROR_OUT_OF_MEMORY,
        /// The desired functionality has not been enabled in this virtual machine
        AccessDenied = JVMTI_ERROR_ACCESS_DENIED,
        /// The thread being used to call this function is not attached to the virtual machine. Calls must be made from attached threads. See AttachCurrentThread in the JNI invocation API
        UnattachedThread = JVMTI_ERROR_UNATTACHED_THREAD,
        /// The JVM TI environment provided is no longer connected or is not an environment
        InvalidEnvironment = JVMTI_ERROR_INVALID_ENVIRONMENT,
        /// The desired functionality is not available in the current phase
        WrongPhase = JVMTI_ERROR_WRONG_PHASE,
        /// An unexpected internal error has occurred
        Internal = JVMTI_ERROR_INTERNAL,
    }
}

jenum! {
    /// Thread error
    ThreadError : jvmtiError {
        /// Thread was not suspended
        NotSuspended = JVMTI_ERROR_THREAD_NOT_SUSPENDED,
        /// Thread already suspended
        Suspended = JVMTI_ERROR_THREAD_SUSPENDED,
        /// This operation requires the thread to be alive
        NotAlive = JVMTI_ERROR_THREAD_NOT_ALIVE,
        /// The state of the thread has been modified, and is now inconsistent
        InvalidTypestate = JVMTI_ERROR_INVALID_TYPESTATE,
    }
}

jenum! {
    /// Class (re)definition error
    ClassError : jvmtiError {
        /// A new class file is malformed (the VM would return a ClassFormatError)
        InvalidFormat = JVMTI_ERROR_INVALID_CLASS_FORMAT,
        /// The new class file definitions would lead to a circular definition (the VM would return a ClassCircularityError)
        CircularDefinition = JVMTI_ERROR_CIRCULAR_CLASS_DEFINITION,
        /// The class bytes fail verification
        FailsVerification = JVMTI_ERROR_FAILS_VERIFICATION,
        /// A new class file has a version number not supported by this VM
        UnsupportedVersion = JVMTI_ERROR_UNSUPPORTED_VERSION,
        /// The class has been loaded but not yet prepared
        NotPrepared = JVMTI_ERROR_CLASS_NOT_PREPARED,
        /// The class name defined in the new class file is different from the name in the old class object
        NamesDontMatch = JVMTI_ERROR_NAMES_DONT_MATCH,
        /// The class cannot be modified
        UnmodifiableClass = JVMTI_ERROR_UNMODIFIABLE_CLASS,
        /// The module cannot be modified
        UnmodifiableModule = JVMTI_ERROR_UNMODIFIABLE_MODULE,

        // Redefinition
        /// A new class file would require adding a method
        MethodAdded = JVMTI_ERROR_UNSUPPORTED_REDEFINITION_METHOD_ADDED,
        /// A new class version does not declare a method declared in the old class version
        MethodDeleted = JVMTI_ERROR_UNSUPPORTED_REDEFINITION_METHOD_DELETED,
        /// A new class version changes a field
        SchemaChanged = JVMTI_ERROR_UNSUPPORTED_REDEFINITION_SCHEMA_CHANGED,
        /// A direct superclass is different for the new class version, or the set of directly implemented interfaces is different
        HierarchyChanged = JVMTI_ERROR_UNSUPPORTED_REDEFINITION_HIERARCHY_CHANGED,
        /// A new class version has different modifiers
        ClassModifiersChanged = JVMTI_ERROR_UNSUPPORTED_REDEFINITION_CLASS_MODIFIERS_CHANGED,
        /// A method in the new class version has different modifiers than its counterpart in the old class version
        MethodModifiersChanged = JVMTI_ERROR_UNSUPPORTED_REDEFINITION_METHOD_MODIFIERS_CHANGED,
        /// A new class version has unsupported differences in class attributes
        ClassAttributeChanged = JVMTI_ERROR_UNSUPPORTED_REDEFINITION_CLASS_ATTRIBUTE_CHANGED,
    }
}

jenum! {
    /// Invalid JVMTI object data
    DataError : jvmtiError {
        /// Invalid thread
        Thread = JVMTI_ERROR_INVALID_THREAD,
        /// Invalid thread jenum
        ThreadGroup = JVMTI_ERROR_INVALID_THREAD_GROUP ,
        /// Invalid thread priority
        ThreadPriority = JVMTI_ERROR_INVALID_PRIORITY,
        /// Invalid raw monitor
        Monitor = JVMTI_ERROR_INVALID_MONITOR,
        /// Invalid field
        FieldID = JVMTI_ERROR_INVALID_FIELDID,
        /// Invalid method
        MethodID = JVMTI_ERROR_INVALID_METHODID,
        /// Invalid module
        Module = JVMTI_ERROR_INVALID_MODULE,
        /// Invalid location
        Location = JVMTI_ERROR_INVALID_LOCATION,
        /// Invalid object
        Object = JVMTI_ERROR_INVALID_OBJECT,
        /// Invalid class
        Class = JVMTI_ERROR_INVALID_CLASS,
        /// Invalid slot
        Slot = JVMTI_ERROR_INVALID_SLOT,
        /// The specified event type ID is not recognized
        EventType = JVMTI_ERROR_INVALID_EVENT_TYPE,
        /// The variable is not an appropriate type for the function used
        TypeMismatch = JVMTI_ERROR_TYPE_MISMATCH,
        /// Illegal argument
        IllegalArgument = JVMTI_ERROR_ILLEGAL_ARGUMENT,
    }
}

jenum! {
    /// Missing support for action
    SupportError : jvmtiError {
        /// The capability being used is false in this environment
        MissingCapability = JVMTI_ERROR_MUST_POSSESS_CAPABILITY,
        /// The class loader does not support this operation
        ClassLoaderUnsupported = JVMTI_ERROR_CLASS_LOADER_UNSUPPORTED,
        /// Functionality is unsupported in this implementation
        OperationUnsupported = JVMTI_ERROR_UNSUPPORTED_OPERATION,
        /// The functionality is not available in this virtual machine
        NotAvailable = JVMTI_ERROR_NOT_AVAILABLE,
    }
}

jenum! {
    /// Monitor operation failed
    MonitorError : jvmtiError {
        /// This thread doesn't own the raw monitor
        NotMonitorOwner = JVMTI_ERROR_NOT_MONITOR_OWNER,
        /// The call has been interrupted before completion
        Interrupt = JVMTI_ERROR_INTERRUPT,
    }
}

jenum! {
    /// Frame operation failed
    FrameError : jvmtiError {
        /// There are no Java programming language or JNI stack frames at the specified depth
        NoMoreFrames = JVMTI_ERROR_NO_MORE_FRAMES,
        /// Information about the frame is not available (e.g. for native frames), or the function cannot be performed on the thread's current frame
        OpaqueFrame = JVMTI_ERROR_OPAQUE_FRAME,
    }
}

jenum! {
    /// Miscellaneous query and set error
    ItemError : jvmtiError {
        /// Item already set
        Duplicate = JVMTI_ERROR_DUPLICATE,
        /// Desired element (e.g. field or breakpoint) not found
        NotFound = JVMTI_ERROR_NOT_FOUND,
        /// The requested information is not available
        AbsentInformation = JVMTI_ERROR_ABSENT_INFORMATION,
        /// The requested information is not available for native method
        NativeMethod = JVMTI_ERROR_NATIVE_METHOD,
    }
}

pub type Result<T, E = Error> = core::result::Result<T, E>;

error! {
    Universal(UniversalError),
    Thread(ThreadError),
    Class(ClassError),
    Data(DataError),
    Support(SupportError),
    Monitor(MonitorError),
    Frame(FrameError),
    Item(ItemError),
}

impl Error {
    pub fn code_to_result(code: jvmtiError) -> Result<(), Self> {
        match Self::try_from(code) {
            Err(_) => Ok(()),
            Ok(res) => Err(res),
        }
    }
}

#[derive(Debug)]
pub enum JError {
    JNI(JNIError),
    JVMTI(Error),
}

impl fmt::Display for JError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            JError::JNI(_) => "JNI error",
            JError::JVMTI(_) => "JVMTI error",
        };
        f.write_str(msg)
    }
}

impl error::Error for JError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            JError::JNI(e) => Some(e),
            JError::JVMTI(e) => Some(e),
        }
    }
}

impl From<JNIError> for JError {
    fn from(value: JNIError) -> Self {
        Self::JNI(value)
    }
}

impl From<Error> for JError {
    fn from(value: Error) -> Self {
        Self::JVMTI(value)
    }
}

#[derive(Debug)]
pub struct ContextError<T, E> {
    pub data: T,
    pub error: E,
}

impl<T, E> ContextError<T, E> {
    pub fn plain(self) -> E {
        self.error
    }
}

impl<T, E: fmt::Display> fmt::Display for ContextError<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl<T: fmt::Debug, E: error::Error> error::Error for ContextError<T, E> {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.error.source()
    }
}

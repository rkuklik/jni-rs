use jni_sys::jint;
use jvmti_sys::JVMTI_VERSION_INTERFACE_JVMTI;
use jvmti_sys::JVMTI_VERSION_MASK_INTERFACE_TYPE;
use jvmti_sys::JVMTI_VERSION_MASK_MAJOR;
use jvmti_sys::JVMTI_VERSION_MASK_MICRO;
use jvmti_sys::JVMTI_VERSION_MASK_MINOR;
use jvmti_sys::JVMTI_VERSION_SHIFT_MAJOR;
use jvmti_sys::JVMTI_VERSION_SHIFT_MICRO;
use jvmti_sys::JVMTI_VERSION_SHIFT_MINOR;
use jvmti_sys::enum_t;

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Ord, Eq, Hash)]
#[repr(transparent)]
pub struct JVMTIVersion(u32);

impl JVMTIVersion {
    pub const fn new(ver: u32) -> Option<Self> {
        if ver & JVMTI_VERSION_MASK_INTERFACE_TYPE == JVMTI_VERSION_INTERFACE_JVMTI {
            Some(Self(ver))
        } else {
            None
        }
    }

    /// # Safety
    ///
    /// Must be valid JVMTI interface version.
    pub const unsafe fn new_unchecked(ver: u32) -> Self {
        debug_assert!(
            Self::new(ver).is_some(),
            "provided value is not JVMTI version"
        );
        Self(ver)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const V1: Self = Self(jvmti_sys::JVMTI_VERSION_1);
    pub const V1_0: Self = Self(jvmti_sys::JVMTI_VERSION_1_0);
    pub const V1_1: Self = Self(jvmti_sys::JVMTI_VERSION_1_1);
    pub const V1_2: Self = Self(jvmti_sys::JVMTI_VERSION_1_2);
    pub const V9: Self = Self(jvmti_sys::JVMTI_VERSION_9);
    pub const V11: Self = Self(jvmti_sys::JVMTI_VERSION_11);
    pub const V19: Self = Self(jvmti_sys::JVMTI_VERSION_19);
    pub const V21: Self = Self(jvmti_sys::JVMTI_VERSION_21);

    const fn part(&self, mask: enum_t, shift: enum_t) -> u16 {
        let mut res = self.0;
        res &= mask;
        res >>= shift;
        res as u16
    }

    /// Get the major component of the version number
    pub const fn major(&self) -> u16 {
        self.part(JVMTI_VERSION_MASK_MAJOR, JVMTI_VERSION_SHIFT_MAJOR)
    }

    /// Get the minor component of the version number
    pub const fn minor(&self) -> u16 {
        self.part(JVMTI_VERSION_MASK_MINOR, JVMTI_VERSION_SHIFT_MINOR)
    }

    /// Get the micro component of the version number
    pub const fn micro(&self) -> u16 {
        self.part(JVMTI_VERSION_MASK_MICRO, JVMTI_VERSION_SHIFT_MICRO)
    }
}

impl From<JVMTIVersion> for jint {
    fn from(value: JVMTIVersion) -> Self {
        value.raw() as jint
    }
}

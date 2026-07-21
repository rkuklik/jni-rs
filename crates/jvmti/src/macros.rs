macro_rules! invoke {
    ($env:expr, $version:ident, $fn:ident $(,$args:expr )* $(,)?) => {{
        let env: *mut ::jvmti_sys::jvmtiEnv = $env.as_raw();
        let res = ((**env).$version.$fn)(env $(,$args)*);
        $crate::errors::Error::code_to_result(res)
    }};
}

macro_rules! jenum {
    ($(#[doc = $top:literal])* $type:ident : $raw:ident {
        $($(#[doc = $doc:literal])* $variant:ident = $value:ident),*
        $(,)?
    }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        $(#[doc = $top])*
        pub enum $type {
            $(
                $(#[doc = $doc])*
                $variant = $raw::$value.0 as u8,
            )*
        }

        // docs extraction support (for error), please don't be revolted
        #[allow(dead_code)]
        impl $type {
            const ALL: [Self; Self::LEN] = [$(Self::$variant,)*];
            const DOCS_TOP: &str = concat!($($top,'\n',)*);
            const DOCS_VAR: [&str; Self::LEN] = [$(concat!($($doc,'\n',)*),)*];
            const LEN: usize = {
                let mut count = 0;
                $(
                _ = Self::$variant;
                count += 1;
                )*
                count
            };

            const fn index(&self) -> usize {
                let mut idx = 0;
                loop {
                    if Self::ALL[idx] as u8 == *self as u8 {
                        return idx;
                    }
                    idx += 1;
                }
            }
        }

        impl TryFrom<$raw> for $type {
            type Error = $crate::errors::InvalidVariant<$raw>;

            fn try_from(value: $raw) -> Result<Self, Self::Error> {
                match value {
                    $($raw::$value => Ok(Self::$variant),)*
                    _ => Err($crate::errors::InvalidVariant::<$raw>(value)),
                }
            }
        }

        impl From<$type> for $raw {
            fn from(value: $type) -> Self {
                match value {
                    $($type::$variant => Self::$value,)*
                }
            }
        }
    };
}

pub(crate) use invoke;
pub(crate) use jenum;

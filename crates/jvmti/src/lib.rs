//#![warn(missing_docs)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![warn(clippy::alloc_instead_of_core)]
#![warn(clippy::std_instead_of_core)]
#![warn(clippy::std_instead_of_alloc)]
#![deny(missing_debug_implementations)]

//! # Safe JVMTI Bindings in Rust
//!
//! This is a placeholder for now, development happens at:
//! <https://github.com/rkuklik/jni-rs>

extern crate alloc;
extern crate core;

pub use jni;
pub use jvmti_sys as sys;

pub mod agent;
pub mod caps;
pub mod env;
pub mod errors;
pub mod events;
pub mod macros;
pub mod memory;
pub mod thread;
pub mod version;

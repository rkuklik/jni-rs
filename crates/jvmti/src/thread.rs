use core::ffi::c_void;
use core::mem::ManuallyDrop;
use core::mem::MaybeUninit;
use core::mem::transmute;
use core::mem::transmute_copy;
use core::ptr;
use core::ptr::NonNull;

use alloc::boxed::Box;
use alloc::vec::Vec;

use bitflags::bitflags;
use jni::Env as JNIEnv;
use jni::bind_java_type;
use jni::jni_sig;
use jni::jni_str;
use jni::objects::JClassLoader;
use jni::objects::JObject;
use jni::objects::JThread;
use jni::refs::Reference;
use jni::strings::JNIStr;
use jni::vm::AttachGuard;
use jni_sys::jint;
use jni_sys::jobject;
use jvmti_sys::JVMTI_THREAD_MAX_PRIORITY;
use jvmti_sys::JVMTI_THREAD_MIN_PRIORITY;
use jvmti_sys::JVMTI_THREAD_NORM_PRIORITY;
use jvmti_sys::JVMTI_THREAD_STATE_ALIVE;
use jvmti_sys::JVMTI_THREAD_STATE_BLOCKED_ON_MONITOR_ENTER;
use jvmti_sys::JVMTI_THREAD_STATE_IN_NATIVE;
use jvmti_sys::JVMTI_THREAD_STATE_IN_OBJECT_WAIT;
use jvmti_sys::JVMTI_THREAD_STATE_INTERRUPTED;
use jvmti_sys::JVMTI_THREAD_STATE_PARKED;
use jvmti_sys::JVMTI_THREAD_STATE_RUNNABLE;
use jvmti_sys::JVMTI_THREAD_STATE_SLEEPING;
use jvmti_sys::JVMTI_THREAD_STATE_SUSPENDED;
use jvmti_sys::JVMTI_THREAD_STATE_TERMINATED;
use jvmti_sys::JVMTI_THREAD_STATE_VENDOR_1;
use jvmti_sys::JVMTI_THREAD_STATE_VENDOR_2;
use jvmti_sys::JVMTI_THREAD_STATE_VENDOR_3;
use jvmti_sys::JVMTI_THREAD_STATE_WAITING;
use jvmti_sys::JVMTI_THREAD_STATE_WAITING_INDEFINITELY;
use jvmti_sys::JVMTI_THREAD_STATE_WAITING_WITH_TIMEOUT;
use jvmti_sys::enum_t;
use jvmti_sys::jthread;
use jvmti_sys::jthreadGroup;
use jvmti_sys::jvmtiError;
use jvmti_sys::jvmtiMonitorStackDepthInfo;
use jvmti_sys::jvmtiStartFunction;
use jvmti_sys::jvmtiThreadGroupInfo;
use jvmti_sys::jvmtiThreadInfo;

use crate::env::Env;
use crate::env::EnvUntyped;
use crate::errors::Error;
use crate::errors::JError;
use crate::errors::Result;
use crate::macros::invoke;
use crate::memory::JBox;
use crate::memory::cast_box_slice;
use crate::version::JVMTIVersion;

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ThreadState: enum_t {
        const ALIVE = JVMTI_THREAD_STATE_ALIVE;
        const TERMINATED = JVMTI_THREAD_STATE_TERMINATED;
        const RUNNABLE = JVMTI_THREAD_STATE_RUNNABLE;
        const BLOCKED_ON_MONITOR_ENTER = JVMTI_THREAD_STATE_BLOCKED_ON_MONITOR_ENTER;
        const WAITING = JVMTI_THREAD_STATE_WAITING;
        const WAITING_INDEFINITELY = JVMTI_THREAD_STATE_WAITING_INDEFINITELY;
        const WAITING_WITH_TIMEOUT = JVMTI_THREAD_STATE_WAITING_WITH_TIMEOUT;
        const SLEEPING = JVMTI_THREAD_STATE_SLEEPING;
        const IN_OBJECT_WAIT = JVMTI_THREAD_STATE_IN_OBJECT_WAIT;
        const PARKED = JVMTI_THREAD_STATE_PARKED;
        const SUSPENDED = JVMTI_THREAD_STATE_SUSPENDED;
        const INTERRUPTED = JVMTI_THREAD_STATE_INTERRUPTED;
        const IN_NATIVE = JVMTI_THREAD_STATE_IN_NATIVE;
        const VENDOR1 = JVMTI_THREAD_STATE_VENDOR_1;
        const VENDOR2 = JVMTI_THREAD_STATE_VENDOR_2;
        const VENDOR3 = JVMTI_THREAD_STATE_VENDOR_3;

        const _ = !0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadPriority(enum_t);

impl ThreadPriority {
    pub const MIN: Self = Self(JVMTI_THREAD_MIN_PRIORITY);
    pub const MAX: Self = Self(JVMTI_THREAD_MAX_PRIORITY);
    pub const NORMAL: Self = Self(JVMTI_THREAD_NORM_PRIORITY);

    pub const fn new(value: enum_t) -> Option<Self> {
        if Self::MIN.0 <= value && value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// # Safety
    ///
    /// Must be between [`Self::MIN`] and [`Self::MAX`].
    pub const unsafe fn new_unchecked(value: enum_t) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> enum_t {
        self.0
    }
}

impl Default for ThreadPriority {
    fn default() -> Self {
        Self::NORMAL
    }
}

// HACK: can't specify jvmti type in `__sys_type`, so reexport
#[doc(hidden)]
mod __hack {
    pub use jni::*;
    pub mod sys {
        pub use jni_sys::*;
        pub use jvmti_sys::*;
    }
}

// TODO: instance methods
bind_java_type! {
    jni = __hack,
    __sys_type = jthreadGroup,
    pub JThreadGroup => "java.lang.ThreadGroup",
}

#[derive(Debug)]
pub struct ThreadInfo<G, L> {
    pub name: Option<JBox<JNIStr>>,
    pub priority: ThreadPriority,
    pub daemon: bool,
    pub thread_group: G,
    pub context_class_loader: L,
}

#[derive(Debug)]
pub struct ThreadGroupInfo<G> {
    pub name: Option<JBox<JNIStr>>,
    pub parent: G,
    pub max_priority: ThreadPriority,
    pub daemon: bool,
}

#[derive(Debug)]
pub struct ThreadGroupChildren<T, G> {
    pub threads: JBox<[T]>,
    pub groups: JBox<[G]>,
}

#[derive(Debug)]
#[cfg_attr(not(doc), repr(C))] // for transmute, not public
pub struct MonitorStackDepth<M> {
    pub monitor: M,
    pub depth: jint,
}

unsafe fn assume_init_mut<T>(slice: &mut [MaybeUninit<T>]) -> &mut [T] {
    // TODO: `assume_init_mut` after MSRV at 1.93.0
    unsafe { transmute::<&mut [MaybeUninit<T>], &mut [T]>(slice) }
}

fn threads_action<'any, I, F>(threads: I, action: F) -> Result<Box<[Result<()>]>, Error>
where
    I: IntoIterator,
    I::Item: AsRef<JThread<'any>>,
    F: for<'a> FnOnce(
        &[jthread],
        &'a mut [MaybeUninit<jvmtiError>],
    ) -> Result<&'a mut [jvmtiError]>,
{
    let threads: Vec<jthread> = threads
        .into_iter()
        .map(|item| item.as_ref().as_raw())
        .collect();
    let mut results = Box::new_uninit_slice(threads.len());
    // TODO: if `Result<()>` will have same size/align as `jvmtiError`, reuse allocation
    Ok(action(&threads, &mut results)?
        .iter()
        .copied()
        .map(Error::code_to_result)
        .collect())
}

impl EnvUntyped {
    pub fn thread_state_raw(&self, thread: jthread) -> Result<enum_t> {
        let mut state: jint = 0;
        unsafe { invoke!(self, v1, GetThreadState, thread, &mut state) }?;
        Ok(state as enum_t)
    }

    pub fn thread_state<'any>(&self, thread: impl AsRef<JThread<'any>>) -> Result<ThreadState> {
        self.thread_state_raw(thread.as_ref().as_raw())
            .map(ThreadState::from_bits_retain)
    }

    pub fn current_thread_raw(&self) -> Result<jthread, Error> {
        let mut thread: jthread = ptr::null_mut();
        unsafe { invoke!(self, v1_1, GetCurrentThread, &mut thread) }?;
        Ok(thread)
    }

    pub fn current_thread<'local>(&self, env: &mut JNIEnv<'local>) -> Result<JThread<'local>> {
        // SAFETY: mutable reference to `env` guarantees top stack frame
        env.assert_top();
        Ok(unsafe { JThread::from_raw(env, self.current_thread_raw()?) })
    }

    pub fn all_threads_raw(&self) -> Result<JBox<[jthread]>> {
        let mut count: jint = 0;
        let mut threads: *mut jthread = ptr::null_mut();
        unsafe {
            invoke!(self, v1, GetAllThreads, &mut count, &mut threads)?;
            Ok(self.allocator().boxed_slice(threads, count as usize))
        }
    }

    pub fn all_threads<'local>(&self, env: &mut JNIEnv<'local>) -> Result<JBox<[JThread<'local>]>> {
        // SAFETY: mutable reference to `env` guarantees top stack frame
        // SAFETY: J*<'local> and j* are always compatible
        env.assert_top();
        self.all_threads_raw()
            .map(|raw| unsafe { cast_box_slice::<jthread, JThread<'local>>(raw) })
    }

    pub fn suspend_thread_raw(&self, thread: jthread) -> Result<()> {
        unsafe { invoke!(self, v1, SuspendThread, thread) }
    }

    pub fn suspend_thread<'any>(&self, thread: impl AsRef<JThread<'any>>) -> Result<()> {
        self.suspend_thread_raw(thread.as_ref().as_raw())
    }

    pub fn suspend_threads_raw<'res>(
        &self,
        threads: &[jthread],
        results: &'res mut [MaybeUninit<jvmtiError>],
    ) -> Result<&'res mut [jvmtiError], Error> {
        assert!(threads.len() <= results.len());
        let res = unsafe {
            invoke!(
                self,
                v1,
                SuspendThreadList,
                threads.len() as jint,
                threads.as_ptr(),
                results.as_mut_ptr().cast(),
            )?;
            assume_init_mut(results.get_unchecked_mut(..threads.len()))
        };
        Ok(res)
    }

    pub fn suspend_threads<'any, I>(&self, threads: I) -> Result<Box<[Result<()>]>, Error>
    where
        I: IntoIterator,
        I::Item: AsRef<JThread<'any>>,
    {
        threads_action(threads, |t, r| self.suspend_threads_raw(t, r))
    }

    /// # Safety
    ///
    /// [`self`][Env] must be at least version 21
    pub unsafe fn suspend_all_virtual_threads_raw(&self, except: &[jthread]) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v21,
                SuspendAllVirtualThreads,
                except.len() as jint,
                except.as_ptr(),
            )
        }
    }

    pub fn suspend_all_virtual_threads<'any, I>(&self, except: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<JThread<'any>>,
    {
        self.ensure_version(JVMTIVersion::V21)?;
        let except: Vec<jthread> = except
            .into_iter()
            .map(|item| item.as_ref().as_raw())
            .collect();
        // SAFETY: version checked above
        unsafe { self.suspend_all_virtual_threads_raw(&except) }
    }

    pub fn resume_thread_raw(&self, thread: jthread) -> Result<()> {
        unsafe { invoke!(self, v1, ResumeThread, thread) }
    }

    pub fn resume_thread<'any>(&self, thread: impl AsRef<JThread<'any>>) -> Result<()> {
        self.resume_thread_raw(thread.as_ref().as_raw())
    }

    pub fn resume_threads_raw<'res>(
        &self,
        threads: &[jthread],
        results: &'res mut [MaybeUninit<jvmtiError>],
    ) -> Result<&'res mut [jvmtiError], Error> {
        assert!(threads.len() <= results.len());
        let res = unsafe {
            invoke!(
                self,
                v1,
                ResumeThreadList,
                threads.len() as jint,
                threads.as_ptr(),
                results.as_mut_ptr().cast(),
            )?;
            assume_init_mut(results.get_unchecked_mut(..threads.len()))
        };
        Ok(res)
    }

    pub fn resume_threads<'any, I>(&self, threads: I) -> Result<Box<[Result<()>]>, Error>
    where
        I: IntoIterator,
        I::Item: AsRef<JThread<'any>>,
    {
        threads_action(threads, |t, r| self.resume_threads_raw(t, r))
    }

    /// # Safety
    ///
    /// [`self`][Env] must be at least version 21
    pub unsafe fn resume_all_virtual_threads_raw(&self, except: &[jthread]) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v21,
                ResumeAllVirtualThreads,
                except.len() as jint,
                except.as_ptr(),
            )
        }
    }

    pub fn resume_all_virtual_threads<'any, I>(&self, except: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<JThread<'any>>,
    {
        self.ensure_version(JVMTIVersion::V21)?;
        let except: Vec<_> = except
            .into_iter()
            .map(|item| item.as_ref().as_raw())
            .collect();
        // SAFETY: version checked above
        unsafe { self.resume_all_virtual_threads_raw(&except) }
    }

    pub fn stop_thread_raw(&self, thread: jthread, exception: jobject) -> Result<()> {
        unsafe { invoke!(self, v1, StopThread, thread, exception) }
    }

    pub fn stop_thread<'any>(
        &self,
        thread: impl AsRef<JThread<'any>>,
        exception: impl AsRef<JObject<'any>>,
    ) -> Result<()> {
        self.stop_thread_raw(thread.as_ref().as_raw(), exception.as_ref().as_raw())
    }

    pub fn interrupt_thread_raw(&self, thread: jthread) -> Result<()> {
        unsafe { invoke!(self, v1, InterruptThread, thread) }
    }

    pub fn interrupt_thread<'any>(&self, thread: impl AsRef<JThread<'any>>) -> Result<()> {
        self.interrupt_thread_raw(thread.as_ref().as_raw())
    }

    pub fn thread_info_raw(&self, thread: jthread) -> Result<jvmtiThreadInfo> {
        let mut info: MaybeUninit<jvmtiThreadInfo> = MaybeUninit::uninit();
        // SAFETY: initialized on successful return
        unsafe {
            invoke!(self, v1, GetThreadInfo, thread, info.as_mut_ptr())?;
            Ok(info.assume_init())
        }
    }

    pub fn thread_info<'local, 'any, T>(
        &self,
        env: &mut JNIEnv<'local>,
        thread: impl AsRef<JThread<'any>>,
    ) -> Result<ThreadInfo<JThreadGroup<'local>, JClassLoader<'local>>> {
        env.assert_top();
        let jvmtiThreadInfo {
            name,
            priority,
            is_daemon,
            thread_group,
            context_class_loader,
        } = self.thread_info_raw(thread.as_ref().as_raw())?;
        // SAFETY: name is guaranteed to be newly allocated MUTF-8 string
        let name = NonNull::new(name).map(|ptr| unsafe { self.allocator().boxed_jnistr(ptr) });
        // SAFETY: mutable reference to `env` guarantees top stack frame
        let thread_group = unsafe { JThreadGroup::from_raw(env, thread_group) };
        let context_class_loader = unsafe { JClassLoader::from_raw(env, context_class_loader) };
        // SAFETY: it comes as JVMTI saw it
        let priority = unsafe { ThreadPriority::new_unchecked(priority as u32) };
        Ok(ThreadInfo {
            name,
            priority,
            daemon: is_daemon,
            thread_group,
            context_class_loader,
        })
    }

    pub fn owned_monitor_info_raw(&self, thread: jthread) -> Result<JBox<[jthread]>> {
        let mut count: jint = 0;
        let mut monitors: *mut jobject = ptr::null_mut();
        // SAFETY: the slice is allocated by env or zero size (both valid)
        unsafe {
            invoke!(
                self,
                v1,
                GetOwnedMonitorInfo,
                thread,
                &mut count,
                &mut monitors,
            )?;
            Ok(self.allocator().boxed_slice(monitors, count as usize))
        }
    }

    pub fn owned_monitor_info<'local, 'any>(
        &self,
        env: &mut JNIEnv<'local>,
        thread: impl AsRef<JThread<'any>>,
    ) -> Result<JBox<[JObject<'local>]>> {
        // SAFETY: mutable reference to `env` guarantees top stack frame
        // SAFETY: J*<'local> and j* are always compatible
        env.assert_top();
        self.owned_monitor_info_raw(thread.as_ref().as_raw())
            .map(|raw| unsafe { cast_box_slice::<jobject, JObject<'local>>(raw) })
    }

    pub fn owned_monitor_stack_depth_info_raw(
        &self,
        thread: jthread,
    ) -> Result<JBox<[jvmtiMonitorStackDepthInfo]>> {
        let mut count: jint = 0;
        let mut infos: *mut jvmtiMonitorStackDepthInfo = ptr::null_mut();
        // SAFETY: the slice is allocated by env or zero size (both valid)
        unsafe {
            invoke!(
                self,
                v1_1,
                GetOwnedMonitorStackDepthInfo,
                thread,
                &mut count,
                &mut infos,
            )?;
            Ok(self.allocator().boxed_slice(infos, count as usize))
        }
    }

    pub fn owned_monitor_stack_depth_info<'local, 'any>(
        &self,
        env: &mut JNIEnv<'local>,
        thread: impl AsRef<JThread<'any>>,
    ) -> Result<JBox<[MonitorStackDepth<JObject<'local>>]>> {
        // SAFETY: mutable reference to `env` guarantees top stack frame
        // SAFETY: `repr(C)` with compatible compatible fields
        env.assert_top();
        let cast = cast_box_slice::<jvmtiMonitorStackDepthInfo, MonitorStackDepth<JObject<'local>>>;
        self.owned_monitor_stack_depth_info_raw(thread.as_ref().as_raw())
            .map(|raw| unsafe { cast(raw) })
    }

    pub fn current_contended_monitor_raw(&self, thread: jthread) -> Result<jobject> {
        let mut object: jobject = ptr::null_mut();
        unsafe {
            invoke!(self, v1, GetCurrentContendedMonitor, thread, &mut object)?;
            Ok(object)
        }
    }

    pub fn current_contended_monitor<'local, 'any>(
        &self,
        env: &mut JNIEnv<'local>,
        thread: impl AsRef<JThread<'any>>,
    ) -> Result<JObject<'local>> {
        // SAFETY: mutable reference to `env` guarantees top stack frame
        env.assert_top();
        Ok(unsafe {
            JObject::from_raw(
                env,
                self.current_contended_monitor_raw(thread.as_ref().as_raw())?,
            )
        })
    }

    /// # Safety
    ///
    /// The function in `proc` must be safe to call in a newly created thread with `arg`.
    pub unsafe fn run_agent_thread_raw(
        &self,
        thread: jthread,
        proc: jvmtiStartFunction,
        arg: *mut c_void,
        priority: jint,
    ) -> Result<()> {
        unsafe { invoke!(self, v1, RunAgentThread, thread, proc, arg, priority) }
    }

    pub fn set_thread_local_storage_raw(&self, thread: jthread, data: usize) -> Result<()> {
        unsafe {
            invoke!(
                self,
                v1,
                SetThreadLocalStorage,
                thread,
                ptr::without_provenance(data),
            )
        }
    }

    pub fn thread_local_storage_raw(&self, thread: jthread) -> Result<usize> {
        let mut ptr = ptr::null_mut();
        unsafe { invoke!(self, v1, GetThreadLocalStorage, thread, &mut ptr) }?;
        Ok(ptr.addr())
    }

    pub fn top_thread_groups_raw(&self) -> Result<JBox<[jthreadGroup]>> {
        // SAFETY: the slice is allocated by env or zero size (both valid)
        let mut count: jint = 0;
        let mut groups: *mut jthreadGroup = ptr::null_mut();
        unsafe {
            invoke!(self, v1, GetTopThreadGroups, &mut count, &mut groups)?;
            Ok(self.allocator().boxed_slice(groups, count as usize))
        }
    }

    pub fn top_thread_groups<'local>(
        &self,
        env: &mut JNIEnv<'local>,
    ) -> Result<JBox<[JThreadGroup<'local>]>> {
        // SAFETY: mutable reference to `env` guarantees top stack frame
        // SAFETY: J*<'local> and j* are always compatible
        env.assert_top();
        self.top_thread_groups_raw()
            .map(|raw| unsafe { cast_box_slice::<jthreadGroup, JThreadGroup<'local>>(raw) })
    }

    pub fn thread_group_info_raw(&self, group: jthreadGroup) -> Result<jvmtiThreadGroupInfo> {
        let mut info: MaybeUninit<jvmtiThreadGroupInfo> = MaybeUninit::uninit();
        // SAFETY: initialized on successful return
        unsafe {
            invoke!(self, v1, GetThreadGroupInfo, group, info.as_mut_ptr())?;
            Ok(info.assume_init())
        }
    }

    pub fn thread_group_info<'local, 'any, T>(
        &self,
        env: &mut JNIEnv<'local>,
        group: impl AsRef<JThreadGroup<'any>>,
    ) -> Result<ThreadGroupInfo<JThreadGroup<'local>>> {
        env.assert_top();
        let jvmtiThreadGroupInfo {
            parent,
            name,
            max_priority,
            is_daemon,
        } = self.thread_group_info_raw(group.as_ref().as_raw())?;
        // SAFETY: name is guaranteed to be newly allocated MUTF-8 string
        let name = NonNull::new(name).map(|ptr| unsafe { self.allocator().boxed_jnistr(ptr) });
        // SAFETY: mutable reference to `env` guarantees top stack frame
        let parent = unsafe { JThreadGroup::from_raw(env, parent) };
        // SAFETY: it comes as JVMTI saw it
        let max_priority = unsafe { ThreadPriority::new_unchecked(max_priority as u32) };
        Ok(ThreadGroupInfo {
            name,
            parent,
            max_priority,
            daemon: is_daemon,
        })
    }

    pub fn thread_group_children_raw(
        &self,
        group: jthreadGroup,
    ) -> Result<ThreadGroupChildren<jthread, jthreadGroup>> {
        let mut threads_count: jint = 0;
        let mut threads: *mut jthread = ptr::null_mut();
        let mut groups_count: jint = 0;
        let mut groups: *mut jthreadGroup = ptr::null_mut();
        // SAFETY: the slice is allocated by env or zero size (both valid)
        unsafe {
            invoke!(
                self,
                v1,
                GetThreadGroupChildren,
                group,
                &mut threads_count,
                &mut threads,
                &mut groups_count,
                &mut groups,
            )?;
            let alloc = self.allocator();
            let threads = alloc.boxed_slice(threads, threads_count as usize);
            let groups = alloc.boxed_slice(groups, groups_count as usize);
            Ok(ThreadGroupChildren { threads, groups })
        }
    }

    pub fn thread_group_children<'local, 'any>(
        &self,
        env: &mut JNIEnv<'local>,
        group: impl AsRef<JThreadGroup<'any>>,
    ) -> Result<ThreadGroupChildren<JThread<'local>, JThreadGroup<'local>>> {
        // SAFETY: mutable reference to `env` guarantees top stack frame
        // SAFETY: J*<'local> and j* are always compatible
        env.assert_top();
        let raw = self.thread_group_children_raw(group.as_ref().as_raw())?;
        unsafe {
            let threads = cast_box_slice::<jthread, JThread<'local>>(raw.threads);
            let groups = cast_box_slice::<jthreadGroup, JThreadGroup<'local>>(raw.groups);
            Ok(ThreadGroupChildren { threads, groups })
        }
    }
}

impl<T: Send + Sync> Env<T> {
    /// # Panics
    ///
    /// If `func` panics, it will abort the process. Use [`catch_unwind`] to prevent that.
    ///
    /// [`catch_unwind`]: std::panic::catch_unwind
    pub fn run_agent_thread<'any, F>(
        &self,
        thread: impl AsRef<JThread<'any>>,
        priority: ThreadPriority,
        func: F,
    ) -> Result<()>
    where
        F: for<'local> FnOnce(&Env<T>, &mut JNIEnv<'local>) + Send + 'static,
    {
        // The provided `func` is thunked. We construct a wrapper (with the correct ABI)
        // which receives `func` via `arg`, wraps the raw pointer env parameters and
        // delegates the call to `func`.
        let thread = thread.as_ref().as_raw();
        let priority = priority.raw() as jint;
        let spawn = |proc, arg| unsafe { self.run_agent_thread_raw(thread, proc, arg, priority) };

        type JVMTIRaw = *mut jvmti_sys::jvmtiEnv;
        type JNIRaw = *mut jni_sys::JNIEnv;
        type Arg = *mut c_void;

        fn start<T>(jvmti: JVMTIRaw, jni: JNIRaw, func: impl FnOnce(&Env<T>, &mut JNIEnv<'_>)) {
            // SAFETY: called with valid pointers and lifetime of JNI is constrained top-level
            // via [Higher-Rank Trait Bound](https://doc.rust-lang.org/nomicon/hrtb.html)
            let mut guard: AttachGuard<'_> = unsafe { AttachGuard::from_unowned(jni) };
            let env = unsafe { &Env::<T>::from_raw(jvmti) };
            let jni = guard.borrow_env_mut();
            // Rust unwinding will abort on `extern "C" fn`, so any panic will terminate.
            func(env, jni)
        }

        if const { size_of::<F>() == size_of::<Arg>() } {
            /// The `func` is the size of pointer, so smuggle it in via bitcast.
            unsafe extern "C" fn thunk<T, F>(jvmti: JVMTIRaw, jni: JNIRaw, arg: Arg)
            where
                F: FnOnce(&Env<T>, &mut JNIEnv<'_>),
            {
                // SAFETY: see below, the bits contain value exactly as stored there
                let func: F = unsafe { transmute_copy::<Arg, F>(&arg) };
                start(jvmti, jni, func)
            }

            let mut func = ManuallyDrop::new(func);
            // SAFETY: the sizes are equal and no bit-pattern is invalid for raw pointer
            // HACK: there is no way to convince Rust that both types are the same size
            let arg: Arg = unsafe { transmute_copy::<ManuallyDrop<F>, Arg>(&func) };
            let proc: jvmtiStartFunction = Some(thunk::<T, F>);
            let res = spawn(proc, arg);
            if res.is_err() {
                // SAFETY: when error is returned, `proc` never runs, so this is single free
                unsafe { ManuallyDrop::<F>::drop(&mut func) };
            }
            res
        } else {
            /// The sizes do not match, so allocate `func` on the heap and pass that.
            unsafe extern "C" fn thunk<T, F>(jvmti: JVMTIRaw, jni: JNIRaw, arg: Arg)
            where
                F: FnOnce(&Env<T>, &mut JNIEnv<'_>),
            {
                // SAFETY: `arg` was newly allocated below and the types match exactly
                let func: F = *unsafe { Box::from_raw(arg.cast::<F>()) };
                start(jvmti, jni, func)
            }

            let boxed = Box::into_raw(Box::new(func));
            let arg: Arg = boxed.cast();
            let proc: jvmtiStartFunction = Some(thunk::<T, F>);
            let res = spawn(proc, arg);
            if res.is_err() {
                // SAFETY: when error is returned, `proc` never runs, so this is single free
                drop(unsafe { Box::<F>::from_raw(boxed) });
            }
            res
        }
    }

    pub fn run_agent_thread_new<'local, F>(
        &self,
        jni: &mut JNIEnv<'local>,
        priority: ThreadPriority,
        func: F,
    ) -> Result<JThread<'local>, JError>
    where
        F: for<'env> FnOnce(&Env<T>, &mut JNIEnv<'env>) + Send + 'static,
    {
        let thread = jni.new_object(jni_str!("java/lang/Thread"), jni_sig!("()V"), &[])?;
        let thread = unsafe { JThread::from_raw(jni, thread.as_raw()) };
        self.run_agent_thread(&thread, priority, func)?;
        Ok(thread)
    }
}

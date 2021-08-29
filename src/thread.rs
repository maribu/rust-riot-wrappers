//! Create, inspect or modify RIOT processes ("threads")

use riot_sys as raw;
use riot_sys::libc;
use cstr_core::CStr;

use core::marker::PhantomData;

use core::intrinsics::transmute;

// // wrongly detected as u32, it's actually used as an i32
// pub const THREAD_CREATE_SLEEPING: i32 = 1;
// pub const THREAD_AUTO_FREE: i32 = 2;
// pub const THREAD_CREATE_WOUT_YIELD: i32 = 4;
// pub const THREAD_CREATE_STACKTEST: i32 = 8;
//
// // wrongly detected as u32, it's actually used as a u8
// pub const THREAD_PRIORITY_MIN: i8 = 15;
// pub const THREAD_PRIORITY_IDLE: i8 = 15;
// pub const THREAD_PRIORITY_MAIN: i8 = 7;

/// Wrapper around a valid (not necessarily running, but in-range) [riot_sys::kernel_pid_t] that
/// provides access to thread details and signaling.
// Possible optimization: Make this NonZero
#[derive(Debug, PartialEq, Copy, Clone)]
pub struct KernelPID(pub(crate) raw::kernel_pid_t);

pub(crate) mod pid_converted {
    //! Converting the raw constants into consistently typed ones
    use riot_sys as raw;

    // pub const KERNEL_PID_UNDEF: raw::kernel_pid_t = raw::KERNEL_PID_UNDEF as raw::kernel_pid_t;
    pub const KERNEL_PID_FIRST: raw::kernel_pid_t = raw::KERNEL_PID_FIRST as raw::kernel_pid_t;
    pub const KERNEL_PID_LAST: raw::kernel_pid_t = raw::KERNEL_PID_LAST as raw::kernel_pid_t;
    pub const KERNEL_PID_ISR: raw::kernel_pid_t = raw::KERNEL_PID_ISR as raw::kernel_pid_t;
}

mod status_converted {
    //! Converting the raw constants into consistently typed ones for use in match branches. If
    //! that becomes a pattern, it might make sense to introduce a macro that forces a bunch of
    //! symbols (with different capitalizations) into a given type and makes an enum with a
    //! from_int method out of it.

    use riot_sys as raw;

    // STATUS_NOT_FOUND is not added here as it's not a proper status but rather a sentinel value,
    // which moreover can't be processed in its current form by bindgen and would need to be copied
    // over in here by manual expansion of the macro definition.
    pub const STATUS_STOPPED: i32 = raw::thread_status_t_STATUS_STOPPED as i32;
    pub const STATUS_SLEEPING: i32 = raw::thread_status_t_STATUS_SLEEPING as i32;
    pub const STATUS_MUTEX_BLOCKED: i32 = raw::thread_status_t_STATUS_MUTEX_BLOCKED as i32;
    pub const STATUS_RECEIVE_BLOCKED: i32 = raw::thread_status_t_STATUS_RECEIVE_BLOCKED as i32;
    pub const STATUS_SEND_BLOCKED: i32 = raw::thread_status_t_STATUS_SEND_BLOCKED as i32;
    pub const STATUS_REPLY_BLOCKED: i32 = raw::thread_status_t_STATUS_REPLY_BLOCKED as i32;
    pub const STATUS_FLAG_BLOCKED_ANY: i32 = raw::thread_status_t_STATUS_FLAG_BLOCKED_ANY as i32;
    pub const STATUS_FLAG_BLOCKED_ALL: i32 = raw::thread_status_t_STATUS_FLAG_BLOCKED_ALL as i32;
    pub const STATUS_MBOX_BLOCKED: i32 = raw::thread_status_t_STATUS_MBOX_BLOCKED as i32;
    pub const STATUS_RUNNING: i32 = raw::thread_status_t_STATUS_RUNNING as i32;
    pub const STATUS_PENDING: i32 = raw::thread_status_t_STATUS_PENDING as i32;
}


#[derive(Debug)]
#[non_exhaustive]
pub enum Status {
    // I would not rely on any properties of the assigned values, but it might make the conversion
    // points easier on the generated code if it can be reasoned down to a simple check of whether
    // it's in range.
    Stopped = status_converted::STATUS_STOPPED as isize,
    Sleeping = status_converted::STATUS_SLEEPING as isize,
    MutexBlocked = status_converted::STATUS_MUTEX_BLOCKED as isize,
    ReceiveBlocked = status_converted::STATUS_RECEIVE_BLOCKED as isize,
    SendBlocked = status_converted::STATUS_SEND_BLOCKED as isize,
    ReplyBlocked = status_converted::STATUS_REPLY_BLOCKED as isize,
    FlagBlockedAny = status_converted::STATUS_FLAG_BLOCKED_ANY as isize,
    FlagBlockedAll = status_converted::STATUS_FLAG_BLOCKED_ALL as isize,
    MboxBlocked = status_converted::STATUS_MBOX_BLOCKED as isize,
    Running = status_converted::STATUS_RUNNING as isize,
    Pending = status_converted::STATUS_PENDING as isize,

    /// A status value not known to riot-wrappers. Don't match for this explicitly: Other values
    /// may, at any minor riot-wrappers update, become actual process states again.
    Other, // Not making this Other(i32) as by the time this is reached, the code can't react
           // meaningfully to it, and if that shows up in any debug output, someone will need to
           // reproduce this anyway and can hook into from_int then.
}

impl Status {
    #[deprecated(note = "Not used by any known code, and if kept should be a wrapper around thread_is_active by mechanism and name")]
    pub fn is_on_runqueue(&self) -> bool {
        // FIXME: While we do get STATUS_ON_RUNQUEUE, the information about whether an Other is on
        // the runqueue or not is lost. Maybe split Other up to OtherOnRunqueue and
        // OtherNotOnRunqueue?
        match self {
            Status::Pending => true,
            Status::Running => true,
            _ => false,
        }
    }

    fn from_int(status: i32) -> Self {
        match status {
            status_converted::STATUS_STOPPED => Status::Stopped,
            status_converted::STATUS_SLEEPING => Status::Sleeping,
            status_converted::STATUS_MUTEX_BLOCKED => Status::MutexBlocked,
            status_converted::STATUS_RECEIVE_BLOCKED => Status::ReceiveBlocked,
            status_converted::STATUS_SEND_BLOCKED => Status::SendBlocked,
            status_converted::STATUS_REPLY_BLOCKED => Status::ReplyBlocked,
            status_converted::STATUS_FLAG_BLOCKED_ANY => Status::FlagBlockedAny,
            status_converted::STATUS_FLAG_BLOCKED_ALL => Status::FlagBlockedAll,
            status_converted::STATUS_MBOX_BLOCKED => Status::MboxBlocked,
            status_converted::STATUS_RUNNING => Status::Running,
            status_converted::STATUS_PENDING => Status::Pending,
            _ => Status::Other,
        }
    }
}

impl KernelPID {
    pub fn new(pid: raw::kernel_pid_t) -> Option<Self> {
        // casts needed due to untypedness of preprocessor constants
        if unsafe { raw::pid_is_valid(pid) } != 0 {
            Some(KernelPID(pid))
        } else {
            None
        }
    }

    pub fn all_pids() -> impl Iterator<Item = KernelPID> {
        // Not constructing the KernelPID manually but going through new serves as a convenient
        // validation of the construction (all_pids will panic if the rules of pid_is_valid change,
        // and then this function *should* be reevaluated). As pid_is_valid is static inline, the
        // compiler should be able to see through the calls down to there that the bounds checked
        // for there are the very bounds used in the construction here.
        (pid_converted::KERNEL_PID_FIRST..=pid_converted::KERNEL_PID_LAST)
            .map(|i| KernelPID::new(i).expect("Should be valid by construction"))
    }

    pub fn get_name(&self) -> Option<&str> {
        let ptr = unsafe { raw::thread_getname(self.0) };
        if ptr.is_null() {
            return None;
        }
        // If the thread stops, the name might be not valid any more, but then again the getname
        // function might already have returned anything, and thread names are generally strings in
        // .text. Unwrapping because by the time non-ASCII text shows up in there, something
        // probably already went terribly wrong.
        let name: &str = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        Some(name)
    }

    /// Get the current status of the thread of that number, if one currently exists
    pub fn status(&self) -> Result<Status, ()> {
        let status = unsafe { raw::thread_getstatus(self.0) };
        if status == riot_sys::init_STATUS_NOT_FOUND() {
            Err(())
        } else {
            Ok(Status::from_int(status as _))
        }
    }

    #[deprecated(note = "Use status() instead")]
    pub fn get_status(&self) -> Status {
        let status = unsafe { raw::thread_getstatus(self.0) };
        Status::from_int(status as _)
    }

    pub fn wakeup(&self) -> Result<(), ()> {
        let success = unsafe { raw::thread_wakeup(self.0) };
        match success {
            1 => Ok(()),
            // Actuall STATUS_NOT_FOUND, but all the others are then all error cases.
            _ => Err(()),
        }
    }

    /// Pick the thread_t out of sched_threads for the PID, with NULL mapped to None.
    #[doc(alias="thread_get")]
    fn thread(&self) -> Option<*const riot_sys::thread_t> {
        // unsafe: C function's "checked" precondition met by type constraint on PID validity
        let t = unsafe { riot_sys::thread_get_unchecked(self.0) };
        // .as_ref() would have the null check built in, but we can't build a shared refernce out
        // of this, only ever access its fields with volatility.
        if t == 0 as *mut _ {
            None
        } else {
            Some(crate::inline_cast(t))
        }
    }

    pub fn priority(&self) -> Result<u8, ()> {
        let thread = self.thread()
            .ok_or(())?;
        Ok(unsafe { (*thread).priority })
    }

    /// Gather information about the stack's thread.
    ///
    /// A None being returned can have two reasons:
    /// * The thread does not exist, or
    /// * develhelp is not active.
    pub fn stack_stats(&self) -> Result<StackStats, StackStatsError> {
        let thread = self.thread()
            .ok_or(StackStatsError::NoSuchThread)?;
        #[cfg(riot_develhelp)]
        return Ok(StackStats {
            start: unsafe { (*thread).stack_start },
            size: unsafe { (*thread).stack_size as _ },
            free: unsafe { riot_sys::thread_measure_stack_free((*thread).stack_start) } as usize,
        });
        #[cfg(not(riot_develhelp))]
        return Err(StackStatsError::InformationUnavailable);
    }
}

impl Into<raw::kernel_pid_t> for &KernelPID {
    fn into(self) -> raw::kernel_pid_t {
        self.0
    }
}

impl Into<raw::kernel_pid_t> for KernelPID {
    fn into(self) -> raw::kernel_pid_t {
        self.0
    }
}

/// Gathered information about a thread, returned by [KernelPID::stack_stats()].
///
/// All accessors are unconditional, because the StackStats can't be obtained without develhelp in
/// the first place.
#[derive(Debug)]
pub struct StackStats {
    start: *mut i8,
    size: usize,
    free: usize,
}

impl StackStats {
    pub fn start(&self) -> *mut i8 {
        self.start
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn end(&self) -> *mut i8 {
        unsafe { self.start.offset(self.size as isize) }
    }

    pub fn free(&self) -> usize {
        self.free
    }

    pub fn used(&self) -> usize {
        self.size - self.free
    }
}

#[non_exhaustive]
#[derive(Debug, Copy, Clone)]
pub enum StackStatsError {
    /// Requested PID does not correspond to a thread
    NoSuchThread,
    /// Details on the stack are unavailable because develhelp is disabled
    InformationUnavailable,
}

/// PID of the currently active thread
#[doc(alias = "thread_getpid")]
pub fn get_pid() -> KernelPID {
    // Ignoring the volatile in thread_getpid because it's probably not necessary (any application
    // will only ever see a consistent current PID).
    KernelPID(unsafe { raw::thread_getpid() })
}

pub fn sleep() {
    unsafe { raw::thread_sleep() }
}

/// Internal helper that does all the casting but relies on the caller to establish appropriate
/// lifetimes.
///
/// This also returns a pointer to the created thread's control block inside the stack; that TCB
/// can be used to get the thread's status even when the thread is already stopped and the PID may
/// have been reused for a different thread. For short-lived threads that are done before this
/// function returns, the TCB may be None.
unsafe fn create<R>(
    stack: &mut [u8],
    closure: &mut R,
    name: &CStr,
    priority: u8,
    flags: i32,
) -> (raw::kernel_pid_t, Option<*mut riot_sys::_thread>)
where
    R: Send + FnMut(),
{
    // overwriting name "R" as suggested as "copy[ing] over the parameters" on
    // https://doc.rust-lang.org/error-index.html#E0401
    unsafe extern "C" fn run<R>(x: *mut libc::c_void) -> *mut libc::c_void
    where
        R: Send + FnMut(),
    {
        let closure: &mut R = transmute(x);
        closure();
        0 as *mut libc::c_void
    }

    let pid = raw::thread_create(
        transmute(stack.as_mut_ptr()),
        stack.len() as i32,
        priority,
        flags,
        Some(run::<R>),
        closure as *mut R as *mut _,
        name.as_ptr(),
    );

    let tcb = riot_sys::thread_get(pid);
    // FIXME: Rather than doing pointer comparisons, it'd be nicer to just get the stack's
    // calculated thread control block (TCB) position and look right in there.
    let tcb = if tcb >= &stack[0] as *const u8 as *mut _
        && tcb <= &stack[stack.len() - 1] as *const u8 as *mut _
    {
        // unsafety: Assuming that C2Rust and and bindgen agree on the layout -- although it's
        // actually only a pointer anyway
        Some(core::mem::transmute(tcb))
    } else {
        None
    };

    (pid, tcb)
}

/// Create a context for starting threads that take shorter than 'static references.
///
/// Inside the scope, threads can be created using the `.spawn()` method of the scope passed in,
/// similar to the scoped-threads RFC (which resembles crossbeam's threads). Unlike that, the scope
/// has no dynamic memory of the spawned threads, and no actual way of waiting for a thread. If the
/// callback returns, the caller has call the scope's `.reap()` method with all the threads that
/// were launched; otherwise, the program panics.
pub fn scope<'env, F, R>(callback: F) -> R
where
    F: for<'id> FnOnce(&mut CountingThreadScope<'env, 'id>) -> R,
{
    let mut s = CountingThreadScope { threads: 0, _phantom: PhantomData };

    let ret = callback(&mut s);

    s.wait_for_all();

    ret
}

/// Lifetimed helper through which threads can be spawned.
///
/// ## Lifetimes
///
/// The involved lifetimes ensure that all parts used to build the thread (its closure, stack, and
/// name) outlive the whole process, which (given the generally dynamic lifetime of threads) can
/// only be checked dynamically.
///
/// The lifetimes are:
///
/// * `'env`: A time surrounding the [`scope()`] call. All inputs to the thread are checked to live
///   at least that long (possibly longer; it is commonplace for them to be `'static`).
/// * `'id`: An identifying lifetime (or brand) of the scope. Its lifetime is somewhere inbetween
///   the outer `'env` and the run time of the called closure.
///
///   Practically, don't think of this as a lifetime, but more as a disambiguator: It makes the
///   monomorphized CountingThreadScope unique in the sense that no two instances of
///   CountingThreadScope can ever have the same type.
///
///   By having unique types, it is ensured that a counted thread is only counted down (in
///   [`.reap()`]) in the scope it was born in, and that no shenanigans with counters being swapped
///   around with [core::mem::swap()] are used to trick the compiler into allowing use-after-free.
///
/// This technique was inspired by (and is explained well) in [the GhostCell
/// Paper](http://plv.mpi-sws.org/rustbelt/ghostcell/paper.pdf).
///
pub struct CountingThreadScope<'env, 'id> {
    threads: u16, // a counter, but larger than kernel_pid_t
    _phantom: PhantomData<(&'env (), &'id ())>,
}

impl<'env, 'id> CountingThreadScope<'env,'id> {
    /// Start a thread in the given stack, in which the closure is run. The thread gets a human
    /// readable name (ignored in no-DEVHELP mode), and is started with the priority and flags as
    /// per thread_create documentation.
    ///
    /// The returned thread object can safely be discarded when the scope is not expected to ever
    /// return, and needs to be passed on to `.reap()` otherwise.
    ///
    /// Having the closure as a mutable reference (rather than a moved instance) is a bit
    /// unergonomic as it means that `spawn(..., || { foo }, ..)` one-line invocations are
    /// impossible, but is necessary as it avoids having the callback sitting in the Thread which
    /// can't be prevented from moving around on the stack between the point when thread_create is
    /// called (and the pointer is passed on to RIOT) and the point when the threads starts running
    /// and that pointer is used.
    pub fn spawn<R>(
        &mut self,
        stack: &'env mut [u8],
        closure: &'env mut R,
        name: &'env CStr,
        priority: u8,
        flags: i32,
    ) -> Result<CountedThread<'id>, raw::kernel_pid_t>
    where
        R: Send + FnMut(),
    {
        self.threads = self.threads.checked_add(1).expect("Thread limit exceeded");

        let (pid, tcb) = unsafe { create(stack, closure, name, priority, flags) };

        if pid < 0 {
            return Err(pid);
        }

        Ok(CountedThread {
            thread: TrackedThread {
                pid: KernelPID(pid),
                tcb: tcb,
            },
            _phantom: PhantomData,
        })
    }

    /// Assert that the thread has terminated, and remove it from the list of pending threads in
    /// this context.
    ///
    /// Unlike a (POSIX) wait, this will not block (for there is no SIGCHLDish thing in RIOT --
    /// whoever wants to be notified would need to make their threads send an explicit signal), but
    /// panic if the thread is not actually done yet.
    pub fn reap(&mut self, thread: CountedThread<'id>) {
        match thread.get_status() {
            Status::Stopped => (),
            _ => panic!("Attempted to reap running process"),
        }

        self.threads -= 1;
    }

    fn wait_for_all(self) {
        if self.threads != 0 {
            panic!("Not all threads were waited for at scope end");
        }
    }
}

// The 'id ensures that threads can only be reaped where they were created. (It might make sense to
// move it into TrackedThread and make the tcb usable for more than just pointer comparison).
#[derive(Debug)]
pub struct CountedThread<'id> {
    thread: TrackedThread,
    _phantom: PhantomData<&'id ()>,
}

impl<'id> CountedThread<'id> {
    pub fn get_pid(&self) -> KernelPID {
        self.thread.get_pid()
    }

    pub fn get_status(&self) -> Status {
        self.thread.get_status()
    }
}

/// Create a thread with a statically allocated stack
pub fn spawn<R>(
    stack: &'static mut [u8],
    closure: &'static mut R,
    name: &'static CStr,
    priority: u8,
    flags: i32,
) -> Result<TrackedThread, raw::kernel_pid_t>
where
    R: Send + FnMut(),
{
    let (pid, tcb) = unsafe { create(stack, closure, name, priority, flags) };

    if pid < 0 {
        return Err(pid);
    }

    Ok(TrackedThread {
        pid: KernelPID(pid),
        tcb,
    })
}

/// A thread identified not only by its PID (which can be reused whenever the thread has quit) but
/// also by a pointer to its thread control block. This gives a TrackedThread a better get_status()
/// method that reliably reports Stopped even when the PID is reused.
///
/// A later implementation may stop actually having the pid in the struct and purely rely on the
/// tcb (although that'll need to become a lifetime'd reference to a cell by then).
#[derive(Debug)]
pub struct TrackedThread {
    pid: KernelPID,
    tcb: Option<*mut riot_sys::_thread>,
}

impl TrackedThread {
    pub fn get_pid(&self) -> KernelPID {
        self.pid
    }

    /// Like get_status of a KernelPID, but this returnes Stopped if the PID has been re-used after
    /// our thread has stopped.
    pub fn get_status(&self) -> Status {
        let status = self.pid.get_status();
        let tcb = unsafe { riot_sys::thread_get(self.pid.0) };
        // unsafe: transmutation between C2Rust and bindgen pointer
        let tcb = unsafe { core::mem::transmute(tcb) };
        if Some(tcb) != self.tcb {
            Status::Stopped
        } else {
            status
        }
    }
}

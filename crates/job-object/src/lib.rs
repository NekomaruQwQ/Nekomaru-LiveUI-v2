//! RAII wrapper around a Windows Job Object.
//!
//! Creates an anonymous job configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
//! so that all assigned child processes are automatically terminated when the
//! job handle is closed — including abnormal exits (crash, Task Manager kill).

use std::os::windows::io::AsRawHandle;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::*;

/// RAII wrapper around a Windows Job Object.
///
/// All child processes assigned via [`assign`](Self::assign) are killed when
/// this handle is dropped (or when the owning process exits for any reason).
pub struct JobObject {
    handle: HANDLE,
}

impl JobObject {
    /// Create a new anonymous job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    ///
    /// # Errors
    ///
    /// Returns an error if the Win32 `CreateJobObjectW` or
    /// `SetInformationJobObject` call fails.
    pub fn new() -> std::io::Result<Self> {
        // SAFETY: Creating an anonymous job object with no security attributes.
        let handle = unsafe { CreateJobObjectW(None, None) }
            .map_err(|e| std::io::Error::other(format!("CreateJobObjectW: {e}")))?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `info` is a valid `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` and
        // `handle` is a valid job object handle returned by `CreateJobObjectW`.
        unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .expect("size fits in u32"))
        }
        .map_err(|e| std::io::Error::other(format!("SetInformationJobObject: {e}")))?;

        Ok(Self { handle })
    }

    /// Assign a child process to this job.
    ///
    /// After assignment the child is killed automatically if this process exits
    /// for any reason (including crashes).
    ///
    /// # Errors
    ///
    /// Returns an error if `AssignProcessToJobObject` fails.
    pub fn assign(&self, child: &std::process::Child) -> std::io::Result<()> {
        let process_handle = HANDLE(child.as_raw_handle());

        // SAFETY: Both handles are valid — `self.handle` from `CreateJobObjectW`
        // and `process_handle` from a live `std::process::Child`.
        unsafe { AssignProcessToJobObject(self.handle, process_handle) }
            .map_err(|e| std::io::Error::other(format!("AssignProcessToJobObject: {e}")))
    }
}

// SAFETY: The job object handle is an opaque kernel object — safe to send and
// share across threads.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a valid handle from `CreateJobObjectW`.
        // Closing it triggers `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, terminating
        // all assigned child processes.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

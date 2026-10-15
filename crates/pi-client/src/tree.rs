//! Ties Pi's whole process tree to one handle, so nothing outlives the workbench.

#[cfg(windows)]
mod imp {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
    };

    /// A job object that kills every process in it when closed, including
    /// when the workbench itself exits.
    pub struct Tree(HANDLE);

    // The job handle may be closed from any thread.
    unsafe impl Send for Tree {}

    impl Tree {
        /// On Windows `pi` is a `.cmd` shim: the child is `cmd.exe`, and killing
        /// it would leave `node.exe` running. Processes it starts after this
        /// call join the job too; the shim takes far longer to launch Node than
        /// this takes to run.
        pub fn adopt(child: &std::process::Child) -> Option<Self> {
            // SAFETY: plain Win32 calls on a handle we own; `info` outlives the call.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let configured = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) != 0;
                if !configured || AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
                    CloseHandle(job);
                    return None;
                }
                Some(Self(job))
            }
        }

        /// Ends every process in the job.
        pub fn kill(self) {}
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// Elsewhere `pi` runs Node directly, so the child is the whole tree.
    pub struct Tree;

    impl Tree {
        pub fn adopt(_: &std::process::Child) -> Option<Self> {
            Some(Self)
        }

        /// Nothing beyond the direct child to end.
        pub fn kill(self) {}
    }
}

pub use imp::Tree;

//! Each execution owns a process tree, including cancellation of the Rust future.
#[cfg(unix)]
pub(super) struct ProcessTree {
    pid: u32,
    terminated: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(unix)]
impl ProcessTree {
    pub(super) fn terminate(&self) {
        if self
            .terminated
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        if let Ok(pid) = i32::try_from(self.pid) {
            // The child was spawned as a new process group, never PortMate's group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
pub(super) fn spawn(
    command: &mut tokio::process::Command,
) -> Result<(tokio::process::Child, ProcessTree), String> {
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|e| format!("could not start host script: {e}"))?;
    let id = child.id().ok_or("host script process has no ID")?;
    Ok((
        child,
        ProcessTree {
            pid: id,
            terminated: std::sync::atomic::AtomicBool::new(false),
        },
    ))
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{
                OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
            },
        },
    };

    pub(crate) struct ProcessTree(OwnedHandle);
    // Closing the sole Job handle terminates every associated descendant.
    impl Drop for ProcessTree {
        fn drop(&mut self) {
            let _ = self.0.as_raw_handle();
        }
    }
    impl ProcessTree {
        pub(crate) fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0.as_raw_handle(), 1);
            }
        }
    }

    pub(crate) fn spawn(
        command: &mut tokio::process::Command,
    ) -> Result<(tokio::process::Child, ProcessTree), String> {
        unsafe {
            let raw_job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw_job.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let job = OwnedHandle::from_raw_handle(raw_job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                raw_job,
                JobObjectExtendedLimitInformation,
                (&limits as *const _) as _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            // No script instruction runs before containment is established.
            command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
            let mut child = command
                .spawn()
                .map_err(|e| format!("could not start host script: {e}"))?;
            let process = child.raw_handle().ok_or("missing host process handle")?;
            if AssignProcessToJobObject(raw_job, process) == 0 {
                let error = std::io::Error::last_os_error();
                let _ = child.start_kill();
                return Err(format!("cannot contain host script process: {error}"));
            }
            let tree = ProcessTree(job);
            let pid = child.id().ok_or("missing host process ID")?;
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let snapshot = OwnedHandle::from_raw_handle(snapshot);
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut found = Thread32First(snapshot.as_raw_handle(), &mut entry);
            while found != 0 {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if thread.is_null() {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                    let resumed = ResumeThread(thread);
                    CloseHandle(thread);
                    if resumed == u32::MAX {
                        return Err("cannot resume contained host script".into());
                    }
                    return Ok((child, tree));
                }
                found = Thread32Next(snapshot.as_raw_handle(), &mut entry);
            }
            Err("cannot find suspended host script thread".into())
        }
    }
}

#[cfg(windows)]
pub(super) use windows::{spawn, ProcessTree};

//! Owned worker process with separate address space and group cancellation.
//!
//! This is process separation, NOT an OS permission sandbox: workers retain
//! the host user's filesystem/network privileges. It accepts an executable
//! command rather than forking a Rust closure out of a multithreaded process.
use std::io;
use std::process::{Child, Command, ExitStatus};

pub struct WorkerProcess {
    child: Option<Child>,
}
impl WorkerProcess {
    /// Starts a fresh process group. The command's arguments, environment and
    /// stdio are explicit host choices. Unix only until equivalent Windows
    /// job-object lifecycle handling is implemented.
    #[cfg(unix)]
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        Ok(Self {
            child: Some(command.spawn()?),
        })
    }

    #[cfg(not(unix))]
    pub fn spawn(_command: &mut Command) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "worker process groups require Unix",
        ))
    }

    pub fn id(&self) -> u32 {
        self.child.as_ref().expect("live worker handle").id()
    }

    /// Kills the owned group and waits for its leader, consuming the handle.
    /// OS cleanup closes worker descriptors; Rust destructors do not run on
    /// forced termination. Descendants must not escape the process group.
    pub fn stop(mut self) -> io::Result<ExitStatus> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> io::Result<ExitStatus> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("worker already reaped"))?;
        #[cfg(unix)]
        {
            // The leader is unreaped, so its PID cannot be reused before this
            // signal. A negative PID addresses the group created at spawn.
            let result = unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error);
                }
            }
        }
        #[cfg(not(unix))]
        child.kill()?;
        let status = child.wait()?;
        self.child = None;
        Ok(status)
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.stop_inner();
        }
    }
}

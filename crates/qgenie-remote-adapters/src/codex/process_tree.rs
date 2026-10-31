//! Linux tools can create sessions outside the app-server's process group.
//! Retain pidfds and freeze each parent before discovering its children. No
//! saved PID or process-name match is a signaling authority.
use std::{
    collections::BTreeSet,
    fs, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::Duration,
};
use tokio::time::{Instant, sleep};

const MAX_PROCESSES: usize = 1024;
const FREEZE_TIMEOUT: Duration = Duration::from_secs(2);

struct Process {
    pid: u32,
    descriptor: OwnedFd,
    resume: bool,
}
impl Process {
    fn open(pid: u32) -> io::Result<Self> {
        // SAFETY: zero flags, a positive PID, and an owned descriptor on success.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            pid,
            // SAFETY: the successful syscall transferred this fresh descriptor.
            descriptor: unsafe { OwnedFd::from_raw_fd(fd as i32) },
            resume: false,
        })
    }
    fn signal(&self, signal: i32) -> io::Result<()> {
        // SAFETY: the pidfd remains owned here; null siginfo and zero flags are valid.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.descriptor.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.resume {
            // Undo only our own temporary stop if collection, authorization, or
            // the caller fails. This never terminates or adopts another process.
            let _ = self.signal(libc::SIGCONT);
        }
    }
}

fn gone(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH))
}
fn stat(pid: u32) -> io::Result<(u8, u32)> {
    let value = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // comm may contain spaces and parentheses. Fields after its final ')' start
    // with state and parent PID; they do not contain caller-controlled strings.
    let mut fields = value
        .rsplit_once(')')
        .ok_or(io::ErrorKind::InvalidData)?
        .1
        .split_whitespace();
    let state = fields
        .next()
        .and_then(|value| value.as_bytes().first().copied())
        .ok_or(io::ErrorKind::InvalidData)?;
    let parent = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or(io::ErrorKind::InvalidData)?;
    Ok((state, parent))
}
fn stopped(state: u8) -> bool {
    matches!(state, b'T' | b't' | b'Z' | b'X')
}
fn children_when_stopped(pid: u32, deadline: Instant) -> io::Result<Option<BTreeSet<u32>>> {
    for (index, task) in fs::read_dir(format!("/proc/{pid}/task"))?.enumerate() {
        if index == MAX_PROCESSES || Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let task = task?;
        let tid = task
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
            .ok_or(io::ErrorKind::InvalidData)?;
        match stat(tid) {
            Ok((state, _)) if stopped(state) => (),
            Ok(_) => return Ok(None),
            Err(error) if gone(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    // /proc/<tid>/children requires CONFIG_CHECKPOINT_RESTORE. Scan parent
    // identities instead, so cleanup also works on kernels without that option.
    let mut children = BTreeSet::new();
    for entry in fs::read_dir("/proc")? {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let entry = entry?;
        let Some(candidate) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        match stat(candidate) {
            Ok((_, parent)) if parent == pid => {
                children.insert(candidate);
                if children.len() > MAX_PROCESSES {
                    return Err(io::ErrorKind::OutOfMemory.into());
                }
            }
            Ok(_) => (),
            Err(error) if gone(&error) => (),
            // hidepid can conceal other users. Managed tools run as our user.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => (),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(children))
}

/// The caller retains the unreaped root Child throughout this future. `effect`
/// checks the same original grant before every nonblocking signal, never across
/// filesystem reads or waits. Dropping the future releases all temporary stops.
pub(super) async fn stop(
    root: u32,
    effect: &mut impl FnMut(&mut dyn FnMut() -> io::Result<usize>) -> io::Result<usize>,
) -> io::Result<()> {
    let deadline = Instant::now() + FREEZE_TIMEOUT;
    let mut processes = vec![Process::open(root)?];
    let mut next = 0;
    while next < processes.len() {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let process = &mut processes[next];
        let state = match stat(process.pid) {
            Ok((state, _)) => state,
            Err(error) if gone(&error) => {
                next += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        if matches!(state, b'Z' | b'X') {
            next += 1;
            continue;
        }
        if !stopped(state) {
            match effect(&mut || process.signal(libc::SIGSTOP).map(|_| 0)) {
                Ok(_) => (),
                Err(error) if gone(&error) => {
                    next += 1;
                    continue;
                }
                Err(error) => return Err(error),
            }
            process.resume = true;
        }
        let children = loop {
            match children_when_stopped(process.pid, deadline) {
                Ok(Some(children)) => break children,
                Ok(None) => (),
                Err(error) if gone(&error) => break BTreeSet::new(),
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            sleep(Duration::from_millis(2)).await;
        };
        let parent = process.pid;
        for pid in children {
            if processes.len() == MAX_PROCESSES {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            let child = match Process::open(pid) {
                Ok(child) => child,
                Err(error) if gone(&error) => continue,
                Err(error) => return Err(error),
            };
            // The frozen, still-live parent cannot spawn a replacement child.
            // Check after pidfd_open; an exited/reused candidate must not be adopted.
            match stat(pid) {
                Ok((_, actual_parent)) if actual_parent == parent => (),
                Ok(_) => continue,
                Err(error) if gone(&error) => continue,
                Err(error) => return Err(error),
            }
            if let Err(error) = processes[next].signal(0).and_then(|_| child.signal(0)) {
                if gone(&error) {
                    continue;
                }
                return Err(error);
            }
            processes.push(child);
        }
        next += 1;
    }
    for process in processes.iter_mut().rev() {
        match effect(&mut || process.signal(libc::SIGKILL).map(|_| 0)) {
            Ok(_) => process.resume = false,
            Err(error) if gone(&error) => process.resume = false,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

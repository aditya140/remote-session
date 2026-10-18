use crate::LaunchEnvironment;
use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use qgenie_remote_proto::{StartSpec, validate_size, wire::Capabilities};
use std::{
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub struct PtyControl {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
}
pub struct PtyProcess {
    pub control: PtyControl,
    pub reader: PtyReader,
    pub child: PtyChild,
}

pub struct PtyChild(Arc<Mutex<Box<dyn Child + Send + Sync>>>);
/// Captured exit facts without exposing the PTY implementation's native types.
pub struct NativeExit {
    code: u32,
    success: bool,
}
impl NativeExit {
    pub fn exit_code(&self) -> u32 {
        self.code
    }
    pub fn success(&self) -> bool {
        self.success
    }
}
/// Validation and native capability discovery happen before durable launch acceptance.
pub struct PreparedPty {
    spec: StartSpec,
    environment: LaunchEnvironment,
    directory: Option<std::fs::File>,
}
impl PreparedPty {
    pub fn new(spec: StartSpec, environment: LaunchEnvironment) -> Result<Self> {
        validate_size(spec.rows, spec.cols)?;
        anyhow::ensure!(
            !spec.command.is_empty()
                && spec.command.len() <= 256
                && spec.command.iter().all(|arg| !arg.contains('\0'))
                && !spec.command[0].is_empty()
                && spec.command.iter().map(String::len).sum::<usize>() <= 64 * 1024,
            "invalid or oversized command"
        );
        anyhow::ensure!(
            spec.cwd.len() <= 4096
                && !spec.cwd.contains('\0')
                && std::path::Path::new(&spec.cwd).is_absolute(),
            "invalid launch directory"
        );
        Ok(Self {
            spec,
            environment,
            directory: None,
        })
    }
    /// Use a validated directory capability supplied by the native owner.
    /// The descriptor remains alive through the child's atomic fchdir/exec.
    pub fn with_directory(
        spec: StartSpec,
        environment: LaunchEnvironment,
        directory: std::fs::File,
    ) -> Result<Self> {
        anyhow::ensure!(
            directory.metadata()?.is_dir(),
            "cwd capability is not a directory"
        );
        let mut prepared = Self::new(spec, environment)?;
        prepared.directory = Some(directory);
        Ok(prepared)
    }
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            input: true,
            interrupt: true,
            resize: true,
            terminate: true,
            ..Capabilities::default()
        }
    }
    pub fn launch(self) -> Result<PtyProcess> {
        spawn_prepared(&self.spec, &self.environment, self.directory)
    }
}
impl PtyChild {
    pub fn try_wait(&mut self) -> io::Result<Option<NativeExit>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| io::Error::other("PTY child lock unavailable"))?
            .try_wait()?
            .map(|status| NativeExit {
                code: status.exit_code(),
                success: status.success(),
            }))
    }
    pub fn wait(&mut self) -> io::Result<NativeExit> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn spawn(spec: &StartSpec) -> Result<PtyProcess> {
    PreparedPty::new(spec.clone(), LaunchEnvironment::default())?.launch()
}
fn spawn_prepared(
    spec: &StartSpec,
    environment: &LaunchEnvironment,
    directory: Option<std::fs::File>,
) -> Result<PtyProcess> {
    validate_size(spec.rows, spec.cols)?;
    let executable = spec.command.first().context("command is empty")?;
    let pair = native_pty_system().openpty(size(spec.rows, spec.cols))?;
    let mut command = CommandBuilder::new(executable);
    command.args(&spec.command[1..]);
    command.cwd(&spec.cwd);
    if let Some(directory) = directory {
        command.cwd_directory(directory)?;
    }
    command.env("TERM", "xterm-256color");
    for (name, value) in &environment.0 {
        command.env(name, value);
    }
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    if let Some(fd) = pair.master.as_raw_fd() {
        // SAFETY: the borrowed descriptor belongs to the live PTY master.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    let child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    let child = Arc::new(Mutex::new(child));
    Ok(PtyProcess {
        control: PtyControl {
            master: pair.master,
            writer,
            child: child.clone(),
        },
        reader: PtyReader(reader),
        child: PtyChild(child),
    })
}

impl PtyControl {
    pub fn input(&mut self, bytes: &[u8]) -> Result<()> {
        self.input_until(bytes, Instant::now() + Duration::from_millis(100))
    }
    /// Recheck an owner-supplied monotonic authority deadline before every
    /// nonblocking native write. A partial/timed-out write remains uncertain.
    pub fn input_until(&mut self, bytes: &[u8], authority_deadline: Instant) -> Result<()> {
        self.input_guarded(bytes, || Ok(authority_deadline))
    }
    /// The owner may recheck a suspend-aware clock or other current authority
    /// before each native write; no I/O or authority state is owned here.
    pub fn input_guarded(
        &mut self,
        mut bytes: &[u8],
        mut authority: impl FnMut() -> Result<Instant>,
    ) -> Result<()> {
        let io_deadline = Instant::now() + Duration::from_millis(100);
        while !bytes.is_empty() {
            let deadline = io_deadline.min(authority()?);
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "native input authority deadline elapsed",
                )
                .into());
            }
            match self.writer.write(bytes) {
                Ok(0) => {
                    return Err(
                        io::Error::new(io::ErrorKind::WriteZero, "PTY accepted no bytes").into(),
                    );
                }
                Ok(length) => bytes = &bytes[length..],
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        self.writer.flush()?;
        Ok(())
    }
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        validate_size(rows, cols)?;
        self.master.resize(size(rows, cols))?;
        Ok(())
    }
    pub fn interrupt(&mut self) -> Result<()> {
        self.input(&[3])
    }
    pub fn interrupt_until(&mut self, deadline: Instant) -> Result<()> {
        self.input_until(&[3], deadline)
    }
    pub fn terminate(&mut self) -> Result<()> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| io::Error::other("PTY child lock unavailable"))?;
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if let Some(pid) = child.process_id() {
            // Reaping and signaling share this lock, so a reused PID cannot be signaled.
            if let Some(foreground) = self.master.process_group_leader()
                && foreground > 1
            {
                // SAFETY: this is the foreground group of our owned PTY.
                unsafe {
                    libc::kill(-foreground, libc::SIGKILL);
                }
            }
            // SAFETY: pid identifies our still-unreaped child and its group.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            // SAFETY: same ownership guard; also handles a changed child group.
            if unsafe { libc::kill(pid as i32, libc::SIGKILL) } < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }
}
fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Blocking `Read` for simple consumers, and a nonblocking read for supervisors
/// which must observe/reap the primary child independently of inherited slave FDs.
pub struct PtyReader(Box<dyn Read + Send>);
impl PtyReader {
    pub fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let length = bytes.len().min(qgenie_remote_proto::MAX_PTY_BYTES);
        self.0.read(&mut bytes[..length])
    }
}
impl Read for PtyReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.try_read(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                result => return result,
            }
        }
    }
}

#[cfg(test)]
#[path = "pty/qualification.rs"]
mod qualification;
#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::IdempotencyKey;
    #[test]
    fn real_pty_handles_input_resize_unicode_and_exit() {
        let spec = StartSpec {
            key: IdempotencyKey::new(),
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "read answer; stty size; printf '✓:%s' \"$answer\"".into(),
            ],
            cwd: "/tmp".into(),
            rows: 24,
            cols: 80,
        };
        let mut process = spawn(&spec).unwrap();
        process.control.resize(30, 100).unwrap();
        process.control.input(b"hello\n").unwrap();
        let mut output = String::new();
        process.reader.read_to_string(&mut output).unwrap();
        assert!(process.child.wait().unwrap().success());
        assert!(output.contains("30 100"), "{output}");
        assert!(output.contains("✓:hello"), "{output}");
    }

    #[test]
    fn pinned_cwd_and_relative_executable_survive_path_substitution() {
        use std::{
            fs::{File, Permissions},
            os::unix::fs::{PermissionsExt, symlink},
        };
        let temp = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let requested = root.join("requested");
        let outside = root.join("outside");
        std::fs::create_dir(&requested).unwrap();
        std::fs::create_dir(&outside).unwrap();
        for (directory, marker) in [(&requested, "correct"), (&outside, "wrong")] {
            let runner = directory.join("runner");
            std::fs::write(
                &runner,
                format!("#!/bin/sh\nprintf {marker} > native-marker\npwd -P\n"),
            )
            .unwrap();
            std::fs::set_permissions(runner, Permissions::from_mode(0o700)).unwrap();
        }
        let spec = StartSpec {
            key: IdempotencyKey::new(),
            command: vec!["./runner".into()],
            cwd: requested.display().to_string(),
            rows: 24,
            cols: 80,
        };
        let prepared = PreparedPty::with_directory(
            spec,
            LaunchEnvironment::default(),
            File::open(&requested).unwrap(),
        )
        .unwrap();
        let held = root.join("held");
        std::fs::rename(&requested, &held).unwrap();
        symlink(&outside, &requested).unwrap();
        let mut child = prepared.launch().unwrap();
        let mut output = String::new();
        child.reader.read_to_string(&mut output).unwrap();
        assert!(child.child.wait().unwrap().success());
        assert_eq!(
            std::fs::read(held.join("native-marker")).unwrap(),
            b"correct"
        );
        assert!(!outside.join("native-marker").exists());
        assert!(output.contains(held.to_str().unwrap()));
    }

    #[test]
    fn changed_cwd_permissions_are_rechecked_before_exec() {
        use std::{
            fs::{File, Permissions},
            os::unix::fs::PermissionsExt,
        };
        let temp = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = std::fs::canonicalize(temp.path()).unwrap();
        let spec = StartSpec {
            key: IdempotencyKey::new(),
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "touch must-not-execute".into(),
            ],
            cwd: path.display().to_string(),
            rows: 24,
            cols: 80,
        };
        let prepared = PreparedPty::with_directory(
            spec,
            LaunchEnvironment::default(),
            File::open(&path).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&path, Permissions::from_mode(0o777)).unwrap();
        assert!(prepared.launch().is_err());
        assert!(!path.join("must-not-execute").exists());
        std::fs::set_permissions(path, Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn expired_input_deadline_cannot_write_even_a_prefix() {
        use std::{fs::Permissions, os::unix::fs::PermissionsExt};
        let temp = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = std::fs::canonicalize(temp.path()).unwrap();
        let marker = path
            .join("received")
            .display()
            .to_string()
            .replace('\'', "'\"'\"'");
        let spec = StartSpec {
            key: IdempotencyKey::new(),
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("read value; printf '%s' \"$value\" > '{marker}'"),
            ],
            cwd: path.display().to_string(),
            rows: 24,
            cols: 80,
        };
        let mut process = spawn(&spec).unwrap();
        assert!(
            process
                .control
                .input_until(b"denied\n", Instant::now() - Duration::from_millis(1))
                .is_err()
        );
        process.control.input(b"allowed\n").unwrap();
        process.reader.read_to_end(&mut Vec::new()).unwrap();
        assert!(process.child.wait().unwrap().success());
        assert_eq!(std::fs::read(path.join("received")).unwrap(), b"allowed");
    }
}

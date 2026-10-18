use super::*;
use qgenie_remote_proto::IdempotencyKey;
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

struct Owned(PtyProcess);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.control.terminate();
        let _ = self.0.child.wait();
    }
}
fn process(command: &str) -> Owned {
    Owned(
        spawn(&StartSpec {
            key: IdempotencyKey::new(),
            command: vec!["/bin/sh".into(), "-c".into(), command.into()],
            cwd: "/tmp".into(),
            rows: 24,
            cols: 80,
        })
        .unwrap(),
    )
}
fn until(process: &mut Owned, predicate: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let mut bytes = vec![0; qgenie_remote_proto::MAX_PTY_BYTES * 2];
    while !predicate(&output) {
        assert!(Instant::now() < deadline, "PTY output timed out");
        match process.0.reader.try_read(&mut bytes) {
            Ok(0) => panic!("unexpected PTY EOF"),
            Ok(count) => {
                assert!(count <= qgenie_remote_proto::MAX_PTY_BYTES);
                output.extend_from_slice(&bytes[..count]);
                assert!(output.len() <= 2 * 1024 * 1024);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("PTY read failed: {error}"),
        }
    }
    output
}
#[test]
fn interrupt_keeps_the_owned_shell_usable_until_explicit_termination() {
    let mut child = process(
        "stty -echo; trap 'printf INTERRUPTED' INT; printf READY; while :; do read -r value; if [ \"$value\" = probe ]; then printf ALIVE; fi; done",
    );
    until(&mut child, |bytes| bytes.ends_with(b"READY"));
    child.0.control.interrupt().unwrap();
    until(&mut child, |bytes| bytes.ends_with(b"INTERRUPTED"));
    assert!(child.0.child.try_wait().unwrap().is_none());
    child.0.control.input(b"probe\n").unwrap();
    until(&mut child, |bytes| bytes.ends_with(b"ALIVE"));
    child.0.control.terminate().unwrap();
    assert!(!child.0.child.wait().unwrap().success());
}
#[test]
fn output_burst_is_lossless_bounded_and_does_not_block_resize() {
    let mut child = process(
        "stty -echo; awk 'BEGIN { for(i=0;i<16384;i++) printf \"0123456789abcdef0123456789abcdef\"; printf \"BURST_DONE\" }'; read -r value; printf AFTER_BURST",
    );
    child.0.control.resize(40, 120).unwrap();
    let output = until(&mut child, |bytes| bytes.ends_with(b"BURST_DONE"));
    assert_eq!(output.len(), 16384 * 32 + b"BURST_DONE".len());
    assert!(
        output[..16384 * 32]
            .as_chunks::<32>()
            .0
            .iter()
            .all(|chunk| chunk == b"0123456789abcdef0123456789abcdef")
    );
    child.0.control.input(b"continue\n").unwrap();
    until(&mut child, |bytes| bytes.ends_with(b"AFTER_BURST"));
    assert!(child.0.child.wait().unwrap().success());
}
#[test]
fn dropping_the_output_observer_preserves_control_and_native_execution() {
    let directory = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let marker = directory.path().join("observed");
    let mut child = process(&format!(
        "stty -echo; printf READY; read -r value; printf '%s' \"$value\" > '{}'",
        marker.display()
    ));
    until(&mut child, |bytes| bytes.ends_with(b"READY"));
    let replacement = PtyReader(Box::new(io::empty()));
    drop(std::mem::replace(&mut child.0.reader, replacement));
    assert!(child.0.child.try_wait().unwrap().is_none());
    child.0.control.input(b"survived\n").unwrap();
    assert!(child.0.child.wait().unwrap().success());
    assert_eq!(std::fs::read(marker).unwrap(), b"survived");
}

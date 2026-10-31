use super::*;
use std::{
    io,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{process::Command, time::timeout};

struct Grant(AtomicUsize);
impl NativeEffectAuthorization for Grant {
    fn perform(&self, effect: &mut dyn FnMut() -> io::Result<usize>) -> io::Result<usize> {
        self.0
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .map_err(|_| io::ErrorKind::PermissionDenied)?;
        effect()
    }
}
fn sleeper() -> Child {
    Command::new("sleep")
        .arg("60")
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}
async fn running(pid: u32) {
    timeout(Duration::from_secs(3), async {
        loop {
            let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            let state = status
                .lines()
                .find(|line| line.starts_with("State:"))
                .unwrap();
            assert!(
                !state.contains("zombie"),
                "a denied request killed its owner"
            );
            if !state.contains("stopped") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn denied_tree_termination_restores_a_partial_stop_and_keeps_ownership() {
    let mut owner = ProcessOwner::start(sleeper(), None).unwrap();
    let pid = owner.pid().unwrap();
    let control = owner.control();
    for allowed in [0, 1] {
        let result = control
            .terminate(Arc::new(Grant(AtomicUsize::new(allowed))))
            .await;
        assert!(matches!(
            (allowed, result),
            (0, Err(CodexError::NotSent)) | (1, Err(CodexError::Unknown))
        ));
        assert_eq!(owner.pid(), Some(pid));
        running(pid).await;
    }
    // A new explicit request can still stop the retained owner.
    control
        .terminate(Arc::new(Grant(AtomicUsize::new(100))))
        .await
        .unwrap();
    owner.shutdown().await.unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[tokio::test]
async fn cancelled_termination_caller_does_not_cancel_cleanup_or_signal_another_owner() {
    let mut unrelated = sleeper();
    let mut owner = ProcessOwner::start(sleeper(), None).unwrap();
    let pid = owner.pid().unwrap();
    let control = owner.control();
    {
        let request = control.terminate(Arc::new(Grant(AtomicUsize::new(100))));
        tokio::pin!(request);
        // On this current-thread runtime, enqueue the command before the owner
        // worker runs. Then drop its pending caller and reply receiver.
        assert!(futures_util::poll!(request.as_mut()).is_pending());
    }
    let mut live = owner.liveness();
    timeout(Duration::from_secs(3), async {
        while *live.borrow_and_update() {
            live.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    owner.shutdown().await.unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().await.unwrap();
}

#[tokio::test]
async fn denied_attempt_does_not_resume_a_process_that_was_already_stopped() {
    let mut owner = ProcessOwner::start(sleeper(), None).unwrap();
    let pid = owner.pid().unwrap();
    // SAFETY: the owner retains this exact unreaped child throughout the test.
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGSTOP) }, 0);
    let state = || std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    timeout(Duration::from_secs(3), async {
        while !state().contains("T (stopped)") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        owner
            .control()
            .terminate(Arc::new(Grant(AtomicUsize::new(0))))
            .await,
        Err(CodexError::NotSent)
    ));
    assert!(state().contains("T (stopped)"));
    owner.shutdown().await.unwrap();
}

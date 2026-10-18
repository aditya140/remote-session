//! Domain authority is injected without connector/transport dependencies.
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::UnixStream,
};

/// Admit only the supplied nonblocking OS effect under original authority and
/// its replacement barrier. Invoke it at most once, never retain it or renew the grant.
/// Expired/fenced authority returns PermissionDenied.
pub trait NativeEffectAuthorization: Send + Sync {
    fn perform(&self, effect: &mut dyn FnMut() -> io::Result<usize>) -> io::Result<usize>;
}
#[derive(Clone, Default)]
pub(super) struct WriteScope(Arc<Mutex<Option<Arc<dyn NativeEffectAuthorization>>>>);
pub(super) struct Writing(WriteScope);
impl Drop for Writing {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().take();
    }
}
impl WriteScope {
    pub fn enter(&self, authorization: Option<Arc<dyn NativeEffectAuthorization>>) -> Writing {
        *self.0.lock().unwrap() = authorization;
        Writing(self.clone())
    }
}
pub(super) struct NativeStream {
    stream: UnixStream,
    scope: WriteScope,
    authorization_retry: Option<Pin<Box<tokio::time::Sleep>>>,
}
impl NativeStream {
    pub fn new(stream: UnixStream, scope: WriteScope) -> Self {
        Self {
            stream,
            scope,
            authorization_retry: None,
        }
    }
}
impl AsyncRead for NativeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}
impl AsyncWrite for NativeStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let authorization = self.scope.0.lock().unwrap().clone();
        if let Some(authorization) = authorization {
            let mut polled = false;
            let result = authorization.perform(&mut || {
                polled = true;
                match Pin::new(&mut self.stream).poll_write(context, bytes) {
                    Poll::Ready(result) => result,
                    Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
                }
            });
            match result {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !polled {
                        // A busy authority lock has no socket readiness waker.
                        // Retry briefly without blocking this native drain, while
                        // keeping the original work/grant and write deadline.
                        let retry = self.authorization_retry.get_or_insert_with(|| {
                            Box::pin(tokio::time::sleep(std::time::Duration::from_millis(2)))
                        });
                        if retry.as_mut().poll(context).is_ready() {
                            self.authorization_retry = None;
                            context.waker().wake_by_ref();
                        }
                    } else {
                        self.authorization_retry = None;
                    }
                    Poll::Pending
                }
                result => {
                    self.authorization_retry = None;
                    Poll::Ready(result)
                }
            }
        } else {
            Pin::new(&mut self.stream).poll_write(context, bytes)
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct Grant {
        enabled: AtomicBool,
        attempts: AtomicUsize,
    }
    impl NativeEffectAuthorization for Grant {
        fn perform(&self, effect: &mut dyn FnMut() -> io::Result<usize>) -> io::Result<usize> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if !self.enabled.load(Ordering::SeqCst) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            effect()
        }
    }
    #[tokio::test]
    async fn blocked_native_socket_rechecks_authority_before_every_later_os_write() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let scope = WriteScope::default();
        let grant = Arc::new(Grant {
            enabled: AtomicBool::new(true),
            attempts: AtomicUsize::new(0),
        });
        let writing = scope.enter(Some(grant.clone()));
        let mut stream = NativeStream::new(stream, scope);
        let work = tokio::spawn(async move {
            let _writing = writing;
            stream.write_all(&vec![b'x'; 8 * 1024 * 1024]).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while grant.attempts.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !work.is_finished(),
            "native socket must actually be backpressured"
        );
        grant.enabled.store(false, Ordering::SeqCst);
        let received = tokio::spawn(async move {
            let mut data = Vec::new();
            peer.read_to_end(&mut data).await.unwrap();
            data.len()
        });
        let error = tokio::time::timeout(std::time::Duration::from_secs(2), work)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(received.await.unwrap() < 8 * 1024 * 1024);
    }
}

//! Owned same-user IPC fault fixture: lose one delivery before native I/O,
//! then lose its application ACK after native I/O. All other bytes are relayed.
use qgenie_remote_proto::{ipc::DispatchFrame, wire::Message};
use qgenie_remote_transport::ipc::{Listener, connect, read_dispatch, write_dispatch};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
pub(super) struct Proxy {
    task: tokio::task::JoinHandle<()>,
    stage: Arc<AtomicUsize>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Proxy {
    pub fn start(directory: &Path, original: &Path) -> Self {
        let listener = Listener::bind(&directory.join("dispatch.sock")).unwrap();
        let stage = Arc::new(AtomicUsize::new(0));
        let observed = stage.clone();
        let original = original.join("dispatch.sock");
        let task = tokio::spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    incoming=listener.accept()=> {
                        let Ok(mut client)=incoming else {continue;};
                        let stage=observed.clone();let original=original.clone();
                        jobs.spawn(async move {
                            let mut native=connect(&original).await?;
                            let hello=read_dispatch(&mut client).await?;write_dispatch(&mut native,&hello).await?;
                            let welcome=read_dispatch(&mut native).await?;write_dispatch(&mut client,&welcome).await?;
                            let Ok(request)=read_dispatch(&mut client).await else {return Ok::<_,std::io::Error>(());};
                            let decision=matches!(&request,DispatchFrame::Remote{message,..} if matches!(message.message,Message::ApprovalCommitted(_)));
                            if decision && stage.compare_exchange(0,1,Ordering::SeqCst,Ordering::SeqCst).is_ok(){return Ok(());}
                            write_dispatch(&mut native,&request).await?;
                            if matches!(request,DispatchFrame::WatchChanges{..}) {
                                tokio::io::copy_bidirectional(&mut client,&mut native).await?;
                                return Ok(());
                            }
                            let response=read_dispatch(&mut native).await?;
                            if decision && matches!(&response,DispatchFrame::RemoteReply(message) if matches!(message.message,Message::ApprovalApplication{..})) && stage.compare_exchange(1,2,Ordering::SeqCst,Ordering::SeqCst).is_ok(){return Ok(());}
                            write_dispatch(&mut client,&response).await
                        });
                    },
                    _=jobs.join_next(),if !jobs.is_empty()=>{},
                }
            }
        });
        Self { task, stage }
    }
    pub fn complete(&self) -> bool {
        self.stage.load(Ordering::SeqCst) == 2
    }
}

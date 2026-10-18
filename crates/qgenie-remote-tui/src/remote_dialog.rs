//! Dialogs own no renderer cursor. Drain bounded unsolicited state while their
//! SDK requests run; their parent installs fresh state/checkpoints on return.
use qgenie_remote_client::LiveClient;
use std::future::Future;

pub(crate) async fn drain<T>(
    mut client: Option<&mut LiveClient>,
    work: impl Future<Output = T>,
) -> T {
    tokio::pin!(work);
    loop {
        tokio::select! {
            result=&mut work=>return result,
            event=async {match &mut client {Some(client)=>client.next().await,None=>std::future::pending().await}}=>{
                if event.is_none() {client=None;}
            }
        }
    }
}

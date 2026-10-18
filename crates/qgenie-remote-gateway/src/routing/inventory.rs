//! Per-client cutovers. A reconciliation skipped during an initial snapshot is
//! compared with that client's last delivered state on the next pass.
use super::{hub::Hub, now_ms};
use crate::Error;
use qgenie_remote_proto::{
    ClientId, MachineId, MessageId, UserId,
    wire::{Envelope, Machine, Message},
};
use qgenie_remote_transport::ws::{Frame, Priority};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
};

pub(super) struct Sent {
    revision: u64,
    digest: Arc<HashMap<MachineId, [u8; 32]>>,
}
struct Capture {
    revision: u64,
    machines: Vec<Machine>,
    digest: Arc<HashMap<MachineId, [u8; 32]>>,
}
impl Capture {
    fn sent(&self) -> Sent {
        Sent {
            revision: self.revision,
            digest: self.digest.clone(),
        }
    }
    fn updates(&self, previous: &Sent) -> Result<Vec<Frame>, Error> {
        if self.revision <= previous.revision {
            return Ok(Vec::new());
        }
        if previous
            .digest
            .keys()
            .any(|id| !self.digest.contains_key(id))
        {
            return pages(&self.machines);
        }
        Ok(self
            .machines
            .iter()
            .filter(|machine| previous.digest.get(&machine.id) != self.digest.get(&machine.id))
            .map(|machine| {
                let mut frame = Envelope::new(Message::MachineUpdated(machine.clone()));
                frame.machine = Some(machine.id);
                Frame::Text(Box::new(frame))
            })
            .collect())
    }
}
impl Hub {
    async fn capture_inventory(&self, owner: UserId) -> Result<Capture, Error> {
        let read = self
            .inventory_reads
            .lock()
            .unwrap()
            .entry(owner)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(0)))
            .clone();
        // Serialize authoritative reads, but never hold this lock while waiting
        // on a client writer. A later read always has a later process revision.
        let mut revision = read.lock().await;
        let machines = self.machines(owner).await?;
        if machines.len() > 1024 {
            return Err(Error::Overloaded);
        }
        let mut total = 0;
        let mut digest = HashMap::new();
        for machine in &machines {
            let bytes = serde_json::to_vec(machine).map_err(|_| Error::Invalid)?;
            total += bytes.len();
            if total > 8 * 1024 * 1024 {
                return Err(Error::Overloaded);
            }
            digest.insert(machine.id, Sha256::digest(bytes).into());
        }
        *revision = revision.checked_add(1).ok_or(Error::Overloaded)?;
        Ok(Capture {
            revision: *revision,
            machines,
            digest: Arc::new(digest),
        })
    }
    pub async fn pages(&self, client: ClientId, owner: UserId) -> Result<(), Error> {
        let _permit = self
            .snapshots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Overloaded)?;
        let peer = self
            .peers
            .lock()
            .unwrap()
            .clients
            .get(&client)
            .cloned()
            .ok_or(Error::NotFound)?;
        let mut sent = peer.inventory.lock().await;
        let capture = self.capture_inventory(owner).await?;
        for frame in pages(&capture.machines)? {
            if peer.expires.load(Ordering::Acquire).saturating_mul(1000) <= now_ms()? {
                return Err(Error::Unauthenticated);
            }
            let receipt = peer
                .sender
                .try_send(Priority::Control, frame)
                .map_err(|_| Error::Overloaded)?;
            receipt.transmitted().await.map_err(|_| {
                peer.stop.send_replace(true);
                Error::Unavailable
            })?;
        }
        *sent = Some(capture.sent());
        Ok(())
    }
    pub async fn reconcile_inventory(&self, owner: UserId) -> Result<(), Error> {
        let capture = self.capture_inventory(owner).await?;
        let peers = self
            .peers
            .lock()
            .unwrap()
            .clients
            .values()
            .filter(|peer| peer.owner == owner)
            .cloned()
            .collect::<Vec<_>>();
        for peer in peers {
            // Initial/explicit page transmission can wait on only its own
            // writer. Skipping it retains its old digest for the next pass.
            let Ok(mut sent) = peer.inventory.try_lock() else {
                continue;
            };
            let Some(previous) = sent.as_ref() else {
                continue;
            };
            if capture.revision <= previous.revision {
                continue;
            }
            let frames = capture.updates(previous)?;
            let mut delivered = true;
            for frame in frames {
                // Pages and deltas share one FIFO priority lane. A newer
                // snapshot cannot overtake an older delta in another lane.
                if Hub::deliver(&peer.sender, &peer.stop, frame, Priority::Control).is_err() {
                    delivered = false;
                    break;
                }
            }
            if delivered {
                *sent = Some(capture.sent());
            }
        }
        Ok(())
    }
}
fn pages(machines: &[Machine]) -> Result<Vec<Frame>, Error> {
    let mut pages = vec![Vec::new()];
    let mut size = 0usize;
    for machine in machines {
        let bytes = serde_json::to_vec(machine)
            .map_err(|_| Error::Invalid)?
            .len();
        if bytes > 512 * 1024 {
            return Err(Error::Overloaded);
        }
        if size + bytes > 512 * 1024 || pages.last().unwrap().len() >= 64 {
            pages.push(Vec::new());
            size = 0;
        }
        pages.last_mut().unwrap().push(machine.clone());
        size += bytes;
    }
    let epoch = MessageId::new().to_string();
    let total_pages = pages.len() as u16;
    Ok(pages
        .into_iter()
        .enumerate()
        .map(|(page, machines)| {
            Frame::Text(Box::new(Envelope::new(Message::MachinePage {
                epoch: epoch.clone(),
                page: page as u16,
                total_pages,
                machines,
            })))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::wire::Timestamp;
    fn machine(name: &str) -> Machine {
        Machine {
            id: MachineId::new(),
            owner: UserId::new(),
            name: name.into(),
            hostname: "host".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            cli_version: "test".into(),
            labels: Default::default(),
            agents: Default::default(),
            online: false,
            last_seen: Timestamp::from_unix_millis(0).unwrap(),
            sessions: 0,
        }
    }
    fn capture(revision: u64, machines: Vec<Machine>) -> Capture {
        let digest = Arc::new(
            machines
                .iter()
                .map(|machine| {
                    (
                        machine.id,
                        Sha256::digest(serde_json::to_vec(machine).unwrap()).into(),
                    )
                })
                .collect(),
        );
        Capture {
            revision,
            machines,
            digest,
        }
    }
    #[tokio::test]
    async fn rename_during_initial_cutover_is_retained_and_old_captures_cannot_roll_back() {
        let old = machine("old");
        let mut renamed = old.clone();
        renamed.name = "renamed while initial pages were sending".into();
        let initial = capture(1, vec![old]);
        let changed = capture(2, vec![renamed.clone()]);
        let delivered = tokio::sync::Mutex::new(None);
        let mut initial_write = delivered.lock().await;
        assert!(delivered.try_lock().is_err()); // reconciliation skips this client
        *initial_write = Some(initial.sent());
        drop(initial_write);
        let mut client = delivered.lock().await;
        let updates = changed.updates(client.as_ref().unwrap()).unwrap();
        assert!(
            matches!(&updates[..],[Frame::Text(frame)] if matches!(&frame.message,Message::MachineUpdated(machine) if machine.name==renamed.name))
        );
        *client = Some(changed.sent());
        assert!(
            initial
                .updates(client.as_ref().unwrap())
                .unwrap()
                .is_empty()
        );
        let removed = capture(3, Vec::new());
        assert!(
            matches!(&removed.updates(client.as_ref().unwrap()).unwrap()[..],[Frame::Text(frame)] if matches!(&frame.message,Message::MachinePage {machines,total_pages:1,..} if machines.is_empty()))
        );
    }
}

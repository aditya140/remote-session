use super::hub::Pending;
use crate::Error;
use qgenie_remote_proto::{
    Counter, MessageId,
    wire::{Message, SnapshotPage},
};
use std::collections::HashMap;

#[derive(Clone)]
pub(super) struct Replies {
    attach: bool,
    input: bool,
    native_read: bool,
    snapshot: Option<Progress>,
}
#[derive(Clone)]
struct Progress {
    epoch: String,
    seq: Counter,
    rows: u16,
    cols: u16,
    total: u16,
    next: u16,
}
impl Replies {
    pub fn new(request: &Message) -> Self {
        Self {
            attach: matches!(
                request,
                Message::Attach { .. } | Message::Capture | Message::ReplayRequest { .. }
            ),
            input: matches!(request, Message::Input { .. }),
            native_read: matches!(request, Message::Capture | Message::ReplayRequest { .. }),
            snapshot: None,
        }
    }
    pub fn is_continuation(&self) -> bool {
        self.input || self.native_read
    }
    fn accept(&mut self, message: &Message) -> Result<bool, Error> {
        if matches!(message, Message::Error(_)) {
            return Ok(true);
        }
        if self.input && !matches!(message, Message::InputAck { .. }) {
            return Err(Error::Invalid);
        }
        if let Message::Snapshot { snapshot } = message {
            if !self.attach {
                return Err(Error::Invalid);
            }
            snapshot_page(&mut self.snapshot, snapshot)?;
            return Ok(false); // Final Head confirms the post-capture stream head.
        }
        if let Some(progress) = &self.snapshot
            && !matches!(message,Message::Head {head,..} if progress.next==progress.total && *head>=progress.seq)
        {
            return Err(Error::Invalid);
        }
        Ok(true)
    }
}
fn snapshot_page(progress: &mut Option<Progress>, page: &SnapshotPage) -> Result<(), Error> {
    if page.total_pages == 0
        || page.total_pages > 32
        || page.epoch.is_empty()
        || page.epoch.len() > 128
    {
        return Err(Error::Invalid);
    }
    let expected = progress.get_or_insert_with(|| Progress {
        epoch: page.epoch.clone(),
        seq: page.seq,
        rows: page.rows,
        cols: page.cols,
        total: page.total_pages,
        next: 0,
    });
    if page.page != expected.next
        || page.total_pages != expected.total
        || page.epoch != expected.epoch
        || page.seq != expected.seq
        || page.rows != expected.rows
        || page.cols != expected.cols
        || page.page >= page.total_pages
    {
        return Err(Error::Invalid);
    }
    expected.next += 1;
    Ok(())
}
pub(super) fn response(
    pending: &mut HashMap<MessageId, Pending>,
    id: MessageId,
    message: &Message,
) -> Result<Option<Pending>, Error> {
    let Some(entry) = pending.get_mut(&id) else {
        return Ok(None);
    };
    let complete = entry.replies.accept(message)?;
    let value = entry.clone();
    if complete {
        pending.remove(&id);
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::SnapshotFormat;
    fn page(index: u16) -> Message {
        Message::Snapshot {
            snapshot: SnapshotPage {
                epoch: "capture".into(),
                seq: Counter(10),
                page: index,
                total_pages: 2,
                rows: 24,
                cols: 80,
                format: SnapshotFormat::Vt100StateV1,
                data: String::new(),
            },
        }
    }
    #[test]
    fn snapshot_reply_requires_contiguous_pages_and_a_final_head() {
        let mut replies = Replies::new(&Message::Attach { after: Counter(0) });
        assert!(replies.accept(&page(1)).is_err());
        let mut replies = Replies::new(&Message::Attach { after: Counter(0) });
        assert!(!replies.accept(&page(0)).unwrap());
        assert!(replies.accept(&page(0)).is_err());
        assert!(
            replies
                .accept(&Message::Head {
                    floor: Counter(1),
                    head: Counter(10),
                    last_input: Counter(0)
                })
                .is_err()
        );
        assert!(!replies.accept(&page(1)).unwrap());
        assert!(
            replies
                .accept(&Message::Head {
                    floor: Counter(1),
                    head: Counter(9),
                    last_input: Counter(0)
                })
                .is_err()
        );
        assert!(
            replies
                .accept(&Message::Head {
                    floor: Counter(1),
                    head: Counter(12),
                    last_input: Counter(0)
                })
                .unwrap()
        );
        let mut other = Replies::new(&Message::MachineList);
        assert!(other.accept(&page(0)).is_err());
    }
}

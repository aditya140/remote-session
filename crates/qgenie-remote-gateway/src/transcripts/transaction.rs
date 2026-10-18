//! Recording transactions share the bounded primary commit supervisor.
pub(super) use crate::transaction::{AuthorityTransaction as RecordingTransaction, Commits};
impl super::RecordingDatabase {
    pub(super) async fn transaction(&self) -> Result<RecordingTransaction, crate::Error> {
        crate::transaction::begin(&self.pool, self.commits.clone()).await
    }
}

use super::transaction::RecordingTransaction as Transaction;
use super::{ProtectedRecording, RecordingDatabase, RecordingRole};
use crate::{Error, IdentityService, MachinePrincipal};
use qgenie_remote_proto::{
    ReceiptId, SessionId,
    wire::{CommitReceipt, GapCommitReceipt, LossRange, ReceiptDisposition, Timestamp},
};
use serde_json::{Value, json};
use sqlx::{Row, postgres::PgRow};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::{Date, Month, OffsetDateTime};

pub(super) struct AccessWindow {
    started: Duration,
    until: Duration,
}
impl AccessWindow {
    fn new(actor: &MachinePrincipal) -> Result<Self, Error> {
        Self::user(actor.expires_at)
    }
    pub(super) fn user(expires_at: u64) -> Result<Self, Error> {
        let started = qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)?;
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Unavailable)?;
        let left = Duration::from_secs(expires_at)
            .checked_sub(wall)
            .filter(|v| !v.is_zero())
            .ok_or(Error::Unauthenticated)?;
        Ok(Self {
            started,
            until: started + left.min(Duration::from_secs(10)),
        })
    }
    pub(super) fn check(&self) -> Result<(), Error> {
        let now = qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)?;
        if now < self.started || now >= self.until {
            Err(Error::Unauthenticated)
        } else {
            Ok(())
        }
    }
}
struct Scope {
    epoch: String,
    created: OffsetDateTime,
    ended: Option<OffsetDateTime>,
    days: i32,
    retry: OffsetDateTime,
}
pub(super) fn db(error: sqlx::Error) -> Error {
    match error.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("42501") => Error::Forbidden,
        Some("22023") => Error::Invalid,
        Some("23505") | Some("23514") => Error::Conflict,
        _ => Error::Database,
    }
}
fn datetime(time: &Timestamp) -> Result<OffsetDateTime, Error> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(time.unix_millis()) * 1_000_000)
        .map_err(|_| Error::Invalid)
}
fn timestamp(time: OffsetDateTime) -> Result<Timestamp, Error> {
    Timestamp::from_unix_millis(
        (time.unix_timestamp_nanos() / 1_000_000)
            .try_into()
            .map_err(|_| Error::Database)?,
    )
    .map_err(|_| Error::Database)
}
fn text<T: serde::Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_value(value)
        .map_err(|_| Error::Invalid)?
        .as_str()
        .map(str::to_owned)
        .ok_or(Error::Invalid)
}

impl RecordingDatabase {
    /// Independent primary-authorized epoch exchange. It writes a fresh durability
    /// barrier and returns only after remote apply; a receipt never supplies this binding.
    pub async fn recording_authority(
        &self,
        identity: &IdentityService,
        actor: &MachinePrincipal,
        session: SessionId,
        challenge: qgenie_remote_proto::ChallengeId,
    ) -> Result<qgenie_remote_proto::rest::RecordingAuthority, Error> {
        let (tx, scope, access) = self.begin_ingest(identity, actor, session, false).await?;
        Self::current(identity, actor, &access)?;
        tx.commit().await?;
        Self::current(identity, actor, &access)?;
        Ok(qgenie_remote_proto::rest::RecordingAuthority {
            challenge,
            origin: identity.origin().into(),
            owner: actor.owner,
            machine: actor.machine,
            key: actor.key,
            backend_epoch: scope.epoch,
        })
    }
    /// Current identity/directory and protected lifecycle policy are mandatory. This library
    /// entry point is not a public network route. Synthetic qualification exists only in tests.
    pub async fn commit_recording(
        &self,
        identity: &IdentityService,
        actor: &MachinePrincipal,
        recording: ProtectedRecording,
    ) -> Result<CommitReceipt, Error> {
        self.commit_content(identity, actor, recording, false).await
    }
    pub async fn commit_gap(
        &self,
        identity: &IdentityService,
        actor: &MachinePrincipal,
        gap: LossRange,
    ) -> Result<GapCommitReceipt, Error> {
        self.commit_loss(identity, actor, gap, false).await
    }
    async fn begin_ingest(
        &self,
        identity: &IdentityService,
        actor: &MachinePrincipal,
        session: SessionId,
        synthetic: bool,
    ) -> Result<(Transaction, Scope, AccessWindow), Error> {
        if self.role != RecordingRole::Ingest {
            return Err(Error::Role);
        }
        if !synthetic && identity.directory.is_none() {
            return Err(Error::Configuration);
        }
        if !synthetic {
            let version: Option<i32> =
                sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                    .fetch_one(&self.pool)
                    .await
                    .map_err(db)?;
            let version = version.ok_or(Error::Schema)?;
            if version > crate::SCHEMA_MAX {
                return Err(Error::Schema);
            }
            if version < super::RECORDING_RELEASE_SCHEMA {
                return Err(Error::Unavailable);
            }
        }
        let access = AccessWindow::new(actor)?;
        identity.assert_machine(actor).await?;
        access.check()?;
        let mut tx = self.transaction().await?;
        // PG17+ bounds active SQL; the owned transaction supervisor separately bounds
        // synchronous COMMIT, where PostgreSQL disables these server-side timers.
        sqlx::raw_sql("SET LOCAL transaction_timeout='3s'; SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='2s';").execute(&mut *tx).await.map_err(db)?;
        let row = sqlx::query("SELECT * FROM qgr_recordings.ingest_scope($1,$2,$3,$4,$5)")
            .bind(actor.machine.to_string())
            .bind(actor.key.to_string())
            .bind(session.to_string())
            .bind(actor.owner.to_string())
            .bind(i64::try_from(actor.expires_at).map_err(|_| Error::Unauthenticated)?)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let mode: String = row.get("durability");
        let qualified = if synthetic {
            mode == "synthetic_single_server"
        } else {
            mode == "synchronous_standby" && row.get::<bool, _>("synchronous_ready")
        };
        if !qualified
            || !row.get::<bool, _>("lifecycle_ready")
            || row.get::<bool, _>("recovery_pending")
        {
            return Err(Error::Unavailable);
        }
        sqlx::query("SELECT set_config('synchronous_commit',$1,true)")
            .bind(if synthetic { "on" } else { "remote_apply" })
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let scope = Scope {
            epoch: row.get("recovery_epoch"),
            created: row.get("created_at"),
            ended: row.get("ended_upper"),
            days: row.get("retention_days"),
            retry: row.get("retry_not_before"),
        };
        access.check()?;
        Ok((tx, scope, access))
    }
    fn current(
        identity: &IdentityService,
        actor: &MachinePrincipal,
        access: &AccessWindow,
    ) -> Result<(), Error> {
        access.check()?;
        if let Some(directory) = &identity.directory {
            directory.cached(actor.owner)?;
        }
        if crate::identity::jwt::unix_seconds()? >= actor.expires_at {
            return Err(Error::Unauthenticated);
        }
        Ok(())
    }
    async fn commit_content(
        &self,
        identity: &IdentityService,
        actor: &MachinePrincipal,
        protected: ProtectedRecording,
        synthetic: bool,
    ) -> Result<CommitReceipt, Error> {
        let batch = &protected.batch;
        if batch.machine != actor.machine {
            return Err(Error::Forbidden);
        }
        let (mut tx, scope, access) = self
            .begin_ingest(identity, actor, batch.session, synthetic)
            .await?;
        let first = datetime(&batch.captured_from)?;
        let last = datetime(&batch.captured_to)?;
        lifetime(&scope, first, last)?;
        let immutable = json!({"batch":batch,"stored":{"digest":protected.stored_digest,"uncompressed_bytes":protected.uncompressed_bytes,"redaction_saturated":protected.redaction_saturated}});
        let previous = prior(
            &mut tx,
            actor,
            batch.session,
            "content",
            &batch.id.to_string(),
        )
        .await?;
        if let Some(row) = &previous
            && row.get::<Value, _>("immutable").get("batch") != immutable.get("batch")
        {
            return Err(Error::Conflict);
        }
        if let Some(receipt) = epoch_receipt(
            &mut tx,
            actor,
            batch.session,
            "content",
            &batch.id.to_string(),
            &scope.epoch,
        )
        .await?
        {
            let receipt = serde_json::from_value(receipt).map_err(|_| Error::Database)?;
            Self::current(identity, actor, &access)?;
            tx.commit().await?;
            Self::current(identity, actor, &access)?;
            return Ok(receipt);
        }
        if last < scope.retry {
            return Err(Error::Invalid);
        }
        let month =
            Date::from_calendar_date(first.year(), first.month(), 1).map_err(|_| Error::Invalid)?;
        let next = next_month(month)?;
        let expired = next.midnight().assume_utc()
            <= OffsetDateTime::now_utc() - time::Duration::days(i64::from(scope.days));
        let retained = if expired {
            false
        } else {
            sqlx::query_scalar::<_, bool>("SELECT qgr_recordings.ensure_month($1)")
                .bind(month)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?
        };
        let receipt = CommitReceipt {
            id: ReceiptId::new(),
            chunk: batch.id,
            session: batch.session,
            machine: actor.machine,
            stream_epoch: batch.stream_epoch,
            captured_from: batch.captured_from.clone(),
            captured_to: batch.captured_to.clone(),
            from: batch.from,
            to: batch.to,
            digest: batch.digest.clone(),
            recovery_epoch: scope.epoch.clone(),
            disposition: if retained {
                ReceiptDisposition::Stored
            } else {
                ReceiptDisposition::Expired
            },
            committed_at: timestamp(OffsetDateTime::now_utc())?,
        };
        if previous.is_none() {
            insert_base(
                &mut tx,
                actor,
                batch.session,
                "content",
                &batch.id.to_string(),
                &immutable,
                &serde_json::to_value(&receipt).map_err(|_| Error::Invalid)?,
                first,
                last,
            )
            .await?;
        }
        bind_stream(
            &mut tx,
            actor,
            batch.session,
            &batch.stream_epoch.to_string(),
            &text(&batch.stream)?,
        )
        .await?;
        // Fragment identity is retained for every accepted upload, including policy-expired
        // content. It must neither reveal private hold coverage nor allow sequence reuse.
        check_fragments(
            &mut tx,
            actor,
            batch.session,
            &batch.stream_epoch.to_string(),
            &batch.id.to_string(),
            &protected.fragments,
        )
        .await?;
        if retained {
            sqlx::query("INSERT INTO qgr_recordings.content(machine,session,owner,id,captured_from,captured_to,payload,stored_digest) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING").bind(actor.machine.to_string()).bind(batch.session.to_string()).bind(actor.owner.to_string()).bind(batch.id.to_string()).bind(first).bind(last).bind(&protected.payload).bind(&protected.stored_digest).execute(&mut *tx).await.map_err(db)?;
        }
        sqlx::query("INSERT INTO qgr_recordings.fragments(machine,session,owner,stream_epoch,stream,seq,fragment,last,chunk) SELECT $1,$2,$3,$4,$5,x.seq::numeric,x.fragment,x.last,$6 FROM jsonb_to_recordset($7) AS x(seq text,fragment integer,last boolean) ON CONFLICT DO NOTHING").bind(actor.machine.to_string()).bind(batch.session.to_string()).bind(actor.owner.to_string()).bind(batch.stream_epoch.to_string()).bind(text(&batch.stream)?).bind(batch.id.to_string()).bind(&protected.fragments).execute(&mut *tx).await.map_err(db)?;
        // Ordinary expiry determines the public disposition even when private hold policy
        // preserves bytes. Neither receipts nor owner history disclose hold coverage.
        sqlx::query("SELECT qgr_recordings.preserve_upload_content($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(actor.machine.to_string())
            .bind(batch.session.to_string())
            .bind(actor.owner.to_string())
            .bind(batch.id.to_string())
            .bind(first)
            .bind(last)
            .bind(&protected.payload)
            .bind(&protected.stored_digest)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        insert_epoch(
            &mut tx,
            actor,
            batch.session,
            "content",
            &batch.id.to_string(),
            &scope.epoch,
            &serde_json::to_value(&receipt).map_err(|_| Error::Invalid)?,
        )
        .await?;
        Self::current(identity, actor, &access)?;
        tx.commit().await?;
        Self::current(identity, actor, &access)?;
        Ok(receipt)
    }
    async fn commit_loss(
        &self,
        identity: &IdentityService,
        actor: &MachinePrincipal,
        gap: LossRange,
        synthetic: bool,
    ) -> Result<GapCommitReceipt, Error> {
        gap.validate().map_err(|_| Error::Invalid)?;
        let (mut tx, scope, access) = self
            .begin_ingest(identity, actor, gap.session, synthetic)
            .await?;
        let first = datetime(&gap.captured_from)?;
        let last = datetime(&gap.captured_to)?;
        lifetime(&scope, first, last)?;
        let immutable = serde_json::to_value(&gap).map_err(|_| Error::Invalid)?;
        let previous = prior(&mut tx, actor, gap.session, "gap", &gap.id.to_string()).await?;
        if let Some(row) = &previous
            && row.get::<Value, _>("immutable") != immutable
        {
            return Err(Error::Conflict);
        }
        if let Some(receipt) = epoch_receipt(
            &mut tx,
            actor,
            gap.session,
            "gap",
            &gap.id.to_string(),
            &scope.epoch,
        )
        .await?
        {
            let receipt = serde_json::from_value(receipt).map_err(|_| Error::Database)?;
            Self::current(identity, actor, &access)?;
            tx.commit().await?;
            Self::current(identity, actor, &access)?;
            return Ok(receipt);
        }
        if last < scope.retry {
            return Err(Error::Invalid);
        }
        let month =
            Date::from_calendar_date(last.year(), last.month(), 1).map_err(|_| Error::Invalid)?;
        let retained = next_month(month)?.midnight().assume_utc()
            > OffsetDateTime::now_utc() - time::Duration::days(i64::from(scope.days));
        let receipt = GapCommitReceipt {
            id: ReceiptId::new(),
            gap: gap.id,
            session: gap.session,
            machine: actor.machine,
            stream_epoch: gap.stream_epoch,
            from: gap.from,
            to: gap.to,
            captured_from: gap.captured_from.clone(),
            captured_to: gap.captured_to.clone(),
            digest: gap.receipt_digest().map_err(|_| Error::Invalid)?,
            recovery_epoch: scope.epoch.clone(),
            disposition: if retained {
                ReceiptDisposition::Stored
            } else {
                ReceiptDisposition::Expired
            },
            committed_at: timestamp(OffsetDateTime::now_utc())?,
        };
        if previous.is_none() {
            insert_base(
                &mut tx,
                actor,
                gap.session,
                "gap",
                &gap.id.to_string(),
                &immutable,
                &serde_json::to_value(&receipt).map_err(|_| Error::Invalid)?,
                first,
                last,
            )
            .await?;
        }
        bind_stream(
            &mut tx,
            actor,
            gap.session,
            &gap.stream_epoch.to_string(),
            &text(&gap.stream)?,
        )
        .await?;
        if retained {
            sqlx::query("INSERT INTO qgr_recordings.gaps(machine,session,owner,id,captured_from,captured_to,body) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING").bind(actor.machine.to_string()).bind(gap.session.to_string()).bind(actor.owner.to_string()).bind(gap.id.to_string()).bind(first).bind(last).bind(&immutable).execute(&mut *tx).await.map_err(db)?;
        }
        sqlx::query("SELECT qgr_recordings.preserve_upload_gap($1,$2,$3,$4,$5,$6,$7)")
            .bind(actor.machine.to_string())
            .bind(gap.session.to_string())
            .bind(actor.owner.to_string())
            .bind(gap.id.to_string())
            .bind(first)
            .bind(last)
            .bind(&immutable)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        insert_epoch(
            &mut tx,
            actor,
            gap.session,
            "gap",
            &gap.id.to_string(),
            &scope.epoch,
            &serde_json::to_value(&receipt).map_err(|_| Error::Invalid)?,
        )
        .await?;
        Self::current(identity, actor, &access)?;
        tx.commit().await?;
        Self::current(identity, actor, &access)?;
        Ok(receipt)
    }
}

fn lifetime(scope: &Scope, first: OffsetDateTime, last: OffsetDateTime) -> Result<(), Error> {
    let skew = time::Duration::minutes(5);
    if first > last
        || first < scope.created - skew
        || last > OffsetDateTime::now_utc() + skew
        || scope.ended.is_some_and(|end| last > end + skew)
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
fn next_month(month: Date) -> Result<Date, Error> {
    let (year, next) = if month.month() == Month::December {
        (month.year() + 1, Month::January)
    } else {
        (month.year(), month.month().next())
    };
    Date::from_calendar_date(year, next, 1).map_err(|_| Error::Invalid)
}
async fn prior(
    tx: &mut Transaction,
    actor: &MachinePrincipal,
    session: SessionId,
    kind: &str,
    id: &str,
) -> Result<Option<PgRow>, Error> {
    sqlx::query("SELECT immutable,receipt FROM qgr_recordings.receipts WHERE machine=$1 AND session=$2 AND kind=$3 AND id=$4").bind(actor.machine.to_string()).bind(session.to_string()).bind(kind).bind(id).fetch_optional(&mut **tx).await.map_err(db)
}
async fn epoch_receipt(
    tx: &mut Transaction,
    actor: &MachinePrincipal,
    session: SessionId,
    kind: &str,
    id: &str,
    epoch: &str,
) -> Result<Option<Value>, Error> {
    sqlx::query_scalar("SELECT receipt FROM qgr_recordings.receipt_epochs WHERE machine=$1 AND session=$2 AND kind=$3 AND id=$4 AND recovery_epoch=$5").bind(actor.machine.to_string()).bind(session.to_string()).bind(kind).bind(id).bind(epoch).fetch_optional(&mut **tx).await.map_err(db)
}
#[allow(clippy::too_many_arguments)]
async fn insert_base(
    tx: &mut Transaction,
    actor: &MachinePrincipal,
    session: SessionId,
    kind: &str,
    id: &str,
    immutable: &Value,
    receipt: &Value,
    first: OffsetDateTime,
    last: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO qgr_recordings.receipts(machine,session,owner,kind,id,immutable,receipt,captured_from,captured_to) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(actor.machine.to_string()).bind(session.to_string()).bind(actor.owner.to_string()).bind(kind).bind(id).bind(immutable).bind(receipt).bind(first).bind(last).execute(&mut **tx).await.map_err(db)?;
    Ok(())
}
async fn insert_epoch(
    tx: &mut Transaction,
    actor: &MachinePrincipal,
    session: SessionId,
    kind: &str,
    id: &str,
    epoch: &str,
    receipt: &Value,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO qgr_recordings.receipt_epochs(machine,session,owner,kind,id,recovery_epoch,receipt) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(actor.machine.to_string()).bind(session.to_string()).bind(actor.owner.to_string()).bind(kind).bind(id).bind(epoch).bind(receipt).execute(&mut **tx).await.map_err(db)?;
    Ok(())
}
async fn bind_stream(
    tx: &mut Transaction,
    actor: &MachinePrincipal,
    session: SessionId,
    epoch: &str,
    stream: &str,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO qgr_recordings.streams(machine,session,owner,stream_epoch,stream) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(actor.machine.to_string()).bind(session.to_string()).bind(actor.owner.to_string()).bind(epoch).bind(stream).execute(&mut **tx).await.map_err(db)?;
    let matches:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr_recordings.streams WHERE machine=$1 AND session=$2 AND owner=$3 AND stream_epoch=$4 AND stream=$5)").bind(actor.machine.to_string()).bind(session.to_string()).bind(actor.owner.to_string()).bind(epoch).bind(stream).fetch_one(&mut **tx).await.map_err(db)?;
    if matches {
        Ok(())
    } else {
        Err(Error::Conflict)
    }
}
async fn check_fragments(
    tx: &mut Transaction,
    actor: &MachinePrincipal,
    session: SessionId,
    epoch: &str,
    chunk: &str,
    manifest: &Value,
) -> Result<(), Error> {
    let conflict:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr_recordings.fragments f JOIN jsonb_to_recordset($5) AS x(seq text,fragment integer,last boolean) ON f.seq=x.seq::numeric WHERE f.machine=$1 AND f.session=$2 AND f.stream_epoch=$3 AND ((f.fragment=x.fragment AND (f.chunk<>$4 OR f.last<>x.last)) OR (f.last AND f.fragment<x.fragment) OR (x.last AND x.fragment<f.fragment)))").bind(actor.machine.to_string()).bind(session.to_string()).bind(epoch).bind(chunk).bind(manifest).fetch_one(&mut **tx).await.map_err(db)?;
    if conflict {
        Err(Error::Conflict)
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests;

use super::{IdentityService, db, machines, now_ms};
use crate::{AuditAction, AuditEvent, AuditMetadata, Error, Store};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use qgenie_remote_proto::{
    ChallengeId, Counter, KeyId, MachineId, UserId,
    rest::{Challenge, Enrolled, EnrollmentRequest, MachineToken, Proof},
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, postgres::PgRow};
#[derive(Serialize, Deserialize)]
struct Payload {
    challenge: Challenge,
    enrollment: Option<EnrollmentRequest>,
}
impl IdentityService {
    pub async fn enrollment_challenge(
        &self,
        owner: UserId,
        request: EnrollmentRequest,
    ) -> Result<Challenge, Error> {
        machines::validate(&request)?;
        self.assert_directory(owner).await?;
        let mut transaction = self.store.transaction().await?;
        machines::assert_user(&mut transaction, owner).await?;
        let scope = format!("{owner}/{}", request.installation);
        lock(&mut transaction, &scope).await?;
        machines::cleanup(&mut transaction).await?;
        let now = now_ms()?;
        let pending:i64=sqlx::query_scalar("SELECT count(*) FROM qgr.machine_challenges WHERE owner=$1 AND expires_at_ms>$2 AND NOT consumed").bind(owner.to_string()).bind(now as i64).fetch_one(&mut *transaction).await.map_err(db)?;
        if pending >= 128 {
            return Err(Error::Overloaded);
        }
        let existing=sqlx::query("SELECT m.*,k.public_key,k.revoked FROM qgr.machines m JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id WHERE m.owner=$1 AND m.installation=$2 FOR UPDATE OF m,k").bind(owner.to_string()).bind(request.installation.to_string()).fetch_optional(&mut *transaction).await.map_err(db)?;
        let (machine, kind) = if let Some(row) = existing {
            if row.get::<bool, _>("disabled")
                || row.get::<bool, _>("revoked")
                || row.get::<String, _>("os_user") != request.os_user
            {
                return Err(Error::Forbidden);
            }
            let machine: MachineId = row
                .get::<String, _>("id")
                .parse()
                .map_err(|_| Error::Database)?;
            let same = row.get::<String, _>("active_key") == request.key.to_string()
                && row.get::<Vec<u8>, _>("public_key") == machines::public_key(&request)?;
            if !same
                && !request.recovery.as_ref().is_some_and(|recovery| {
                    recovery.confirmed
                        && recovery.machine == machine
                        && row.get::<String, _>("active_key") != request.key.to_string()
                })
            {
                return Err(Error::Conflict);
            }
            if request
                .recovery
                .as_ref()
                .is_some_and(|recovery| recovery.machine != machine)
            {
                return Err(Error::NotFound);
            }
            (machine, if same { "enroll" } else { "recover" })
        } else {
            if request.recovery.is_some() {
                return Err(Error::NotFound);
            }
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.machines WHERE owner=$1")
                .bind(owner.to_string())
                .fetch_one(&mut *transaction)
                .await
                .map_err(db)?;
            if count >= 1024 {
                return Err(Error::Overloaded);
            }
            let provisional:Option<String>=sqlx::query_scalar("SELECT machine FROM qgr.machine_challenges WHERE owner=$1 AND kind='enroll' AND NOT consumed AND expires_at_ms>$2 AND payload->'enrollment'->>'installation'=$3 ORDER BY issued_at_ms DESC LIMIT 1").bind(owner.to_string()).bind(now as i64).bind(request.installation.to_string()).fetch_optional(&mut *transaction).await.map_err(db)?;
            (
                provisional
                    .map(|id| id.parse().map_err(|_| Error::Database))
                    .transpose()?
                    .unwrap_or_default(),
                "enroll",
            )
        };
        let challenge = challenge(&self.tokens.origin, machine, request.key, now)?;
        save(&mut transaction, owner, kind, &challenge, Some(request)).await?;
        transaction.commit().await?;
        Ok(challenge)
    }
    pub async fn enroll(&self, owner: UserId, proof: Proof) -> Result<Enrolled, Error> {
        self.assert_directory(owner).await?;
        let mut transaction = self.store.transaction().await?;
        machines::assert_user(&mut transaction, owner).await?;
        let preliminary =
            sqlx::query("SELECT owner,payload FROM qgr.machine_challenges WHERE id=$1")
                .bind(proof.challenge.to_string())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(db)?
                .ok_or(Error::Unauthenticated)?;
        if preliminary.get::<String, _>("owner") != owner.to_string() {
            return Err(Error::Forbidden);
        }
        let provisional: Payload =
            serde_json::from_value(preliminary.get("payload")).map_err(|_| Error::Database)?;
        let installation = provisional
            .enrollment
            .as_ref()
            .ok_or(Error::Invalid)?
            .installation;
        lock(&mut transaction, &format!("{owner}/{installation}")).await?;
        let (row, payload) = load(&mut transaction, &proof).await?;
        if payload.challenge.origin != self.tokens.origin
            || row.get::<String, _>("owner") != owner.to_string()
            || !["enroll", "recover"].contains(&row.get::<String, _>("kind").as_str())
        {
            return Err(Error::Forbidden);
        }
        let request = payload.enrollment.as_ref().ok_or(Error::Invalid)?;
        verify(&payload.challenge, &machines::public_key(request)?, &proof)?;
        let existing=sqlx::query("SELECT m.*,k.public_key,k.revoked FROM qgr.machines m JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id WHERE m.owner=$1 AND m.installation=$2 FOR UPDATE OF m,k").bind(owner.to_string()).bind(request.installation.to_string()).fetch_optional(&mut *transaction).await.map_err(db)?;
        let id = payload.challenge.machine;
        let action = if let Some(row) = existing {
            if row.get::<String, _>("id") != id.to_string()
                || row.get::<String, _>("os_user") != request.os_user
                || row.get::<bool, _>("disabled")
                || row.get::<bool, _>("revoked")
            {
                return Err(Error::Forbidden);
            }
            let same = row.get::<String, _>("active_key") == request.key.to_string()
                && row.get::<Vec<u8>, _>("public_key") == machines::public_key(request)?;
            if !same {
                if !request
                    .recovery
                    .as_ref()
                    .is_some_and(|recovery| recovery.confirmed && recovery.machine == id)
                {
                    return Err(Error::Conflict);
                }
                sqlx::query("INSERT INTO qgr.machine_keys(id,machine,public_key) VALUES($1,$2,$3)")
                    .bind(request.key.to_string())
                    .bind(id.to_string())
                    .bind(machines::public_key(request)?)
                    .execute(&mut *transaction)
                    .await
                    .map_err(db)?;
                sqlx::query("UPDATE qgr.machine_keys SET revoked=true WHERE machine=$1 AND id<>$2")
                    .bind(id.to_string())
                    .bind(request.key.to_string())
                    .execute(&mut *transaction)
                    .await
                    .map_err(db)?;
            }
            sqlx::query("UPDATE qgr.machines SET active_key=$2,hostname=$3,os=$4,arch=$5,cli_version=$6 WHERE id=$1").bind(id.to_string()).bind(request.key.to_string()).bind(&request.hostname).bind(&request.os).bind(&request.arch).bind(&request.cli_version).execute(&mut *transaction).await.map_err(db)?;
            if same {
                AuditAction::MachineEnrolled
            } else {
                AuditAction::MachineKeyRecovered
            }
        } else {
            if request.recovery.is_some() {
                return Err(Error::NotFound);
            }
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.machines WHERE owner=$1")
                .bind(owner.to_string())
                .fetch_one(&mut *transaction)
                .await
                .map_err(db)?;
            if count >= 1024 {
                return Err(Error::Overloaded);
            }
            sqlx::query("INSERT INTO qgr.machines(id,owner,installation,os_user,active_key,hostname,name,os,arch,cli_version,labels) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(id.to_string()).bind(owner.to_string()).bind(request.installation.to_string()).bind(&request.os_user).bind(request.key.to_string()).bind(&request.hostname).bind(&request.name).bind(&request.os).bind(&request.arch).bind(&request.cli_version).bind(serde_json::to_value(&request.labels).map_err(|_| Error::Invalid)?).execute(&mut *transaction).await.map_err(db)?;
            sqlx::query("INSERT INTO qgr.machine_keys(id,machine,public_key) VALUES($1,$2,$3)")
                .bind(request.key.to_string())
                .bind(id.to_string())
                .bind(machines::public_key(request)?)
                .execute(&mut *transaction)
                .await
                .map_err(db)?;
            AuditAction::MachineEnrolled
        };
        consume(&mut transaction, proof.challenge).await?;
        Store::append_audit(
            &mut transaction,
            &AuditEvent {
                actor: owner,
                action,
                machine: Some(id),
                session: None,
                metadata: AuditMetadata::default(),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(Enrolled {
            machine: id,
            key: request.key,
        })
    }
    pub async fn machine_challenge(
        &self,
        machine: MachineId,
        key: KeyId,
    ) -> Result<Challenge, Error> {
        let owner: String =
            sqlx::query_scalar("SELECT owner FROM qgr.machines WHERE id=$1 AND active_key=$2")
                .bind(machine.to_string())
                .bind(key.to_string())
                .fetch_optional(&self.store.pool)
                .await
                .map_err(db)?
                .ok_or(Error::Unauthenticated)?;
        self.assert_directory(owner.parse().map_err(|_| Error::Database)?)
            .await?;
        let mut transaction = self.store.transaction().await?;
        lock(&mut transaction, &machine.to_string()).await?;
        Store::lock_access(&mut transaction).await?;
        machines::cleanup(&mut transaction).await?;
        let row=sqlx::query("SELECT m.owner,m.disabled OR u.disabled OR a.disabled OR k.revoked AS denied FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a WHERE u.directory_object IS NOT NULL AND m.id=$1 AND m.active_key=$2 FOR SHARE OF m,k,u,a").bind(machine.to_string()).bind(key.to_string()).fetch_optional(&mut *transaction).await.map_err(db)?.ok_or(Error::Unauthenticated)?;
        if row.get::<bool, _>("denied") {
            return Err(Error::Forbidden);
        }
        let now = now_ms()?;
        let pending:i64=sqlx::query_scalar("SELECT count(*) FROM qgr.machine_challenges WHERE machine=$1 AND expires_at_ms>$2 AND NOT consumed").bind(machine.to_string()).bind(now as i64).fetch_one(&mut *transaction).await.map_err(db)?;
        if pending >= 16 {
            return Err(Error::Overloaded);
        }
        let owner = row
            .get::<String, _>("owner")
            .parse()
            .map_err(|_| Error::Database)?;
        let challenge = challenge(&self.tokens.origin, machine, key, now)?;
        save(&mut transaction, owner, "auth", &challenge, None).await?;
        transaction.commit().await?;
        Ok(challenge)
    }
    pub async fn machine_token(&self, proof: Proof) -> Result<MachineToken, Error> {
        let owner: String = sqlx::query_scalar(
            "SELECT owner FROM qgr.machine_challenges WHERE id=$1 AND NOT consumed",
        )
        .bind(proof.challenge.to_string())
        .fetch_optional(&self.store.pool)
        .await
        .map_err(db)?
        .ok_or(Error::Unauthenticated)?;
        self.assert_directory(owner.parse().map_err(|_| Error::Database)?)
            .await?;
        let mut transaction = self.store.transaction().await?;
        let (row, payload) = load(&mut transaction, &proof).await?;
        if payload.challenge.origin != self.tokens.origin {
            return Err(Error::Unauthenticated);
        }
        if row.get::<String, _>("kind") != "auth" || payload.enrollment.is_some() {
            return Err(Error::Unauthenticated);
        }
        let challenge = &payload.challenge;
        Store::lock_access(&mut transaction).await?;
        let row=sqlx::query("SELECT m.owner,k.public_key,m.disabled OR u.disabled OR a.disabled OR k.revoked AS denied FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a WHERE u.directory_object IS NOT NULL AND m.id=$1 AND m.active_key=$2 FOR SHARE OF m,k,u,a").bind(challenge.machine.to_string()).bind(challenge.key.to_string()).fetch_optional(&mut *transaction).await.map_err(db)?.ok_or(Error::Unauthenticated)?;
        if row.get::<bool, _>("denied") {
            return Err(Error::Forbidden);
        }
        verify(challenge, &row.get::<Vec<u8>, _>("public_key"), &proof)?;
        let owner = row
            .get::<String, _>("owner")
            .parse()
            .map_err(|_| Error::Database)?;
        let token = self.tokens.issue(challenge.machine, owner, challenge.key)?;
        consume(&mut transaction, proof.challenge).await?;
        transaction.commit().await?;
        Ok(token)
    }
}
fn challenge(origin: &str, machine: MachineId, key: KeyId, now: u64) -> Result<Challenge, Error> {
    let nonce: [u8; 32] = rand::random();
    let challenge = Challenge {
        origin: origin.into(),
        machine,
        key,
        challenge: ChallengeId::new(),
        nonce: STANDARD.encode(nonce),
        issued_at_ms: Counter(now),
        expires_at_ms: Counter(now.checked_add(60_000).ok_or(Error::Database)?),
    };
    challenge
        .signing_bytes()
        .map_err(|_| Error::Configuration)?;
    Ok(challenge)
}
async fn lock(transaction: &mut sqlx::PgConnection, scope: &str) -> Result<(), Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(scope)
        .execute(&mut *transaction)
        .await
        .map_err(db)?;
    Ok(())
}
async fn save(
    transaction: &mut sqlx::PgConnection,
    owner: UserId,
    kind: &str,
    challenge: &Challenge,
    enrollment: Option<EnrollmentRequest>,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO qgr.machine_challenges(id,machine,key,owner,kind,payload,issued_at_ms,expires_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(challenge.challenge.to_string()).bind(challenge.machine.to_string()).bind(challenge.key.to_string()).bind(owner.to_string()).bind(kind).bind(serde_json::to_value(Payload { challenge:challenge.clone(),enrollment }).map_err(|_| Error::Invalid)?).bind(challenge.issued_at_ms.0 as i64).bind(challenge.expires_at_ms.0 as i64).execute(&mut *transaction).await.map_err(db)?;
    Ok(())
}
async fn load(
    transaction: &mut sqlx::PgConnection,
    proof: &Proof,
) -> Result<(PgRow, Payload), Error> {
    let row = sqlx::query("SELECT * FROM qgr.machine_challenges WHERE id=$1 FOR UPDATE")
        .bind(proof.challenge.to_string())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(db)?
        .ok_or(Error::Unauthenticated)?;
    let now = now_ms()?;
    if row.get::<bool, _>("consumed")
        || row.get::<i64, _>("expires_at_ms") <= now as i64
        || row.get::<i64, _>("issued_at_ms") > (now + 5000) as i64
    {
        return Err(Error::Unauthenticated);
    }
    let payload: Payload =
        serde_json::from_value(row.get("payload")).map_err(|_| Error::Database)?;
    if payload.challenge.challenge != proof.challenge
        || payload.challenge.machine.to_string() != row.get::<String, _>("machine")
        || payload.challenge.key.to_string() != row.get::<String, _>("key")
    {
        return Err(Error::Database);
    }
    Ok((row, payload))
}
fn verify(challenge: &Challenge, key: &[u8], proof: &Proof) -> Result<(), Error> {
    let key = VerifyingKey::from_bytes(key.try_into().map_err(|_| Error::Unauthenticated)?)
        .map_err(|_| Error::Unauthenticated)?;
    if proof.signature.len() != 88 {
        return Err(Error::Unauthenticated);
    }
    let signature = STANDARD
        .decode(&proof.signature)
        .map_err(|_| Error::Unauthenticated)?;
    if STANDARD.encode(&signature) != proof.signature {
        return Err(Error::Unauthenticated);
    }
    let signature = Signature::from_slice(&signature).map_err(|_| Error::Unauthenticated)?;
    key.verify_strict(
        &challenge
            .signing_bytes()
            .map_err(|_| Error::Unauthenticated)?,
        &signature,
    )
    .map_err(|_| Error::Unauthenticated)
}
async fn consume(
    transaction: &mut sqlx::PgConnection,
    challenge: ChallengeId,
) -> Result<(), Error> {
    sqlx::query("UPDATE qgr.machine_challenges SET consumed=true WHERE id=$1 AND NOT consumed")
        .bind(challenge.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(db)?;
    Ok(())
}

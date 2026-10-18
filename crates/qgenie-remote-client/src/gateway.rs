//! Injected user credentials and verified HTTPS; no OAuth, credential storage or UI.
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    rest::{
        AccessToken, AdminHistoryQuery, HistoricalChunk, HistoryItem, HistoryPage, HistoryQuery,
    },
    wire::{Machine, RecordingCodec},
    *,
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone, Debug)]
pub struct GatewayConfig {
    pub origin: String,
    pub ca_bundle: Option<PathBuf>,
}
#[derive(Clone)]
pub struct GatewayClient {
    pub(super) config: GatewayConfig,
    http: reqwest::Client,
    requests: Arc<tokio::sync::Semaphore>,
    decoders: Arc<tokio::sync::Semaphore>,
}
impl std::fmt::Debug for GatewayClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayClient([configured TLS origin])")
    }
}
fn invalid() -> RemoteError {
    RemoteError::rejected(ErrorCode::InvalidFrame, "invalid gateway response")
}
fn not_sent(code: ErrorCode, message: &str) -> RemoteError {
    RemoteError {
        code,
        outcome: OperationOutcome::NotSent,
        message: message.into(),
        retryable: false,
    }
}
fn machine_mutation_error(mut error: RemoteError) -> RemoteError {
    if error.code == ErrorCode::InvalidFrame && !matches!(error.outcome, OperationOutcome::NotSent)
    {
        error.outcome = OperationOutcome::Unknown;
        error.message =
            "machine change response invalid; reconcile owner inventory before another change"
                .into();
    }
    error
}
impl GatewayClient {
    pub async fn history_streams(
        &self,
        token: &AccessToken,
        session: SessionId,
        query: qgenie_remote_proto::rest::HistoryStreamsQuery,
    ) -> Result<qgenie_remote_proto::rest::HistoryStreamsPage, RemoteError> {
        query.validate()?;
        let page: qgenie_remote_proto::rest::HistoryStreamsPage = self
            .request(
                token,
                &format!("/v1/sessions/{session}/history/streams"),
                Some(&query),
            )
            .await?;
        validate_streams(&page, session, &query)?;
        Ok(page)
    }
    pub async fn admin_history_streams(
        &self,
        token: &AccessToken,
        session: SessionId,
        query: qgenie_remote_proto::rest::AdminHistoryStreamsQuery,
    ) -> Result<qgenie_remote_proto::rest::HistoryStreamsPage, RemoteError> {
        query.history.validate()?;
        if query.reason.trim().is_empty()
            || query.reason.len() > 512
            || (query.include_held && query.case_ref.is_none())
        {
            return Err(not_sent(
                ErrorCode::InvalidFrame,
                "admin history requires a reason and applicable case",
            ));
        }
        let page: qgenie_remote_proto::rest::HistoryStreamsPage = self
            .request(
                token,
                &format!("/v1/admin/sessions/{session}/history/streams"),
                Some(&query),
            )
            .await?;
        validate_streams(&page, session, &query.history)?;
        Ok(page)
    }
    pub fn new(config: GatewayConfig) -> Result<Self, RemoteError> {
        validate_gateway_origin(&config.origin)?;
        let url = url::Url::parse(&config.origin).map_err(|_| invalid())?;
        if url.origin().ascii_serialization() != config.origin {
            return Err(invalid());
        }
        let mut builder = reqwest::Client::builder()
            .https_only(true)
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(4);
        if let Some(path) = &config.ca_bundle {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| not_sent(ErrorCode::Internal, "configured gateway CA is unavailable"))?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid())?;
            if bytes.len() > 1024 * 1024 {
                return Err(invalid());
            }
            let certificates =
                reqwest::Certificate::from_pem_bundle(&bytes).map_err(|_| invalid())?;
            if certificates.is_empty() {
                return Err(invalid());
            }
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        Ok(Self {
            config,
            http: builder.build().map_err(|_| invalid())?,
            requests: Arc::new(tokio::sync::Semaphore::new(16)),
            decoders: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }
    pub fn origin(&self) -> &str {
        &self.config.origin
    }
    async fn request<T: DeserializeOwned>(
        &self,
        token: &AccessToken,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T, RemoteError> {
        self.request_method(
            token,
            if body.is_some() {
                reqwest::Method::POST
            } else {
                reqwest::Method::GET
            },
            path,
            body,
        )
        .await
    }
    async fn request_method<T: DeserializeOwned>(
        &self,
        token: &AccessToken,
        method: reqwest::Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T, RemoteError> {
        let _permit = self
            .requests
            .try_acquire()
            .map_err(|_| not_sent(ErrorCode::Overloaded, "gateway request capacity exceeded"))?;
        let mut header =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.as_str()))
                .map_err(|_| not_sent(ErrorCode::Unauthenticated, "invalid injected credential"))?;
        header.set_sensitive(true);
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.config.origin))
            .header(reqwest::header::AUTHORIZATION, header);
        if let Some(body) = body {
            let bytes = serde_json::to_vec(body).map_err(|_| invalid())?;
            if bytes.len() > 64 * 1024 {
                return Err(not_sent(
                    ErrorCode::InvalidFrame,
                    "gateway request body exceeds its budget",
                ));
            }
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes);
        }
        let mut response = request.send().await.map_err(|_| {
            RemoteError::unknown("gateway response unavailable; no request was retried")
        })?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_JSON_BYTES
        } else {
            64 * 1024
        };
        if response.content_length().is_some_and(|v| v > limit as u64) {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        while let Some(part) = response.chunk().await.map_err(|_| {
            RemoteError::unknown("gateway response interrupted; no request was retried")
        })? {
            if bytes.len() + part.len() > limit {
                return Err(invalid());
            }
            bytes.extend_from_slice(&part);
        }
        if status.is_success() {
            return decode_json(&bytes);
        }
        #[derive(serde::Deserialize)]
        struct Failure {
            error: RemoteError,
        }
        if let Ok(mut failure) = decode_json::<Failure>(&bytes) {
            failure.error.message = "gateway rejected the request".into();
            return Err(failure.error);
        }
        let code = match status.as_u16() {
            401 => ErrorCode::Unauthenticated,
            403 => ErrorCode::Forbidden,
            404 => ErrorCode::NotFound,
            429 => ErrorCode::Overloaded,
            _ => ErrorCode::AccessStale,
        };
        Err(RemoteError {
            code,
            outcome: if status.is_server_error() {
                OperationOutcome::Unknown
            } else {
                OperationOutcome::Rejected
            },
            message: "gateway rejected the request".into(),
            retryable: false,
        })
    }
    /// Caller injects its current token on every call; no token is stored or refreshed here.
    pub async fn machines(&self, token: &AccessToken) -> Result<Vec<Machine>, RemoteError> {
        self.request(token, "/v1/machines", None::<&()>).await
    }
    /// Change owner metadata once, including for offline machines. A lost or
    /// invalid success response remains uncertain; reconcile inventory explicitly.
    pub async fn patch_machine(
        &self,
        token: &AccessToken,
        machine: MachineId,
        patch: qgenie_remote_proto::rest::MachinePatch,
    ) -> Result<Machine, RemoteError> {
        let valid = |text: &str, minimum: usize, maximum: usize| {
            text.len() >= minimum && text.len() <= maximum && !text.chars().any(char::is_control)
        };
        if patch
            .name
            .as_ref()
            .is_some_and(|value| !valid(value, 1, 128))
            || patch.labels.as_ref().is_some_and(|labels| {
                labels.len() > 16
                    || labels
                        .iter()
                        .any(|(key, value)| !valid(key, 1, 64) || !valid(value, 0, 128))
            })
            || patch.name.is_none() && patch.labels.is_none()
        {
            return Err(not_sent(
                ErrorCode::InvalidFrame,
                "invalid machine metadata change",
            ));
        }
        let updated: Machine = self
            .request_method(
                token,
                reqwest::Method::PATCH,
                &format!("/v1/machines/{machine}"),
                Some(&patch),
            )
            .await
            .map_err(machine_mutation_error)?;
        if updated.id != machine {
            return Err(RemoteError::unknown(
                "machine change response differs; reconcile owner inventory",
            ));
        }
        Ok(updated)
    }
    /// Revocation changes remote access only. Native ownership remains on the
    /// device. Confirmation is explicit and this SDK never retries the mutation.
    pub async fn revoke_machine(
        &self,
        token: &AccessToken,
        machine: MachineId,
        confirmed: bool,
    ) -> Result<(), RemoteError> {
        if !confirmed {
            return Err(not_sent(
                ErrorCode::Forbidden,
                "machine revocation needs explicit confirmation",
            ));
        }
        let response: qgenie_remote_proto::rest::Revoked = self
            .request_method(
                token,
                reqwest::Method::POST,
                &format!("/v1/machines/{machine}/revoke"),
                Some(&qgenie_remote_proto::rest::MachineRevoke { confirmed }),
            )
            .await
            .map_err(machine_mutation_error)?;
        if response.machine != machine {
            return Err(RemoteError::unknown(
                "revocation response differs; reconcile owner inventory",
            ));
        }
        Ok(())
    }
    pub async fn history(
        &self,
        token: &AccessToken,
        session: SessionId,
        query: HistoryQuery,
    ) -> Result<HistoryPage, RemoteError> {
        query.validate()?;
        let page = self
            .request(
                token,
                &format!("/v1/sessions/{session}/history"),
                Some(&query),
            )
            .await?;
        validate_page(&page, session, &query)?;
        Ok(page)
    }
    /// Admin authority is verified by the gateway; this never falls back to another path.
    pub async fn admin_history(
        &self,
        token: &AccessToken,
        session: SessionId,
        query: AdminHistoryQuery,
    ) -> Result<HistoryPage, RemoteError> {
        query.history.validate()?;
        if query.reason.trim().is_empty()
            || query.reason.len() > 512
            || query
                .case_ref
                .as_ref()
                .is_some_and(|v| v.trim().is_empty() || v.len() > 256)
            || (query.include_held && query.case_ref.is_none())
        {
            return Err(not_sent(
                ErrorCode::InvalidFrame,
                "admin history requires a reason and applicable case",
            ));
        }
        let page = self
            .request(
                token,
                &format!("/v1/admin/sessions/{session}/history"),
                Some(&query),
            )
            .await?;
        validate_page(&page, session, &query.history)?;
        Ok(page)
    }
    /// Verify stored redacted bytes independently of the original upload receipt hash.
    /// Blocking permits remain with CPU work if the caller is canceled.
    pub async fn decode_history(
        &self,
        chunk: HistoricalChunk,
    ) -> Result<Vec<RecordingRecord>, RemoteError> {
        let permit = self.decoders.clone().try_acquire_owned().map_err(|_| {
            not_sent(
                ErrorCode::Overloaded,
                "historical decoding capacity exceeded",
            )
        })?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            decode_chunk(chunk)
        })
        .await
        .map_err(|_| invalid())?
    }
}
#[cfg(test)]
#[path = "gateway/machine_tests.rs"]
mod machine_tests;
fn validate_streams(
    page: &qgenie_remote_proto::rest::HistoryStreamsPage,
    session: SessionId,
    query: &qgenie_remote_proto::rest::HistoryStreamsQuery,
) -> Result<(), RemoteError> {
    if page.session != session
        || page.streams.len() > usize::from(query.limit)
        || page
            .streams
            .windows(2)
            .any(|pair| pair[0].stream_epoch >= pair[1].stream_epoch)
    {
        return Err(invalid());
    }
    if let Some(after) = &page.next {
        qgenie_remote_proto::rest::HistoryStreamsQuery {
            after: Some(after.clone()),
            limit: query.limit,
        }
        .validate()?;
    }
    Ok(())
}
fn validate_page(
    page: &HistoryPage,
    session: SessionId,
    query: &HistoryQuery,
) -> Result<(), RemoteError> {
    if page.session != session
        || page.stream_epoch != query.stream_epoch
        || page.items.len() > usize::from(query.limit)
    {
        return Err(invalid());
    }
    if let Some(cursor) = &page.next {
        HistoryQuery {
            after: Some(cursor.clone()),
            ..query.clone()
        }
        .validate()?;
    }
    for item in &page.items {
        match item {
            HistoryItem::Content(c)
                if c.session == session
                    && c.machine == page.machine
                    && c.stream_epoch == query.stream_epoch
                    && c.from <= c.to
                    && c.captured_from <= c.captured_to
                    && c.data.len() <= 64 * 1024 * 4 / 3 + 4
                    && c.uncompressed_bytes <= 1024 * 1024 => {}
            HistoryItem::Gap(g) if g.session == session && g.stream_epoch == query.stream_epoch => {
                g.validate()?
            }
            _ => return Err(invalid()),
        }
    }
    Ok(())
}
fn decode_chunk(chunk: HistoricalChunk) -> Result<Vec<RecordingRecord>, RemoteError> {
    if !matches!(chunk.codec, RecordingCodec::ZstdJsonl)
        || chunk.uncompressed_bytes > 1024 * 1024
        || chunk.data.len() > 64 * 1024 * 4 / 3 + 4
    {
        return Err(invalid());
    }
    let bytes = STANDARD.decode(&chunk.data).map_err(|_| invalid())?;
    if bytes.len() > 64 * 1024
        || format!("{:x}", Sha256::digest(&bytes)) != chunk.stored_digest
        || zstd::zstd_safe::find_frame_compressed_size(&bytes).map_err(|_| invalid())?
            != bytes.len()
    {
        return Err(invalid());
    }
    let raw = zstd::bulk::decompress(&bytes, 1024 * 1024).map_err(|_| invalid())?;
    if raw.len() != chunk.uncompressed_bytes as usize || raw.last() != Some(&b'\n') {
        return Err(invalid());
    }
    let mut records = Vec::new();
    let mut previous: Option<(Counter, u32, bool)> = None;
    for line in raw[..raw.len() - 1].split(|b| *b == b'\n') {
        if line.is_empty() || line.len() > 32 * 1024 || records.len() >= 4096 {
            return Err(invalid());
        }
        let record: RecordingRecord = decode_json(line)?;
        record.validate(&chunk.stream)?;
        let (fragment, last) = match &record.content {
            RecordedContent::Output { fragment, last, .. }
            | RecordedContent::AgentText { fragment, last, .. } => (*fragment, *last),
            _ => (0, true),
        };
        if let Some((seq, part, complete)) = previous
            && ((complete && (seq.0.checked_add(1) != Some(record.seq.0) || fragment != 0))
                || (!complete && (record.seq != seq || part.checked_add(1) != Some(fragment))))
        {
            return Err(invalid());
        }
        previous = Some((record.seq, fragment, last));
        if record.seq < chunk.from
            || record.seq > chunk.to
            || record.captured_at < chunk.captured_from
            || record.captured_at > chunk.captured_to
        {
            return Err(invalid());
        }
        records.push(record);
    }
    if records.first().map(|r| r.seq) != Some(chunk.from)
        || records.last().map(|r| r.seq) != Some(chunk.to)
        || records.iter().map(|r| &r.captured_at).min() != Some(&chunk.captured_from)
        || records.iter().map(|r| &r.captured_at).max() != Some(&chunk.captured_to)
    {
        return Err(invalid());
    }
    Ok(records)
}

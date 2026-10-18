use super::*;
impl Authority {
    pub(super) fn recording_challenge(
        &mut self,
        context: &RemoteContext,
    ) -> Result<DispatchFrame, RemoteError> {
        self.validate(context, Instant::now())?;
        let active = self.active.get_mut(&context.scope).ok_or_else(expired)?;
        let grant = active
            .grants
            .get_mut(&context.access_nonce)
            .ok_or_else(expired)?;
        let challenge = *grant
            .recording_challenge
            .get_or_insert_with(ChallengeId::new);
        Ok(DispatchFrame::RecordingEpochChallengeIssued { challenge })
    }
    pub(super) fn bind_recording(
        &mut self,
        context: &RemoteContext,
        authority: &qgenie_remote_proto::rest::RecordingAuthority,
    ) -> Result<DispatchFrame, RemoteError> {
        self.validate(context, Instant::now())?;
        if authority.origin != context.scope.origin
            || authority.owner != context.scope.owner
            || authority.machine != context.scope.machine
            || authority.backend_epoch.is_empty()
            || authority.backend_epoch.len() > 128
            || !authority
                .backend_epoch
                .bytes()
                .all(|b| b.is_ascii_graphic())
        {
            return Err(forbidden());
        }
        let active = self.active.get_mut(&context.scope).ok_or_else(expired)?;
        let grant = active
            .grants
            .get_mut(&context.access_nonce)
            .ok_or_else(expired)?;
        if grant.recording_challenge != Some(authority.challenge)
            || grant
                .recording_epoch
                .as_ref()
                .is_some_and(|epoch| epoch != &authority.backend_epoch)
            || active.recording_bound.is_some_and(|(issued, nonce)| {
                issued >= grant.issued && nonce != context.access_nonce
            })
        {
            return Err(expired());
        }
        grant.recording_epoch = Some(authority.backend_epoch.clone());
        active.recording_bound = Some((grant.issued, context.access_nonce));
        Ok(DispatchFrame::RecordingEpochBound {
            challenge: authority.challenge,
        })
    }
    pub(in super::super) fn recording_epoch(
        &self,
        context: &RemoteContext,
        challenge: ChallengeId,
    ) -> Result<String, RemoteError> {
        self.validate(context, Instant::now())?;
        let active = self.active.get(&context.scope).ok_or_else(expired)?;
        let grant = active
            .grants
            .get(&context.access_nonce)
            .ok_or_else(expired)?;
        if grant.recording_challenge != Some(challenge)
            || active.recording_bound != Some((grant.issued, context.access_nonce))
        {
            return Err(expired());
        }
        grant.recording_epoch.clone().ok_or_else(expired)
    }
}

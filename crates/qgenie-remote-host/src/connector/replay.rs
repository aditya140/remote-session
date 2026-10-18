//! Capture delivery does not retain the connector's control/renewal loop.
use super::*;
use qgenie_remote_transport::ws::Sender;

pub(super) async fn attachment(
    sender: Sender,
    native: crate::NativeClient,
    access: crate::NativeAccess,
    context: HostContext,
    request: Envelope,
) -> Result<(), Error> {
    let result = tokio::time::timeout(access.remaining()?, async {
        let bundle = native
            .attachment(&access, context.client, request.clone())
            .await?;
        for reply in bundle.frames() {
            if !access.is_fresh() || context.expires_at.unix_millis() <= unix_ms() {
                return Err(Error::Expired);
            }
            transmit(&sender, reply.clone(), Priority::Bulk).await?;
        }
        Ok(())
    })
    .await
    .map_err(|_| Error::Unavailable)
    .and_then(|value| value);
    if let Err(error) = result {
        transmit(
            &sender,
            failure(&request, native_error(error)),
            Priority::Control,
        )
        .await?;
    }
    Ok(())
}

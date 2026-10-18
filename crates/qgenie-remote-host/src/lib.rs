//! Outbound host identity and policy bridge. No native process ownership.
mod credentials;
mod enrollment;
pub use enrollment::{
    GatewayApi, MachineAuthentication, MachineMetadata, RECORDING_DISCLOSURE, RecordingConsent,
};
mod error;
mod http;
pub use credentials::{
    Binding, BindingStatus, CredentialOptions, Identity, MachineCredentials, Protection,
};
mod connector;
mod native;
mod recording;
pub use native::{
    NativeAccess, NativeAttachment, NativeChallenge, NativeChanges, NativeClient,
    NativeControlProposal, NativeControlState, NativeControllerLease,
};
pub use recording::UploadProgress;
pub mod filesystem;
pub use connector::{ConnectionStatus, Connector};
pub mod oauth;
pub use error::Error;

//! als-api — typed HTTP client for the als-api server.
//!
//! Hosts the `Client` type plus the JSON envelope and `ApiError` taxonomy
//! shared by every endpoint module. Endpoint files (`deploy.rs`,
//! `sites.rs`, …) live under `endpoints/`; this crate intentionally keeps
//! the transport layer separated from the resource layer.

pub mod client;
pub mod endpoints;
pub mod envelope;
pub mod error;

pub use client::Client;
pub use endpoints::account::{AccountMe, me as account_me};
pub use endpoints::deploy::{DeployRequest, DeployResponse, deploy};
pub use endpoints::device::{DeviceAuthorization, DeviceTokenResponse};
pub use endpoints::password::SetPasswordResponse;
pub use endpoints::quota::{Quota, me as quota_me};
pub use endpoints::sites::{
    ListSitesQuery, ListSitesResponse, PatchSiteRequest, Site, SiteState, delete, get, list, patch,
};
pub use endpoints::versions::Version;
pub use endpoints::{account, device, password, quota, tokens, versions};
pub use envelope::PaginationMeta;
pub use error::ApiError;

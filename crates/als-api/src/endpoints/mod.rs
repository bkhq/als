//! Resource-layer endpoint modules.
//!
//! Each file wraps a small group of `/api/*` URLs behind typed
//! request/response structs and free `async` functions that take
//! `&Client`. Keeping endpoints in their own files (rather than methods
//! on `Client`) keeps the transport layer small and lets each task own
//! its module without merge churn.

pub mod account;
pub mod deploy;
pub mod device;
pub mod password;
pub mod quota;
pub mod sites;
pub mod tokens;
pub mod versions;

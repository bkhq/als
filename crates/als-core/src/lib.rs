//! als CLI core types: brand URLs, config, duration parser, error
//! enum + exit-code mapping, output sink, and the per-site pin store.

pub mod brand;
pub mod config;
pub mod duration;
pub mod error;
pub mod output;
pub mod sites;

pub use brand::{DEFAULT_API_URL, HOMEPAGE_URL, LLMS_TXT_URL};
pub use config::{Config, mask_token};
pub use duration::{Expires, parse as parse_duration};
pub use error::{Error, ExitCode};
pub use output::{Output, OutputMode};

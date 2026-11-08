//! Provider adapter (ADR-0002 D1–D3 & D5, ADR-0004 section 4.2).
//!
//! Four responsibilities, all pure so the webserver, controller and worker
//! share one code path:
//!
//! - [`template`]: fill a provider request template from client fields,
//!   credentials and constants (D1, D2).
//! - [`respons`]: read a provider response, delegating the `status_skor`
//!   rules to the pilot parser in `poolrouter_core` (D3, ARC-46).
//! - [`error`]: map a failed call onto the shared `retry.rs` table (ARC-4).
//! - [`client`]: build HTTP client with TLS trust per provider (D5).
//! - [`secret`]: resolve a provider secret from `provider_secret` (ARC-48, AC-23).

pub mod client;
pub mod error;
pub mod respons;
pub mod secret;
pub mod template;

pub use client::{
    Allowlist, BATAS_CONNECT, BATAS_TOTAL, GalatKlien, TlsTrust, build_provider_client,
};
pub use error::{GagalPanggilProvider, dari_status_http, kelas_error};
pub use respons::{BacaResponsError, HasilBaca, baca_respons};
pub use secret::{GalatSecret, resolusi_secret};
pub use template::{Nilai, RenderError, TemplateRequest, render_template};

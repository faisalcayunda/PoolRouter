//! Provider adapter (ADR-0002 D1–D3, ADR-0004 section 4.2).
//!
//! Three responsibilities, all pure so the webserver, controller and worker
//! share one code path:
//!
//! - [`template`]: fill a provider request template from client fields,
//!   credentials and constants (D1, D2).
//! - [`respons`]: read a provider response, delegating the `status_skor`
//!   rules to the pilot parser in `poolrouter_core` (D3, ARC-46).
//! - [`error`]: map a failed call onto the shared `retry.rs` table (ARC-4).
//! - [`client`]: build and configure HTTP clients per provider trust mode (ADR-0004 section 4.2).
//! - [`dokumen`]: dokumen konfigurasi provider dan render template ganda
//!   kabel/log (dipakai worker dan jalur sync, ARC-66).
//! - [`hasil`]: respons mentah → `status_akses`/`error_class`/output (ARC-66).

pub mod client;
pub mod dokumen;
pub mod error;
pub mod hasil;
pub mod respons;
pub mod template;

pub use error::{GagalPanggilProvider, dari_status_http, kelas_error};
pub use respons::{BacaResponsError, HasilBaca, baca_respons};
pub use template::{Nilai, RenderError, TemplateRequest, render_template};

//! Writes the OpenAPI documents for `backend/openapi/`. Run via
//! `make -C backend openapi` (no waiting for a resize in any gate).

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use poolrouter_webserver::api::{admin, downstream};

/// Backend (workspace) directory: the nearest ancestor that holds both
/// `Cargo.toml` and `crates/`. Works from `cargo run` inside the container
/// (`/work/crates/webserver`) and natively, with the executable location as
/// fallback.
fn backend_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
        let mut path = PathBuf::from(dir);
        loop {
            if path.join("crates").is_dir() && path.join("Cargo.toml").is_file() {
                return path;
            }
            if !path.pop() {
                break;
            }
        }
    }
    let exe = std::env::current_exe().expect("current executable path");
    exe.ancestors()
        .skip(1)
        .find(|anc| anc.join("Cargo.toml").is_file())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() -> ExitCode {
    let dir = backend_dir().join("openapi");
    let result = (|| -> Result<(String, String), String> {
        Ok((
            downstream::yaml().map_err(|e| e.to_string())?,
            admin::yaml().map_err(|e| e.to_string())?,
        ))
    })()
    .and_then(|(downstream_yaml, admin_yaml)| {
        fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                fs::write(dir.join("downstream-api.yaml"), downstream_yaml)
                    .map_err(|e| e.to_string())
            })
            .and_then(|()| {
                fs::write(dir.join("admin-api.yaml"), admin_yaml).map_err(|e| e.to_string())
            })
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("openapi-gen: {e}");
            ExitCode::FAILURE
        }
    }
}

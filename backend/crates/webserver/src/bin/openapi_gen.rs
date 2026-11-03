//! Writes the OpenAPI documents from code. `make -C backend openapi` runs it;
//! `openapi-check` regenerates and fails on `git diff` (ADR-0003 section 4.2).
//!
//! Usage: `openapi-gen <output-dir>`

use std::{fs, path::PathBuf, process::ExitCode};

use poolrouter_webserver::api::downstream;

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: openapi-gen <output-dir>");
        return ExitCode::from(2);
    };
    let result = downstream::yaml()
        .map_err(|e| e.to_string())
        .and_then(|y| fs::write(dir.join("downstream-api.yaml"), y).map_err(|e| e.to_string()));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("openapi-gen: {e}");
            ExitCode::FAILURE
        }
    }
}

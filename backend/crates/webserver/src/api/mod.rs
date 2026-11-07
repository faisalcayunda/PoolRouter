//! API modules: shared enums, problem+json, downstream and admin contracts
//! (SHAA-175), plus the generation tests.

pub mod admin;
pub mod downstream;
pub mod downstream_sesi;
pub mod enums;
pub mod header;
pub mod problem;
pub mod problem_admin;
#[cfg(test)]
mod tests;

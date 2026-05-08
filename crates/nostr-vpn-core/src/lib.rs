pub mod config;
mod config_defaults;
mod config_magic_dns;
pub mod control;
pub mod data_plane;
pub mod diagnostics;
pub mod fips_control;
pub mod fips_mesh;
pub mod join_requests;
pub mod magic_dns;
pub mod nat;
mod network_roster;
mod network_routes;
pub mod paths;
pub mod platform_paths;

pub use config::DEFAULT_RELAYS;

//! src/routes/admin/mod.rs
mod dashboard;
mod password;
pub mod logout;

pub use dashboard::admin_dashboard;
pub use password::*;
pub use logout::log_out;
//! src/routes/admin/mod.rs
mod dashboard;
mod newsletters;
mod password;
pub mod logout;

pub use dashboard::admin_dashboard;
pub use newsletters::*;
pub use password::*;
pub use logout::log_out;
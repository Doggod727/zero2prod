//! src/routes/admin/mod.rs
mod dashboard;
mod newsletters;
mod password;
mod logout;

mod subscribers;
pub use dashboard::admin_dashboard;
pub use newsletters::*;
pub use password::*;
pub use logout::log_out;
pub use subscribers::*;
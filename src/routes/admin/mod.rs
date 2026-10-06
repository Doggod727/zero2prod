//! src/routes/admin/mod.rs
mod dashboard;
mod logout;
mod newsletters;
mod password;

mod subscribers;
pub use dashboard::admin_dashboard;
pub use logout::log_out;
pub use newsletters::*;
pub use password::*;
pub use subscribers::*;

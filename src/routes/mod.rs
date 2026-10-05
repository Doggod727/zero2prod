//! src/routes/mod.rs
mod health_check;
mod home;
mod login;
mod subscriptions;
mod admin;

pub use health_check::*;
pub use home::*;
pub use login::*;
pub use subscriptions::*;
pub use admin::*;
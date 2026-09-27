//! src/routes/mod.rs
mod health_check;
mod subscriptions;
mod subscriptions_confirm;
mod newsletters;
mod login;
mod home;
pub use health_check::*;
pub use subscriptions::*;
pub use subscriptions_confirm::*;
pub use newsletters::*;
pub use home::*;
pub use login::*;
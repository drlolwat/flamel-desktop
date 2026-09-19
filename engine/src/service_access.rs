//! Optional user-authorized hosted integrations, supplied by the native credential host.
use std::{future::Future, pin::Pin};
pub trait ServiceAccess: Send + Sync {
    fn forge_assertion(&self) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + '_>>;
}

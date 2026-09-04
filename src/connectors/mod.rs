pub mod telegram;

use async_trait::async_trait;

use crate::error::AlfredError;

#[async_trait]
pub trait Connector: Send + Sync {
    async fn start(&self) -> Result<(), AlfredError>;
}

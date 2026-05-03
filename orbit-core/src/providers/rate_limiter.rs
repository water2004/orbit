//! RateLimiter — 基于 tokio Semaphore 的 API 并发控制。

use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct RateLimiter {
    semaphore: Arc<Semaphore>,
}

impl RateLimiter {
    pub fn new(max_concurrency: usize) -> Self {
        Self { semaphore: Arc::new(Semaphore::new(max_concurrency)) }
    }

    /// 获取并发槽位。Semaphore 关闭时返回错误（正常运行时不会发生）。
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, crate::error::OrbitError> {
        self.semaphore.clone().acquire_owned().await
            .map_err(|_| crate::error::OrbitError::Other(
                anyhow::anyhow!("RateLimiter semaphore unexpectedly closed")
            ))
    }
}

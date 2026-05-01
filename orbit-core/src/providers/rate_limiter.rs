//! RateLimiter — 基于 tokio Semaphore 的 API 并发控制。
//!
//! 每个 Provider 实例持有自己的 RateLimiter，
//! 调用方并发 spawn 任务时，Semaphore 自动排队串行化，对外完全透明。

use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct RateLimiter {
    semaphore: Arc<Semaphore>,
}

impl RateLimiter {
    pub fn new(max_concurrency: usize) -> Self {
        Self { semaphore: Arc::new(Semaphore::new(max_concurrency)) }
    }

    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.semaphore.clone().acquire_owned().await
            .expect("RateLimiter semaphore closed")
    }
}

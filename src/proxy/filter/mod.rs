use crate::{errors::Error, model::TrafficRecord};
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod response;

// 提供静态的 FilterChain 实例，支持多线程下使用
pub static ONCE_FILTER_CHAIN: Lazy<Arc<Mutex<FilterChain>>> = Lazy::new(|| {
    let mut chain = FilterChain::new();
    chain.add_filter(Box::new(response::ResponseFilter::new(None)));
    Arc::new(Mutex::new(chain))
});

/// 过滤器接口
pub trait Filter: Send + Sync {
    fn filter(&self, record: &TrafficRecord) -> Result<TrafficRecord, Error>;
}

/// 过滤器链
pub struct FilterChain {
    filters: Vec<Box<dyn Filter>>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self { filters: Vec::new() }
    }

    pub fn add_filter(&mut self, filter: Box<dyn Filter>) {
        self.filters.push(filter);
    }

    pub fn filter(&self, record: &TrafficRecord) -> Result<TrafficRecord, Error> {
        let mut filtered = record.clone();
        for filter in &self.filters {
            filtered = filter.filter(&filtered)?;
        }
        Ok(filtered)
    }
}

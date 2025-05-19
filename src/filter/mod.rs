use crate::{errors::Error, model::TrafficRecord};

pub mod response_filter;

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

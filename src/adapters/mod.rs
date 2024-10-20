use std::{future::Future, pin::Pin, time::Duration};

use bytes::Bytes;
pub mod twitter;
pub mod bluesky;

pub type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a + Send>>;
pub const USER_AGENT: &'static str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.1 Safari/605.1.15";
pub const FIVE_SECOUND: Duration = Duration::from_secs(5);
pub const BEARER: &'static str = "Bearer ";

pub trait Adapters: Send + Sync {
  fn platform(&self) -> &'static str;
  fn path(&self) -> &str;
  fn name(&self) -> &str;
  fn next(&mut self) -> BoxedFuture<'_, Option<Box<dyn Item>>>;
}

pub trait Item: Send + Sync {
  fn filename(&self) -> &str;
  fn url(&self) -> &str;
  fn media_url(&self) -> &str;
  fn get(&self) -> BoxedFuture<'_, Bytes>;
}

#[macro_export]
macro_rules! insert {
  ($h:expr, $($k:literal, $v:expr),*) => {
    $($h.insert($k, HeaderValue::from_str(($v).as_str()).unwrap());)*
  }
}
use std::{collections::HashMap, error::Error, fmt::Debug};
use url::Url;

pub enum Method {
  GET,
  HEAD,
  OPTIONS,
  TRACE,
  PUT,
  DELETE,
  POST,
  PATCH,
  CONNECT,
}

pub struct Request<U, P>
where
  U: TryInto<Url>,
  P: TryInto<Url>,
  U::Error: Error,
  U::Error: Error,
{
  method: Method,
  url: U,
  headers: HashMap<String, String>,
  proxy: Option<P>,
  body: Vec<u8>,
}

pub struct Response {
  request: Request<Url, Url>,
  headers: HashMap<String, String>,
  body: Vec<u8>,

}

impl<U, P> Request<U, P>
where
  U: TryInto<Url>,
  P: TryInto<Url>,
  U::Error: Error,
  P::Error: Error,
{
  fn url<V>(self, url: V) -> Request<V, P>
  where
    V: TryInto<Url>,
    V::Error: Error,
  {
    Request { url, ..self }
  }

  fn header<K, V>(mut self, key: K, value: V) -> Self
  where
    K: ToString,
    V: ToString,
  {
    self.headers.insert(key.to_string(), value.to_string());
    self
  }

  fn headers<K, V, I, H>(mut self, headers: H) -> Self
  where
    K: ToString,
    V: ToString,
    I: Iterator<Item = (K, V)>,
    H: Into<I>,
  {
    let iter: I = headers.into();
    self.headers.extend(iter.map(|(k, v)| (k.to_string(), v.to_string())));
    self
  }

  fn proxy<Q>(self, p: Q) -> Request<U, Q>
  where
    Q: TryInto<Url>,
    Q::Error: Error,
  {
    Request { proxy: Some(p), ..self }
  }

  fn body(mut self, body: impl Into<Vec<u8>>) {
    self.body = body.into();
  }

  async fn send(self) -> Result<Response, Box<dyn Error>> {
    let url: Url = self.url.try_into()?;
    let proxy: Option<Url> = match self.proxy.map(TryInto::try_into) {
      Some(Ok(url)) => Some(url),
      Some(Err(err)) => return Err(err.into()),
      _ => None,
    };

    Ok(Response {
      request: Request {
        url
      },
    })
  }
}

pub fn get<U>(url: U) -> Request<U, Url>
where
  U: TryInto<Url>,
  U::Error: Error,
{
  Request {
    method: Method::GET,
    url,
    headers: HashMap::new(),
    proxy: None,
    body: Vec::with_capacity(0),
  }
}
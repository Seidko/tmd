use std::{collections::HashMap, error::Error, fmt::Debug, os::unix::net::SocketAddr};
use async_native_tls::TlsConnector;
use smol::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream};
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
  stream

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

    let host = url.host().ok_or("No host field in url.")?;
    let port = url.host().ok_or("No port field in url.")?;

    let stream = match proxy {
      None => TcpStream::connect(SocketAddr::from((host, port))),
      Some(p) if p.scheme() == "http" => async {
        let stream = TcpStream::connect(SocketAddr::from((
          p.host().ok_or("No host field in proxy url.")?,
          p.port().ok_or("No port field in proxy url.")?,
        ))).await?;
        let mut buf = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n").into_bytes();
        buf.extend_from_slice(b"\r\n");

        stream.write_all(&buf).await?;

        let mut buf = [0u8; 32768];
        stream.read(&buf);
      },
      _ => Err("Unsupport proxy scheme.")
    }.await?;

    match 

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
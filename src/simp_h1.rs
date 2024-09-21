use std::{any::Any, collections::HashMap, error::Error, fmt::{Debug, Display}, net::SocketAddr};
use async_native_tls::TlsConnector;
use smol::{io::{AsyncReadExt, AsyncWriteExt}, net::{AsyncToSocketAddrs, TcpStream}, stream::StreamExt};
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

impl Display for Method {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      f.write_str(
        match self {
          Self::GET => "GET",
          Self::HEAD => "HEAD",
          Self::OPTIONS => "OPTIONS",
          Self::TRACE => "TRACE",
          Self::PUT => "PUT",
          Self::DELETE => "DELETE",
          Self::POST => "POST",
          Self::PATCH => "PATCH",
          Self::CONNECT => "CONNECT",
        }
      )
  }
}

pub struct Request<U, P>
where
  U: TryInto<Url>,
  P: TryInto<Url>,
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
  // stream: 
}

#[derive(Debug)]
pub enum HttpError {
  UrlError(url::ParseError),
  IOError(smol::io::Error),
  TunnelError(String),
  UnknownError(Box<dyn Any>),
}

impl From<Box<dyn Any>> for HttpError {
  fn from(value: Box<dyn Any>) -> Self {
    let value = value.downcast::<url::ParseError>();
    if let Ok(err) = value {
      return HttpError::UrlError(*err);
    };
    HttpError::UnknownError(value.unwrap_err())
  } 
}

impl From<url::ParseError> for HttpError {
  fn from(value: url::ParseError) -> Self {
    HttpError::UrlError(value)
  }
}

impl From<smol::io::Error> for HttpError {
  fn from(value: smol::io::Error) -> Self {
    HttpError::IOError(value)
  }
}

impl From<&str> for  HttpError {
  fn from(value: &str) -> Self {
    HttpError::TunnelError(value.to_owned())
  }
}

async fn tunnel(proxy: Option<Url>, host: &str, port: u16) -> Result<TcpStream, HttpError> {
  let tun = match proxy {
    None => TcpStream::connect((host, port)).await?,
    Some(p) if p.scheme() == "http" => {
      let mut stream = TcpStream::connect((
        p.host_str().ok_or("No host field in proxy url.")?,
        p.port().ok_or("No port field in proxy url.")?,
      )).await?;
      let mut buf = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n").into_bytes();
      buf.extend_from_slice(b"\r\n");

      stream.write_all(&buf).await?;

      let mut buf = Vec::with_capacity(8192);

      let mut result = None;
      loop {
        stream.read(buf)
      }
      while let Some(byte) = iter.next().await {
        let byte = byte?;
        buf.push(byte);
        if buf.len() >= 12 {
          if [b"HTTP/1.0 200", b"HTTP/1.1 200"].contains(&buf.first_chunk().unwrap()) {
            if buf.ends_with(b"\r\n\r\n") {
              return Ok(iter);
            }
          } else if buf.starts_with(b"HTTP/1.1 407") {
            result = Some(Err("proxy authentication required"));
          } else {
            result = Some(Err("unsuccessful tunnel"));
          }
        }
      };
      result.unwrap_or(Err("tunnel EOF."))?
    },
    _ => Err("Unsupport proxy scheme.")?
  };
  Ok(tun)
}

impl Display for HttpError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::UrlError(e) => write!(f, "{}", e),
      Self::IOError(e) => write!(f, "{}", e),
      Self::TunnelError(e) => write!(f, "{}", e),
      Self::UnknownError(_) => write!(f, "Unknown Error"),
    }
  }
}

impl Error for HttpError {}

impl<U, P> Request<U, P>
where
  U: TryInto<Url>,
  P: TryInto<Url>,
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

  async fn send(self) -> Result<Response, HttpError> {
    let url: Url = self.url.try_into().map_err(|e| Box::new(e) as Box<dyn Any>)?;
    let proxy: Option<Url> = match self.proxy.map(TryInto::try_into) {
      Some(Ok(url)) => Some(url),
      Some(Err(e)) => Err(Box::new(e) as Box<dyn Any>)?,
      _ => None,
    };

    let host = url.host_str().ok_or("No host field in url.")?;
    let port = url.port().ok_or("No port field in url.")?;

    let tunnel: TcpStream = tunnel(proxy, host, port).await?;

    let mut buf = b""

    Ok(Response {
      request: Request {
        url,
        proxy,
        ..self
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
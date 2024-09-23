use std::any::Any;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::pin::Pin;
use async_native_tls::{TlsConnector, TlsStream};
use smol::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use smol::net::TcpStream;
use url::Url;

use crate::subseq::SubSequence;

const PROTOCOL: [&[u8]; 2] = [b"HTTP/1.0 200", b"HTTP/1.1 200"];

trait AsyncRW: AsyncRead + AsyncWrite {}

impl AsyncRW for TcpStream {}
impl AsyncRW for TlsStream<TcpStream> {}

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
  code: u16,
  stream: Pin<Box<dyn AsyncRW>>
}

#[derive(Debug)]
pub enum HttpError {
  UrlError(url::ParseError),
  IOError(smol::io::Error),
  TlsError(async_native_tls::Error),
  GeneralError(String),
  UnknownError(Box<dyn Any>),
}

impl From<Box<dyn Any>> for HttpError {
  fn from(value: Box<dyn Any>) -> Self {
    use HttpError::*;
    let value = value.downcast::<url::ParseError>();
    if let Ok(err) = value {
      return UrlError(*err);
    };
    let value = value.unwrap_err().downcast::<&str>();
    if let Ok(err) = value {
      return GeneralError(err.to_string());
    }
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

impl From<async_native_tls::Error> for HttpError {
  fn from(value: async_native_tls::Error) -> Self {
    HttpError::TlsError(value)
  }
}

impl From<&str> for HttpError {
  fn from(value: &str) -> Self {
    HttpError::GeneralError(value.to_owned())
  }
}

async fn tunnel(proxy: &Option<Url>, host: &str, port: u16) -> Result<TcpStream, HttpError> {
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

      let mut rec = Vec::with_capacity(8192);
      let mut buf = [0u8; 1460];

      let tun: Result<TcpStream, HttpError> = loop {
        let size = stream.read(&mut buf).await?;

        if size == 0 {
          break Err("tunnel EOF.".into());
        }

        rec.extend_from_slice(&buf[..size]);

        if let Some(end) = rec.first_chunk::<12>() && PROTOCOL.contains(&end.as_slice()) {
          if rec.ends_with(b"\r\n\r\n") {
            break Ok(stream);
          }
        } else if buf.starts_with(b"HTTP/1.1 407") {
          break Err("proxy authentication required".into());
        } else {
          break Err("unsuccessful tunnel".into());
        }
      };
      tun?
    },
    _ => Err("Unsupport proxy scheme.")?
  };
  Ok(tun)
}

impl Display for HttpError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    use HttpError::*;
    match self {
      UrlError(e) => write!(f, "{}", e),
      IOError(e) => write!(f, "{}", e),
      TlsError(e) => write!(f, "{}", e),
      GeneralError(e) => write!(f, "{}", e),
      UnknownError(_) => write!(f, "Unknown Error"),
    }
  }
}

impl Error for HttpError {}

impl<U, P> Request<U, P>
where
  U: TryInto<Url>,
  P: TryInto<Url>,
  U::Error: Error + 'static,
  P::Error: Error + 'static,
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

  async fn send<T>(self) -> Result<Response, HttpError> {
    let url: Url = self.url.try_into().map_err(|e| Box::new(e) as Box<dyn Any>)?;
    let proxy: Option<Url> = match self.proxy.map(TryInto::try_into) {
      Some(Ok(url)) => Some(url),
      Some(Err(e)) => Err(Box::new(e) as Box<dyn Any>)?,
      _ => None,
    };

    let host = url.host_str().ok_or("No host field in url.")?;
    let port = url.port().ok_or("No port field in url.")?;

    let mut stream: Pin<Box<dyn AsyncRW>> = {
      let stream: TcpStream = tunnel(&proxy, host, port).await?;

      if url.scheme() == "https" {
        let conn = TlsConnector::new();
        Box::pin(conn.connect(host, stream).await?)
      } else {
        Box::pin(stream)
      }
    };

    let mut buf = format!(
      "{} {}{} HTTP/1.1\r\n{}",
      self.method,
      url.path(),
      url.query().map(|s| "?".to_owned() + s).unwrap_or(String::new()),
      self.headers.iter().map(|(k, v)| k.to_owned() + ":" + &v + "\r\n").collect::<String>(),
    ).into_bytes();

    use Method::*;
    match self.method {
      GET | HEAD | OPTIONS | DELETE | TRACE | CONNECT => {
        buf.extend_from_slice(b"\r\n");
        stream.write(&buf).await?;
      }
      _ => unimplemented!("Unsupport Method {}", self.method),
    }

    let mut rec = Vec::with_capacity(8192);
    let mut buf = [0u8; 1460];

    let pos = loop {
      let size = stream.read(&mut buf).await?;
      if size == 0 {
        return Err("connection receive EOF.".into());
      };
      
      rec.extend_from_slice(&buf[..size]);
      if let Some(pos) = rec.find_slice(b"\r\n\r\n") {
        break pos;
      }
    };

    let header = String::from_utf8_lossy(&rec[..pos]);
    let mut header_lines = header.split("\r\n");
    let mut first_line = header_lines.next().ok_or("Malform HTTP response header")?.split(" ");

    macro_rules! lnext {
      ($v:expr) => {
        $v.next().ok_or("Malform HTTP response header")?
      };
    }


    let ptc = lnext!(first_line);
    let code = lnext!(first_line).parse().map_err(|_| "Malform HTTP status code")?;

    if !PROTOCOL.contains(&ptc.as_bytes()) {
      Err("Unsupported protocol or protocol version.")?;
    }
    
    let mut headers = HashMap::new();
    for line in header_lines {
      let mut split = line.split(":");
      let k = lnext!(split).to_lowercase();
      let v = lnext!(split).to_lowercase();
      headers.insert(k, v);
    }

    let body = if let Some(len) = headers.get("content-length") {
      let mut rest = Vec::from(&rec[pos + 4..]);
      let len = len.parse::<usize>().map_err(|_| "`content-length` field is not number.")? - rest.len();
      let mut buf = vec![0u8; len];
      stream.read_exact(&mut buf);
      rest.append(&mut buf);
      rest
    } else {
      return Err("cannot recogize body length head.".into());
    };

    Ok(Response {
      request: Request {
        url,
        proxy,
        ..self
      },
      headers,
      stream,
      body,
      code,
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
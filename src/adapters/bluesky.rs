use std::{collections::LinkedList, sync::{Arc, OnceLock}};

use reqwest::{Client, Proxy, StatusCode};
use serde::Deserialize;
use serde_json::{from_value, json, Value};
use tokio::{sync::Semaphore, time::sleep};

use super::{Adapters, BoxedFuture, Item, BEARER, USER_AGENT, FIVE_SECOUND};

#[derive(Deserialize)]
struct BlueSkyConfig {
  account: String,
  pass: String,
  page_size: Option<i32>,
  concurrency: Option<usize>,
}

#[derive(Deserialize)]
struct GetActorLikes {
  cursor: String,
  feed: Vec<Value>,
}

pub struct BlueSkyAdapter {
  account: String,
  pass: String,
  auth: OnceLock<Auth>,
  cache: LinkedList<BlueSkyItem>,
  cursor: Option<String>,
  page_size: i32,
  client: Client,
  sem: Arc<Semaphore>,
}

#[derive(Debug)]
struct Auth {
  token: String,
  did: String,
  endpoint: String,
}

pub struct BlueSkyItem {
  pub url: String,
  pub media_url: String,
  pub client: Client,
  pub filename: String,
  sem: Arc<Semaphore>,
}

impl BlueSkyAdapter {
  pub fn new(config: Value, proxy: Option<Proxy>) -> Self {
    let config: BlueSkyConfig = from_value(config).unwrap();

    let mut builder = Client::builder()
    .user_agent(USER_AGENT)
    .gzip(true);

    if let Some(proxy) = proxy.clone() {
      builder = builder.proxy(proxy);
    }

    let client = builder.build().unwrap();

    Self {
      page_size: config.page_size.unwrap_or(50),
      account: config.account,
      pass: config.pass,
      auth: OnceLock::new(),
      cache: LinkedList::new(),
      sem: Arc::new(Semaphore::new(config.concurrency.unwrap_or(50))),
      cursor: None,
      client,
    }
  }
}

impl Adapters for BlueSkyAdapter {
  fn platform(&self) -> &'static str {
    "bluesky"
  }

  fn count(&self) -> Option<BoxedFuture<'_, u64>> {
    None
  }

  #[inline(never)]
  fn next(&mut self) -> BoxedFuture<'_, Option<Box<dyn Item>>> {
    Box::pin(async {
      if let Some(item) = self.cache.pop_front() {
        return Some(Box::new(item) as Box<dyn Item>);
      }

      let Auth { token, did, endpoint} = if let Some(auth) = self.auth.get() {
        auth
      } else {
        loop {
          match async {
            self.client.post("https://bsky.social/xrpc/com.atproto.server.createSession")
              .body(json!({
                "identifier": self.account,
                "password": self.pass,
              }).to_string())
              .header("content-type", "application/json")
              .send().await?.json::<Value>().await
          }.await {
            Ok(json) if json.get("error").is_some() => {
              panic!("{} {}", json["error"].as_str().unwrap(), json["message"].as_str().unwrap());
            }
            Ok(json) => {
              break self.auth.get_or_init(|| Auth {
                did: json["did"].as_str().unwrap().to_owned(),
                token: json["accessJwt"].as_str().unwrap().to_owned(),
                endpoint: json["didDoc"]["service"][0]["serviceEndpoint"].as_str().unwrap().to_string(),
              });
            },
            Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
              println!("Warning: too many request, sleep 5 secs and retrying...");
              sleep(FIVE_SECOUND).await;
            }
            Err(err) => {
              println!("Unknown request error {:?}, retrying...", err);
            }
          }
        }
      };
      
      let mut query = vec![
        ("actor", did.clone()),
        ("limit", self.page_size.to_string()),
      ];

      if let Some(cursor) = &self.cursor {
        query.push(("cursor", cursor.clone()));
      }

      let mut likes = None;
      for _ in 0..5 {
        let json = loop {
          match async {
            self.client.get(endpoint.to_owned() + "/xrpc/app.bsky.feed.getActorLikes")
              .header("authorization", BEARER.to_owned() + token)
              .header("content-type", "application/json")
              .query(&query)
              .send().await?.error_for_status()?.json::<GetActorLikes>().await
          }.await {
            Ok(json) => break json,
            Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
              println!("Warning: too many request, sleep 5 secs and retrying...");
              sleep(FIVE_SECOUND).await;
            }
            Err(err) => {
              println!("Unknown request error {:?}, retrying...", err);
            }
          }
        };
        if !json.feed.is_empty() {
          likes = Some(json);
          break;
        } else {
          query.pop();
          query.push(("cursor", json.cursor));
        }
      };
      if likes.is_none() {
        return None;
      }
      let likes = likes.unwrap();
      
      for post in likes.feed {
        let author = post["post"]["author"]["handle"].as_str().unwrap().to_owned();
        let id = post["post"]["uri"].as_str().unwrap().split("/").last().unwrap().to_owned();
        if let Some(embed) = post["post"].get("embed") {
          for (index, image) in embed["images"].as_array().unwrap().iter().enumerate() {
            let index = index + 1;
            let url = image["fullsize"].as_str().unwrap().to_owned();
            self.cache.push_back(BlueSkyItem {
              url: format!("https://bsky.app/profile/{author}/post/{id}"),
              media_url: url.replace("@jpeg", "@png"),
              client: self.client.clone(),
              filename: format!("{author} {id} {index}.png"),
              sem: self.sem.clone(),
            });
          }
        }
      }

      self.cursor = Some(likes.cursor);
      self.cache.pop_front().map(|v| Box::new(v) as Box<dyn Item>)
    })
  }
}

impl Item for BlueSkyItem {
  fn is_last(&self) -> Option<bool> {
    None
  }
  
  fn filename(&self) -> &str {
    &self.filename
  }
  
  fn url(&self) -> &str {
    &self.url
  }
  
  fn media_url(&self) -> &str {
    &self.media_url
  }
  
  fn get(&self) -> BoxedFuture<'_, bytes::Bytes> {
    Box::pin(async {
      let _guard = self.sem.acquire().await.unwrap();
      loop {
        match self.client.get(&self.media_url).send().await.and_then(|r| r.error_for_status()){
          Ok(res) => return res.bytes().await.unwrap(),
          Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
            println!("Warning: too many request, sleep 5 secs and retrying...");
            sleep(FIVE_SECOUND).await;
          }
          Err(err) => {
            println!("Unknown request error {:?}, retrying...", err);
          }
        }
      };
    })
  }
}
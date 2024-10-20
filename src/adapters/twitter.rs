use std::collections::LinkedList;
use std::path::Path;
use std::sync::{Arc, LazyLock, OnceLock};
use bytes::Bytes;
use reqwest::{Client, Proxy, StatusCode};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::{from_value, json, Value};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use crate::insert;

use super::{Adapters, BoxedFuture, Item, USER_AGENT, FIVE_SECOUND};

static USER_FEATUREL: LazyLock<String> = LazyLock::new(|| {
  json!({
    "hidden_profile_subscriptions_enabled": true,
    "rweb_tipjar_consumption_enabled": true,
    "responsive_web_graphql_exclude_directive_enabled": true,
    "verified_phone_label_enabled": false,
    "subscriptions_verification_info_is_identity_verified_enabled": true,
    "subscriptions_verification_info_verified_since_enabled": true,
    "highlights_tweets_tab_ui_enabled": true,
    "responsive_web_twitter_article_notes_tab_enabled": true,
    "subscriptions_feature_can_gift_premium": true,
    "creator_subscriptions_tweet_preview_api_enabled": true,
    "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
    "responsive_web_graphql_timeline_navigation_enabled": true
  }).to_string()
});
static TWEET_FEATURE: LazyLock<String> = LazyLock::new(|| {
  json!({
    "responsive_web_twitter_blue_verified_badge_is_enabled": true,
    "verified_phone_label_enabled": false,
    "responsive_web_graphql_timeline_navigation_enabled": true,
    "view_counts_public_visibility_enabled": true,
    "view_counts_everywhere_api_enabled": true,
    "longform_notetweets_consumption_enabled": false,
    "tweetypie_unmention_optimization_enabled": true,
    "responsive_web_uc_gql_enabled": true,
    "vibe_api_enabled": true,
    "responsive_web_edit_tweet_api_enabled": true,
    "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
    "standardized_nudges_misinfo": true,
    "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": false,
    "interactive_text_enabled": true,
    "responsive_web_text_conversations_enabled": false,
    "responsive_web_enhance_cards_enabled": false,
  }).to_string()
});

#[inline(always)]
fn tweet_variables(user_id: &str, cursor: &Value, page_size: i32) -> String {
  json!({
    "userId": user_id,
    "count": page_size,
    "cursor": cursor,
    "includePromotedContent": false,
    "withSuperFollowsUserFields": false,
    "withDownvotePerspective": false,
    "withReactionsMetadata": false,
    "withReactionsPerspective": false,
    "withSuperFollowsTweetFields": false,
    "withClientEventToken": false,
    "withBirdwatchNotes": false,
    "withVoice": false,
    "withV2Timeline": false,
  }).to_string()
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct TwitterConfig {
  user_name: String,
  authorization: String,
  cookies: String,
  csrf_token: String,
  page_size: Option<i32>,
  concurrency: Option<usize>,
}

pub struct TwitterAdapter {
  pub username: String,
  userid: OnceLock<String>,
  cursor: Value,
  xhr: Client,
  file: Client,
  cache: LinkedList<TwitterItem>,
  page_size: i32,
  sem: Arc<Semaphore>,
}

pub struct TwitterItem {
  pub url: String,
  pub media_url: String,
  pub client: Client,
  pub filename: String,
  pub is_last: bool,
  sem: Arc<Semaphore>,
}

impl TwitterAdapter {
  pub fn new(config: Value, proxy: Option<Proxy>) -> Self {
    let mut headers = HeaderMap::new();
    let config: TwitterConfig = from_value(config).unwrap();

    insert!(
      headers,
      "Authorization", config.authorization,
      "X-Csrf-Token", config.csrf_token,
      "Cookie", config.cookies
    );

    let mut builder = Client::builder()
    .user_agent(USER_AGENT)
    .default_headers(headers)
    .gzip(true);

    if let Some(proxy) = proxy.clone() {
      builder = builder.proxy(proxy);
    }

    let xhr = builder.build().unwrap();

    builder = Client::builder().user_agent(USER_AGENT).gzip(true);
    if let Some(proxy) = proxy.clone() {
      builder = builder.proxy(proxy);
    }

    let file = builder.build().unwrap();

    Self {
      username: config.user_name,
      page_size: config.page_size.unwrap_or(100),
      cursor: Value::Null,
      userid: OnceLock::new(),
      cache: LinkedList::new(),
      sem: Arc::new(Semaphore::new(config.concurrency.unwrap_or(50))),
      xhr,
      file,
    }
  }

  async fn init(&self) {
    let user_variables = json!({
      "screen_name": self.username,
      "withSafetyModeUserFields": true,
    }).to_string();

    let query = [
      ("variables", user_variables.as_str()),
      ("features", USER_FEATUREL.as_str()),
      ("fieldToggles", "{\"withAuxiliaryUserLabels\":false}")
    ];

    loop {
      match async {
        self.xhr.get("https://x.com/i/api/graphql/Yka-W8dz7RaEuQNkroPkYw/UserByScreenName")
          .query(&query)
          .send().await?.error_for_status()?.json::<Value>().await
      }.await {
        Ok(json) => {
          let result = &json["data"]["user"]["result"];
          let userid = result["rest_id"].as_str().unwrap().to_owned();

          self.userid.set(userid).unwrap();
          break;
        }
        Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
          println!("Warning: too many request, sleep 5 secs and retrying...");
          sleep(FIVE_SECOUND).await;
        }
        Err(err) => {
          println!("Unknown request error {:?}, retrying...", err);
        }
      }
    };
  }
}

impl Adapters for TwitterAdapter {
  fn platform(&self) -> &'static str {
    "twitter"
  }

  fn name(&self) -> &str {
    &self.username
  }

  fn next(&mut self) -> BoxedFuture<'_, Option<Box<dyn Item>>> {
    let futures = async {
      if let Some(item) = self.cache.pop_front() {
        return Some(Box::new(item) as Box<dyn Item>);
      }

      let new_cursor: String;
      
      if self.userid.get().is_none() {
        self.init().await;
      }
      let query =   [
        ("variables", &tweet_variables(self.userid.get().unwrap(), &self.cursor, self.page_size)),
        ("features", &*TWEET_FEATURE)
      ];

      let json = loop {
        match async {
          self.xhr.get("https://api.twitter.com/graphql/QK8AVO3RpcnbLPKXLAiVog/Likes")
            .query(&query)
            .send().await?.error_for_status()?.json::<Value>().await
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

      let timeline = json["data"]["user"]["result"].get("timeline")
        .or(json["data"]["user"]["result"].get("timeline_v2"))
        .unwrap();

      if let serde_json::Value::Array(likes) = &timeline["timeline"]["instructions"][0]["entries"] {
        new_cursor = likes.last().unwrap()["content"]["value"].as_str().unwrap().to_owned();
        for item in likes {
          let result = &item["content"]["itemContent"]["tweet_results"]["result"];
          if result.is_null() {
            continue;
          }

          let snowflake = result["legacy"]["id_str"].as_str()
            .or(result["tweet"]["legacy"]["id_str"].as_str())
            .unwrap().to_owned().parse::<u64>().unwrap();

          let username = result["core"]["user_results"]["result"]["legacy"]["screen_name"].as_str()
            .or(result["tweet"]["core"]["user_results"]["result"]["legacy"]["screen_name"].as_str())
            .unwrap().to_owned();

          let temp = &result["legacy"]["entities"]["media"];
          let media = temp.as_array();

          if let Some(media) = media {
            for (media_index, item) in media.iter().enumerate() {
              let media_index = media_index + 1;
              let media_type = item["type"].as_str().unwrap();
              let (media_url, ext) = match media_type {
                "photo" => {
                  let media_url_https = item["media_url_https"].as_str().unwrap().to_owned();
                  let url = media_url_https.clone() + "?name=orig";
                  let ext = Path::new(media_url_https.as_str()).extension().unwrap().to_str().unwrap().to_owned();
                  (url, ext)
                }
                "animated_gif" | "video" => {
                  let media_url_https = item["video_info"]["variants"]
                    .as_array().unwrap()
                    .last().unwrap()["url"]
                    .as_str().unwrap().to_owned();
                  (media_url_https, "mp4".to_owned())
                }
                _ => panic!("Unknown media type {}.", media_type)
              };
              let filename = format!("{username} {snowflake} {media_index}.{ext}");
              self.cache.push_back(TwitterItem {
                client: self.file.clone(),
                url: format!("http://x.com/{username}/status/{snowflake}/photo/{media_index}"),
                media_url,
                filename,
                is_last: false,
                sem: self.sem.clone(),
              });
            }
            if let Some(item) = self.cache.back_mut() {
              item.is_last = true
            }
          }
        }
      } else {
        panic!("Data may be Null, please check your user name.")
      }

      if new_cursor == self.cursor {
        return None;
      }

      self.cursor = Value::String(new_cursor);

      self.cache.pop_front().map(|v| Box::new(v) as Box<dyn Item>)
    };
    Box::pin(futures)
  }
}

impl Item for TwitterItem {
  fn filename(&self) -> &str {
    &self.filename
  }

  fn media_url(&self) -> &str {
    &self.media_url
  }

  fn url(&self) -> &str {
    &self.url
  }

  fn get(&self) -> BoxedFuture<'_, Bytes> {
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
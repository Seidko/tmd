#![feature(try_blocks)]
use std::{collections::HashSet, env, fs, io::Read, panic, path::Path, sync::{Arc, LazyLock}, time::Duration};
use reqwest::{header, Client, IntoUrl, Proxy, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{from_str, json, Value};
use sysproxy::Sysproxy;
use tokio::{fs::File, time};
use futures::StreamExt;
use tokio::io::copy;

#[inline(always)]
async fn get<U: IntoUrl>(url: U, proxy: &Option<String>) -> Result<Response, reqwest::Error> {
  if let Some(proxy) = proxy {
    return Client::builder().proxy(Proxy::all(proxy).unwrap()).build()?.get(url).send().await;
  }
  Client::builder().build()?.get(url).send().await
}

#[inline(always)]
fn variables(user_id: &str, cursor: &Value, page_size: i32) -> String {
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

static FEATURE: LazyLock<String> = LazyLock::new(|| {
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

const FIVE_SECOUND: Duration = time::Duration::from_secs(5);

#[derive(Serialize, Deserialize)]
struct Config {
  user_id: String,
  authorization: String,
  cookies: String,
  csrf_token: String,
  concurrency: Option<usize>,
  page_size: Option<i32>,
  proxy: Option<String>,
  path: Option<String>,
}

#[tokio::main]
async fn main() {
  if env::var("RUST_BACKTRACE").is_err() {
    env::set_var("RUST_BACKTRACE", "1");
  }

  let hook = panic::take_hook();
  panic::set_hook(Box::new(move |info| {
    hook(info);
    let buf = &mut [0u8];
    std::io::stdin().read_exact(buf).unwrap();
  }));

  let raw = fs::read_to_string("./config.json").unwrap();
  let mut cursor = Value::Null;

  let config: Config = from_str(raw.as_str()).unwrap();
  let page_size = config.page_size.unwrap_or(100);
  let sem = Arc::new(tokio::sync::Semaphore::new(config.concurrency.unwrap_or(50)));
  let dir = config.path.unwrap_or("./media".to_string());

  let mut set: HashSet<[String; 2]> = HashSet::new();
  if let Ok(paths) = std::fs::read_dir(&dir) {
    for entry in paths {
      let path = entry.unwrap().path();
      let file_stem = path.file_stem().unwrap();
      let mut split = file_stem.to_str().unwrap().split(" ");

      let _: Option<_> = try {
        set.insert([split.next()?.to_string(), split.next()?.to_string()]);
      };
    }
  }

  macro_rules! insert {
    ($h:expr, $($k:literal, $v:literal),*) => {
      $($h.insert($k, header::HeaderValue::from_static($v));)*
    }
  }

  macro_rules! ins_str {
    ($h:expr, $($k:literal, $v:expr),*) => {
      $($h.insert($k, header::HeaderValue::from_str(($v).as_str()).unwrap());)*
    }
  }

  let mut headers = header::HeaderMap::new();
  insert!(
    headers,
    "Accept", "*/*",
    "Accept-Language", "en-US,en;q=0.9",
    "Content-Type", "application/json",
    "Connection", "keep-alive",
    "Host", "api.twitter.com",
    "Origin", "https://twitter.com",
    "Referer", "https://twitter.com/",
    "X-Twitter-Active-User", "yes",
    "X-Twitter-Client-Language", "en",
    "X-Twitter-Auth-Type", "OAuth2Session"
  );
  ins_str!(
    headers,
    "Authorization", config.authorization,
    "X-Csrf-Token", config.csrf_token,
    "Cookie", config.cookies
  );

  let mut builder = Client::builder()
    .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.1 Safari/605.1.15")
    .default_headers(headers)
    .gzip(true);

  let proxy: Arc<Option<String>> = config.proxy.or_else(|| {
    let sysproxy = Sysproxy::get_system_proxy().ok()?;
    if !sysproxy.enable {
      return None;
    }
    Some(format!("http://{}:{}", sysproxy.host, sysproxy.port))
  }).into();

  if let Some(proxy) = &*proxy {
    builder = builder.proxy(Proxy::all(proxy).unwrap());
  }

  let client = builder.build().unwrap();
  let _ = fs::create_dir(&dir);

  loop {
    let new_cursor: String;

    let res = loop {
      let query = [
        ("variables", &variables(&config.user_id, &cursor, page_size)),
        ("features", &*FEATURE)
      ];

      if let Ok(res) = client.get("https://api.twitter.com/graphql/QK8AVO3RpcnbLPKXLAiVog/Likes")
      .query(&query)
      .send().await {
        break res;
      } else {
        println!("Warning: 429 or network err.")
      }
    };

    let json: Value = res.json().await.unwrap();
    // let text = res.text().await.unwrap();
    // let _ = tokio::fs::write("sample/out.json", &text).await;
    // let mut json: Value = serde_json::from_str(&text).unwrap();

    let timeline = json["data"]["user"]["result"].get("timeline")
      .or(json["data"]["user"]["result"].get("timeline_v2")).unwrap();

    if let serde_json::Value::Array(likes) = &timeline["timeline"]["instructions"][0]["entries"] {
      new_cursor = likes.last().unwrap()["content"]["value"].as_str().unwrap().to_string();
      for item in likes {
        let result = &item["content"]["itemContent"]["tweet_results"]["result"];
        if result.is_null() {
          continue;
        }

        let id = result["legacy"]["id_str"].as_str()
          .or(result["tweet"]["legacy"]["id_str"].as_str())
          .unwrap().to_string();

        let username = result["core"]["user_results"]["result"]["legacy"]["screen_name"].as_str()
          .or(result["tweet"]["core"]["user_results"]["result"]["legacy"]["screen_name"].as_str())
          .unwrap().to_string();

        if set.contains(&[username.clone(), id.clone()]) {
          continue;
        }

        let temp = &result["legacy"]["entities"]["media"];
        let media = temp.as_array();

        if let Some(media) = media {
          let mut media_index = 1;
          for item in media {
            let media_type = item["type"].as_str().unwrap();
            let (url, ext) = match media_type {
              "photo" => {
                let media_url_https = item["media_url_https"].as_str().unwrap().to_string();
                let url = media_url_https.clone() + "?name=orig";
                let ext = Path::new(media_url_https.as_str()).extension().unwrap().to_str().unwrap().to_string();
                (url, ext)
              }
              "animated_gif" | "video" => {
                let media_url_https = item["video_info"]["variants"]
                  .as_array().unwrap().last().unwrap()["url"].as_str().unwrap().to_string();
                (media_url_https, "mp4".to_string())
              }
              _ => panic!("Unknown media type {}.", media_type)
            };
            let proxy = proxy.clone();
            let id = id.clone();
            let username = username.clone();
            let dir = dir.clone();
            let sem = sem.clone();
            tokio::spawn(async move {
              let _permit = sem.acquire().await.unwrap();
              'retry: loop {
                let mut stream = loop {
                  let result = get(&url, &proxy).await;
                  match result {
                    Ok(res) => break res.bytes_stream(),
                    Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
                      println!("Warning: too many request, sleep 5 secs and retrying...");
                      time::sleep(FIVE_SECOUND).await;
                    }
                    Err(err) => {
                      println!("Unknown request error {:?}, retrying...", err);
                    }
                  }
                };
                let file_name = format!("{username} {id} {media_index}.{ext}");
                let path = Path::new(&dir).join(&file_name);
                let file = File::create(&path).await;
                if let Ok(mut file) = file {
                  while let Some(item) = stream.next().await {
                    if copy(&mut item.unwrap().as_ref(), &mut file).await.is_err() {
                      let _ = tokio::fs::remove_file(&path).await;
                      continue 'retry;
                    }
                  }
                } else {
                  println!("Cannot create file {}, skipped.", &file_name);
                }
                break;
              }
            });
            media_index += 1;
          }
        }
      }
    } else {
      panic!("Unknown value of `entries`.")
    }

    if new_cursor == cursor {
      break;
    }

    cursor = Value::String(new_cursor);
  }
}

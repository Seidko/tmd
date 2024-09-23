#![feature(try_blocks, type_changing_struct_update, let_chains, negative_impls)]
mod simp_h1;
mod subseq;

use std::{collections::HashSet, env, panic, path::Path, sync::{Arc, LazyLock}, time::Duration};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::{from_str, to_string_pretty, json, Value};
use sysproxy::Sysproxy;
use smol::{block_on, fs, io::AsyncWriteExt, Executor, Timer};
use simp_h1::get;

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

#[inline(always)]
fn user_variables(screen_name: &str) -> String {
  json!({
    "screen_name": screen_name,
    "withSafetyModeUserFields": true,
  }).to_string()
}

#[inline(always)]
fn pause() {
  use std::io::Read;
  let buf = &mut [0u8];
  std::io::stdin().read_exact(buf).unwrap();
}

fn output_malform_json(json: &Value, name: &str) -> ! {
  block_on(async {
    let mut file = fs::File::create(format!("./{}.json", name)).await.unwrap();
    file.write(to_string_pretty(json).unwrap().as_bytes()).await.unwrap();
  });
  panic!("Error: malform json")
}

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

const FIVE_SECOUND: Duration = Duration::from_secs(5);
const USER_AGENT: &'static str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.1 Safari/605.1.15";

#[derive(Serialize, Deserialize)]
struct Config {
  user_name: String,
  authorization: String,
  cookies: String,
  csrf_token: String,
  concurrency: Option<usize>,
  page_size: Option<i32>,
  proxy: Option<String>,
  path: Option<String>,
  pause_on_end: Option<bool>,
  pause_on_panic: Option<bool>,
}

fn main() {
  let config: Config = from_str(std::fs::read_to_string("./config.json").unwrap().as_str()).unwrap();
  let dir = config.path.unwrap_or("./media".to_string());
  let page_size = config.page_size.unwrap_or(100);

  let a = get("");

  if config.pause_on_panic.unwrap_or(false) {
    if env::var("RUST_BACKTRACE").is_err() {
      env::set_var("RUST_BACKTRACE", "1");
    }
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
      hook(info);
      pause();
    }));
  };

  let mprogress = MultiProgress::new();
  let style = ProgressStyle::with_template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
    .unwrap()
    .progress_chars("##-");
  let total_pb = mprogress.add(ProgressBar::new(0));
  let media_pb = mprogress.add(ProgressBar::new(0));
  total_pb.set_style(style.clone());
  media_pb.set_style(style.clone());

  let mut headers = header::HeaderMap::new();
  headers.insert("Authorization", config.authorization.parse().unwrap());
  headers.insert("X-Csrf-Token", config.csrf_token.parse().unwrap());
  headers.insert("Cookie", config.cookies.parse().unwrap());

  let (xhr_client, media_client) = {
    let mut builder = Client::builder()
    .user_agent(USER_AGENT)
    .default_headers(headers)
    .gzip(true);

    let proxy: Option<Proxy> = config.proxy.or_else(|| {
      let sysproxy = Sysproxy::get_system_proxy().ok()?;
      if !sysproxy.enable {
        return None;
      }
      Some(format!("http://{}:{}", sysproxy.host, sysproxy.port))
    }).map(|s| Proxy::all(s).unwrap());

    if let Some(proxy) = proxy.clone() {
      builder = builder.proxy(proxy);
    }

    let xhr_client = builder.build().unwrap();

    builder = Client::builder().user_agent(USER_AGENT).gzip(true);
    if let Some(proxy) = proxy.clone() {
      builder = builder.proxy(proxy);
    }

    let media_client = builder.build().unwrap();
    (xhr_client, media_client)
  };
  let ex = Executor::new();

  block_on(ex.run(async {
    let (user_id, exist_file, ) = {
      let exist_file_task = ex.spawn(async {
        let mut set: HashSet<(String, u64, usize)> = HashSet::new();
        let _ = fs::create_dir(&dir).await;
        if let Ok(mut paths) = fs::read_dir(&dir).await {
          while let Some(entry) = paths.next().await {
            let path = entry.unwrap().path();
            let file_stem = path.file_stem().unwrap();
            let mut split = file_stem.to_str().unwrap().split(" ");
          
            let _: Option<_> = try {
              set.insert((split.next()?.to_string(), split.next()?.parse::<u64>().ok()?, split.next()?.parse::<usize>().ok()?));
            };
          }
        }
        set
      });

      let user_id_task = ex.spawn(async {
        let uv = user_variables(&config.user_name);
        let query = [
          ("variables", uv.as_str()),
          ("features", USER_FEATUREL.as_str()),
          ("fieldToggles", "{\"withAuxiliaryUserLabels\":false}")
        ];
  
        loop {
          match async { 
            xhr_client.get("https://x.com/i/api/graphql/Yka-W8dz7RaEuQNkroPkYw/UserByScreenName")
              .query(&query)
              .send().await?.error_for_status()?.json::<Value>().await
          }.await {
            Ok(json) => {
              let result = &json["data"]["user"]["result"];
              let user_id = result["rest_id"].as_str().unwrap_or_else(|| output_malform_json(&json, "likes")).to_owned();
              let fav_count = result["legacy"]["favourites_count"].as_u64().unwrap_or_else(|| output_malform_json(&json, "likes"));
              total_pb.set_length(fav_count);
              break user_id;
            },
            Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
              println!("Warning: too many request, sleep 5 secs and retrying...");
              Timer::after(FIVE_SECOUND).await;
            }
            Err(err) => {
              println!("Unknown request error {:?}, retrying...", err);
            }
          }
        }
      });

      (user_id_task.await, exist_file_task.await)
    };
  
    let mut cursor = Value::Null;
    let sem = Arc::new(smol::lock::Semaphore::new(config.concurrency.unwrap_or(50)));
    loop {
      let new_cursor: String;

      let json = loop {
        let query = [
          ("variables", &tweet_variables(&user_id, &cursor, page_size)),
          ("features", &*TWEET_FEATURE)
        ];

        match async { 
          xhr_client.get("https://api.twitter.com/graphql/QK8AVO3RpcnbLPKXLAiVog/Likes")
            .query(&query)
            .send().await?.error_for_status()?.json::<Value>().await
        }.await {
          Ok(json) => break json,
          Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
            println!("Warning: too many request, sleep 5 secs and retrying...");
            Timer::after(FIVE_SECOUND).await;
          }
          Err(err) => {
            println!("Unknown request error {:?}, retrying...", err);
          }
        }
      };

      let timeline = json["data"]["user"]["result"].get("timeline")
        .or(json["data"]["user"]["result"].get("timeline_v2"))
        .unwrap_or_else(|| output_malform_json(&json, "likes"));

      if let serde_json::Value::Array(likes) = &timeline["timeline"]["instructions"][0]["entries"] {
        new_cursor = likes.last().unwrap_or_else(|| output_malform_json(&json, "likes"))["content"]["value"].as_str()
          .unwrap_or_else(|| output_malform_json(&json, "likes")).to_string();
        for item in likes {
          let result = &item["content"]["itemContent"]["tweet_results"]["result"];
          if result.is_null() {
            continue;
          }

          let snowflake = result["legacy"]["id_str"].as_str()
            .or(result["tweet"]["legacy"]["id_str"].as_str())
            .unwrap_or_else(|| output_malform_json(&json, "likes")).to_string().parse::<u64>().unwrap();

          let username = result["core"]["user_results"]["result"]["legacy"]["screen_name"].as_str()
            .or(result["tweet"]["core"]["user_results"]["result"]["legacy"]["screen_name"].as_str())
            .unwrap_or_else(|| output_malform_json(&json, "likes")).to_string();

          let temp = &result["legacy"]["entities"]["media"];
          let media = temp.as_array();

          if let Some(media) = media {
            for (media_index, item) in media.iter().enumerate() {
              let media_index = media_index + 1;
              if exist_file.contains(&(username.clone(), snowflake, media_index)) {
                continue;
              }
              media_pb.inc_length(1);
              let media_type = item["type"].as_str().unwrap_or_else(|| output_malform_json(&json, "likes"));
              let (url, ext) = match media_type {
                "photo" => {
                  let media_url_https = item["media_url_https"].as_str().unwrap_or_else(|| output_malform_json(&json, "likes")).to_string();
                  let url = media_url_https.clone() + "?name=orig";
                  let ext = Path::new(media_url_https.as_str()).extension().unwrap().to_str().unwrap().to_string();
                  (url, ext)
                }
                "animated_gif" | "video" => {
                  let media_url_https = item["video_info"]["variants"]
                    .as_array().unwrap_or_else(|| output_malform_json(&json, "likes"))
                    .last().unwrap_or_else(|| output_malform_json(&json, "likes"))["url"]
                    .as_str().unwrap_or_else(|| output_malform_json(&json, "likes")).to_string();
                  (media_url_https, "mp4".to_string())
                }
                _ => panic!("Unknown media type {}.", media_type)
              };
              let username = username.clone();
              let dir = dir.clone();
              let sem = sem.clone();
              let client = media_client.clone();
              let media_pb = media_pb.clone();
              ex.spawn(async move {
                let _permit = sem.acquire().await;
                'retry: loop {
                  let mut stream = loop {
                    media_pb.set_message(format!("http://x.com/{username}/status/{snowflake}/photo/{media_index}"));
                    match client.get(&url).send().await.and_then(|r| r.error_for_status()){
                      Ok(res) => break res.bytes_stream(),
                      Err(err) if err.status() == Some(StatusCode::TOO_MANY_REQUESTS) => {
                        println!("Warning: too many request, sleep 5 secs and retrying...");
                        Timer::after(FIVE_SECOUND).await;
                      }
                      Err(err) => {
                        println!("Unknown request error {:?}, retrying...", err);
                      }
                    }
                  };
                  let file_name = format!("{username} {snowflake} {media_index}.{ext}");
                  let path = Path::new(&dir).join(&file_name);
                  if let Ok(mut file) = fs::File::create(&path).await {
                    while let Some(item) = stream.next().await {
                      if item.is_err() || file.write(&item.unwrap()).await.is_err() {
                        let _ = smol::fs::remove_file(&path).await;
                        println!("Error on writing file or disconnected, retrying...");
                        continue 'retry;
                      }
                    }
                  } else {
                    println!("Cannot create file {}, skipped.", &file_name);
                  }
                  media_pb.inc(1);
                  break;
                }
              }).detach();
            }
          }
          total_pb.inc(1);
        }
      } else {
        panic!("Data may be Null, please check your user name.")
      }

      if new_cursor == cursor {
        break;
      }
      cursor = Value::String(new_cursor);
    }
  }));

  while !ex.is_empty() {
    block_on(ex.tick());
  }
  total_pb.set_message("Done!");
  media_pb.set_message("Done!");
  if config.pause_on_end.unwrap_or(false) {
    pause();
  }
}

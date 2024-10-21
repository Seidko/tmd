#![feature(try_blocks, async_iterator, let_chains)]

mod adapters;

use std::collections::LinkedList;
use std::time::Duration;
use std::{collections::HashSet, env, fs, io::Read, panic, path::Path, sync::Arc};
use adapters::Adapters;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Proxy;
use serde::{Deserialize, Serialize};
use serde_json::{from_str, Value};
use sysproxy::Sysproxy;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio::{fs::File, io::AsyncWriteExt};
use adapters::twitter::TwitterAdapter;
use adapters::bluesky::BlueSkyAdapter;

#[inline(always)]
fn pause() {
  let buf = &mut [0u8];
  std::io::stdin().read_exact(buf).unwrap();
}

#[derive(Serialize, Deserialize)]
struct Config {
  accounts: Vec<Value>,
  proxy: Option<String>,
  pause_on_end: Option<bool>,
  pause_on_panic: Option<bool>,
}

#[tokio::main]
async fn main() {
  let raw = fs::read_to_string("./config.json").unwrap();
  let config = Arc::new(from_str::<Config>(raw.as_str()).unwrap());
  if config.pause_on_panic.unwrap_or(false) {
    if env::var("RUST_BACKTRACE").is_err() {
      env::set_var("RUST_BACKTRACE", "1");
    }
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
      hook(info);
      pause();
    }));
  }

  let mprogress = MultiProgress::new();
  let style = ProgressStyle::with_template("[{prefix}] [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
  .unwrap()
  .progress_chars("##-");

  let proxy: Option<Proxy> = config.proxy.clone().or_else(|| {
    let sysproxy = Sysproxy::get_system_proxy().ok()?;
    if !sysproxy.enable {
      return None;
    }
    Some(format!("http://{}:{}", sysproxy.host, sysproxy.port))
  }).map(|s| Proxy::all(s).unwrap());

  let accounts: Vec<_> = config.accounts.clone().into_iter().map(|v| {
    let account = match v.get("platform").and_then(|v| v.as_str()) {
      Some(_p @ "twitter" | _p @ "x") => Box::new(TwitterAdapter::new(v, proxy.clone())) as Box<dyn Adapters + Send>,
      Some(_p @ "bluesky" | _p @ "bsky") => Box::new(BlueSkyAdapter::new(v, proxy.clone())),
      Some(_) => panic!(),
      None => panic!()
    };

    let mut set = HashSet::<String>::new();
    if let Ok(paths) = std::fs::read_dir(account.path()) {
      for entry in paths {
        let path = entry.unwrap().path();
        set.insert(path.file_name().unwrap().to_str().unwrap().to_string());
      }
    }
    let set = Arc::new(set);
    let _ = fs::create_dir(account.path());
    (account, set)
  }).collect();

  let mut handles = LinkedList::<JoinHandle<()>>::new();

  for (mut account, set) in accounts.into_iter() {
    let set = set.clone();
    let mprogress = mprogress.clone();
    let style = style.clone();
    handles.push_back(tokio::spawn(async move {
      let pb = mprogress.add(ProgressBar::new(0));
      pb.set_style(style.clone());
      pb.set_prefix(format!("{} {}", account.platform(), account.name()));
      let tick = pb.clone();
      let ticker = tokio::spawn(async move {
        loop {
          sleep(Duration::from_secs(1)).await;
          tick.tick();
        }
      });
      let mut handles = LinkedList::<JoinHandle<()>>::new();

      while let Some(item) = account.next().await {
        pb.inc_length(1);
        if set.contains(item.filename()) {
          pb.inc(1);
          continue;
        }
        let pb = pb.clone();
        let dir = account.path().to_owned();
        handles.push_back(tokio::spawn(async move {
          pb.set_message(item.url().to_owned());
          let path = Path::new(&dir).join(item.filename());
          match async {
            let bytes = item.get().await;
            let mut file = File::create(&path).await?;
            file.write(&bytes).await
          }.await {
            Err(_) => println!("Cannot create file {}, url {}, skipped.", item.filename(), item.media_url()),
            _ => {}
          }
          pb.inc(1);
        }));
      }
      
      for handle in handles {
        handle.await.unwrap();
      };
      ticker.abort();
      let secs = pb.elapsed().as_secs();
      let h = secs / 3600;
      let m = (secs % 3600) / 60;
      let s = secs % 60;
      mprogress.println(format!("[{} {}] [{h:02}:{m:02}:{s:02}] all tasks Done!", account.platform(), account.name())).unwrap();
    }));
  }
  
  for handle in handles {
    handle.await.unwrap();
  }
  if config.pause_on_end.unwrap_or(false) {
    pause();
  }
}

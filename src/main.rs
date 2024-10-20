#![feature(try_blocks, async_iterator, let_chains)]

mod adapters;

use std::sync::Mutex;
use std::{collections::HashSet, env, fs, io::Read, panic, path::Path, sync::Arc};
use adapters::Adapters;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Proxy;
use serde::{Deserialize, Serialize};
use serde_json::{from_str, Value};
use sysproxy::Sysproxy;
use tokio::task::JoinHandle;
use tokio::{fs::File, io::AsyncWriteExt, sync::mpsc::unbounded_channel};
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

  let (fsx, mut frx) = unbounded_channel::<JoinHandle<()>>();
  let pbs = Arc::new(Mutex::new(Vec::<ProgressBar>::new()));

  for (mut account, set) in accounts.into_iter() {
    let set = set.clone();
    let mprogress = mprogress.clone();
    let style = style.clone();
    let fsx2 = fsx.clone();
    let pbs = pbs.clone();
    fsx.send(tokio::spawn(async move {
      let pb = mprogress.add(ProgressBar::new(0));
      pb.set_style(style.clone());
      pb.set_prefix(format!("{} {}", account.platform(), account.name()));
      pbs.lock().unwrap().push(pb.clone());

      while let Some(item) = account.next().await {
        pb.inc_length(1);
        if set.contains(item.filename()) {
          pb.inc(1);
          continue;
        }
        let pb = pb.clone();
        let dir = account.path().to_owned();
        fsx2.send(tokio::spawn(async move {
          pb.set_message(item.url().to_owned());
          let path = Path::new(&dir).join(item.filename());
          match async {
            let mut file = File::create(&path).await?;
            file.write(&item.get().await).await
          }.await {
            Err(_) => println!("Cannot create file {}, url {}, skipped.", item.filename(), item.media_url()),
            _ => {}
          }
          pb.inc(1);
        })).unwrap();
      }
    })).unwrap();
  }
  
  while let Ok(future) = frx.try_recv() {
    future.await.unwrap();
  }
  pbs.lock().unwrap().iter().for_each(|v| v.set_message("Done!"));
  if config.pause_on_end.unwrap_or(false) {
    pause();
  }
}

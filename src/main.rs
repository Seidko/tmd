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
use tokio::{fs::File, io::AsyncWriteExt, sync::{Semaphore, mpsc::unbounded_channel}};
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
  concurrency: Option<usize>,
  proxy: Option<String>,
  path: Option<String>,
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
  let style = ProgressStyle::with_template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
  .unwrap()
  .progress_chars("##-");

  let dir = config.path.clone().unwrap_or("./media".to_string());

  let mut set = HashSet::<String>::new();
  if let Ok(paths) = std::fs::read_dir(&dir) {
    for entry in paths {
      let path = entry.unwrap().path();
      set.insert(path.file_name().unwrap().to_str().unwrap().to_string());
    }
  }
  let set = Arc::new(set);

  let proxy: Option<Proxy> = config.proxy.clone().or_else(|| {
    let sysproxy = Sysproxy::get_system_proxy().ok()?;
    if !sysproxy.enable {
      return None;
    }
    Some(format!("http://{}:{}", sysproxy.host, sysproxy.port))
  }).map(|s| Proxy::all(s).unwrap());

  let accounts: Vec<_> = config.accounts.clone().into_iter().map(|v| {
    match v.get("platform").and_then(|v| v.as_str()) {
      Some(_p @ "twitter" | _p @ "x") => Box::new(TwitterAdapter::new(v, proxy.clone())) as Box<dyn Adapters + Send>,
      Some(_p @ "bluesky" | _p @ "bsky") => Box::new(BlueSkyAdapter::new(v, proxy.clone())),
      Some(_) => panic!(),
      None => panic!()
    }
  }).collect();

  let _ = fs::create_dir(&dir);
  let total_pbs: Arc<Mutex<Vec<ProgressBar>>> = Arc::new(Mutex::new(Vec::new()));
  let (fsx, mut frx) = unbounded_channel::<JoinHandle<()>>();
  let media_pb = mprogress.add(ProgressBar::new(0));
  media_pb.set_style(style.clone());

  for mut account in accounts.into_iter() {
    let config = config.clone();
    let set = set.clone();
    let dir = dir.clone();
    let total_pbs = total_pbs.clone();
    let media_pb = media_pb.clone();
    let mprogress = mprogress.clone();
    let style = style.clone();
    let fsx2 = fsx.clone();
    fsx.send(tokio::spawn(async move {
      let sem = Arc::new(Semaphore::new(config.concurrency.unwrap_or(10)));
      let total_pb = if let Some(count) = account.count() {
        mprogress.add(ProgressBar::new(count.await))
      } else {
        let total_pb = mprogress.add(ProgressBar::new(1));
        total_pb.set_message(format!("{} has no total count API.", account.platform()));
        total_pb
      };
      total_pb.set_style(style);
      total_pb.inc(1);
      total_pbs.lock().unwrap().push(total_pb.clone());
      while let Some(item) = account.next().await {
        if set.contains(item.filename()) {
          continue;
        }
        media_pb.inc_length(1);
        let media_pb = media_pb.clone();
        let total_pb = total_pb.clone();
        let dir = dir.clone();
        let sem = sem.clone();
        fsx2.send(tokio::spawn(async move {
          let _guard = sem.acquire();
          media_pb.set_message(item.url().to_owned());

          let path = Path::new(&dir).join(item.filename());
          match async {
            let mut file = File::create(&path).await?;
            file.write(&item.get().await).await
          }.await {
            Err(_) => println!("Cannot create file {}, url {}, skipped.", item.filename(), item.media_url()),
            _ => {}
          }
          media_pb.inc(1);
          if item.is_last().unwrap_or(false) {
            total_pb.inc(1);
          }
        })).unwrap();
      }
    })).unwrap();
  }
  
  while let Ok(future) = frx.try_recv() {
    future.await.unwrap();
  }
  total_pbs.lock().unwrap().iter().for_each(|v| v.set_message("Done!"));
  media_pb.set_message("Done!");
  if config.pause_on_end.unwrap_or(false) {
    pause();
  }
}

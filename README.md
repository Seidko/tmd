# Twitter media downloader
Download media in your social media likes. Now support Twitter and Bluesky.
### How to configurate

#### Twitter
Open `x.com/[your account]/likes`, open devtools and search url like `https://x.com/i/api/graphql/*/Likes`

Find value and fill it in `config.json` under the same path with executable.

#### BlueSky
Just need your accounts and password.

#### `config.json` sample
```json5
{
    // required config
    "accounts": [
        {
            // required config
            "platform": "twitter", // or "x"
            "user_name": "", // your user name
            "authorization": "", // in header, `authorization`
            "cookies": "", // in header, `cookies`
            "csrf_token": "", // in header, `x-csrf-token`

            // optional config
            "concurrency": 50, // the maximum concurrent amount, default is 50
            "page_size": 100, // post count in single request, default is 100
        },
        {
            // required config
            "platform": "bluesky", // or "bsky"
            "account": "", // your handle or email
            "pass": "", // your password
            
            // optional config
            "concurrency": 50, // the maximum concurrent amount, default is 50
            "page_size": 50, // post count in single request, default is 50
        }
    ],

    // optional config
    "path": "./media", // the path name will media were downloaded, default is "./media"
    "proxy": "", // proxy will programme follow, default is your system proxy

    // debug config
    "pause_on_end": false, // pause program on complete
    "pause_on_panic": false, // pause program on panic, this config will force enable `RUST_BACKTRACE`
    // remember remove these comments
}
```

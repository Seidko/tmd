# Twitter media downloader
Download media in your twitter likes.
### How to configurate

Open `x.com/[your account]/likes`, open devtools and search url like `https://x.com/i/api/graphql/*/Likes`

Find these value and fill it in `config.json` under the same path with executable.

`config.json`
```json5
{
    "user_id": "", // in query string, `variables.userId`
    "authorization": "", // in header, `authorization`
    "cookies": "", // in header, `cookies`
    "csrf_token": "", // in header, `x-csrf-token`

    // optional config
    "concurrency": 50, // the maximum concurrent amount, default is 50
    "page_size": 100, // post count in single request, default is 100
    "path": "./media", // the path name will media were downloaded, default is "./media"
    "proxy": "" // proxy will programme follow, default is your system proxy

    // remember remove these comments
}
```

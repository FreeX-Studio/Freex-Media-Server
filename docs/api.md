<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# API

FreeX Web talks to the server through a JSON API under `/api`; the upcoming FreeX apps will use the same API. It is
not yet frozen: until version 1.0 of the server, endpoints may still change.

## Authentication

Sign in with `POST /api/login` and send the returned token in the header `X-FreeX-Token` with every request. The
sign-in cookie is only accepted for `GET` requests (images, video, subtitles), so other websites cannot trigger
actions on behalf of a signed-in user. Requests that change state must also come from the server's own origin or
without an `Origin` header (native apps).

```bash
curl -s -X POST https://media.example.org/api/login \
  -H 'Content-Type: application/json' -d '{"username":"alice","password":"…"}'
# {"token":"…","user":{…}}  or  {"twoFactorRequired":true,"challenge":"…"}

curl -s https://media.example.org/api/hubs -H 'X-FreeX-Token: …'
```

## Language and errors

Messages are returned in the display language of the signed-in user, or before sign-in in the language of the
`Accept-Language` header (English and German, otherwise English). Errors look like this; `code` stays the same in
every language, so apps can react to it:

```json
{"status": 401, "code": "error.wrongLogin", "error": "Wrong username or password"}
```

## Endpoints

### Server and account

| Method | Path | |
|---|---|---|
| `GET` | `/api/status` | Server name, setup state; for signed-in users the versions of server and web client |
| `POST` | `/api/login` | `{username, password}` → `{token, user}` or `{twoFactorRequired, challenge}` |
| `POST` | `/api/login/verify` | `{challenge, code}` → `{token, user}` |
| `GET` | `/api/me` | The signed-in user with preferences |
| `PUT` | `/api/me/pin` | `{currentPassword, pin}` sets the PIN for switching users (4–8 digits, not for admins); an empty `pin` removes it |
| `GET` / `POST` | `/api/switch` | From a signed-in device: the accounts with a PIN / `{userId, pin}` switches to one (new session and cookie, like a sign-in; a wrong PIN is 403) |
| `PUT` | `/api/me/preferences` | `{audioLanguage, subtitleMode: off\|forced\|foreign\|always, subtitleLanguage, uiLanguage: ""\|en\|de, unpinned: ["history", "library:<id>", …], qualityHome, qualityAway, subtitleStyle: {size: s|m|l|xl, background: box|outline, position: bottom|high, color: white|yellow}}` (qualities: `auto` or a quality id); send only the fields to change. `/api/me` also says from which `network` (`home`, `away`) the request comes |
| `PUT` / `DELETE` | `/api/me/avatar` | Upload the profile picture as raw request body (JPEG/PNG/GIF, max. 10 MB) / remove it |
| `GET` | `/api/users/{id}/avatar?v=` | Profile picture |

### Users (admin)

| Method | Path | |
|---|---|---|
| `GET` | `/api/users` | All users; `libraries` is `"all"` or the list of shared library ids |
| `POST` | `/api/users` | `{username, password, admin, libraries: "all" \| [libraryId, …]}` |
| `PUT` | `/api/users/{id}` | Change `admin`, `password`, `resetTwoFactor`, `libraries`, `maxAge` (0 = no limit), `allowUnrated` or `removePin` |
| `DELETE` | `/api/users/{id}` | Delete a user |

### Library and playback

| Method | Path | |
|---|---|---|
| `GET` | `/api/libraries` | Libraries |
| `GET` | `/api/libraries/{id}/items?sort=title\|added\|year\|rating\|duration&order=asc\|desc` and filters | Movies or shows of a library. Filters: `watched=unwatched\|inProgress\|watched`, `genre`, `yearFrom`, `yearTo`, `minRating`, `resolution=4K\|1080p\|720p\|576p\|SD`, `contentRating`, `person` (part of an actor's or director's name), `addedDays`, `q` (part of the title); `unwatched=1` still works. The answer has the choices (`genres`, `contentRatings`, `resolutions`, `years`) and the applied `filter` as a query string |
| `GET` | `/api/libraries/{id}/collections` | Collections of a library (`auto` for movie series, `smart` for smart collections) |
| `POST` / `PUT` | `/api/collections`, `/api/collections/{id}` | Admins: `{libraryId, name, itemId}` makes a collection; with `filter` (a query string of the filters above) a smart collection, whose entries are the matching titles for each user. `PUT` changes `name`, `summary` and, for smart collections, `filter` |
| `GET` | `/api/hubs` | Rows of the home screen |
| `GET` | `/api/search?q=` | Search |
| `GET` | `/api/items/{id}` | Details, with seasons and episodes; for movies and episodes `media` (tracks, subtitles, `chapters`), `markers` and `previews`; for movies in several versions `versions` (`id`, `label`, `resolution`, `videoCodec`, `size`, `primary`, `current`; a version id works wherever an item id does, e.g. `/api/items/{versionId}/playback`, and its progress is the movie's); for movies and shows `extras` (entries of type `extra` with `extraType`; they play like movies, keep no progress and do not appear in lists or search) |
| `GET` | `/api/items/{id}/image/{poster\|backdrop\|thumb}` | Artwork |
| `GET` | `/api/items/{id}/previews` | Seek preview sprite sheet (JPEG). The grid is `previews` in the item details: `{intervalMs, width, height, columns, rows, count, version}`; frame `n` shows `n × intervalMs` and sits in column `n % columns`, row `n / columns`. Missing while it is not made yet |
| `POST` | `/api/items/{id}/playback` | `{caps, quality, bandwidth, audio, subtitle, burnSubtitle, startMs}` → playback mode and stream URL (`burnSubtitle: true` burns a text `subtitle` into the picture; `textBurnIn` says whether the server can), the `quality` used, whether it was `automatic`, and the `network`. `quality` `auto` (or missing) takes the user's default for the network, `bandwidth` (bits/s) is what the client measured; with `"player": "external"` (a native player that decodes every format, e.g. mpv in FreeX Desktop) always direct play |
| `GET` | `/api/items/{id}/stream` | The file itself (supports range requests) |
| `GET` | `/api/transcode/{session}/index.m3u8` | HLS playlist (VOD) of a conversion over the whole title; segments `s<n>.m4s`, converted on request |
| `GET` | `/api/items/{id}/subtitles/{id}.vtt` | Subtitle as WebVTT |
| `PUT` | `/api/items/{id}/progress` | `{offsetMs}` resume position |
| `POST` / `DELETE` | `/api/items/{id}/watched` | Mark watched / unwatched |
| `POST` | `/api/items/{id}/cast` | `{quality, audio, subtitle, startMs}` → URLs with a token for a Chromecast |

### Lists, history and Watch Together

| Method | Path | |
|---|---|---|
| `GET` / `POST` | `/api/playlists` | Playlists of the user / create one |
| `GET` | `/api/watchlist` | Watchlist; `PUT` / `DELETE /api/watchlist/{itemId}` to change it |
| `GET` | `/api/me/history?offset=&limit=` | Own watch history, newest first; `DELETE /api/me/history[/{id}]` removes all or one entry |
| `POST` | `/api/together` | `{itemId}` → Watch Together room; `GET /api/together/{code}/events` is its event stream |

### Administration

| Method | Path | |
|---|---|---|
| `GET` | `/api/admin/activity` | Running playbacks (with the estimated `bitrate` and, for conversions on Linux, the `cpu` of ffmpeg in percent of one core) and library scans |
| `GET` | `/api/admin/dashboard?days=30&tz=-60` | The dashboard: `load` (CPU, server memory, bandwidth, playbacks, conversions), `plays` over the period (per day by the browser's time zone, per user, the ten most watched titles) and `libraries` (entries, files, size, length, resolutions, codecs, HDR) |
| `GET` | `/api/history?user=` | Watch history of all (or one) users |
| `GET` | `/api/admin/settings` | Settings of `config.toml` (secrets only as set / not set) and `pendingRestart` |
| `PUT` | `/api/admin/settings` | `{currentPassword, values: {"server.port": 32600, …}}`; a secret sent as `null` stays unchanged |
| `POST` | `/api/admin/restart` | Restart the server with the saved configuration |
| `GET` / `PUT` | `/api/items/{id}/edit` | Admins: the editable fields and which are `edited` / `{values: {title, sortTitle, originalTitle, year, summary, tagline, genres, rating, contentRating}, reset: [field, …]}` |
| `PUT` / `DELETE` | `/api/items/{id}/artwork/{poster\|backdrop}` | Admins: upload an own picture (raw image body, max. 10 MB) / remove it |
| `POST` | `/api/admin/maintenance` | Run the nightly maintenance now (409 while it runs); its schedule and last report are `maintenance` in the dashboard |
| `GET` / `POST` | `/api/admin/webhooks` | Webhooks with their secret and last delivery / `{url, events}` adds one |
| `PUT` / `DELETE` | `/api/admin/webhooks/{id}` | `{url, events, newSecret}` changes one / deletes it; `POST …/{id}/test` sends a test event |

## Webhooks

Admins can have the server call addresses on events (*Settings → Webhooks*), for home automation or notifications:
an HTTP `POST` with a JSON body, the headers `X-FreeX-Event: <event>` and
`X-FreeX-Signature: sha256=<hex HMAC-SHA256 of the body, keyed with the webhook's secret>`, a timeout of ten seconds
and one retry after five seconds. Redirects are not followed.

| Event | When |
|---|---|
| `play` | A playback starts (not again for jumps or quality changes) |
| `pause`, `resume` | The player pauses or continues |
| `stop` | The player is closed or moves on |
| `watched` | A title counts as watched (90 %) |
| `library.new` | A scan found new movies or episodes (not on a library's first scan); up to 100 of them |

```json
{
  "event": "play", "time": 1794246580212, "server": "FreeX",
  "user": {"id": "67db…", "username": "niklas"},
  "item": {"id": "e8eb…", "type": "episode", "title": "Secrets", "year": 2017, "show": "Dark", "season": 1, "episode": 1, "durationMs": 3112000},
  "player": {"device": "Chrome on Windows", "ip": "192.168.1.20", "mode": "direct"},
  "offsetMs": 0
}
```

`library.new` has `library` (`id`, `name`), `count` and `items` instead. A check in Python:
`hmac.compare_digest(sig, "sha256=" + hmac.new(secret.encode(), body, hashlib.sha256).hexdigest())`.

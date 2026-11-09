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
| `PUT` | `/api/me/preferences` | `{audioLanguage, subtitleMode: off\|forced\|foreign\|always, subtitleLanguage, uiLanguage: ""\|en\|de, unpinned: ["history", "library:<id>", …]}`; send only the fields to change |
| `PUT` / `DELETE` | `/api/me/avatar` | Upload the profile picture as raw request body (JPEG/PNG/GIF, max. 10 MB) / remove it |
| `GET` | `/api/users/{id}/avatar?v=` | Profile picture |

### Users (admin)

| Method | Path | |
|---|---|---|
| `GET` | `/api/users` | All users; `libraries` is `"all"` or the list of shared library ids |
| `POST` | `/api/users` | `{username, password, admin, libraries: "all" \| [libraryId, …]}` |
| `PUT` | `/api/users/{id}` | Change `admin`, `password`, `resetTwoFactor` or `libraries` |
| `DELETE` | `/api/users/{id}` | Delete a user |

### Library and playback

| Method | Path | |
|---|---|---|
| `GET` | `/api/libraries` | Libraries |
| `GET` | `/api/libraries/{id}/items?sort=title\|added\|year\|rating&unwatched=1&genre=` | Movies or shows of a library |
| `GET` | `/api/libraries/{id}/collections` | Collections of a library |
| `GET` | `/api/hubs` | Rows of the home screen |
| `GET` | `/api/search?q=` | Search |
| `GET` | `/api/items/{id}` | Details, with seasons and episodes; for movies and episodes `media` (tracks, subtitles, `chapters`), `markers` and `previews`; for movies and shows `extras` (entries of type `extra` with `extraType`; they play like movies, keep no progress and do not appear in lists or search) |
| `GET` | `/api/items/{id}/image/{poster\|backdrop\|thumb}` | Artwork |
| `GET` | `/api/items/{id}/previews` | Seek preview sprite sheet (JPEG). The grid is `previews` in the item details: `{intervalMs, width, height, columns, rows, count, version}`; frame `n` shows `n × intervalMs` and sits in column `n % columns`, row `n / columns`. Missing while it is not made yet |
| `POST` | `/api/items/{id}/playback` | `{caps, quality, audio, subtitle, startMs}` → playback mode and stream URL; with `"player": "external"` (a native player that decodes every format, e.g. mpv in FreeX Desktop) always direct play |
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
| `GET` | `/api/admin/activity` | Running playbacks and library scans |
| `GET` | `/api/history?user=` | Watch history of all (or one) users |
| `GET` | `/api/admin/settings` | Settings of `config.toml` (secrets only as set / not set) and `pendingRestart` |
| `PUT` | `/api/admin/settings` | `{currentPassword, values: {"server.port": 32600, …}}`; a secret sent as `null` stays unchanged |
| `POST` | `/api/admin/restart` | Restart the server with the saved configuration |

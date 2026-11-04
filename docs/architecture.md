<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Architecture

This page explains how FreeX Media Server works inside: which parts there are, how a request is handled, where data
lives and what happens when a title is played. It is meant for contributors and for authors of FreeX apps.

- [Overview](#overview)
- [Startup and restart](#startup-and-restart)
- [Handling a request](#handling-a-request)
- [Data](#data)
- [Scanning a library](#scanning-a-library)
- [Playing a title](#playing-a-title)
- [Background work](#background-work)
- [FreeX Web](#freex-web)

## Overview

```
                       ┌──────────────────────────── FreeX Media Server ────────────────────────────┐
 FreeX Web  ─┐         │  WebServer ── AuthApi · MediaApi · ExtrasApi · ProfileApi · SettingsApi     │
 FreeX apps ─┼─ HTTP ──┤                ActivityApi · CastApi                                         │
 Chromecast ─┘         │      │                                                                       │
                       │      ├── DataStore ── libraries + index (JSON)    ── UserStore / ListStore ─┐ │
                       │      │                                                    SQLite freex.db ◄─┘ │
                       │      ├── LibraryScanner ── NameParser · NfoReader · MediaProbe (ffprobe)     │
                       │      ├── MetadataService ── TmdbClient          MarkerService ── ffmpeg      │
                       │      ├── PlaybackPlanner · TranscodeManager ── ffmpeg ── HLS segments       │
                       │      └── TogetherService (Watch Together)                                    │
                       └──────────────────────────────────────────────────────────────────────────────┘
```

The server is a single Java 21 process without a framework. It uses the HTTP server of the JDK
(`com.sun.net.httpserver`) with one virtual thread per request, so long-running video streams and event streams do
not block anything. Its only library is the SQLite JDBC driver; ffmpeg and ffprobe run as external programs.

The source is grouped by topic; each package has a `package-info.java` describing it.

| Package | Responsibility |
|---|---|
| `config` | `config.yml`: reading it at start, validating and writing settings from the web interface |
| `http` | Web server, routing, authentication, security headers and all API endpoints |
| `store` | SQLite database (users, sessions, watch state, history, lists) and the library index |
| `model` | Libraries, items, media files and tracks, users, watch state |
| `scan` | Walking library folders, recognising titles, reading `.nfo` files and artwork, ffprobe |
| `metadata` | Matching titles on TMDB and merging online with local metadata |
| `transcode` | Choosing direct play or conversion, ffmpeg sessions, subtitles |
| `markers` | Audio fingerprints and intro/credits detection |
| `together` | Watch Together rooms |
| `auth` | Password hashing, TOTP, brute-force protection |

## Startup and restart

`FreeXMediaServer.main` reads `config.yml` and starts an *instance*: it opens the database, loads the library
index, creates the services, registers the API endpoints and starts the web server. Every service is registered
with the instance so it can be closed again in reverse order.

When an admin saves settings and restarts the server, the instance is closed and a new one is started from the file,
inside the same process; sign-ins survive because sessions are in the database. If the new configuration does not
start (port in use, broken keystore, …), the previous file is restored and started again, and the failed one is kept
as `config.yml.failed`.

## Handling a request

1. `WebServer` matches the path against the registered routes. Each route declares who may call it: everyone,
   signed-in users or admins.
2. The user is resolved from the `X-FreeX-Token` header, or for `GET` requests from the cookie (so `<img>` and
   `<video>` work). Requests that change state must carry the header and come from the server's own origin, which
   blocks cross-site request forgery.
3. The handler gets an `Exchange` with the user, the path parameters and helpers for JSON bodies and responses.
   Errors are thrown as `ApiException` with a status and a message for the user.
4. Every response gets strict security headers (Content-Security-Policy, frame and sniffing protection, HSTS over
   HTTPS). Only URLs under `/api/cast/` may be loaded by other origins, for the Chromecast.

**Library sharing** is enforced in the store: `DataStore.item(user, id)`, `librariesOf(user)` and `canSee(user, …)`
only return what the user may see. Handlers use these instead of the unchecked lookups, so a title of a library that
is not shared answers `404` everywhere, as if it did not exist.

## Data

| What | Where | Notes |
|---|---|---|
| Users, sessions, login history, preferences, watch state, watch history, playlists, watchlist, own collections, library sharing | `freex.db` (SQLite) | Versioned schema in `Database.MIGRATIONS`; migrations are applied in order at start and never edited afterwards. WAL mode, foreign keys |
| Libraries and the scanned index | `libraries.json`, `items/<library>.json` | Rebuilt by every scan; kept in memory and written a few seconds after a change |
| Metadata from TMDB, artwork | `metadata.json`, `metadata/` | Refreshed after 30 days |
| Intro and credits markers | `markers.json` | Tied to the size and date of the analysed file |
| Profile pictures | `avatars/` | Re-encoded by the server |
| Thumbnails, subtitles, conversion segments | cache directory | Recreated as needed |

`UserStore` and `ListStore` keep their data in memory and write every change through to the database immediately.
Session tokens exist in plain text only in the browser; the database stores their SHA-256 hash.

## Scanning a library

`LibraryScanner` walks the folders of a library on a background thread:

1. Video files are found, bonus folders (`Extras`, `Samples`, …) and folders with a `.ignore` file are skipped.
2. `NameParser` turns folder and file names into titles, years, seasons and episodes, including scene release names.
3. Local `.nfo` files, artwork and subtitle files next to the video are picked up.
4. `MediaProbe` runs ffprobe for duration, codecs, resolution and tracks. Unchanged files (same size and date) reuse
   their previous result, so rescans are fast.
5. The new index replaces the old one at once. A folder that is missing (for example an unmounted disk) keeps its old
   items instead of wiping them.

After a scan, the listeners run: `MetadataService` matches new titles on TMDB, `MarkerService` analyses new episodes.

## Playing a title

1. The client sends `POST /api/items/{id}/playback` with what it can decode (containers, video and audio codecs),
   the chosen quality, audio track, subtitle and start position.
2. `PlaybackPlanner` decides: **direct play** (the file as it is), **direct stream** (video copied into HLS, audio
   converted if needed) or **transcode** (video converted to H.264). Bitmap subtitles are burned into the picture.
3. For direct play the client loads `/api/items/{id}/stream`, which supports range requests. Otherwise
   `TranscodeManager` starts ffmpeg, which writes HLS segments of four seconds into the conversion folder; the
   client plays `/api/transcode/{session}/index.m3u8`.
4. A conversion starts at the requested position, so the client keeps that offset and shows absolute times. Seeking
   inside the converted part is instant; further ahead the client asks for a new playback at that position.
5. ffmpeg is paused when it is far ahead of the player and stopped one minute after the player stops asking for
   segments. At most `transcoder.maxSessions` conversions run at once.
6. The client reports the position every few seconds (`PUT /api/items/{id}/progress`). This saves the resume
   position, marks the title as watched at 90 % and adds it to the watch history, and keeps the playback visible in
   the admins' activity view.

For a **Chromecast**, `CastApi` plans the playback for the receiver's formats and hands out a random token that
unlocks only that title for a limited time, because the Chromecast cannot sign in.

## Background work

| Task | Where |
|---|---|
| Periodic rescans | `LibraryScanner` (every `scanner.intervalMinutes`) |
| TMDB matching and refresh | `MetadataService` (one worker, rate-friendly) |
| Intro and credits detection | `MarkerService` (one worker, once per season and file version) |
| Stopping idle conversions | `TranscodeManager` (reaper every 10 seconds) |
| Writing the library index | `DataStore` (a few seconds after a change) |
| Cleaning up Watch Together rooms | `TogetherService` (every minute) |

All of them are closed and awaited when the instance stops, so nothing writes to a closed store during a restart.

## FreeX Web

The web client lives in `src/main/resources/web` and is served by the server from the jar. It is plain JavaScript
without a framework or build step:

| File | Content |
|---|---|
| `core.js` | State, DOM helper `h()`, API access, routing, the app shell (header, sidebar, activity), cards |
| `views.js` | Home screen, libraries, detail pages, search |
| `player.js` | The player: direct and HLS playback, tracks, skip intro, progress, Watch Together, Chromecast |
| `account.js` | Sign-in and setup, account, security, users and library sharing, server settings, about page |
| `extras.js` | Watchlist, playlists, watch history, collections, people, fix match, trailers, Watch Together dialogs |
| `cast.js` | Google Cast sender (only loaded in Chrome over HTTPS) |
| `boot.js` | Starts the app after all parts have registered |

Texts are English in the code and go through `t()`, which looks them up in the catalog of the user's display
language (`i18n.js`, `i18n/de.js`). The server answers with messages in the same language: error answers carry the
translated `error` and a stable `code` (the catalog key) for apps.

Each user chooses which sidebar entries are pinned. Unpinned entries are stored as ids (`history`,
`library:<id>`) in the user's preferences, so the choice follows them to every device; the *More* view at the end of
the sidebar lists all entries with pin buttons.

All markup is created with `h()`, which sets text and attributes but never parses HTML from data, so titles or file
names cannot inject markup. The parts share the global object `window.FreeX`. FreeX Web has its own version in
`version.json`.

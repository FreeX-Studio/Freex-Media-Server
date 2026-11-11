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
 FreeX Web  ─┐         │  axum router ── api/: account · users · server · media · playback · metadata   │
 FreeX apps ─┼─ HTTP ──┤                 lists · profile · extras · admin                              │
 Chromecast ─┘         │      │                                                                       │
                       │      ├── Catalog (library index in memory) ───── Db (SQLite freex.db) ◄────┐ │
                       │      ├── media::scanner ── names · nfo · probe (ffprobe)                   │ │
                       │      ├── metadata::Service ── TMDB · TVmaze      markers::Service ── ffmpeg │ │
                       │      ├── media::playback · transcode::Transcoder ── ffmpeg ── HLS segments  │ │
                       │      └── together::Together (Watch Together) · activity::Activity           │ │
                       └──────────────────────────────────────────────────────────────────────────────┘
```

The server is one Rust program on the tokio runtime with the axum web framework. SQLite is compiled in, TLS
comes from rustls; ffmpeg and ffprobe run as external programs. Requests are handled on a few threads with async
tasks, so long-running video streams and event streams do not block anything; CPU-heavy work (password hashing,
fingerprints, file system walks) runs on blocking threads.

| Module | Responsibility |
|---|---|
| `config`, `settings` | `config.toml`: reading it at start, validating, and writing settings from the web interface |
| `http`, `api/` | Request context, authentication, CSRF check, security headers, errors, and all API endpoints |
| `api/jellyfin/` | Compatibility layer for native Jellyfin apps under `/jellyfin` (off by default): translates their requests to the catalog, watch state and playback decision of FreeX, see [Jellyfin apps](jellyfin.md) |
| `api/dlna/` | DLNA/UPnP media server (off by default): SSDP discovery, device and service descriptions, ContentDirectory browsing as DIDL-Lite, streaming with DLNA headers on its own port, see [DLNA](dlna.md) |
| `db` | SQLite (users, sessions, libraries, index, watch state, history, lists, metadata, markers) |
| `media` | Library entries, the scanner (names, `.nfo`, artwork, ffprobe), the in-memory index, the playback planner, subtitles, preview frames |
| `metadata` | Matching titles on TMDB (with TVmaze as fallback for shows) and merging online with local metadata |
| `transcode` | ffmpeg sessions and their background workers |
| `markers` | Audio fingerprints and intro/credits detection |
| `together`, `activity` | Watch Together rooms; running playbacks |
| `system` | Load of the machine for the dashboard (from `/proc` on Linux, every 3 seconds), kept for the graphs: every sample of the last hour, a mean per minute of the last day |
| `logbuf` | Logging setup: the normal output, the last 5000 lines in memory for the log page, and the level filter that can be changed at run time |
| `auth`, `tls` | Password hashing, TOTP, brute-force protection; HTTPS from PEM files |

## Startup and restart

`main` reads `config.toml` and runs the server: it opens the database, loads the library index, creates the
services in one shared `AppState`, and serves the router over HTTP or HTTPS. Background tasks (scans, metadata,
conversions, cleanups) run on the same runtime.

When an admin saves settings and restarts the server, the runtime is shut down and the loop in `main` starts again
from the file, inside the same process; sign-ins survive because sessions are in the database. If the new
configuration does not start (port in use, broken certificate, …), the previous file is restored and started again,
and the failed one is kept as `config.toml.failed`.

## Handling a request

1. The `context` middleware runs before every route: it resolves the user from the `X-FreeX-Token` header, or for
   `GET` requests from the cookie (so `<img>` and `<video>` work), and rejects state-changing requests that do not
   come from the server's own origin, which blocks cross-site request forgery.
2. The router matches the path. Each handler says who may call it by asking the context for the user or an admin.
3. Handlers return `ApiError` values with a status and a message key; the middleware renders the message in the
   language of the user (or of `Accept-Language` before sign-in) as `{"error": …, "code": …, "status": …}`.
4. Every response gets strict security headers (Content-Security-Policy, frame and sniffing protection, HSTS over
   HTTPS). Only URLs under `/api/cast/` may be loaded by other origins, for the Chromecast.

**Library sharing** is enforced by the lookups the handlers use (`visible_item`, `visible_library`), which only
return what the user may see, so a title of a library that is not shared answers `404` everywhere, as if it did not
exist.

## Data

| What | Where | Notes |
|---|---|---|
| Users, sessions, login history, preferences, library sharing, watch state, watch history, playlists, watchlist, own collections, TMDB metadata, intro and credits markers | `freex.db` (SQLite) | Versioned schema in `db::MIGRATIONS`; migrations are applied in order at start and never edited afterwards. WAL mode, foreign keys |
| Libraries and the scanned index | `freex.db`, table `items` (one JSON row per entry) | Kept in memory as an immutable snapshot that a scan replaces in one go; the database is the copy for the next start |
| Artwork from TMDB | `metadata/` | Titles, people and movie series; metadata is refreshed after 30 days |
| Profile pictures | `avatars/` | Re-encoded by the server |
| Thumbnails, subtitles, conversion segments | cache directory | Recreated as needed |

Session tokens exist in plain text only in the browser; the database stores their SHA-256 hash.

## Scanning a library

`media::scanner` walks the folders of a library on a blocking thread, one library at a time:

1. Video files are found, bonus folders (`Extras`, `Samples`, …) and folders with a `.ignore` file are skipped.
2. `media::names` turns folder and file names into titles, years, seasons and episodes, including scene release
   names and ids written into names (`{tmdb-603}`, `{imdb-tt0133093}`, `{tvdb-121361}`).
3. Local `.nfo` files, artwork and subtitle files next to the video are picked up.
4. `media::probe` runs ffprobe for duration, codecs, resolution and tracks, several files at a time. Unchanged
   files (same size and date) reuse their previous result, so rescans are fast.
5. The new index replaces the old one at once. A folder that is missing (for example an unmounted disk) keeps its old
   items instead of wiping them.

After a scan, `metadata::Service` matches new or renamed titles on TMDB and `markers::Service` analyses new
episodes and `media::previews` makes the seek preview sheets of new files, all in the background.

## Playing a title

1. The client sends `POST /api/items/{id}/playback` with what it can decode (containers, video and audio codecs),
   the chosen quality, audio track, subtitle and start position.
2. `media::playback` decides: **direct play** (the file as it is), **direct stream** (video copied into HLS, audio
   converted if needed) or **transcode** (video converted to H.264). Bitmap subtitles are burned into the picture.
3. For direct play the client loads `/api/items/{id}/stream`, which supports range requests. Otherwise
   `transcode::Transcoder` creates a session with a `Worker`, and the client plays
   `/api/transcode/{session}/index.m3u8`. The playback is listed in the activity view right away, as *starting*
   until the session runs.
4. That playlist covers the whole title from the start (`Segments`): segments of two seconds when the video is
   converted, from keyframe to keyframe (at least four seconds) when it is copied; the keyframes are read from the
   index of Matroska files directly (`media::mkv`, a few small reads) and with ffprobe through the index for other
   files, already when the title's page is opened, and kept in memory and in `cache/keyframes/`. The player seeks
   like in any other video and asks for whatever segment it needs.
5. The worker runs ffmpeg from the segment asked for. ffmpeg writes one fragment per keyframe with the timestamps
   of the source (`-copyts`, no MP4 edit lists, a fixed offset of one second), so fragments of different runs fit
   together exactly; a segment is sent as its fragments one after the other. When the player asks for a segment
   far ahead of or behind the running ffmpeg, the worker restarts it there; everything converted before is kept
   and served again when the player comes back.
6. ffmpeg runs with lower CPU and I/O priority (`nice`, `ionice` on Linux, below-normal priority on Windows), is
   paused when it is two minutes ahead of the player and stopped one minute after the player stops asking for
   segments. At most `max_sessions` conversions run at once.
7. The client reports the position every few seconds (`PUT /api/items/{id}/progress`). This saves the resume
   position, marks the title as watched at 90 % and adds it to the watch history, and keeps the playback visible in
   the admins' activity view.

For a **Chromecast**, `api::extras` plans the playback for the receiver's formats and hands out a random token that
unlocks only that title for a limited time, because the Chromecast cannot sign in.

## Background work

| Task | Where |
|---|---|
| Periodic rescans | `media::scanner` (every `interval_minutes`) |
| TMDB matching and refresh | `metadata::Service` (one library at a time, at most four requests in parallel) |
| Intro and credits detection | `markers::Service` (one season at a time, its episodes in parallel, once per file version) |
| Seek preview sheets | `media::previews` (several files at a time, once per file version) |
| Converting, one per playback | `transcode::Worker` (an ffmpeg process at a time, restarted where the player asks) |
| Pausing and stopping conversions | `transcode::Transcoder` (every 5 seconds) |
| Expired sessions | `main` (every hour) |
| Cleaning up Watch Together rooms | `together::Together` (loading members every 2 seconds, rooms every minute) |

Background work must not get in the way of someone watching:

- ffprobe and ffmpeg of scans, intro detection and previews run at low CPU and I/O priority.
- Intro detection and previews use a quarter of the cores (one to three jobs) and wait while a playback is running
  (`activity::yield_to_playback`), before each file or season.
- `db::Db` has one connection for writing and a few read-only connections (SQLite WAL), so reading requests do not
  queue behind a scan's writes. A scan writes only the entries that changed.
- `media::catalog::Catalog` prepares a new index and writes it to the database without holding the index lock; the
  lock is only taken for swapping in the finished index, so requests keep reading the old one meanwhile.

All of them live on the server's tokio runtime, which is shut down as a whole on a stop or restart; running
conversions are ended explicitly, so no ffmpeg outlives the server.

## FreeX Web

The web client lives in `web/` and is compiled into the server program. It is plain JavaScript without a framework
or build step:

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

The server adds a build id (a hash of the scripts and styles) to their URLs in `index.html`, so browsers cache them
for good and still get every update at once. API answers carry the current build in `X-FreeX-Build`; a page that
notices a newer one reloads with the next page change, never during a playback.

All markup is created with `h()`, which sets text and attributes but never parses HTML from data, so titles or file
names cannot inject markup. The parts share the global object `window.FreeX`. FreeX Web has its own version in
`version.json`.

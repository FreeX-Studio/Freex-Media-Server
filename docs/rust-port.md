<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Rust port

FreeX Media Server 0.20 is a rewrite of the Java server (0.14 and earlier) in Rust, done step by step on the
branch `rust`. This page records why, what was decided, what changed for users, and notes on each step for whoever
reads the code with the Java version in mind. The last Java release is tagged `v0.14.0`.

## Why

- **Memory and start-up:** one native program of a few megabytes that starts instantly and needs a fraction of
  the memory of the JVM.
- **No Java runtime:** the Debian package and the Windows installer only need ffmpeg.
- **One codebase for Linux and Windows,** built natively or cross-compiled.

Playback and conversion speed are set by ffmpeg and do not change.

## Decisions

- **API and FreeX Web stay the same.** The web client, FreeX Desktop and the future apps keep working unchanged;
  FreeX Web lives in `web/` and is compiled into the program.
- **Fresh data format.** No migration from the Java server: configuration in `config.toml` (TOML), a new SQLite
  schema, passwords hashed with Argon2id.
- **Error texts** stay in `i18n/messages_<language>.properties`, compiled in; error answers keep the format
  `{"error", "code", "status"}`.

## What changed for users

- **Configuration** is `config.toml` with `snake_case` keys instead of `config.yml`; the settings page knows both
  spellings. The data of a Java installation is not taken over (see [Installation](installation.md)).
- **HTTPS** takes a PEM certificate and key instead of a PKCS#12 keystore with password.
- **Packages** need no Java: the Debian package depends only on ffmpeg, the Windows installer ships ffmpeg and the
  service wrapper but no runtime. The server finds ffmpeg in an `ffmpeg` folder next to its program.
- **Matching on TMDB** is more robust (ids in names and `.nfo` files, several search variants, scoring, TVmaze as a
  fallback for shows); a new switch `tmdb.tvmaze`.
- **Restart** from the settings page takes well under a second instead of about two.
- Everything else (the API, FreeX Web, FreeX Desktop, Chromecast, Watch Together, markers) works as before.

## Layout

| Path | Content |
|---|---|
| `src/main.rs` | Start-up, shutdown, background tasks |
| `src/config.rs` | `config.toml`, defaults, validation |
| `src/db.rs` | SQLite schema (versioned migrations) and data access |
| `src/auth.rs` | Argon2id, tokens, brute-force protection, TOTP, recovery codes |
| `src/http.rs` | Request context, errors, security headers, CSRF protection, FreeX Web |
| `src/i18n.rs` | Message catalogs, language negotiation |
| `src/files.rs` | Sending files: content types, caching headers, byte ranges |
| `src/media/` | Library entries, name parser, `.nfo` reader, ffprobe, scanner, in-memory index, video frames |
| `src/metadata/` | TMDB and TVmaze clients, matching, online metadata of the library entries |
| `src/activity.rs` | Running playbacks for the activity view |
| `src/markers/` | Intro and credits detection by audio fingerprints |
| `src/together.rs` | Watch Together rooms with server-sent events |
| `src/settings.rs` | Settings of the web interface and how they are written into `config.toml` |
| `src/tls.rs` | HTTPS from PEM files |
| `src/transcode/` | Conversion sessions: segment plan, ffmpeg command lines, background workers |
| `src/api/` | The API endpoints, grouped like the Java handlers |
| `web/` | FreeX Web (unchanged) |
| `packaging/deb/` | Debian package: `build-deb.sh`, control scripts, systemd service, default `config.toml` |
| `packaging/windows/` | Windows installer: `build-installer.sh`, NSIS script, WinSW service definition |
| `packaging/dist.sh` | Plain release archives and `SHA256SUMS` |
| `Jenkinsfile` | Build pipeline: checks, tests, Linux and Windows builds, .deb, installer, archives |

## Progress

| Step | Content | State |
|---|---|---|
| 1 | Foundation: configuration, database, web server, FreeX Web, security headers, CSRF, setup, sign-in with two-factor, sessions, login history, preferences, users and library sharing, library definitions | done |
| 2 | Library scanning: folder walk, name parsing, `.nfo` and artwork, ffprobe, index; libraries, hubs, items, search, folder picker | done |
| 3 | Direct play: streaming with range requests, progress, watched state, history, subtitles | done |
| 4 | Conversion: playback planner, transcode workers, HLS segments, hardware acceleration | done |
| 5 | Metadata from TMDB, artwork, people, collections, fix match, trailers | done |
| 6 | Lists and extras: playlists, watchlist, own collections, profile pictures, activity view | done |
| 7 | Intro and credits detection, Watch Together, Chromecast | open |
| 8 | Server settings in the browser, in-process restart, HTTPS | open |
| 9 | Release: upgrades from the Java version, documentation, versions and changelog | done |

## Notes on step 2

- The library index lives in memory as an immutable snapshot that a finished scan replaces in one go; pages never
  wait for a running scan. The database keeps a copy (`items`, one JSON row per entry) for the next start.
- Scans run one after another; ffprobe runs `scanner.threads` times in parallel. Unchanged files keep their probe
  data, ids come from the file paths, a missing folder (unmounted disk) keeps its entries.
- The watch state table exists already, so rows like "Continue watching" work; saving progress follows in step 3.
- Checked against the Java server on the same folders: items, children, rows and search give the same data,
  apart from fields of later steps (watchlist).

## Notes on step 3

- The playback planner is ported completely (direct play, container only, audio only, full conversion).
- Subtitles: `.srt`/`.vtt` files next to the video are converted on the fly (UTF-8 or Windows-1252), text streams
  inside the video are extracted once with ffmpeg and cached in `<cache>/subtitles`.
- The activity view of running playbacks follows with step 6.

## Notes on step 4

- Same design as the Java server: a complete VOD playlist from the start, one ffmpeg run per jump target writing
  one fMP4 fragment per keyframe with the source timestamps, segments assembled from the fragments' `tfdt` times,
  earlier runs kept for jumping back, ffmpeg paused (SIGSTOP) when it is two minutes ahead of the player.
- ffmpeg runs with `nice`/`ionice` on Linux and with below-normal priority on Windows (a process creation flag,
  no PowerShell call). Running conversions end with the server.
- New section `[transcoder]` in `config.toml`: `enabled`, `directory`, `max_sessions`, `hardware_acceleration`
  (`none`, `nvenc`, `qsv`, `vaapi`), `vaapi_device`, `preset`.

## Notes on step 5

Matching is more robust than in the Java server:

- **Ids first:** a TMDB id (`{tmdb-603}`), IMDb id (`{imdb-tt0133093}`, `<imdbid>`, `<uniqueid type="imdb">`) or,
  for shows, TVDB id (`{tvdb-121361}`, `<tvdbid>`) in the name or the `.nfo` leads straight to the entry (TMDB
  `/find` for IMDb and TVDB ids). The tags are removed from the displayed title.
- **Several searches:** the title, the original title, the title without a subtitle after `:` or ` - `, umlauts
  spelled out in the file name (`Schoene` → `Schöne`), `&` written out; with and without the year, and once in
  English. The search stops as soon as a sure match (close title and the right year) turns up.
- **Scoring:** title similarity (edit distance, word containment) against title and original title, the year
  (exact, ±1, off), TMDB's order and popularity only as tie-breakers, entries without votes slightly lower. Without
  a believable title match the entry stays unmatched rather than getting a wrong one.
- **TVmaze** (free, no key; `tmdb.tvmaze`) for shows the TMDB search misses: its IMDb or TVDB id is resolved by
  TMDB exactly.
- **Absolute episode numbers** (`S01E105` in one folder) are mapped onto the TMDB seasons by their episode counts.
- An entry whose title, year or id hints change is searched again at once instead of after three days.
- "Remove match" removes the online data and rescans the library, so the entry shows the data of its files again
  right away.

Configuration: `[tmdb]` with `api_key`, `language`, `region`, `tvmaze`. For tests against a local stand-in the
addresses can be changed with the environment variables `FREEX_TMDB_API`, `FREEX_TMDB_IMAGES` and
`FREEX_TVMAZE_API`. Hand-made collections follow with step 6.

## Notes on step 6

- Playlists, watchlist and hand-made collections live in SQLite (schema 5); they go with their user or library.
- Profile pictures are decoded with the `image` crate (JPEG, PNG, GIF, BMP, WebP; dimensions checked before
  decoding), cropped to the centre square and stored as 256 × 256 PNG in `<data>/avatars`.
- Request bodies over the limit are answered in the API's error format (`error.requestTooLarge`).

## Notes on step 7

- Markers: the audio fingerprint (8 kHz mono, 2048-point FFT, 33 bands, 32-bit hashes per frame) and the
  comparison of neighbouring episodes are ported as they are; results live in SQLite (schema 6) and are tied to
  the file's modification time. Detection runs after scans and on request (`POST /api/libraries/{id}/markers`).
- Watch Together: rooms live in memory; the event stream uses axum's SSE support with a 15 s keep-alive. The
  profile picture of a member is the one at the time of joining.
- Chromecast: the receiver gets URLs with a token that is valid for the length of the title plus six hours;
  the token routes need no login. Casting is counted in the activity view like any playback.
- Packaging is done here (step 9 adds the upgrade path from the Java version and the documentation): the
  Debian package (`packaging/deb/build-deb.sh`, needs `dpkg-deb`) and the Windows installer
  (`packaging/windows/build-installer.sh`, needs `makensis`, `curl`, `unzip`) are built in the Jenkins pipeline
  and archived with the plain archives. The installer no longer ships a Java runtime, only ffmpeg and WinSW; the
  server finds ffmpeg in the `ffmpeg` folder next to its own program.

## Notes on step 8

- The settings page keeps the field names of FreeX Web (`server.bindAddress`); each maps to a key of
  `config.toml`. Changes are written with `toml_edit`, so comments and the layout of the file survive; a
  changed value replaces only the value of its line. The written file is parsed once more before it replaces
  the old one, so the server can always start with it.
- A restart from the settings page happens inside the process: the server loop ends and starts again with the
  configuration as it is now (same PID, so systemd and WinSW see nothing). Stopping with Ctrl+C or SIGTERM
  ends the process as before.
- HTTPS takes a certificate and key as PEM files (`server.tls_certificate`, `server.tls_key`), as Let's Encrypt
  writes them, instead of the Java server's PKCS#12 keystore. Served with `axum-server` and rustls (ring), with
  HTTP/2. The settings page checks new files before they are saved. FreeX Web's HTTPS section was changed
  accordingly (two path fields instead of keystore and password).
- Under HTTP/2 the CSRF check reads the host from the `:authority` pseudo-header.

## Notes on step 9

- The server refuses to open a database of the Java version (recognised by its `recovery_codes` table) instead of
  migrating on top of it; the Debian package and the Windows installer move the Java configuration and data into a
  backup folder on the upgrade.
- A restart from the settings page that fails (the configuration does not start) goes back to the previous file,
  which the settings page keeps as `config.toml.previous`; the failed one stays as `config.toml.failed`.
- README and the documentation describe the Rust server; the Java-specific pages are gone.

## Building

```bash
cargo build --release          # target/release/freex-media-server
cargo test                     # unit tests
./target/release/freex-media-server config.toml
```

The first start writes a commented `config.toml`. Set `FREEX_LOG=debug` for more log output.

### Windows build and Jenkins

The Windows program is cross-compiled on Linux:

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64
cargo build --release --target x86_64-pc-windows-gnu   # target/x86_64-pc-windows-gnu/release/freex-media-server.exe
packaging/dist.sh                                       # dist/*.tar.gz, dist/*.zip, dist/SHA256SUMS
```

The `Jenkinsfile` does the same on a Linux agent with rustup, MinGW-w64, dpkg, NSIS, curl, unzip and python3:
formatting and clippy (warnings fail the build), tests, both release builds, the Debian package, the Windows
installer and the plain archives in `dist/`, which it keeps as artifacts:

```bash
packaging/deb/build-deb.sh 0.20.0 target/release/freex-media-server dist
packaging/windows/build-installer.sh 0.20.0 target/x86_64-pc-windows-gnu/release/freex-media-server.exe dist
```

The installer script downloads ffmpeg (github.com) and WinSW (github.com) once into `~/.cache/freex-build` and
checks ffmpeg against the publisher's checksum (gyan.dev; set `FFMPEG_SHA256` to pin it where gyan.dev is not
reachable).

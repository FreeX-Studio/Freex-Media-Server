<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Development

## Building

```bash
cargo build --release          # target/release/freex-media-server
cargo test                     # unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt
```

`rust-toolchain.toml` pins the toolchain (currently 1.99.1) with rustfmt, clippy and the Windows target, which
rustup installs on the first build; the code itself needs Rust 1.89 or newer. No other build dependency. FreeX Web and the message catalogs are compiled into the
program, so a change to `web/` or `i18n/` needs a rebuild. Run the server from the project directory with
`target/release/freex-media-server`; it creates `config.toml`, `data/` and `cache/` there. Install `ffmpeg` to work
on scanning, thumbnails or conversion. `FREEX_LOG=debug` raises the log level.

### Windows build

The Windows program is cross-compiled on Linux:

```bash
sudo apt install gcc-mingw-w64-x86-64
cargo build --release --target x86_64-pc-windows-gnu   # target/x86_64-pc-windows-gnu/release/freex-media-server.exe
```

### Packages

```bash
packaging/deb/build-deb.sh <version> target/release/freex-media-server dist                                  # needs dpkg-deb
packaging/windows/build-installer.sh <version> target/x86_64-pc-windows-gnu/release/freex-media-server.exe dist   # needs makensis, curl, unzip
packaging/dist.sh                                                                                             # plain archives and SHA256SUMS
```

The installer script downloads ffmpeg (the essentials build by Gyan) and WinSW in fixed versions into
`~/.cache/freex-build` (`FREEX_DOWNLOAD_CACHE`), checks ffmpeg against the publisher's checksum (`FFMPEG_SHA256` pins
it where gyan.dev is not reachable; `WINSW_SHA256` pins WinSW, which has no published checksum) and packs them with
the server. To update one of them, change its version at the top of the script.

### Jenkins

The `Jenkinsfile` runs on a Linux agent with rustup, the MinGW-w64 cross compiler, dpkg, NSIS, curl, unzip and
python3:

```bash
curl https://sh.rustup.rs -sSf | sh -s -- -y
sudo apt install build-essential gcc-mingw-w64-x86-64 dpkg nsis curl unzip python3
```

Stages: toolchain (installs the pinned toolchain), checks (`cargo fmt --check`, clippy with warnings as errors), tests,
both release builds, packaging. Artifacts: the `.deb`, the setup `.exe`, the archives and `SHA256SUMS` in `dist/`; `dist/` is emptied before packaging, so a build only publishes the packages of its own version.
The build needs to reach crates.io, github.com and gyan.dev.

## Project structure

```
src/
  main.rs           entry point: configuration, services, HTTP(S) server, in-process restart
  config.rs         config.toml: defaults, validation, the commented default file
  settings.rs       the settings of the web interface and how they are written back into config.toml
  http.rs           request context, authentication, CSRF and security headers, errors, FreeX Web delivery
  files.rs          sending files with caching headers and byte ranges
  auth.rs           password hashing, tokens, TOTP, brute-force protection
  db.rs             SQLite: schema migrations, users, sessions, libraries, index, watch state, lists, markers
  i18n.rs           message catalogs (i18n/messages_<language>.properties)
  api/              the API endpoints: account, users, server, media, playback, metadata, lists, profile, extras, admin
  media/            library entries, name parser, .nfo reader, ffprobe, scanner, in-memory index, playback planner,
                    subtitles, preview frames, seek preview sheets
  metadata/         TMDB and TVmaze clients, matching, online metadata
  transcode/        conversion sessions: segment plan, ffmpeg command lines, background workers
  markers/          intro and credits detection (audio fingerprints)
  together.rs       Watch Together rooms with server-sent events
  activity.rs       running playbacks for the activity view
  tls.rs            HTTPS from PEM files
web/                FreeX Web (plain JavaScript, no build step), compiled into the program
i18n/               server messages in English and German
packaging/deb/      Debian package: build script, control files, systemd service, default configuration
packaging/windows/  Windows installer: build script, NSIS script, WinSW service definition, icon
docs/               documentation, screenshots and brand assets
```

How the parts work together (requests, data, scanning, playback) is described in [Architecture](architecture.md).

The server is one Rust program on tokio and axum; SQLite is compiled in (`rusqlite`, bundled), TLS uses rustls.
ffmpeg and ffprobe run as external programs. The web client uses no framework; the bundled libraries are hls.js and
a QR code generator.

## Brand assets

The FreeX logo is an X with a play symbol cut out of its centre, in the brand gradient from `#d93b42` to `#f4b75c`.
`docs/brand/` contains the logo (`freex-logo.svg`, `freex-logo-512.png`), the app icon on the dark background
(`freex-app-icon.svg`, `freex-app-icon-1024.png`) for the FreeX apps, and the GitHub social preview
(`social-preview.png`, 1280 × 640). The web interface uses the same logo as `logo.svg` and `icon.svg`, plus
`icons/` (favicon, Apple touch icon, Android icons) and `manifest.webmanifest`, so FreeX can be added to the home
screen of a phone or tablet like an app.

## Translations

FreeX Web and the server are available in English and German. English is the source language: texts are written in
English in the code, and English is used wherever a translation is missing.

- **FreeX Web:** wrap every visible text in `t("…")` (placeholders `{0}`, `{1}`, …), `tn(count, one, other)` for
  plurals and `tParts(…)` when a placeholder is an element. The German texts are in `web/i18n/de.js`, keyed by the
  English text.
- **Server:** messages that reach users (errors, home screen rows, playback reasons, activity, settings validation)
  are keys in `i18n/messages_<language>.properties`. Return them as `ApiError::bad_request("error.someKey").arg(…)`;
  texts in JSON answers are `Text` values, which the response renders in the language of the user (or of the
  `Accept-Language` header before sign-in). Log messages stay English. A test checks that every English key has a
  German translation.

To add a language, add its code to `LANGUAGES` in `src/i18n.rs` and in `web/i18n.js`, create
`i18n/messages_<code>.properties` and `web/i18n/<code>.js`, and load the latter in `index.html`.

## Versioning

FreeX Media Server and FreeX Web are separate products and are versioned independently:

| Product | Version | Format |
|---|---|---|
| FreeX Media Server | `version` in `Cargo.toml` | three parts, e.g. `0.21.0` |
| FreeX Web | `version` in `web/version.json` | four parts, e.g. `1.6.0.0` |

Raise the server version for changes to the server and its API, the web client version for changes to the web
interface. Both appear in the log at startup, under *Settings → Über FreeX* and in `GET /api/status`.

## Conventions

- Rust 2024 edition; `cargo clippy -- -D warnings` and `cargo fmt` clean. New dependencies need a good reason.
- Code, comments and documentation in English; the web interface is English and German.
- Every source file starts with the LGPLv3 license header.
- Keep the security properties described in [Security](security.md) intact: state-changing requests need the token
  header, secrets never reach the browser, uploaded files are never served as they were uploaded.

## Contributing

1. Open an issue to discuss larger changes first.
2. Fork the repository and create a branch.
3. Build and test your change locally (`cargo test`, `cargo clippy`, then run the server against a test library).
4. Open a pull request with a short description of what changed and why.

Please report security problems privately to niklas.linz@mirranet.de.

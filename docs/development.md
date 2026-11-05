<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Development

## Building

```bash
mvn clean package
```

This produces

- `target/freex-media-server.jar`, a self-contained jar with FreeX Web and the SQLite driver,
- `target/freex-media-server_<version>_all.deb`, the Debian package (built with jdeb), and
- `target/freex-media-server-<version>-setup.exe`, the Windows installer, when NSIS is installed.

### Windows installer

The installer is built on Linux with [NSIS](https://nsis.sourceforge.io/): the Maven profile `windows-installer`
switches itself on when `/usr/bin/makensis` exists and runs `src/windows/build-installer.sh`. The script downloads
the bundled programs in fixed versions (Eclipse Temurin JRE 21, the FFmpeg essentials build by Gyan, WinSW) into
`~/.cache/freex-build`, checks them against the checksums their publishers provide, and packs them with the server.
To update one of them, change its version at the top of the script.

On a build server such as Jenkins:

```bash
sudo apt install nsis curl unzip       # once, on the machine that runs the job
mvn clean package                      # builds jar, .deb and setup .exe
```

Archive `target/*.jar, target/*.deb, target/*.exe` as artifacts. The build needs to reach github.com and gyan.dev;
set `FREEX_DOWNLOAD_CACHE` to share the download cache between jobs. `WINSW_SHA256` pins the WinSW file, which
has no published checksum.

Run the server from the project directory with `LANG=C.UTF-8 java -jar target/freex-media-server.jar`; it creates
`config.yml` and `data/` there. Install `ffmpeg` to work on scanning, thumbnails or conversion.

## Project structure

```
src/main/java/de/mirranet/freex/server/
  FreeXMediaServer.java   entry point; wires the services, restarts in-process after settings changes
  auth/                   password hashing, TOTP, brute-force protection
  config/                 config.yml reader and writer, settings definitions
  http/                   web server, API endpoints, file delivery
  json/                   small JSON reader/writer
  markers/                intro and credits detection (audio fingerprints)
  metadata/               TMDB client, local and online metadata
  model/                  libraries, items, media files, users, watch state
  scan/                   library scanner, file name parser, .nfo reader, ffprobe, thumbnails
  store/                  SQLite database, users, lists, library index
  together/               Watch Together rooms
  transcode/              playback planning, ffmpeg/HLS sessions, subtitles
  util/                   ids and file helpers
src/main/resources/web/   FreeX Web (plain JavaScript, no build step)
src/deb/                  Debian package: control files, launcher, systemd service, default configuration
src/windows/              Windows installer: NSIS script, service definition, build script, icon
docs/                     documentation, screenshots and brand assets
```

How the parts work together (requests, data, scanning, playback) is described in [Architecture](architecture.md).

The server depends only on the JDK and the SQLite JDBC driver. The web client uses no framework; the bundled
libraries are hls.js and a QR code generator.

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
  plurals and `tParts(…)` when a placeholder is an element. The German texts are in
  `src/main/resources/web/i18n/de.js`, keyed by the English text.
- **Server:** messages that reach users (errors, home screen rows, playback reasons, activity, settings validation)
  are keys in `src/main/resources/i18n/messages_<language>.properties`. Throw them as
  `ApiException.badRequest("error.someKey", args…)`; texts in JSON answers are `Text` values, which the response
  writes in the language of the user (or of the `Accept-Language` header before sign-in). Log messages stay English.

To add a language, add its code to `Messages.LANGUAGES` and `LANGUAGES` in `web/i18n.js`, create
`messages_<code>.properties` and `web/i18n/<code>.js`, and load the latter in `index.html`.

## Versioning

FreeX Media Server and FreeX Web are separate products and are versioned independently:

| Product | Version | Format |
|---|---|---|
| FreeX Media Server | `<version>` in `pom.xml` | three parts, e.g. `0.9.0` |
| FreeX Web | `version` in `src/main/resources/web/version.json` | four parts, e.g. `1.0.0.0` |

Raise the server version for changes to the server and its API, the web client version for changes to the web
interface. Both appear in the log at startup, under *Settings → Über FreeX* and in `GET /api/status`.

## Conventions

- Java 21, no further runtime dependencies without a good reason.
- Code, comments and documentation in English; the web interface is currently German.
- Every source file starts with the LGPLv3 license header.
- Keep the security properties described in [Security](security.md) intact: state-changing requests need the token
  header, secrets never reach the browser, uploaded files are never served as they were uploaded.

## Contributing

1. Open an issue to discuss larger changes first.
2. Fork the repository and create a branch.
3. Build and test your change locally (`mvn clean package`, then run the jar against a test library).
4. Open a pull request with a short description of what changed and why.

Please report security problems privately to niklas.linz@mirranet.de.

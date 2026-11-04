# Development

## Building

```bash
mvn clean package
```

This produces

- `target/freex-media-server.jar`, a self-contained jar with FreeX Web and the SQLite driver, and
- `target/freex-media-server_<version>_all.deb`, the Debian package (built with jdeb).

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
docs/                     documentation and screenshots
```

The server depends only on the JDK and the SQLite JDBC driver. The web client uses no framework; the bundled
libraries are hls.js and a QR code generator.

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

# Configuration

`config.yml` holds the server settings; libraries and users are managed in the web interface. A commented file is
created on the first start. Keys missing in an older file use the defaults below.

## Settings page

Admins can change most settings in the browser under **Settings → Server**:

- Saving requires the current password. Only the changed values are written back to `config.yml`; comments, order
  and unknown keys are kept, missing keys are added to their section.
- A new port is test-bound and a new TLS keystore test-loaded before anything is written; a new transcoding folder is
  checked for write access.
- Changes take effect after **Restart server**, which restarts the server inside its process in about two seconds.
  Sign-ins stay valid; running playback is interrupted briefly.
- If the new configuration does not start anyway, the server restores the previous file, keeps the failed one as
  `config.yml.failed` and starts again.

The data and cache directories and the ffmpeg/ffprobe paths are shown but can only be changed in the file: through
them the server runs programs and keeps its state, and a stolen admin session must not be able to change that. The
TMDB key and the keystore password are never sent to the browser.

## Reference

| Key | Default | Description |
|---|---|---|
| `server.name` | `FreeX` | Name shown in the web interface and in authenticator apps |
| `server.bindAddress` | `0.0.0.0` | Address to listen on; `127.0.0.1` behind a reverse proxy on the same host |
| `server.port` | `32500` | HTTP(S) port |
| `server.tlsKeystore` | empty | PKCS#12 keystore (`.p12`) for HTTPS in the server itself |
| `server.tlsKeystorePassword` | empty | Password of the keystore |
| `server.trustProxy` | `false` | Trust `X-Forwarded-For/-Proto/-Host`; only behind your own reverse proxy |
| `data.directory` | `data` | Database, index and metadata (relative to the configuration file) |
| `data.cacheDirectory` | empty | Thumbnails, subtitles, transcoding segments; empty = the data directory |
| `ffmpeg.ffprobe` | `ffprobe` | Path to ffprobe |
| `ffmpeg.ffmpeg` | `ffmpeg` | Path to ffmpeg |
| `tmdb.apiKey` | empty | TMDB API key (v3) or read access token (v4); empty = local metadata only |
| `tmdb.language` | `de-DE` | Language of titles and descriptions (English where no translation exists) |
| `tmdb.region` | `DE` | Country for age ratings |
| `markers.enabled` | `true` | Detect intros and end credits of episodes |
| `security.sessionDays` | `30` | A sign-in stays valid this many days without use |
| `security.minPasswordLength` | `10` | Minimum length of new passwords |
| `transcoder.enabled` | `true` | Convert files the client cannot play |
| `transcoder.directory` | `transcode` | Segments of running conversions; relative to the cache directory or absolute (e.g. on an SSD) |
| `transcoder.maxSessions` | `3` | Simultaneous conversions |
| `transcoder.hardwareAcceleration` | `none` | `none`, `nvenc`, `qsv` or `vaapi` |
| `transcoder.vaapiDevice` | `/dev/dri/renderD128` | Render device for VAAPI and Quick Sync on Linux |
| `transcoder.preset` | `veryfast` | x264 preset for software encoding |
| `scanner.intervalMinutes` | `30` | Automatic rescan; `0` = manual only |
| `scanner.threads` | `4` | Files probed in parallel |

The server only ever deletes its own session folders inside `transcoder.directory`, so pointing it at a shared disk
is safe.

## HTTPS

HTTPS is recommended for access from outside your network and required for Chromecast. Two options:

**Reverse proxy** (nginx, Caddy, Traefik, …) terminates TLS and forwards to the server. Set
`server.trustProxy: true` so client addresses and HTTPS are detected correctly, and bind the server to `127.0.0.1` if
the proxy runs on the same host. Forward the `Host`, `X-Forwarded-For` and `X-Forwarded-Proto` headers and allow long
responses (video streams).

**HTTPS in the server**, for example with a Let's Encrypt certificate:

```bash
openssl pkcs12 -export -in fullchain.pem -inkey privkey.pem -out freex.p12 -passout pass:CHANGE_ME
```

```yaml
server:
  tlsKeystore: "freex.p12"
  tlsKeystorePassword: "CHANGE_ME"
```

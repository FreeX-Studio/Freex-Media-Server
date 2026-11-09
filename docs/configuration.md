<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Configuration

`config.toml` holds the server settings; libraries and users are managed in the web interface. A commented file is
created on the first start. Keys missing in an older file use the defaults below; unknown keys are an error, so a
typo cannot silently switch something off.

## Settings page

Admins can change most settings in the browser under **Settings → Server**:

- Saving requires the current password. Only the changed values are written back to `config.toml`; comments and
  the layout of the file are kept, missing keys are added to their section.
- A new address must belong to the machine, a new port is test-bound and new certificate files are test-loaded
  before anything is written; a new transcoding folder is checked for write access.
- Changes take effect after **Restart server**, which restarts the server inside its process in well under a
  second. Sign-ins stay valid; running playback is interrupted briefly.
- If the new configuration does not start anyway, the server restores the previous file, keeps the failed one as
  `config.toml.failed` and starts again.

The data and cache directories and the ffmpeg/ffprobe paths are shown but can only be changed in the file: through
them the server runs programs and keeps its state, and a stolen admin session must not be able to change that. The
TMDB key is never sent to the browser.

## Reference

| Key | Default | Description |
|---|---|---|
| `[server]` `name` | `FreeX` | Name shown in the web interface and in authenticator apps |
| `[server]` `bind` | `0.0.0.0` | Address to listen on; `127.0.0.1` behind a reverse proxy on the same host |
| `[server]` `port` | `32500` | HTTP(S) port |
| `[server]` `tls_certificate` | empty | Certificate with its chain (PEM) for HTTPS in the server itself |
| `[server]` `tls_key` | empty | Private key of the certificate (PEM, unencrypted) |
| `[server]` `trust_proxy` | `false` | Trust `X-Forwarded-For/-Proto/-Host`; only behind your own reverse proxy |
| `[data]` `directory` | `data` | Database, index and metadata (relative to the configuration file) |
| `[data]` `cache_directory` | `cache` | Thumbnails, seek previews, subtitles, transcoding segments; safe to delete while stopped |
| `[security]` `session_days` | `30` | A sign-in stays valid this many days without use |
| `[security]` `min_password_length` | `10` | Minimum length of new passwords |
| `[ffmpeg]` `ffprobe` | `ffprobe` | ffprobe: a name on the PATH or a full path; an `ffmpeg` folder next to the server program is tried first |
| `[ffmpeg]` `ffmpeg` | `ffmpeg` | ffmpeg, likewise |
| `[tmdb]` `api_key` | empty | TMDB API key (v3) or read access token (v4); empty = local metadata only |
| `[tmdb]` `language` | `de-DE` | Language of titles and descriptions (English where no translation exists) |
| `[tmdb]` `region` | `DE` | Country for age ratings |
| `[tmdb]` `tvmaze` | `true` | Ask TVmaze (free, no key) for shows the TMDB search does not find |
| `[markers]` `enabled` | `true` | Detect intros and end credits of episodes |
| `[previews]` `enabled` | `true` | Make seek preview thumbnails after scans (see [Playback](playback.md#seek-previews)) |
| `[transcoder]` `enabled` | `true` | Convert files the client cannot play |
| `[transcoder]` `directory` | `transcode` | Segments of running conversions; relative to the cache directory or absolute (e.g. on an SSD) |
| `[transcoder]` `max_sessions` | `3` | Simultaneous conversions |
| `[transcoder]` `hardware_acceleration` | `none` | `none`, `nvenc`, `qsv` or `vaapi` |
| `[transcoder]` `vaapi_device` | `/dev/dri/renderD128` | Render device for VAAPI and Quick Sync on Linux |
| `[transcoder]` `preset` | `veryfast` | x264 preset for software encoding |
| `[scanner]` `interval_minutes` | `30` | Automatic rescan; `0` = manual only |
| `[scanner]` `threads` | `4` | Files probed in parallel |

The server only ever deletes its own session folders inside `transcoder.directory`, so pointing it at a shared disk
is safe.

## HTTPS

HTTPS is recommended for access from outside your network and required for Chromecast. Two options:

**Reverse proxy** (nginx, Caddy, Traefik, …) terminates TLS and forwards to the server. Set
`trust_proxy = true` so client addresses and HTTPS are detected correctly, and bind the server to `127.0.0.1` if
the proxy runs on the same host. Forward the `Host`, `X-Forwarded-For` and `X-Forwarded-Proto` headers and allow long
responses (video streams).

**HTTPS in the server**, for example with a Let's Encrypt certificate (paths relative to `config.toml` or
absolute; the files must be readable by the user the server runs as):

```toml
[server]
tls_certificate = "/etc/letsencrypt/live/example.org/fullchain.pem"
tls_key = "/etc/letsencrypt/live/example.org/privkey.pem"
```

The server then speaks HTTP/2 and sends `Strict-Transport-Security`. A renewed certificate takes effect with
*Restart server* on the settings page (or a restart of the service).

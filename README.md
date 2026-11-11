<p align="center">
  <img src="docs/brand/freex-logo.svg" alt="FreeX" width="128">
</p>

<h1 align="center">FreeX Media Server</h1>

<p align="center">
  <strong>Your Media. Your Server. Your Freedom.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: LGPL-3.0" src="https://img.shields.io/badge/license-LGPL--3.0-blue"></a>
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.89%2B-orange">
  <img alt="Platforms" src="https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey">
  <img alt="Debian package" src="https://img.shields.io/badge/package-.deb%20%7C%20setup.exe-a80030">
</p>

FreeX Media Server is a self-hosted, open-source media server for your movies and TV shows. It organizes your
folders into a library with artwork and metadata, remembers where every user stopped, and streams to any modern
browser, converting on the fly when a device cannot play a file. No account with a third party, no cloud, no
telemetry: your media stays on your hardware.

It is the core of **FreeX**, a family of apps around one server: the web client **FreeX Web** is built in, and
there are apps for TVs, phones and tablets and the desktop (see [The FreeX family](#the-freex-family)).

<p align="center">
  <img src="docs/screenshots/library.png" alt="Movie library with poster grid" width="49%">
  <img src="docs/screenshots/show.png" alt="Detail page of a TV show" width="49%">
</p>
<p align="center"><sub>The titles and artwork in the screenshots are made up for the demo.</sub></p>

## Highlights

- **Beautiful library** – home screen with *Continue watching*, *Up next* and *Recently added*, poster grids,
  collections, playlists, watchlist and search
- **Plays everything** – direct play when the browser can decode the file, otherwise on-the-fly conversion to HLS
  with ffmpeg, with optional hardware acceleration (NVENC, Quick Sync, VAAPI)
- **Rich metadata** – from [TMDB](https://www.themoviedb.org/) with your own free key, or fully offline from Kodi
  `.nfo` files and local artwork
- **Skip intro and credits** – detected automatically from the audio, with a *Next episode* countdown
- **Chromecast** – cast from Chrome, with the browser as remote control
- **Jellyfin apps** – optionally, native apps made for Jellyfin (Android TV and Fire TV, Findroid, Swiftfin) can use
  your server, with Quick Connect sign-in
- **DLNA** – optionally, smart TVs, consoles and players in the home network find your server on their own
- **Watch Together** – watch in sync with other users of your server
- **Multi-user** – share all or only selected libraries per user; personal watch state, history, profile pictures
  and audio/subtitle language preferences
- **Secure by default** – two-factor sign-in, brute-force protection, per-device sessions, strict security headers
- **Easy to run** – one small native program without a runtime, a Debian package with a hardened systemd service,
  or a Windows installer that brings ffmpeg along
- **Managed in the browser** – libraries, users and server settings, a live activity view, CPU and memory graphs,
  database upkeep and the server log for admins
- **Your language, your sidebar** – English and German interface and server messages per user; pin and unpin
  sidebar entries

## The FreeX family

All apps talk to the same server with the same account; watch state, *Continue watching* and settings follow you
from device to device. They play with the device's own player, so many files play directly, and the server converts
whatever a device cannot decode.

| App | Platforms | Repository |
|---|---|---|
| **FreeX Media Server** | The core: libraries, users, streaming, conversion and the API; Linux and Windows | this repository |
| **FreeX Web** | Web interface for every modern browser, built into the server | this repository (`web/`) |
| **FreeX TV** | Samsung Smart TVs (Tizen 4.0+, models from 2018) with the TV's own player; Fire TV and Android TV with ExoPlayer. A home screen and player made for the remote | [FreeX-Studio/FreeX-TV-Apps](https://github.com/FreeX-Studio/FreeX-TV-Apps) |
| **FreeX Mobile** | iPhone and iPad (iOS 15+, AVPlayer, AirPlay, picture in picture) and Android phones and tablets (Android 7+, ExoPlayer); a player made for touch | [FreeX-Studio/FreeX-Mobile](https://github.com/FreeX-Studio/FreeX-Mobile) |
| **FreeX Desktop** | Windows and Linux app with FreeX Web in its own window; plays every file directly with mpv, several servers, media keys, tray icon | [FreeX-Studio/freex-desktop](https://github.com/FreeX-Studio/freex-desktop) |

Devices without a FreeX app can still play from the server: optionally through native
[Jellyfin apps](docs/jellyfin.md) or as a [DLNA](docs/dlna.md) server in the home network.

Server and web client are versioned separately: the server uses the version in `Cargo.toml` (e.g. `0.21.0`),
FreeX Web the version in [`web/version.json`](web/version.json) (e.g. `1.6.0.0`). Both are shown under
*Settings → About FreeX*.

## Quick start

### Debian / Ubuntu

Add the APT repository (see [APT repository](docs/apt-repository.md)) and install with
`sudo apt install freex-media-server`, or take the package from a release or your CI (or build it, see
[Development](docs/development.md)) and install it; ffmpeg comes in as a dependency:

```bash
sudo apt install ./freex-media-server_<version>_amd64.deb
```

Open `http://<your-server>:32500/`, create the admin account and add your movie and TV show folders. The service
runs as the user `freex`, which needs read access to your media, for example
`sudo setfacl -R -m u:freex:rX /srv/media`.

### Windows

Run `freex-media-server-<version>-setup.exe`. It brings ffmpeg along, sets up a Windows service and opens the web
interface when it is done.

### Any Linux or Windows system

Unpack the archive of a release (or build with `cargo build --release`) and start the program:

```bash
./freex-media-server            # uses config.toml in the current folder, creates it on the first start
./freex-media-server /etc/freex/config.toml
```

Install `ffmpeg` for media info, preview images and conversion.

See [Installation](docs/installation.md) for details, paths, updates and running as a service.

## Documentation

| | |
|---|---|
| [Installation](docs/installation.md) | Debian package, Windows installer, manual setup, file locations, backups |
| [APT repository](docs/apt-repository.md) | Your own APT repository for the Debian package: setup with reprepro, Jenkins publishing, client sources |
| [Configuration](docs/configuration.md) | All settings, the settings page, HTTPS and reverse proxies |
| [Organizing your media](docs/media-library.md) | Folder and file naming, local artwork, `.nfo` files, TMDB |
| [Playback](docs/playback.md) | Direct play and conversion, hardware acceleration, subtitles, skip intro, Watch Together, Chromecast |
| [Jellyfin apps](docs/jellyfin.md) | Using native Jellyfin apps with FreeX: setup, sign-in, playback, limits |
| [DLNA](docs/dlna.md) | Playing on smart TVs and other DLNA players: setup, firewall, what devices see, limits |
| [Security](docs/security.md) | Sign-in, two-factor authentication, sessions and how the server is hardened |
| [Architecture](docs/architecture.md) | How the server works inside: requests, data, scanning, playback, FreeX Web |
| [API](docs/api.md) | The JSON API used by FreeX Web and the FreeX apps |
| [Changelog](CHANGELOG.md) | What is new in each release |
| [Roadmap](docs/roadmap.md) | What is still missing, and in which order it comes |
| [Development](docs/development.md) | Project structure, building, packaging, versioning and contributing |

## Requirements

- Linux (x86_64, glibc 2.34 or newer, e.g. Debian 12 / Ubuntu 22.04) or Windows 10/11 (64-bit); other systems
  build from source with Rust 1.89 or newer
- `ffmpeg` and `ffprobe` (recommended; without them only files the browser can play directly work)
- A modern browser: Chrome, Edge, Firefox or Safari, on desktop or mobile

## Roadmap

FreeX Media Server is under active development and has not reached 1.0 yet. The [roadmap](docs/roadmap.md) lists
what is still missing: next up are offline downloads for the apps, then music and photo libraries.

## Contributing

Bug reports, ideas and pull requests are welcome. Please open an issue first for larger changes so we can agree on
the approach. See [Development](docs/development.md) for how to build and test the server. Please report security
problems privately to niklas.linz@mirranet.de instead of opening a public issue.

## License

FreeX Media Server is free software under the [GNU Lesser General Public License v3.0](LICENSE).

It bundles the font Manrope (SIL Open Font License), [hls.js](https://github.com/video-dev/hls.js) (Apache 2.0) and
[qrcode-generator](https://github.com/kazuhikoarase/qrcode-generator) (MIT); their licenses are in `web/`.
Metadata can be loaded from TMDB: this product uses the TMDB API but is not endorsed or certified by TMDB.

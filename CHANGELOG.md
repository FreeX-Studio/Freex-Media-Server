<p align="center"><img src="docs/brand/freex-logo.svg" alt="FreeX" width="72"></p>

# Changelog

## FreeX Media Server 0.21.0 · FreeX Web 1.6.0.0

- **Seek previews:** hovering over or dragging the seek bar shows a frame of that point with its time. The server
  makes one sprite sheet per movie and episode after scans, in the background and at low priority, from the
  keyframes only (see [Playback](docs/playback.md#seek-previews)); `[previews] enabled` switches it off
- **Chapters** from the file: marks in the seek bar, the chapter name above it while scrubbing, a chapter menu in
  the player and `Page Down` / `Page Up` to jump between chapters (see [Playback](docs/playback.md#chapters)). The
  item details have `media.chapters` (`startMs`, `endMs`, `title`); files are probed once more on the next scan to
  read them
- **Trailers and extras from disk:** `Trailers/`, `Featurettes/`, `Behind The Scenes/` and the other extras folders
  next to a movie or in a show, and files named `-trailer`, `-featurette` and so on, are shown under *Trailers and
  extras* on the item page and play there (see [Media library](docs/media-library.md#trailers-and-extras)).
  Before, they were skipped; `-trailer` files over 300 MB were taken for movies
- **Several versions of a movie** (4K and 1080p, Director's Cut) in one folder are one entry with a version picker
  on the item page and in the player; the versions share their watch state (see
  [Media library](docs/media-library.md#several-versions-of-a-movie))
- **Library filters:** watch state, genre, years, rating, resolution, age rating, recently added, actor or director
  and title, in a filter panel on the *Library* tab; a filter can be saved as a **smart collection** that fills
  itself and shows every user their own watch state (see
  [Media library](docs/media-library.md#filters-and-smart-collections))
- **Parental controls:** an age limit per user (with or without titles that have no rating) hides everything rated
  above it in every list, the search and playback; age ratings also come from `.nfo` files. **Switching users with a
  PIN** on a shared screen, for every account except admins (see [Security](docs/security.md#parental-controls))
- **Quality per user and network:** a default quality for the home network and for on the go, kept on the server
  and remembered from the player; *Automatic* plays the original at home and picks the quality from the measured
  connection on the go (see [Playback](docs/playback.md#how-a-file-is-played))
- **HDR tone mapping:** converted HDR10, HLG and Dolby Vision (profiles 7 and 8) videos are tone mapped to SDR, in
  software or with VAAPI, and keep their colours; thumbnails and seek previews too. The item page shows HDR
  (see [Playback](docs/playback.md#hdr)). Files are probed once more on the next scan
- **Audio passthrough:** the sound is copied whenever the player decodes it, also when the video is converted, so
  AC3, E-AC3 and other surround tracks stay 5.1/7.1; DTS and TrueHD become E-AC3 or AC3 5.1 for players that decode
  those, AAC stereo otherwise
- FreeX Web 1.6.0.0: seek previews and chapters in the player; trailers and extras and the version picker on the item page; the filter panel and smart collections; parental controls in the user administration, the PIN under Account and *Switch user* in the account menu; *Automatic* quality and the quality defaults under Account; a switch for the previews in the server settings

## FreeX Media Server 0.20.0 · FreeX Web 1.5.0.0

**The server is now written in Rust.** One native program of a few megabytes that starts instantly, needs a
fraction of the memory of the Java version and no runtime: the Debian package depends only on ffmpeg, the Windows
installer ships ffmpeg and the service wrapper. The API and FreeX Web are unchanged, so FreeX Desktop and the
upcoming apps keep working; playback and conversion are still done by ffmpeg exactly as before.

- **Metadata matching** finds the right title more often: TMDB, IMDb and TVDB ids in file names
  (`{imdb-tt0133093}`) and `.nfo` files lead straight to the entry; the search tries the original title, the title
  without a subtitle, spelled-out umlauts and English; results are scored by title similarity and year; TVmaze
  (free, no key) is asked for shows the TMDB search misses; absolute episode numbers are mapped onto seasons;
  renamed entries are searched again at once instead of after three days
- **HTTPS** takes a certificate and key as PEM files (as Let's Encrypt writes them) instead of a PKCS#12 keystore,
  and speaks HTTP/2
- **Configuration** is `config.toml`; the settings page writes it keeping comments and layout, and a configuration
  that does not start is rolled back automatically
- **Restart** from the settings page takes well under a second
- Profile pictures also accept BMP and WebP
- Packages: `.deb` for amd64 with the hardened systemd service, Windows installer with service; both move the
  configuration and data of the Java version to a backup folder on the upgrade, since the new server starts fresh
  (see [Installation](docs/installation.md))
- FreeX Web 1.5.0.0: the HTTPS section of the server settings has fields for certificate and key; a switch for
  TVmaze

## FreeX Media Server 0.14.0 · FreeX Web 1.4.0.0

**FreeX Desktop: the Windows app.** [FreeX Desktop](https://github.com/LinzN/freex-desktop) shows FreeX Web from
your server in its own window and can play every file directly with mpv. This release adds what the app needs:

- FreeX Web recognizes the app and adds *Settings → FreeX Desktop* (player choice, switch server, app version),
  *Switch server* in the account menu, on the sign-in page and when the server cannot be reached, and the app
  version under *About FreeX*
- Playback in mpv: the file plays directly without conversion, the position and watch state are saved as usual,
  and the next episode starts after a short countdown
- The player's full screen also makes the app window full screen
- API: `POST /api/items/{id}/playback` accepts `"player": "external"` for native players that decode every format
  themselves; the server then always plans direct play and lists the playback in the activity view

## FreeX Media Server 0.13.0 · FreeX Web 1.3.2.0

**The first release of FreeX: your media, your server, your freedom.**

FreeX Media Server is a self-hosted, open-source media server for movies and TV shows. It turns your folders
into a library with artwork and metadata, remembers where every user stopped and streams to any modern browser,
converting on the fly when a device cannot play a file. No third-party account, no cloud, no telemetry.

This release contains the server and its built-in web client **FreeX Web**. Native apps for iOS, Android and TV
will follow and use the same API.

### Highlights

- Libraries for movies and TV shows with artwork and metadata from TMDB, or fully offline from `.nfo` files
- Plays everything: direct play when the browser can, otherwise conversion in a background worker per playback,
  with hardware acceleration (NVENC, Quick Sync, VAAPI)
- Instant seeking in converted streams, with picture and sound in sync across every jump
- Skip intro and credits, detected automatically
- Watch Together and Chromecast
- Multiple users with library sharing, personal watch state and their own display language (English, German)
- Two-factor sign-in, hardened by default
- Debian/Ubuntu package and Windows installer, both with a system service

### Library and metadata

- Movie and TV show libraries from any number of folders, with periodic and manual scans; unchanged files are not
  probed again, and a missing folder (e.g. an unmounted disk) keeps its titles
- Robust recognition of titles, years, seasons and episodes, including scene release names
- Metadata and artwork from [TMDB](https://www.themoviedb.org/) with your own free key, merged with local `.nfo`
  files and artwork; fix a wrong match from the detail page
- Home screen with *Continue watching*, *Up next* and *Recently added*, poster grids with sorting and filters,
  collections, people, trailers and search
- Playlists, watchlist, own collections and a personal watch history

### Playback

- **Direct play** when the browser can decode the file, **direct stream** (video copied, sound converted if
  needed) or **transcode** to H.264, decided per file and device
- Converted streams cover the whole title from the start: the player seeks like in any video and keeps a minute
  ahead and two minutes back buffered. A background worker converts whatever part is asked for, restarts ffmpeg
  within about a second after a far jump and reuses what it has converted before
- Picture and sound stay in sync over the whole film and every jump (measured within 3–16 ms, as with direct play)
- ffmpeg runs with low CPU and disk priority, pauses while it is far ahead and stops when nobody watches
- Quality from *Original* down to 1 Mbit/s; audio track and subtitles selectable, with preferred languages per user
- Subtitles from `.srt` and `.vtt` files next to the video and embedded text subtitles (SRT, ASS/SSA, WebVTT,
  MP4 text and more), shown as WebVTT; embedded bitmap subtitles (PGS, VobSub) are burned into the picture
- Skip intro and credits from audio fingerprints, with a *Next episode* countdown
- Reliable player: retries network errors, resumes conversions the server has ended, restarts stuck playback by
  itself, close button, full-screen controls (also with Chrome's hardware overlay on Windows), volume with the
  arrow keys and the mouse wheel, keyboard shortcuts
- **Watch Together**: watch in sync with other users; the room waits while someone is loading and evens out small
  differences without jumps
- **Chromecast** from Chrome, with the browser as remote control

### Users and security

- Multiple users; admins share all libraries or selected ones per user, enforced on every request
- Profile pictures, preferred audio and subtitle languages, display language (English or German, or the browser's)
- Server messages in the user's language; errors carry a stable code for apps
- Pinnable sidebar entries, collapsible sidebar on desktop and an icon rail on phones
- PBKDF2 password hashes, two-factor sign-in (TOTP) with recovery codes, brute-force protection
- Sessions per device that can be ended, login history, strict security headers, CSRF protection
- Installable as a web app (PWA)

### Administration

- Libraries, users and server settings in the browser; settings that need it are applied by an in-process restart,
  which falls back to the last working configuration if the new one does not start
- Live activity view of all playbacks and scans
- Separate versions of server and web client under *Settings → About FreeX*; open pages load a new FreeX Web
  automatically after an update

### Installation

- **Debian / Ubuntu**: `.deb` package following the Filesystem Hierarchy Standard, with a hardened systemd service,
  its own system user and the address of the web interface printed after installing
- **Windows 10/11**: `setup.exe` that brings Java and ffmpeg along, runs the server as a Windows service, keeps
  configuration and data in ProgramData and sets up the firewall; built on Linux with NSIS, for example on Jenkins
- **Any system with Java 21**: one self-contained jar

See [Installation](docs/installation.md) for details.

### Good to know

- FreeX is before 1.0: the [API](docs/api.md) may still change between versions.
- Tested mostly in Chrome and Chromium. Firefox, Safari, iPhone/iPad and Chromecast use the same standard streams,
  but have been tested less; feedback is welcome.
- The Windows service runs as *Local System*, which cannot reach network shares; use another service account for
  media on a NAS.
- Music and photo libraries, parental controls and downloads for offline playback are planned for later releases.

### Requirements

- Java 21 or newer (included in the Windows installer)
- ffmpeg and ffprobe (included in the Windows installer; a dependency of the Debian package)
- A modern browser: Chrome, Edge, Firefox or Safari, on desktop or mobile

FreeX Media Server is free software under the [GNU Lesser General Public License v3.0](LICENSE).

<p align="center"><img src="docs/brand/freex-logo.svg" alt="FreeX" width="72"></p>

# Changelog

## Unreleased

- **Built-in TMDB key:** release builds can carry a TMDB key (environment variable `FREEX_TMDB_KEY` at build time,
  set in Jenkins); servers whose `tmdb.api_key` is empty use it, a key of their own still takes precedence. The
  *About* page names TMDB as the source of the metadata, as the TMDB API terms require
- **FreeX apps by name:** signed-in devices, the sign-in history and the activity view show FreeX Mobile, FreeX TV
  and FreeX Desktop with their version and platform (e.g. "FreeX Mobile 1.0.2 on Android") instead of the browser
  engine they run on. FreeX Desktop needs version 1.1.1 or newer for this
- **API for the FreeX TV apps:** CORS for apps whose pages have an origin of their own (FreeX TV on Samsung TVs and
  Fire TV), without credentials; from another origin, state-changing requests need the token header, sign-in a
  JSON body, and the first setup stays with the server's own pages. `POST /api/items/{id}/playback` takes
  `"tokenUrls": true` for players that cannot send the token: the stream URL then carries its own access token, like
  for a Chromecast (see [API](docs/api.md))
- **Faster start of copied video:** for MKV files that are played as HLS with the video copied (direct stream), the
  keyframes are read from the file's index in a few small reads instead of one ffprobe seek per two seconds of
  video, which took up to half a minute on hard disks and network shares when the title's page had not been opened
  first (e.g. from *Continue watching*, the Jellyfin apps or DLNA). A start while the page is still reading them
  waits for that instead of reading the file a second time
- **Better matching:** titles are also searched without a numbering in front (`2 - Pippi geht von Bord`), a scene
  group in front of lower-case release names (`7sins-jackass…`) and a studio in front (`Marvels What If`); when
  nothing is found, with their first words (`The Continental Aus der Welt von John Wick` → `The Continental`), whose
  results need a close title or the right year. Show folders named like season packs (`Show.S01.German…`) no
  longer keep `S01` in the title. Entries without a match are searched again right after the update
- **Jellyfin apps start faster with subtitles:** when the app plays the file itself, subtitles inside the file are
  offered as embedded (as Jellyfin does) to players that read them from the file (mpv, VLC, ExoPlayer), instead of
  as a separate file the server first has to extract, which for a large file meant reading all of it before the
  player started
- **Jellyfin layer log:** each request is logged (at debug level) with the requested byte range and the time
  until the answer starts, so a slow start can be told apart: server, disk or network
- **Swiftfin 1.6 plays directly:** it loads the file without a token (Jellyfin serves it to anyone), which the
  layer answered with 401. The file is now served for the `PlaySessionId` of the app's own playback info, for that
  title, for a day and only while the app is signed in
- **10-bit HEVC in the browser:** FreeX Web asks the browser for HEVC Main10 (HDR, most 4K) separately; browsers
  that decode HEVC only with 8 bits get such files converted instead of copied, which failed in the browser's
  decoder. If the browser still fails on copied video, the player switches to a full conversion by itself and
  plays on
- **Playback errors in the server log:** failed segments are logged with what ffmpeg said, and the player writes
  errors it cannot recover from to the server log (title, user, device, cause), so *Details are in the server log*
  holds true
- **Season names in the viewer's language:** seasons without a name of their own are shown as "Season 2" or
  "Staffel 2" by the display language of the user (or, when the user follows the browser, the server's metadata
  language), in FreeX Web and the Jellyfin apps; before, they were always "Staffel 2"
- **`.nfo` files:** numeric character references such as `&#x27;` and `&#233;` are decoded, so titles like *The
  Cartographer's Daughter* no longer show `&#x27;`
- **Screenshots** in the README show the current design with a made-up English demo library
- **APT repository:** the Jenkins build can publish the Debian package to an APT repository of your own
  (reprepro, `packaging/apt/`, stage *Publish* when `APT_REPO` is set), so servers install and update FreeX with
  `apt`. Every build gets its own, increasing package version (`0.21.0+202611110746.git7ccb57a`). See
  docs/apt-repository.md
- **Search by the folder name:** when the title finds nothing, a movie in a folder of its own is searched by its
  folder name (scene files like `exq-captainphillips.mkv` in `Captain.Phillips.German.DL…`), or by its file name
  when the title came from the folder; a show by the name in its episode files. The search also leaves out version
  and quality words at the end (`DC`, `Uncut`, `part 1`, `1080`, `webhd`) and parts in brackets, and tries the title
  without a series in front (`Edgar Wallace …`)
- **Refresh the metadata of a whole library** with *Refresh metadata* under *Settings → Libraries*. Refreshing
  (a library, a single entry, or the monthly refresh) now keeps the match of entries whose name did not change and
  only loads their data again, so a working match cannot be replaced; entries without a match are searched again
- **Activity view:** a playback is listed as soon as it is requested, as *Starting* until its conversion runs
- **FreeX Desktop for Linux:** *About FreeX* names the desktop app as Linux or Windows app, depending on where it runs

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
- **Subtitle look:** size, background, position and colour of text subtitles per user (*Account → Playback*, the size
  also in the player); text subtitles can be **burned into the picture** from the player menu
- **Server dashboard** (*Settings → Dashboard*): CPU, memory, bandwidth and conversions right now, plays per day,
  per user and per title, and what each library holds (size, length, resolutions, codecs, HDR); the activity view
  shows the bandwidth of each playback and the CPU of each conversion
- **Nightly maintenance** (`[maintenance]`, 03:00 by default): a backup of the database (seven kept in
  `<data>/backups`), clean-up of thumbnails and subtitles of removed or replaced files, and a rescan of all libraries;
  *Run now* and the last report on the dashboard
- **Webhooks** (*Settings → Webhooks*): an HTTP POST with JSON on play, pause, resume, stop, watched and new titles,
  signed with HMAC-SHA256, with a test button and the outcome of the last call (see [API](docs/api.md#webhooks))
- **Editing metadata by hand** (admins, *… → Edit metadata*): title, sort title, year, rating, age rating, genres,
  tagline, summary and own poster and background; edited fields stay through scans and refreshes until reset
- **Theme music:** a `theme.mp3` in the folder of a show or movie plays quietly on its pages (the show's also on its
  seasons), with a button to switch it off on the device
- **New look of FreeX Web:** translucent glass surfaces (panels, menus, dialogs, the player's controls as a
  capsule), a sidebar that runs as a tinted column from the top of the window, over a soft light from the brand
  colours, rounder corners that grow with the surface, capsule buttons, a segmented control for the library tabs,
  artwork that lifts on hover, and the system's own typeface (SF Pro on Apple devices, Segoe UI Variable on
  Windows); without translucency on systems that ask for less
- **Smoother playback while the server is busy:** scans, intro detection and seek previews no longer hold the
  database or the library index while they write, run their ffmpeg and ffprobe at low priority and pause while
  someone is watching; reading requests use their own database connections, so pages and the player answer during a
  scan. Seek previews and the intro detection use several cores (a quarter of them, at most three)
- **Faster start:** converted video is cut into two-second segments, so the picture appears after two seconds of
  encoding instead of four; the keyframes a copied video is cut at are read when its page is opened and kept on disk
  (`cache/keyframes/`), so pressing play does not wait for them, also after a restart
- A library opens in the view last used on the device (Recommended, Library with its sorting and filters, or
  Collections)
- **Performance graphs:** the dashboard shows CPU (system, FreeX, conversions) and memory (system, FreeX) of the last
  hour or day as graphs, with a crosshair, values on hover and a table view (Linux; `GET /api/admin/performance`)
- **Database upkeep:** faster settings for every connection (`synchronous = NORMAL` with WAL, a larger page cache,
  memory-mapped reads), fresh planner statistics every hour, and an optimization in the nightly maintenance and on
  the dashboard: integrity check, expired sessions removed, `ANALYZE`, `VACUUM` when much space is free (or on
  request) and an empty write-ahead log. The dashboard shows size, free space, the largest tables and the last run
- **Server log:** *Settings → Log* shows the last 5000 lines live, filtered by level and text, as a download, and
  the level of detail can be raised while the server runs (e.g. only for the Jellyfin apps)
- **Jellyfin apps:** native apps made for Jellyfin (Jellyfin for Android TV and Fire TV, Findroid, Swiftfin, ...)
  can use the server at `<address>/jellyfin`, switched on with `[jellyfin] enabled` (off by default). Libraries,
  home rows, seasons and episodes, search, watch state, favorites (the watchlist), intro and credits skipping, movie
  versions, extras, direct play and conversion to HLS from the app's device profile, subtitles, and the activity view.
  Sign-in with password (accounts without two-factor sign-in) or Quick Connect, confirmed in FreeX Web under
  *Settings → Security* (`POST /api/me/quick-connect`). Image links for the apps are signed. See docs/jellyfin.md
- **DLNA:** smart TVs, consoles and players in the home network find the server on their own (SSDP) and browse
  *Continue watching*, *Recently added* and the libraries, with shows as folders of seasons and episodes, posters,
  external and embedded text subtitles (also the Samsung `CaptionInfo.sec` header) and resume points. Playback is
  direct only and shows in the activity view; watch progress is saved for the account set in `[dlna] user`. Off by
  default, on its own plain HTTP port (`[dlna] port`, 32501) that answers only the home network. See docs/dlna.md
- **Search preview:** typing in the search field shows the first movies, shows and episodes in a list below it,
  with the matching part of the title marked; arrow keys and Enter open a hit, Enter without a pick or *Show all
  results* the search page. `GET /api/search` takes an optional `limit` per kind
- **Letter bar:** a library sorted by title has an A–Z bar on the right; a click or a drag along it jumps to the
  first title of that letter (leading articles are skipped as in the sort, `Ä` counts as `A`, digits and signs as
  `#`), and the letter at the top of the view is marked while scrolling. The library list has `sortTitle` per entry
- FreeX Web 1.6.0.0: seek previews and chapters in the player; trailers and extras and the version picker on the
  item page; the filter panel and smart collections; parental controls in the user administration, the PIN under
  Account and *Switch user* in the account menu; *Automatic* quality and the quality defaults under Account; the
  subtitle look and burning in text subtitles; the server dashboard with the maintenance; webhooks; editing metadata
  in the item menu; theme music; a switch for the previews in the server settings

## FreeX Media Server 0.20.0 · FreeX Web 1.5.0.0

**The first release of FreeX Media Server: your media, your server, your freedom.** A self-hosted, open-source
media server for movies and TV shows: one native program of a few megabytes without a runtime, with the web client
**FreeX Web** built in. No third-party account, no cloud, no telemetry.

### Library and metadata

- Movie and TV show libraries from any number of folders, with periodic and manual scans; unchanged files are not
  probed again, and a missing folder (e.g. an unmounted disk) keeps its titles
- Recognition of titles, years, seasons and episodes, including scene release names and absolute episode numbers
- Metadata and artwork from [TMDB](https://www.themoviedb.org/) with your own free key, merged with local `.nfo`
  files and artwork; TMDB, IMDb and TVDB ids in file names (`{imdb-tt0133093}`) and `.nfo` files lead straight to
  the entry; TVmaze (free, no key) for shows TMDB misses; fix a wrong match from the detail page
- Home screen with *Continue watching*, *Up next* and *Recently added*, poster grids with sorting and filters,
  collections, people, trailers and search; playlists, watchlist, own collections and a watch history

### Playback

- **Direct play** when the browser can decode the file, **direct stream** (video copied, sound converted if
  needed) or **transcode** to H.264 with ffmpeg, decided per file and device, with hardware acceleration (NVENC,
  Quick Sync, VAAPI)
- Converted streams cover the whole title from the start and seek like any video, with picture and sound in sync
  across every jump; ffmpeg runs with low priority and stops when nobody watches
- Quality from *Original* down to 1 Mbit/s; audio tracks and subtitles (external and embedded, bitmap subtitles
  burned in) with preferred languages per user
- Skip intro and credits from audio fingerprints, with a *Next episode* countdown
- **Watch Together** with other users of the server, and **Chromecast** from Chrome

### Users, security and administration

- Multiple users with library sharing, profile pictures, language preferences and the display language (English
  or German)
- Two-factor sign-in (TOTP) with recovery codes, brute-force protection, sessions per device, login history,
  strict security headers
- HTTPS with a certificate and key as PEM files (as Let's Encrypt writes them), with HTTP/2
- Libraries, users and server settings in the browser; the configuration (`config.toml`) is written keeping
  comments and layout, settings that need it are applied by a restart that takes well under a second and falls
  back to the last working configuration
- Live activity view of all playbacks and scans
- API for native players (`"player": "external"`), used by FreeX Desktop to play every file with mpv

### Installation

- **Debian / Ubuntu**: `.deb` for amd64 with a hardened systemd service and its own system user; depends only on
  ffmpeg
- **Windows 10/11**: `setup.exe` that brings ffmpeg along and runs the server as a Windows service
- **Any Linux or Windows system**: plain archives with the program

See [Installation](docs/installation.md) for details.

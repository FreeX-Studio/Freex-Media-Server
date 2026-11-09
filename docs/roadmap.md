<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Roadmap

What FreeX Media Server does not do yet, and in which order it comes. Status as of 0.21.0 (November 2026).
Everything in FreeX is and stays free; nothing on this page will need an account with a third party.

## Phase 1 – rounding off movies and shows

The gaps that show in daily use. All of them fit the current architecture.

| Feature | Notes |
|---|---|
| ~~**Seek preview thumbnails**~~ | **Delivered in 0.21.0** – see [Playback](playback.md#seek-previews). |
| ~~**Chapters**~~ | **Delivered in 0.21.0** – see [Playback](playback.md#chapters). |
| ~~**Trailers and extras from disk**~~ | **Delivered in 0.21.0** – see [Media library](media-library.md#trailers-and-extras). |
| ~~**Multiple versions of a title**~~ | **Delivered in 0.21.0** – see [Media library](media-library.md#several-versions-of-a-movie). |
| ~~**Library filters and saved filters**~~ | **Delivered in 0.21.0** (smart collections) – see [Media library](media-library.md#filters-and-smart-collections). |
| ~~**Parental controls**~~ | **Delivered in 0.21.0** – age limit per user and switching users with a PIN, see [Security](security.md#parental-controls); hidden libraries are the library sharing per user. |
| ~~**Per-user quality defaults**~~ | **Delivered in 0.21.0** – see [Playback](playback.md#how-a-file-is-played). |
| ~~**HDR tone mapping**~~ | **Delivered in 0.21.0** – see [Playback](playback.md#hdr). |
| ~~**Audio passthrough and multi-channel**~~ | **Delivered in 0.21.0** – see [Playback](playback.md#how-a-file-is-played). |
| **Subtitle styling and burn-in options** | Font size and position for text subtitles in FreeX Web; the option to burn text subtitles into the picture. |
| **Server dashboard** | Current bandwidth and CPU per playback, play statistics per user and title, library statistics. Today: the activity view. |
| **Scheduled maintenance** | A nightly window for rescans, metadata refresh, marker detection, cache clean-up and a database backup. Today: periodic scans only. |
| **Webhooks** | HTTP calls on play, pause, stop and new items, for home automation. |
| **Delete files from the library** | Delete a movie or episode from disk from the item page (admins only, with confirmation). |
| **Manual metadata editing** | Edit title, summary, artwork and genres by hand and lock them against refreshes. Today: fix match only. |
| **Theme music** | Play a show's theme on its page. Low priority. |

## Phase 2 – more kinds of media and the apps

| Feature | Notes |
|---|---|
| **Native apps** | The biggest gap for the living room. The API is ready: Android TV / Fire TV first, then iOS and Android, then Apple TV. |
| **Music library** | Artists, albums and tracks from tags (ID3, Vorbis, FLAC), artwork, gapless playback, playlists; metadata from MusicBrainz. A new library type and a new player. |
| **Photo library** | Folders and dates, timeline, EXIF, thumbnails, slideshows. |
| **Other videos** | A library type without metadata lookups, for home videos and recordings; titles from file names only. |
| **Offline downloads** | Once the apps exist: download a title in a chosen quality (pre-converted on the server) for playback without a connection. |
| **Pre-conversion** | Convert titles in advance to a target quality, stored on disk, for weak servers and for downloads. |
| **Remote access setup** | Guided setup with UPnP/NAT-PMP and a dynamic-DNS helper; today port forwarding or a reverse proxy is set up by hand. FreeX stays usable without any external service. |
| **Inviting friends** | Invitations by link or e-mail, and managed users without a password who switch with a PIN. |

## Phase 3 – beyond the home library

| Feature | Notes |
|---|---|
| **Ratings and reviews** | Personal ratings and reviews per user, shown to the other users of the server. |
| **More metadata sources** | TVDB, AniDB for anime, MusicBrainz for music; selectable per library. |
| **DLNA** | Serving to DLNA renderers. Low priority, most devices will have an app. |
| **Several servers in one app** | A server list in the apps; FreeX Desktop can already switch servers. |

## Not planned

- Anything that needs an account with a third party to use the server at home.
- Advertising, streaming channels, rentals.

## Order of work

1. Phase 1, roughly in the table's order.
2. FreeX apps for Android TV / Fire TV and iOS/Android, then Apple TV; offline downloads once they exist.
3. Music, then photos.
4. Remote access setup and inviting friends.

Each item gets its own issue when it is started; this page is updated with the version that delivered it.

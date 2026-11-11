<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Roadmap

What FreeX Media Server does not do yet, and in which order it comes. Everything in FreeX is and stays free;
nothing on this page will need an account with a third party.

## Delivered

Movies and TV shows are complete for daily use: seek previews and chapters, trailers and extras, several versions of
a title, filters and smart collections, parental controls, quality defaults per user, HDR tone mapping, audio
passthrough, subtitle styling, the server dashboard with graphs, the log and scheduled maintenance, webhooks,
editing metadata by hand and theme music (0.21.0). Since then: DLNA, the compatibility layer for Jellyfin apps and
the APT repository. The apps of the [FreeX family](../README.md#the-freex-family) for TVs, phones and the desktop use
the same server.

## Next

| Feature | Notes |
|---|---|
| **Offline downloads** | Download a title in a chosen quality (converted on the server beforehand) to the phone, for playback without a connection. |
| **Pre-conversion** | Convert titles in advance to a target quality, stored on disk, for weak servers and for downloads. |
| **Apple TV** | A FreeX app for tvOS. |
| **Music library** | Artists, albums and tracks from tags (ID3, Vorbis, FLAC), artwork, gapless playback, playlists; metadata from MusicBrainz. A new library type and a new player. |
| **Photo library** | Folders and dates, timeline, EXIF, thumbnails, slideshows. |
| **Other videos** | A library type without metadata lookups, for home videos and recordings; titles from file names only. |
| **Remote access setup** | Guided setup with UPnP/NAT-PMP and a dynamic-DNS helper; today port forwarding or a reverse proxy is set up by hand. FreeX stays usable without any external service. |
| **Inviting friends** | Invitations by link or e-mail, and managed users without a password who switch with a PIN. |

## Later

| Feature | Notes |
|---|---|
| **Ratings and reviews** | Personal ratings and reviews per user, shown to the other users of the server. |
| **More metadata sources** | TVDB, AniDB for anime, MusicBrainz for music; selectable per library. |

## Not planned

- Anything that needs an account with a third party to use the server at home.
- Advertising, streaming channels, rentals.
- Deleting media files from the server through the web interface.

Each item gets its own issue when it is started; this page is updated with the version that delivered it.

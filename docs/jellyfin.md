<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Jellyfin apps

FreeX Media Server can act as a server for native apps made for Jellyfin, so TVs, phones and tablets for which there
is no FreeX app yet can use your libraries. The apps see your FreeX libraries, play through the FreeX playback
decision and conversion, and keep the FreeX watch state: what you watch in an app shows up as *Continue watching* in
FreeX Web and the other way round.

The layer is **off by default**. Switch it on under *Settings → Server → Jellyfin apps* (or `[jellyfin] enabled = true`
in `config.toml`) and restart the server.

## Supported apps

The layer speaks the API that the apps call directly, so it works with apps that have their own interface:

| App | Platforms |
|---|---|
| Jellyfin for Android TV | Android TV, Google TV, Fire TV, NVIDIA Shield |
| Findroid | Android phones and tablets, Android TV |
| Swiftfin | iPhone, iPad, Apple TV |
| Streamyfin, Fladder, Infuse and other apps with an interface of their own | various (not tested yet) |

Apps that only show the Jellyfin web interface in a frame do **not** work: the official Jellyfin apps for Android
phones and iOS, Jellyfin Media Player, and the Jellyfin apps for Samsung (Tizen) and LG (webOS) TVs. They load that
web interface from the server, which FreeX does not have. For Samsung and Fire TV there are the FreeX TV apps.

The layer was built from the requests these apps make (taken from their source code) and tested against the data
models of the SDKs of Jellyfin for Android TV (Kotlin SDK 1.8) and Findroid (Kotlin SDK 1.9): every answer contains
the fields and values these models require. Swiftfin uses the same API; it was not tested on a device yet.

## Connecting an app

1. In the app, add a server with the address of your FreeX server followed by `/jellyfin`, for example
   `http://192.168.1.10:32500/jellyfin` or `https://media.example.org/jellyfin`. *Settings → Security → Sign in a
   Jellyfin app* in FreeX Web shows the exact address.
2. Sign in, in one of two ways:
   - **Username and password** of your FreeX account. This only works for accounts **without** two-factor sign-in:
     the apps know no second factor, and the server will not let them skip it.
   - **Quick Connect** (works for every account): choose Quick Connect in the app, then enter the six-digit code it
     shows in FreeX Web under *Settings → Security → Sign in a Jellyfin app*. The app is then signed in with the
     account you confirmed it with. A code is valid for ten minutes and can be used once. Apps that are already
     signed in can confirm codes too.

The app appears under *Signed-in devices* with its name and device ("Findroid 0.15.2 (Pixel 8)") and can be signed
out there like any browser. Sign-ins and failed attempts appear in *Recent sign-ins*.

## What maps to what

| In the app | In FreeX |
|---|---|
| Libraries | The libraries shared with the user (age limits apply as everywhere) |
| Continue watching, Next up, Latest | Started titles, the next episode of shows you watch, recently added |
| Played / unplayed, resume position | The FreeX watch state; at 90 % a title counts as watched |
| Favorites | The watchlist (movies and shows) |
| Intro and credits skipping (media segments) | The intros and end credits FreeX detected |
| Several versions of a movie | One media source per version |
| Trailers and special features | Extras from the folders next to the movie or show |
| Audio and subtitle tracks | The tracks of the file and subtitle files next to it; text subtitles as WebVTT or SubRip |

## Playback

When an app starts a title it sends a description of what the device can decode (its *device profile*). FreeX
checks container, video and audio codecs and the limits of the profile (bit depth, frame size, number of audio
channels) against the file:

- **Direct play** when the device takes the file as it is. The app loads the file with byte ranges, like any player.
- **Conversion** otherwise: the app gets an HLS address; the conversion starts when its player loads the playlist,
  copies what the device can decode and converts the rest, exactly as for FreeX Web, with the same limits
  (`[transcoder] max_sessions`) and hardware acceleration. The quality follows the bitrate limit the app asks for.
- Bitmap subtitles (PGS, DVD) are burned into the picture when the device cannot show them itself.

Running playbacks appear in the activity view of admins with the app and device name, and trigger webhooks like
playbacks in FreeX Web.

## Security

- Every request needs a FreeX session, except the server information the apps ask for before signing in, the
  artwork, and the files of a running conversion.
- **Artwork:** the apps load images without signing in. FreeX therefore signs every image link it hands out
  (the `tag` the apps send back): only an app that got the entry from the server, signed in, can load its image.
  The key is kept in `<data>/jellyfin.key`.
- **Conversion files** are reached through a grant in the address that is valid for one playback, as for Chromecast.
- Library sharing, age limits, brute-force protection and session expiry apply exactly as for FreeX Web.
- To the apps, accounts are never administrators: the server is managed in FreeX Web only.
- The apps expect the server to call itself "Jellyfin Server" in its technical information; they refuse other
  servers. Your server name is shown everywhere else.

## Limits

Not available through the layer: the admin screens of the apps, music, live TV, SyncPlay (use Watch Together in
FreeX Web), remote control of other devices, trickplay thumbnails, cast and crew lists, and deleting titles.

## Behind a reverse proxy

Forward `/jellyfin` like the rest of the server; with `trust_proxy = true` the address the apps are told follows
`X-Forwarded-Host` and `X-Forwarded-Proto`. Conversions and file streams need the same timeouts and buffering
settings as for FreeX Web (see [Configuration](configuration.md#https)).

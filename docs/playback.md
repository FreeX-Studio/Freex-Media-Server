<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Playback

- [How a file is played](#how-a-file-is-played)
- [Hardware acceleration](#hardware-acceleration)
- [Audio tracks and subtitles](#audio-tracks-and-subtitles)
- [Skip intro and credits](#skip-intro-and-credits)
- [Watch Together](#watch-together)
- [Chromecast](#chromecast)
- [Keyboard shortcuts](#keyboard-shortcuts)

## How a file is played

For every playback the client reports which containers and codecs it can decode, and the server picks the cheapest
way:

| Mode | When | What ffmpeg does |
|---|---|---|
| Direct play | Container, video and audio are supported and the bitrate fits the chosen quality | nothing, the file is sent as it is |
| Direct stream | Video is supported, but not the container (e.g. MKV in Safari) or the audio (DTS, TrueHD) | copies the video into HLS and converts the audio to AAC if needed |
| Transcode | Video codec not supported (e.g. HEVC in Firefox), a lower quality is chosen, or bitmap subtitles are burned in | converts video to H.264 and audio to AAC stereo |

Converted streams are HLS with fragmented MP4 segments (hls.js in Chrome, Firefox and Edge, native in Safari and on
iOS). The playlist covers the whole title from the start, so the player seeks as in any video and keeps what it
has buffered (a minute ahead, two minutes back). A background worker per playback converts whatever part the player
asks for: after a far jump it restarts ffmpeg there within about a second, and parts converted before are served
again without converting them twice. Segments carry the real timestamps of the source, and converted sound follows
them, so picture and sound stay in sync over the whole film and across every jump. ffmpeg runs with low CPU and disk
priority, is paused when it is far ahead of the player and stopped shortly after the player is gone. If direct play fails in a
browser after all, the player switches to conversion automatically. Network errors are retried, a conversion the
server has ended (for example after a long pause) is started again at the same position, and playback that stops
making progress is restarted by itself.

Quality can be chosen from *Original* down to 1 Mbit/s. Admins see all running playbacks, direct or converted, in the
activity menu in the header.

## Hardware acceleration

`transcoder.hardwareAcceleration` selects `nvenc` (NVIDIA), `qsv` (Intel Quick Sync) or `vaapi` (Intel/AMD on Linux).
It needs an ffmpeg build with that encoder and access to the GPU for the server user; the Debian package adds the user
`freex` to the groups `video` and `render`. Software encoding is the tested default; please report your experience
with hardware encoders.

## Audio tracks and subtitles

Each user sets a preferred audio language and when subtitles are switched on: *off*, *forced only* (the default; for
foreign-language parts), *with foreign audio* or *always*, in a chosen language. A choice on the detail page or in
the player always wins. Language tags may be ISO 639-1 or 639-2 (`de`, `ger`, `deu`).

Subtitles next to the video (`.srt`, `.vtt`) and inside it are supported. Text subtitles (SRT, ASS, WebVTT, mov_text)
are rendered by the player; bitmap subtitles (PGS, VobSub, DVB) are burned into the picture during conversion.

A playback that reaches 90 % marks the item as watched and adds it to the user's watch history.

## Skip intro and credits

Episodes of a season share their intro music and their end credits. After a scan, the server analyses the first ten
and the last six minutes of the audio of every episode and builds an audio fingerprint (33 frequency bands, a value
every 64 ms). The intro is the longest stretch between 15 seconds and 2.5 minutes that sounds the same in an episode
and its neighbours; credits are found the same way at the end. Only matches with two neighbouring episodes are used,
so a scene with similar music is not skipped by mistake. The analysis runs in the background once per season and file
version.

During playback a *Skip intro* button appears in the intro. When the credits start, *Next episode* appears with a
countdown; for the last episode *Skip credits*.

## Watch Together

*Watch together* on a movie or episode opens a room. Everyone who opens the invite link (with an account on the same
server) watches in sync: everybody can play, pause, seek or start the next episode, and all others follow. Each
viewer streams separately and can pick their own quality, audio and subtitles.

While a viewer's player is still loading, after joining, after a jump or when the conversion falls behind, the room
pauses for everyone and shows whom it is waiting for; playback then continues for all at the same moment. Small
differences between the players are evened out by playing a few percent faster or slower, so nobody notices a jump.
A player that does not get ready within 20 seconds no longer holds up the others.

## Chromecast

In Chrome, the player shows a cast button as soon as a Chromecast or a TV with Google Cast is in the network. The
title then plays on the Chromecast and the player in the browser becomes its remote control (play/pause, seeking,
volume, audio, subtitles, quality). After disconnecting, playback continues in the browser at the same position; the
next episode is cast automatically.

- **Chrome over HTTPS:** Google offers the Cast API in Chrome only on secure pages (or `localhost`).
- **Reachable for the Chromecast:** the device loads the video itself from the address the web interface was opened
  with. It must reach that address and trust its certificate; use a certificate from a public CA, e.g. Let's Encrypt
  behind a reverse proxy. Self-signed certificates do not work.
- **Formats:** Google's Default Media Receiver is used. H.264 with AAC or MP3 in MP4 plays directly, everything else
  is converted to HLS.
- **Access without sign-in:** every cast gets a random 256-bit token that unlocks only that title, for its length
  plus six hours. Only URLs under `/api/cast/` may be loaded by other origins, because the receiver runs on Google's
  domain.

## Keyboard shortcuts

| Key | Action |
|---|---|
| `Space` / `K` | Play / pause |
| `←` / `J` | 10 seconds back |
| `→` / `L` | 30 seconds forward |
| `F` | Fullscreen |
| `M` | Mute |
| `↑` / `↓`, mouse wheel over the picture | Volume |
| `Esc` | Close the player (the × at the top right does the same) |

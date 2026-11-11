<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# DLNA

FreeX Media Server can act as a DLNA (UPnP AV) media server. Smart TVs, game consoles, AV receivers and player apps
in your home network then find the server on their own, without an app or a sign-in, and play movies and episodes
from your libraries.

DLNA is **off by default**. Switch it on under *Settings → Server → DLNA* (or in the `[dlna]` section of
`config.toml`), pick the account whose libraries the devices should see, and restart the server.

```toml
[dlna]
enabled = true
user = "family"      # required: an existing FreeX account
name = ""            # name the devices show; empty = [server] name
port = 32501         # plain HTTP port for DLNA devices
```

If `user` is empty or names no existing account, the DLNA server does not start and the log says why.

## What devices see

The server appears under the name you chose (or the server name) with the FreeX icon. Its top level contains:

| Folder | Content |
|---|---|
| *Continue watching* | Movies and episodes the account has started, newest first (up to 50) |
| *Recently added* | The newest movies and episodes (up to 50) |
| One folder per library | Movies directly; shows as folders of seasons, seasons as folders of episodes |

Titles, posters, years, plot, genres, duration, resolution and audio channels come from the FreeX catalog. The
folder names follow the interface language of the account (English or German).

## Playback

- **Direct play only.** The device gets the original file, with byte ranges for seeking. There is no conversion:
  whether a file plays depends on the device's own support for its container, video and audio codecs. Most current
  TVs play MKV and MP4 with H.264 and HEVC; older ones may not.
- **Subtitles:** external subtitle files and text subtitles inside the file are offered as SRT, in the account's
  preferred subtitle language when it has one (none when the account has subtitles switched off). Samsung TVs get
  them through the `CaptionInfo.sec` header, other devices through the subtitle resource in the listing. Picture
  subtitles (PGS, VobSub) are only shown if the device reads them from the file itself.
- **Watch progress:** what plays shows up in the activity view of the dashboard (as *DLNA – device name*). When
  playback stops, the server estimates the position from the amount of data sent and saves it as the resume point
  of the account, as FreeX Web does. Samsung TVs report their exact position
  (`X_SetBookmark`), which is used instead, and they get the resume point back when the item is listed again.

Because the position is estimated, resume points from devices other than Samsung TVs are approximate (the server
saves them 10 seconds early).

## Network and firewall

DLNA works only inside the home network:

- **Discovery** uses SSDP multicast on **UDP port 1900** (address 239.255.255.250). The server announces itself
  when it starts, every 15 minutes, and says goodbye when it stops.
- **Everything else** (descriptions, browsing, streams, posters, subtitles) runs over plain HTTP on the DLNA port
  (default **32501**), because TVs cannot use HTTPS. It must differ from the server port.

Allow both in the firewall of the server, for the private network only. On Linux with `ufw`, for example:

```sh
sudo ufw allow from 192.168.0.0/16 to any port 1900 proto udp
sudo ufw allow from 192.168.0.0/16 to any port 32501 proto tcp
```

The server answers only addresses of the home network (private, link-local and loopback addresses) and ignores
discovery requests from anywhere else. In a container, discovery needs the host network (for Docker
`--network host`), since multicast does not cross the container's own network. Devices in another subnet or VLAN
than the server only find it if the router forwards SSDP between them.

## Security

DLNA has no sign-in: any device that reaches the DLNA port sees what the account in `[dlna] user` may see, and its
watch progress is saved to that account. Choose an account with only the libraries and the age limit that every
device and person in the house may see; a separate account such as *family* or *living room* keeps the progress of
the shared TV out of your own *Continue watching*. Never forward the DLNA port or UDP 1900 from the internet.

## Limits

- No conversion, no choice of audio track or quality: the device plays the file as it is.
- One account for all DLNA devices.
- Search from the device is not supported; devices browse the folders.
- Watch Together, intro skipping and extras are only available in FreeX apps.

## Troubleshooting

| Problem | What to check |
|---|---|
| The device does not list the server | DLNA switched on and the server restarted; UDP 1900 allowed; device and server in the same subnet; in a container, the host network |
| The server is listed but empty or does not open | The DLNA port allowed in the firewall; the log (*Settings → Log*, raise the level to `debug` for details) |
| A title is listed but does not play | The device does not support the file's format; play it in FreeX Web or a FreeX app instead |
| No subtitles | Picture subtitles are not converted; on Samsung TVs, switch subtitles on in the player's options |

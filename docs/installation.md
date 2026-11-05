<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Installation

FreeX Media Server is a single Java application. It runs anywhere Java 21 runs; on Debian and Ubuntu the package
sets up everything including a hardened systemd service, on Windows the installer including a Windows service.

- [Requirements](#requirements)
- [Debian / Ubuntu package](#debian--ubuntu-package)
- [Windows](#windows)
- [Manual installation](#manual-installation)
- [Data and backups](#data-and-backups)

## Requirements

- **Java 21 or newer.** Ubuntu 24.04 and Debian 13 ship it (`openjdk-21-jre-headless`). Debian 12 only has Java 17;
  install Java 21 from [Adoptium](https://adoptium.net/installation/linux/) there first.
- **ffmpeg** (includes `ffprobe`) for media information, preview images and conversion. Without it the server still
  works, but only plays files the browser can decode itself.
- **Maven 3** to build from source.

## Debian / Ubuntu package

`mvn clean package` builds `target/freex-media-server.jar` and `target/freex-media-server_<version>_all.deb`.

```bash
sudo apt install ./freex-media-server_<version>_all.deb
```

Use `apt` rather than `dpkg -i`: `apt` installs the dependencies (ffmpeg, Java) in the same step. If the file lies in
a folder the user `_apt` cannot read (such as `/root`), apt says *"Download is performed unsandboxed as root"*; that is
harmless, or copy the file to `/tmp` first. The package

- creates the system user `freex` (also in the groups `video` and `render` for hardware transcoding),
- installs the systemd service `freex-media-server`, enables it and starts it,
- writes the configuration `/etc/freex-media-server/config.yml` on the first install.

At the end the installation prints the address of the web interface (IP of the main network interface and the
configured port). Open it and create the admin account. Your media folders must be readable by the user
`freex`, for example with `sudo setfacl -R -m u:freex:rX /srv/media` or a shared group.

```bash
sudo systemctl status freex-media-server     # state
sudo journalctl -u freex-media-server -f     # log
sudo systemctl restart freex-media-server    # after editing config.yml by hand
sudo apt remove freex-media-server           # keeps configuration and data
sudo apt purge freex-media-server            # also deletes configuration, data, cache and the service user
```

### File locations

The package follows the Filesystem Hierarchy Standard and Debian policy:

| Path | Content |
|---|---|
| `/usr/share/freex-media-server/freex-media-server.jar` | The server, including FreeX Web and the SQLite driver |
| `/usr/share/freex-media-server/config.yml.default` | Template of the configuration |
| `/usr/bin/freex-media-server` | Launcher: finds Java 21+, sets a UTF-8 locale, starts the jar |
| `/usr/lib/systemd/system/freex-media-server.service` | systemd service |
| `/etc/freex-media-server/config.yml` | Configuration, owned by `freex`, mode `640` |
| `/var/lib/freex-media-server/` | Database, library index, metadata, artwork: **this is what you back up** |
| `/var/cache/freex-media-server/` | Thumbnails, extracted subtitles, transcoding segments; recreated as needed |
| journald | Log (`journalctl -u freex-media-server`) |

The configuration is not a dpkg conffile: it is created from the template once and then belongs to the server,
which writes it from its settings page. Updates never overwrite it; keys missing in an older file use their
defaults.

### Hardening

The service runs with `ProtectSystem=strict`, `ProtectHome=read-only`, `NoNewPrivileges` and a private `/tmp`. It can
read media anywhere but only write its state, cache and configuration directories. If you move the transcoding
folder elsewhere (for example to a fast SSD), allow it with a drop-in:

```bash
sudo systemctl edit freex-media-server
```

```ini
[Service]
ReadWritePaths=/mnt/ssd/freex-transcode
```

## Windows

Run `freex-media-server-<version>-setup.exe` (Windows 10 or 11, 64-bit). It brings everything with it, Java and
ffmpeg included, and

- installs the server into `C:\Program Files\FreeX Media Server`,
- sets up the Windows service *FreeX Media Server*, which starts with Windows (also without anyone signed in),
- allows it in the Windows firewall for private and domain networks,
- adds *FreeX* (opens the web interface) and *Configuration and logs* to the start menu.

At the end the installer opens the web interface; create the admin account there. If other devices cannot reach
the server, check that Windows treats your network as *private*.

| Path | Content |
|---|---|
| `C:\Program Files\FreeX Media Server\` | The server, its Java runtime (`jre`), ffmpeg (`ffmpeg`), the service (`freex-media-server-service.exe`) |
| `C:\ProgramData\FreeX Media Server\config.yml` | Configuration, created on the first start |
| `C:\ProgramData\FreeX Media Server\data\` | Database, library index, metadata, artwork: **this is what you back up** |
| `C:\ProgramData\FreeX Media Server\logs\` | Log of the service |

Only the system and administrators can open the folder in ProgramData, because it holds the database. Updates are
installed by running the newer installer; configuration and data are kept. Uninstalling (*Settings → Apps*) asks
whether to delete them as well. The service runs as *Local System*: media on a network share needs a share it can
reach, or a different service account (`services.msc` → *FreeX Media Server* → *Log On*).

```bat
sc query freex-media-server          :: state
net stop freex-media-server          :: stop (start with net start, e.g. after editing config.yml)
```

## Manual installation

```bash
git clone https://github.com/LinzN/freex-media-server.git
cd freex-media-server
mvn clean package
LANG=C.UTF-8 java -jar target/freex-media-server.jar
```

The jar is self-contained (web interface and SQLite driver for Linux, Windows and macOS on x86_64 and ARM). On the
first start it writes a commented `config.yml` into the working directory; pass another path as the first argument:
`java -jar freex-media-server.jar /etc/freex/config.yml`.

> **Use a UTF-8 locale.** Java decodes file names with the system locale. Without one (common for services, where
> `LANG` is not set) umlauts and other non-ASCII characters in file names break. The server logs a warning then.

### As a systemd service

```ini
# /etc/systemd/system/freex-media-server.service
[Unit]
Description=FreeX Media Server
After=network-online.target

[Service]
User=freex
WorkingDirectory=/opt/freex-media-server
Environment=LANG=C.UTF-8
ExecStart=/usr/bin/java -jar /opt/freex-media-server/freex-media-server.jar
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now freex-media-server
```

The SQLite driver unpacks a native library into the temp directory. If `/tmp` is mounted `noexec`, start Java with
`-Dorg.sqlite.tmpdir=<some writable directory>`.

## Data and backups

Everything lives in the data directory (`data/` next to the configuration, `/var/lib/freex-media-server` with the
package). Caches go to `data.cacheDirectory` if set.

| File | Content |
|---|---|
| `freex.db` | SQLite database: users, password hashes, two-factor secrets, sessions, login history, preferences, watch state and history, playlists, watchlist, own collections |
| `libraries.json`, `items/` | Library definitions and the scanned index (rebuilt by a scan) |
| `metadata.json`, `metadata/` | TMDB metadata and downloaded artwork |
| `markers.json` | Detected intros and credits |
| `avatars/` | Profile pictures, re-encoded by the server |
| `thumbs/`, `subtitles/`, `transcode/` | Caches; safe to delete while the server is stopped |

For a backup, stop the server and copy the data directory, or back up the database while it runs:

```bash
sqlite3 /var/lib/freex-media-server/freex.db ".backup freex-backup.db"
```

The database schema is versioned and updated automatically when a newer server starts.

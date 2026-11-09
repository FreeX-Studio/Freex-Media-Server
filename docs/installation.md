<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Installation

FreeX Media Server is a single native program without a runtime to install. On Debian and Ubuntu the package sets
up everything including a hardened systemd service, on Windows the installer including a Windows service; the
plain archives work on any other Linux.

- [Requirements](#requirements)
- [Debian / Ubuntu package](#debian--ubuntu-package)
- [Windows](#windows)
- [Manual installation](#manual-installation)
- [Data and backups](#data-and-backups)

## Requirements

- **Linux** on x86_64 with glibc 2.34 or newer (Debian 12, Ubuntu 22.04 and newer), or **Windows 10/11** (64-bit).
  Other systems (ARM, macOS, Alpine) build from source with Rust 1.89 or newer, see [Development](development.md).
- **ffmpeg** (includes `ffprobe`) for media information, preview images and conversion. Without it the server still
  works, but only plays files the browser can decode itself.

## Debian / Ubuntu package

```bash
sudo apt install ./freex-media-server_<version>_amd64.deb
```

Use `apt` rather than `dpkg -i`: `apt` installs the dependency ffmpeg in the same step. If the file lies in
a folder the user `_apt` cannot read (such as `/root`), apt says *"Download is performed unsandboxed as root"*; that is
harmless, or copy the file to `/tmp` first. The package

- creates the system user `freex` (also in the groups `video` and `render` for hardware transcoding),
- installs the systemd service `freex-media-server`, enables it and starts it,
- writes the configuration `/etc/freex-media-server/config.toml` on the first install.

At the end the installation prints the address of the web interface (IP of the main network interface and the
configured port). Open it and create the admin account. Your media folders must be readable by the user
`freex`, for example with `sudo setfacl -R -m u:freex:rX /srv/media` or a shared group.

```bash
sudo systemctl status freex-media-server     # state
sudo journalctl -u freex-media-server -f     # log
sudo systemctl restart freex-media-server    # after editing config.toml by hand
sudo apt remove freex-media-server           # keeps configuration and data
sudo apt purge freex-media-server            # also deletes configuration, data, cache and the service user
```

### File locations

The package follows the Filesystem Hierarchy Standard and Debian policy:

| Path | Content |
|---|---|
| `/usr/bin/freex-media-server` | The server, including FreeX Web and SQLite |
| `/usr/share/freex-media-server/config.toml.default` | Template of the configuration |
| `/lib/systemd/system/freex-media-server.service` | systemd service |
| `/etc/freex-media-server/config.toml` | Configuration, owned by `freex`, mode `640` |
| `/var/lib/freex-media-server/` | Database, library index, metadata, artwork: **this is what you back up** |
| `/var/cache/freex-media-server/` | Thumbnails, seek previews, extracted subtitles, transcoding segments; recreated as needed |
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

Run `freex-media-server-<version>-setup.exe` (Windows 10 or 11, 64-bit). It brings ffmpeg with it and

- installs the server into `C:\Program Files\FreeX Media Server`,
- sets up the Windows service *FreeX Media Server*, which starts with Windows (also without anyone signed in),
- allows it in the Windows firewall for private and domain networks,
- adds *FreeX* (opens the web interface) and *Configuration and logs* to the start menu.

At the end the installer opens the web interface; create the admin account there. If other devices cannot reach
the server, check that Windows treats your network as *private*.

| Path | Content |
|---|---|
| `C:\Program Files\FreeX Media Server\` | The server (`freex-media-server.exe`), ffmpeg (`ffmpeg`), the service wrapper (`freex-media-server-service.exe`) |
| `C:\ProgramData\FreeX Media Server\config.toml` | Configuration, created on the first start |
| `C:\ProgramData\FreeX Media Server\data\` | Database, library index, metadata, artwork: **this is what you back up** |
| `C:\ProgramData\FreeX Media Server\logs\` | Log of the service |

Only the system and administrators can open the folder in ProgramData, because it holds the database. Updates are
installed by running the newer installer; configuration and data are kept. Uninstalling (*Settings → Apps*) asks
whether to delete them as well. The service runs as *Local System*: media on a network share needs a share it can
reach, or a different service account (`services.msc` → *FreeX Media Server* → *Log On*).

```bat
sc query freex-media-server          :: state
net stop freex-media-server          :: stop (start with net start, e.g. after editing config.toml)
```

## Manual installation

Unpack the archive of a release (`freex-media-server-<version>-linux-x86_64.tar.gz`) or build from source:

```bash
git clone https://github.com/FreeX-Studio/freex-media-server.git
cd freex-media-server
cargo build --release          # target/release/freex-media-server
./target/release/freex-media-server
```

The program is self-contained (web interface and SQLite included). On the first start it writes a commented
`config.toml` into the working directory; pass another path as the first argument:
`freex-media-server /etc/freex/config.toml`. Relative paths in the file count from the file's folder.

### As a systemd service

```ini
# /etc/systemd/system/freex-media-server.service
[Unit]
Description=FreeX Media Server
After=network-online.target

[Service]
User=freex
WorkingDirectory=/opt/freex-media-server
ExecStart=/opt/freex-media-server/freex-media-server /opt/freex-media-server/config.toml
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now freex-media-server
```

Set `FREEX_LOG=debug` in the environment for more log output.

## Data and backups

Everything lives in the data directory (`data/` next to the configuration, `/var/lib/freex-media-server` with the
package). Caches go to the cache directory (`cache/`, `/var/cache/freex-media-server`).

| File | Content |
|---|---|
| `freex.db` | SQLite database: users, password hashes, two-factor secrets, sessions, login history, preferences, libraries and the scanned index, watch state and history, playlists, watchlist, own collections, TMDB metadata, intro and credits markers |
| `metadata/` | Downloaded artwork of titles, people and movie series |
| `avatars/` | Profile pictures, re-encoded by the server |
| `backups/` | Database backups of the nightly maintenance (`freex-YYYY-MM-DD-HHMM.db`, the newest seven by default) |
| cache: `thumbs/`, `previews/`, `subtitles/`, `transcode/` | Preview frames, seek preview sheets, extracted subtitles, segments of running conversions; safe to delete while the server is stopped |

The nightly maintenance keeps copies of the database in `backups/` (see [Configuration](configuration.md)); to restore
one, stop the server and copy it over `freex.db`. For a complete backup, stop the server and copy the data directory,
or back up the database while it runs:

```bash
sqlite3 /var/lib/freex-media-server/freex.db ".backup freex-backup.db"
```

The database schema is versioned and updated automatically when a newer server starts.

## Moving from the Java server (0.14 and earlier)

The Rust server (0.20 and newer) uses a new configuration file (`config.toml` instead of `config.yml`) and a new
database layout; the data of a Java installation is not taken over, and the server refuses to start on a Java
database rather than change it. Set it up as a fresh server: create the admin account, add the libraries and enter
the TMDB key again.

- The Debian package moves the Java configuration and data to `/var/lib/freex-media-server.java-backup` on the
  upgrade, the Windows installer to `C:\ProgramData\FreeX Media Server\java-backup` (and removes the bundled Java
  runtime). Delete the backup once the new server runs.
- A manual installation: point `[data] directory` to a new folder, or move the old one away.

#!/usr/bin/env bash
#
# Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
#
# This file is part of the MirraNET project and is licensed under the
# GNU Lesser General Public License v3.0 (LGPLv3).
#
# You may use, distribute and modify this code under the terms
# of the LGPLv3 license. You should have received a copy of the
# license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
# or contact: niklas.linz@mirranet.de
#
# Builds the Debian package (freex-media-server_<version>_amd64.deb):
#
#   build-deb.sh <version> <freex-media-server binary> <output directory>
#
# The package installs the program to /usr/bin, the systemd service, the default configuration
# (copied to /etc/freex-media-server/config.toml on the first install) and the service user
# "freex". Needs dpkg-deb (package dpkg) and gzip; no root.

set -euo pipefail

VERSION="$1"
BINARY="$2"
OUT="$3"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PKG=freex-media-server

for tool in dpkg-deb gzip; do
    command -v "$tool" >/dev/null || { echo "build-deb: $tool is missing" >&2; exit 1; }
done
[ -f "$BINARY" ] || { echo "build-deb: $BINARY not found (run cargo build --release first)" >&2; exit 1; }

STAGE="$(mktemp -d)"
chmod 0755 "$STAGE"
trap 'rm -rf "$STAGE"' EXIT

install -D -m 0755 "$BINARY" "$STAGE/usr/bin/$PKG"
install -D -m 0644 "$HERE/systemd/$PKG.service" "$STAGE/lib/systemd/system/$PKG.service"
install -D -m 0644 "$HERE/config.toml" "$STAGE/usr/share/$PKG/config.toml.default"
install -D -m 0644 "$ROOT/LICENSE" "$STAGE/usr/share/doc/$PKG/copyright"
install -D -m 0644 "$ROOT/README.md" "$STAGE/usr/share/doc/$PKG/README.md"
# Debian wants the changelog compressed with the best gzip and without a timestamp.
gzip -9 -n -c "$ROOT/CHANGELOG.md" > "$STAGE/usr/share/doc/$PKG/changelog.gz"
chmod 0644 "$STAGE/usr/share/doc/$PKG/changelog.gz"

mkdir -p "$STAGE/DEBIAN"
sed "s/\[\[version\]\]/$VERSION/" "$HERE/control/control" > "$STAGE/DEBIAN/control"
size=$(du -sk --exclude=DEBIAN "$STAGE" | cut -f1)
echo "Installed-Size: $size" >> "$STAGE/DEBIAN/control"
for script in postinst prerm postrm; do
    install -m 0755 "$HERE/control/$script" "$STAGE/DEBIAN/$script"
done
# The configuration is created by postinst from the default file, so it is not a conffile.

mkdir -p "$OUT"
DEB="$OUT/${PKG}_${VERSION}_amd64.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB" >/dev/null
echo "build-deb: $DEB"

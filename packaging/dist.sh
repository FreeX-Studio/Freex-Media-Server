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
# Packs the release builds into plain archives in dist/ (for installations without the Debian
# package or the Windows installer) and writes the checksums of everything in dist/:
#
#   freex-media-server-<version>-linux-x86_64.tar.gz    (from target/release)
#   freex-media-server-<version>-windows-x86_64.zip     (from target/x86_64-pc-windows-gnu/release)
#
# Each archive holds the program, README, LICENSE and CHANGELOG. A build that is missing is
# skipped. Run from the project directory after `cargo build --release [--target ...]`; the .deb
# and the installer are made by packaging/deb/build-deb.sh and packaging/windows/build-installer.sh.

set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
NAME="freex-media-server-$VERSION"
mkdir -p dist
rm -f dist/*.tar.gz dist/*.zip dist/SHA256SUMS

docs() {
    cp README.md LICENSE CHANGELOG.md "$1/"
}

linux="target/release/freex-media-server"
if [ -x "$linux" ]; then
    dir="dist/$NAME-linux-x86_64"
    mkdir -p "$dir"
    cp "$linux" "$dir/"
    docs "$dir"
    tar -C dist -czf "dist/$NAME-linux-x86_64.tar.gz" "$NAME-linux-x86_64"
    rm -rf "$dir"
    echo "dist: $NAME-linux-x86_64.tar.gz"
fi

windows="target/x86_64-pc-windows-gnu/release/freex-media-server.exe"
if [ -f "$windows" ]; then
    dir="dist/$NAME-windows-x86_64"
    mkdir -p "$dir"
    cp "$windows" "$dir/"
    docs "$dir"
    (cd dist && python3 -c 'import shutil,sys; shutil.make_archive(sys.argv[1], "zip", ".", sys.argv[1])' "$NAME-windows-x86_64")
    rm -rf "$dir"
    echo "dist: $NAME-windows-x86_64.zip"
fi

(cd dist && find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%f\n' | sort | xargs -r sha256sum > SHA256SUMS)

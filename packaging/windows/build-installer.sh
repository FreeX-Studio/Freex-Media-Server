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
# Builds the Windows installer (freex-media-server-<version>-setup.exe) on Linux.
#
#   build-installer.sh <version> <freex-media-server.exe> <output directory>
#
# The Jenkins pipeline runs it after the Windows cross build. It downloads ffmpeg and WinSW in
# fixed versions into a cache (FREEX_DOWNLOAD_CACHE, default ~/.cache/freex-build), checks them
# against the checksums of their publishers and packs them with the server into one installer.
# Needs: makensis, curl, unzip, sha256sum.

set -euo pipefail

VERSION="$1"
# makensis runs inside packaging/windows, so the paths handed to it must be absolute.
EXE="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
mkdir -p "$3"
OUT="$(cd "$3" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

# Bundled programs. To update, change the version; the checksums are fetched from the publisher.
FFMPEG_VERSION="9.0.2"
FFMPEG_FILE="ffmpeg-$FFMPEG_VERSION-essentials_build.zip"
FFMPEG_URL="https://github.com/GyanD/codexffmpeg/releases/download/$FFMPEG_VERSION/$FFMPEG_FILE"
FFMPEG_SHA_URL="https://www.gyan.dev/ffmpeg/builds/packages/$FFMPEG_FILE.sha256"
# Set FFMPEG_SHA256 to pin the file when gyan.dev cannot be reached from the build machine.
FFMPEG_SHA256="${FFMPEG_SHA256:-}"
WINSW_VERSION="2.12.0"
WINSW_FILE="WinSW.NET461.exe" # needs .NET Framework 4.6.1, which every Windows 10 and 11 has
WINSW_URL="https://github.com/winsw/winsw/releases/download/v$WINSW_VERSION/$WINSW_FILE"
# WinSW publishes no checksum files. Set WINSW_SHA256 (e.g. in the Jenkins job) to pin the file.
WINSW_SHA256="${WINSW_SHA256:-}"

CACHE="${FREEX_DOWNLOAD_CACHE:-$HOME/.cache/freex-build}"
mkdir -p "$CACHE"

for tool in makensis curl unzip sha256sum; do
    command -v "$tool" >/dev/null || { echo "build-installer: $tool is missing" >&2; exit 1; }
done
[ -f "$EXE" ] || { echo "build-installer: $EXE not found (run the Windows cross build first)" >&2; exit 1; }

# fetch <url> <file name> -> path in the cache (downloaded once)
fetch() {
    local target="$CACHE/$2"
    if [ ! -s "$target" ]; then
        echo "build-installer: downloading $1" >&2
        curl -fsSL --retry 3 -o "$target.part" "$1"
        mv "$target.part" "$target"
    fi
    echo "$target"
}

# verify <file> <expected sha256>
verify() {
    local actual
    actual="$(sha256sum "$1" | cut -d' ' -f1)"
    if [ "$actual" != "$2" ]; then
        rm -f "$1"
        echo "build-installer: checksum of $(basename "$1") does not match (got $actual, expected $2)" >&2
        exit 1
    fi
}

ffmpeg="$(fetch "$FFMPEG_URL" "$FFMPEG_FILE")"
if [ -n "$FFMPEG_SHA256" ]; then
    verify "$ffmpeg" "$FFMPEG_SHA256"
else
    verify "$ffmpeg" "$(curl -fsSL --retry 3 "$FFMPEG_SHA_URL" | tr -d '[:space:]')"
fi

winsw="$(fetch "$WINSW_URL" "WinSW-$WINSW_VERSION.NET461.exe")"
if [ -n "$WINSW_SHA256" ]; then verify "$winsw" "$WINSW_SHA256"; fi

# Everything the installer puts into the program folder.
STAGE="$OUT/windows"
rm -rf "$STAGE"
mkdir -p "$STAGE/ffmpeg"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

unzip -q "$ffmpeg" -d "$TMP/ffmpeg"
cp "$TMP/ffmpeg/"*/bin/ffmpeg.exe "$TMP/ffmpeg/"*/bin/ffprobe.exe "$STAGE/ffmpeg/"
cp "$TMP/ffmpeg/"*/LICENSE "$STAGE/ffmpeg/LICENSE.txt"

cp "$winsw" "$STAGE/freex-media-server-service.exe"
cp "$HERE/freex-media-server-service.xml" "$STAGE/"
cp "$EXE" "$STAGE/freex-media-server.exe"
cp "$HERE/freex.ico" "$HERE/THIRD-PARTY.txt" "$STAGE/"
cp "$ROOT/LICENSE" "$STAGE/LICENSE.txt"

# Windows wants four numbers: 0.20.0 -> 0.20.0.0 (a pre-release suffix is dropped).
numeric="${VERSION%%-*}"
while [ "$(tr -cd . <<<"$numeric" | wc -c)" -lt 3 ]; do numeric="$numeric.0"; done

SETUP="$OUT/freex-media-server-$VERSION-setup.exe"
(cd "$HERE" && makensis -V2 -INPUTCHARSET UTF8 \
    -DVERSION="$VERSION" -DFILEVERSION="$numeric" -DSTAGE="$STAGE" -DOUTFILE="$SETUP" \
    freex-media-server.nsi)
# The staged files are inside the installer now.
rm -rf "$STAGE"
echo "build-installer: $SETUP"

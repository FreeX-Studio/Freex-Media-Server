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
#   build-installer.sh <version> <freex-media-server.jar> <output directory>
#
# Maven runs it in the package phase when NSIS (makensis) is installed. It downloads the Java
# runtime, ffmpeg and WinSW in fixed versions into a cache (FREEX_DOWNLOAD_CACHE, default
# ~/.cache/freex-build), checks them against the checksums of their publishers and packs them with
# the server into one installer. Needs: makensis, curl, unzip, sha256sum.

set -euo pipefail

VERSION="$1"
JAR="$2"
OUT="$3"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

# Bundled programs. To update, change the version; the checksums are fetched from the publisher.
JRE_RELEASE="jdk-21.0.12+8"
JRE_FILE="OpenJDK21U-jre_x64_windows_hotspot_21.0.12_8.zip"
JRE_URL="https://github.com/adoptium/temurin21-binaries/releases/download/${JRE_RELEASE/+/%2B}/$JRE_FILE"
FFMPEG_VERSION="9.0.2"
FFMPEG_FILE="ffmpeg-$FFMPEG_VERSION-essentials_build.zip"
FFMPEG_URL="https://github.com/GyanD/codexffmpeg/releases/download/$FFMPEG_VERSION/$FFMPEG_FILE"
FFMPEG_SHA_URL="https://www.gyan.dev/ffmpeg/builds/packages/$FFMPEG_FILE.sha256"
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

jre="$(fetch "$JRE_URL" "$JRE_FILE")"
verify "$jre" "$(curl -fsSL --retry 3 "$JRE_URL.sha256.txt" | cut -d' ' -f1)"

ffmpeg="$(fetch "$FFMPEG_URL" "$FFMPEG_FILE")"
verify "$ffmpeg" "$(curl -fsSL --retry 3 "$FFMPEG_SHA_URL" | tr -d '[:space:]')"

winsw="$(fetch "$WINSW_URL" "WinSW-$WINSW_VERSION.NET461.exe")"
if [ -n "$WINSW_SHA256" ]; then verify "$winsw" "$WINSW_SHA256"; fi

# Everything the installer puts into the program folder.
STAGE="$OUT/windows"
rm -rf "$STAGE"
mkdir -p "$STAGE/ffmpeg"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

unzip -q "$jre" -d "$TMP/jre"
mv "$TMP/jre/"*/ "$STAGE/jre"

unzip -q "$ffmpeg" -d "$TMP/ffmpeg"
cp "$TMP/ffmpeg/"*/bin/ffmpeg.exe "$TMP/ffmpeg/"*/bin/ffprobe.exe "$STAGE/ffmpeg/"
cp "$TMP/ffmpeg/"*/LICENSE "$STAGE/ffmpeg/LICENSE.txt"

cp "$winsw" "$STAGE/freex-media-server-service.exe"
cp "$HERE/freex-media-server-service.xml" "$STAGE/"
cp "$JAR" "$STAGE/freex-media-server.jar"
cp "$HERE/freex.ico" "$HERE/THIRD-PARTY.txt" "$STAGE/"
cp "$ROOT/LICENSE" "$STAGE/LICENSE.txt"

# Windows wants four numbers: 0.12.2 -> 0.12.2.0 (a -SNAPSHOT suffix is dropped).
numeric="${VERSION%%-*}"
while [ "$(tr -cd . <<<"$numeric" | wc -c)" -lt 3 ]; do numeric="$numeric.0"; done

SETUP="$OUT/freex-media-server-$VERSION-setup.exe"
(cd "$HERE" && makensis -V2 -INPUTCHARSET UTF8 \
    -DVERSION="$VERSION" -DFILEVERSION="$numeric" -DSTAGE="$STAGE" -DOUTFILE="$SETUP" \
    freex-media-server.nsi)
echo "build-installer: $SETUP"

/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

package de.mirranet.freex.server.config;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.logging.Logger;

/**
 * Server settings from {@code config.yml}.
 * <p>
 * Only server-level settings live here. Libraries and users are managed in the web interface
 * and stored in the data directory. On the first start a commented default file is written.
 * <p>
 * The file is read with a small built-in reader for the YAML subset used here: nested maps by
 * indentation, {@code key: value} scalars, quoted strings and {@code #} comments.
 */
public final class Configuration {

    private static final Logger LOG = Logger.getLogger(Configuration.class.getName());

    private static final String DEFAULT_FILE = """
            # FreeX configuration.
            # Libraries and users are managed in the web interface, not here. Most settings below
            # can also be changed there by an admin (Settings > Server), which writes them back here.

            server:
              # Name shown in the web interface
              name: "FreeX"
              # Address and port the HTTP server listens on
              bindAddress: "0.0.0.0"
              port: 32500
              # Optional HTTPS: path to a PKCS#12 keystore (.p12/.pfx) with certificate and key.
              # Leave empty for plain HTTP (e.g. behind a reverse proxy that does TLS).
              tlsKeystore: ""
              tlsKeystorePassword: ""
              # Set to true only when the server runs behind your own reverse proxy: then the
              # X-Forwarded-For / X-Forwarded-Proto headers are trusted for client IP and HTTPS detection.
              trustProxy: false

            data:
              # Where the database (users, watch state, lists), library index and metadata are stored
              directory: "data"
              # Where files the server can recreate at any time are stored: thumbnails,
              # extracted subtitles and transcoding segments. Empty = the data directory.
              cacheDirectory: ""

            ffmpeg:
              # Paths to the ffprobe/ffmpeg binaries. Used for media info and thumbnails;
              # the server still works without them, just with less information.
              ffprobe: "ffprobe"
              ffmpeg: "ffmpeg"

            security:
              # A login stays valid this many days without use
              sessionDays: 30
              # Minimum password length for new passwords
              minPasswordLength: 10

            tmdb:
              # API key or read access token from https://www.themoviedb.org/settings/api (free).
              # Leave empty to use only local .nfo files and artwork.
              apiKey: ""
              # Language of titles and descriptions, and country for age ratings
              language: "de-DE"
              region: "DE"

            markers:
              # Detect intros and end credits of TV episodes ("Skip intro")
              enabled: true

            transcoder:
              # Convert files the browser can't play (codec, container, bitrate) with ffmpeg
              enabled: true
              # Where temporary segments are written: relative to the cache directory or an absolute
              # path, e.g. on a fast SSD. The server only ever deletes its own session folders there.
              directory: "transcode"
              # Simultaneous conversions; more requests get a "busy" message
              maxSessions: 3
              # Hardware encoding: none, nvenc (NVIDIA), qsv (Intel Quick Sync) or vaapi (Intel/AMD on Linux)
              hardwareAcceleration: "none"
              vaapiDevice: "/dev/dri/renderD128"
              # x264 speed/quality trade-off for software encoding (ultrafast … medium)
              preset: "veryfast"

            scanner:
              # Automatic rescan of all libraries, in minutes (0 = only manual scans)
              intervalMinutes: 30
              # Number of files probed in parallel
              threads: 4
            """;

    public final String serverName;
    public final String bindAddress;
    public final int port;
    public final Path dataDirectory;
    public final Path cacheDirectory;
    public final String ffprobe;
    public final String ffmpeg;
    public final int scanIntervalMinutes;
    public final int scanThreads;
    public final String tlsKeystore;
    public final String tlsKeystorePassword;
    public final boolean trustProxy;
    public final int sessionDays;
    public final int minPasswordLength;
    public final boolean transcoderEnabled;
    public final Path transcodeDirectory;
    public final int maxTranscodeSessions;
    public final String hardwareAcceleration;
    public final String vaapiDevice;
    public final String x264Preset;
    public final String tmdbApiKey;
    public final String tmdbLanguage;
    public final String tmdbRegion;
    public final String tmdbApiUrl;
    public final String tmdbImageUrl;
    public final boolean markersEnabled;

    /** The file this configuration was read from. */
    public final Path file;
    /** Values as written in the file (dotted keys), without defaults. */
    private final Map<String, String> raw;

    private Configuration(Map<String, String> v, Path baseDir, Path file) {
        this.file = file;
        this.raw = Map.copyOf(v);
        this.serverName = v.getOrDefault("server.name", "FreeX");
        this.bindAddress = v.getOrDefault("server.bindAddress", "0.0.0.0");
        this.port = intValue(v, "server.port", 32500);
        this.dataDirectory = baseDir.resolve(v.getOrDefault("data.directory", "data")).normalize();
        String cache = v.getOrDefault("data.cacheDirectory", "").trim();
        this.cacheDirectory = cache.isEmpty() ? dataDirectory : baseDir.resolve(cache).normalize();
        this.ffprobe = v.getOrDefault("ffmpeg.ffprobe", "ffprobe");
        this.ffmpeg = v.getOrDefault("ffmpeg.ffmpeg", "ffmpeg");
        this.scanIntervalMinutes = Math.max(0, intValue(v, "scanner.intervalMinutes", 30));
        this.scanThreads = Math.max(1, intValue(v, "scanner.threads", 4));
        String ks = v.getOrDefault("server.tlsKeystore", "").trim();
        this.tlsKeystore = ks.isEmpty() ? "" : baseDir.resolve(ks).normalize().toString();
        this.tlsKeystorePassword = v.getOrDefault("server.tlsKeystorePassword", "");
        this.trustProxy = Boolean.parseBoolean(v.getOrDefault("server.trustProxy", "false").trim());
        this.sessionDays = Math.max(1, intValue(v, "security.sessionDays", 30));
        this.minPasswordLength = Math.max(8, intValue(v, "security.minPasswordLength", 10));
        this.transcoderEnabled = !"false".equalsIgnoreCase(v.getOrDefault("transcoder.enabled", "true").trim());
        this.transcodeDirectory = cacheDirectory.resolve(v.getOrDefault("transcoder.directory", "transcode")).normalize();
        this.maxTranscodeSessions = Math.max(1, intValue(v, "transcoder.maxSessions", 3));
        this.hardwareAcceleration = v.getOrDefault("transcoder.hardwareAcceleration", "none").trim().toLowerCase();
        this.vaapiDevice = v.getOrDefault("transcoder.vaapiDevice", "/dev/dri/renderD128");
        this.x264Preset = v.getOrDefault("transcoder.preset", "veryfast").trim();
        this.tmdbApiKey = v.getOrDefault("tmdb.apiKey", "").trim();
        this.tmdbLanguage = v.getOrDefault("tmdb.language", "de-DE").trim();
        this.tmdbRegion = v.getOrDefault("tmdb.region", "DE").trim();
        // Not in the default file: only needed to point the server at a test double.
        this.tmdbApiUrl = v.getOrDefault("tmdb.apiUrl", "https://api.themoviedb.org/3").trim();
        this.tmdbImageUrl = v.getOrDefault("tmdb.imageUrl", "https://image.tmdb.org/t/p").trim();
        this.markersEnabled = !"false".equalsIgnoreCase(v.getOrDefault("markers.enabled", "true").trim());
    }

    /** Loads the file, writing the commented default first if it does not exist yet. */
    public static Configuration load(Path file) throws IOException {
        if (!Files.exists(file)) {
            Files.writeString(file, DEFAULT_FILE, StandardCharsets.UTF_8);
            LOG.info("Created default configuration " + file.toAbsolutePath());
        }
        Map<String, String> values = parse(Files.readString(file, StandardCharsets.UTF_8));
        Path base = file.toAbsolutePath().getParent();
        return new Configuration(values, base == null ? Path.of(".") : base, file.toAbsolutePath());
    }

    /** Values of the commented default file, i.e. the default of every documented key. */
    public static Map<String, String> defaults() {
        return parse(DEFAULT_FILE);
    }

    /** The value of a key as the server uses it: from the file, otherwise the default. */
    public String effective(String key) {
        String v = raw.get(key);
        return v != null ? v : defaults().getOrDefault(key, "");
    }

    private static int intValue(Map<String, String> v, String key, int def) {
        String s = v.get(key);
        if (s == null) return def;
        try {
            return Integer.parseInt(s.trim());
        } catch (NumberFormatException e) {
            LOG.warning("Invalid number for " + key + ": '" + s + "', using " + def);
            return def;
        }
    }

    /** Parses the YAML subset into a flat map with dotted keys ({@code server.port}). */
    static Map<String, String> parse(String text) {
        Map<String, String> out = new LinkedHashMap<>();
        // Stack of (indent, key) for the current nesting path
        Deque<Map.Entry<Integer, String>> path = new ArrayDeque<>();
        int lineNo = 0;
        for (String raw : text.split("\r?\n")) {
            lineNo++;
            String line = stripComment(raw);
            if (line.isBlank()) continue;
            int indent = 0;
            while (indent < line.length() && line.charAt(indent) == ' ') indent++;
            String content = line.trim();
            int colon = content.indexOf(':');
            if (colon <= 0) {
                LOG.warning("config.yml line " + lineNo + " ignored: " + raw);
                continue;
            }
            while (!path.isEmpty() && path.peek().getKey() >= indent) path.pop();
            String key = content.substring(0, colon).trim();
            String value = content.substring(colon + 1).trim();
            StringBuilder full = new StringBuilder();
            path.descendingIterator().forEachRemaining(e -> full.append(e.getValue()).append('.'));
            full.append(key);
            if (value.isEmpty()) {
                path.push(Map.entry(indent, key));
            } else {
                out.put(full.toString(), unquote(value));
            }
        }
        return out;
    }

    static String stripComment(String line) {
        boolean inSingle = false;
        boolean inDouble = false;
        for (int i = 0; i < line.length(); i++) {
            char c = line.charAt(i);
            if (inDouble && c == '\\' && i + 1 < line.length() && (line.charAt(i + 1) == '"' || line.charAt(i + 1) == '\\')) {
                i++; // \" or \\ inside a quoted string, e.g. in a password
            } else if (c == '"' && !inSingle) {
                inDouble = !inDouble;
            } else if (c == '\'' && !inDouble) {
                inSingle = !inSingle;
            } else if (c == '#' && !inSingle && !inDouble && (i == 0 || Character.isWhitespace(line.charAt(i - 1)))) {
                return line.substring(0, i);
            }
        }
        return line;
    }

    static String unquote(String v) {
        if (v.length() >= 2 && v.startsWith("\"") && v.endsWith("\"")) {
            StringBuilder out = new StringBuilder();
            for (int i = 1; i < v.length() - 1; i++) {
                char c = v.charAt(i);
                // Only \" and \\ are escapes; other backslashes (Windows paths) stay as they are.
                if (c == '\\' && i + 1 < v.length() - 1 && (v.charAt(i + 1) == '"' || v.charAt(i + 1) == '\\')) c = v.charAt(++i);
                out.append(c);
            }
            return out.toString();
        }
        if (v.length() >= 2 && v.startsWith("'") && v.endsWith("'")) {
            return v.substring(1, v.length() - 1).replace("''", "'");
        }
        return v;
    }

    /** Writes a string as a double-quoted YAML scalar that {@link #unquote} reads back unchanged. */
    static String quote(String v) {
        return "\"" + v.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }
}

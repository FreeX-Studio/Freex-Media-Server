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

package de.mirranet.freex.server.http;

import com.sun.net.httpserver.Headers;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.channels.Channels;
import java.nio.channels.FileChannel;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.time.Instant;
import java.time.ZoneOffset;
import java.time.format.DateTimeFormatter;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Sends files with support for HTTP range requests (needed for seeking in videos), plus
 * {@code ETag}/{@code Last-Modified} so the browser caches artwork.
 */
public final class FileSender {

    private static final Pattern RANGE = Pattern.compile("bytes=(\\d*)-(\\d*)");
    private static final DateTimeFormatter HTTP_DATE = DateTimeFormatter.RFC_1123_DATE_TIME.withZone(ZoneOffset.UTC);
    private static final int BUFFER = 256 * 1024;

    private static final Map<String, String> TYPES = Map.ofEntries(
            Map.entry("mp4", "video/mp4"), Map.entry("m4v", "video/mp4"), Map.entry("mov", "video/quicktime"),
            Map.entry("mkv", "video/x-matroska"), Map.entry("webm", "video/webm"), Map.entry("avi", "video/x-msvideo"),
            Map.entry("ts", "video/mp2t"), Map.entry("m2ts", "video/mp2t"), Map.entry("mts", "video/mp2t"),
            Map.entry("wmv", "video/x-ms-wmv"), Map.entry("mpg", "video/mpeg"), Map.entry("mpeg", "video/mpeg"),
            Map.entry("flv", "video/x-flv"), Map.entry("ogv", "video/ogg"), Map.entry("3gp", "video/3gpp"),
            Map.entry("jpg", "image/jpeg"), Map.entry("jpeg", "image/jpeg"), Map.entry("png", "image/png"),
            Map.entry("webp", "image/webp"), Map.entry("svg", "image/svg+xml"), Map.entry("ico", "image/x-icon"),
            Map.entry("html", "text/html; charset=utf-8"), Map.entry("css", "text/css; charset=utf-8"),
            Map.entry("js", "text/javascript; charset=utf-8"), Map.entry("json", "application/json"),
            Map.entry("vtt", "text/vtt; charset=utf-8"), Map.entry("woff2", "font/woff2"),
            Map.entry("txt", "text/plain; charset=utf-8"), Map.entry("srt", "text/plain; charset=utf-8"));

    private FileSender() {
    }

    public static String contentType(String fileName) {
        int dot = fileName.lastIndexOf('.');
        String ext = dot >= 0 ? fileName.substring(dot + 1).toLowerCase(Locale.ROOT) : "";
        return TYPES.getOrDefault(ext, "application/octet-stream");
    }

    /**
     * Sends a file. {@code maxAgeSeconds} > 0 lets the browser cache it (artwork); videos use 0.
     */
    public static void send(Exchange ex, Path file, int maxAgeSeconds) throws IOException {
        if (!Files.isRegularFile(file)) throw ApiException.notFound();
        long size = Files.size(file);
        long modified = Files.getLastModifiedTime(file).toMillis();
        String etag = "\"" + Long.toHexString(size) + "-" + Long.toHexString(modified) + "\"";
        Headers h = ex.raw.getResponseHeaders();
        h.set("Content-Type", contentType(file.getFileName().toString()));
        h.set("Accept-Ranges", "bytes");
        h.set("ETag", etag);
        h.set("Last-Modified", HTTP_DATE.format(Instant.ofEpochMilli(modified)));
        h.set("Cache-Control", maxAgeSeconds > 0 ? "private, max-age=" + maxAgeSeconds : "no-cache");

        if (etag.equals(ex.header("If-None-Match"))) {
            ex.markResponded();
            ex.raw.sendResponseHeaders(304, -1);
            ex.raw.close();
            return;
        }

        long start = 0;
        long end = size - 1;
        int status = 200;
        String range = ex.header("Range");
        if (range != null && size > 0) {
            Matcher m = RANGE.matcher(range.trim());
            // Multiple ranges ("bytes=0-1,5-9") are not supported; browsers never ask for them for media.
            if (m.matches() && !(m.group(1).isEmpty() && m.group(2).isEmpty())) {
                if (m.group(1).isEmpty()) {
                    long suffix = Long.parseLong(m.group(2));
                    start = Math.max(0, size - suffix);
                } else {
                    start = Long.parseLong(m.group(1));
                    if (!m.group(2).isEmpty()) end = Math.min(Long.parseLong(m.group(2)), size - 1);
                }
                if (start >= size || start > end) {
                    h.set("Content-Range", "bytes */" + size);
                    ex.markResponded();
                    ex.raw.sendResponseHeaders(416, -1);
                    ex.raw.close();
                    return;
                }
                status = 206;
                h.set("Content-Range", "bytes " + start + "-" + end + "/" + size);
            }
        }

        long length = size == 0 ? 0 : end - start + 1;
        ex.markResponded();
        if (ex.method().equals("HEAD")) {
            h.set("Content-Length", Long.toString(length));
            ex.raw.sendResponseHeaders(status, -1);
            ex.raw.close();
            return;
        }
        ex.raw.sendResponseHeaders(status, length == 0 ? -1 : length);
        if (length == 0) {
            ex.raw.close();
            return;
        }
        try (FileChannel ch = FileChannel.open(file, StandardOpenOption.READ);
             OutputStream out = ex.raw.getResponseBody()) {
            ch.position(start);
            InputStream in = Channels.newInputStream(ch);
            byte[] buf = new byte[BUFFER];
            long remaining = length;
            while (remaining > 0) {
                int n = in.read(buf, 0, (int) Math.min(buf.length, remaining));
                if (n < 0) break;
                out.write(buf, 0, n);
                remaining -= n;
            }
        } catch (IOException e) {
            // The player closed the connection (seeking, stopping) – normal for video streams.
        } finally {
            ex.raw.close();
        }
    }
}

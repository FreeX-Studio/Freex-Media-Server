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

package de.mirranet.freex.server.transcode;

import de.mirranet.freex.server.model.MediaFile;
import de.mirranet.freex.server.model.MediaItem;

import java.io.IOException;
import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;

/**
 * Delivers subtitles as WebVTT: sidecar files are converted directly, text subtitle streams
 * inside the video are extracted with ffmpeg once and cached in {@code <data>/subtitles}.
 */
public final class SubtitleService {

    private static final long MAX_FILE = 10L * 1024 * 1024;

    private final String ffmpeg;
    private final Path cache;
    private final Map<String, Object> locks = new ConcurrentHashMap<>();

    public SubtitleService(String ffmpeg, Path cacheDir) throws IOException {
        this.ffmpeg = ffmpeg;
        this.cache = cacheDir.resolve("subtitles");
        Files.createDirectories(cache);
    }

    /** WebVTT text for the subtitle id ({@code x<n>} sidecar, {@code e<stream>} embedded), or null. */
    public String vtt(MediaItem item, String id) throws IOException, InterruptedException {
        MediaFile f = item.file;
        if (f == null || id == null || id.length() < 2) return null;
        int n;
        try {
            n = Integer.parseInt(id.substring(1));
        } catch (NumberFormatException e) {
            return null;
        }
        if (id.charAt(0) == 'x') {
            if (n < 0 || n >= f.subtitles.size()) return null;
            MediaFile.Subtitle s = f.subtitles.get(n);
            Path p = Path.of(s.path());
            if (!Files.isRegularFile(p) || Files.size(p) > MAX_FILE) return null;
            String text = readText(p);
            return s.format().equals("vtt") ? text : srtToVtt(text);
        }
        if (id.charAt(0) == 'e') {
            MediaFile.SubtitleStream s = f.subtitleStream(n);
            if (s == null || !s.textBased()) return null;
            return extract(item, s);
        }
        return null;
    }

    private String extract(MediaItem item, MediaFile.SubtitleStream s) throws IOException, InterruptedException {
        String key = item.id + "-" + s.streamIndex() + "-" + Long.toHexString(item.file.modified);
        Path out = cache.resolve(key + ".vtt");
        if (Files.exists(out)) return Files.readString(out, StandardCharsets.UTF_8);
        Object lock = locks.computeIfAbsent(key, k -> new Object());
        synchronized (lock) {
            try {
                if (Files.exists(out)) return Files.readString(out, StandardCharsets.UTF_8);
                Path tmp = cache.resolve(key + ".tmp.vtt");
                Process p = new ProcessBuilder(ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
                        "-i", item.file.path, "-map", "0:" + s.streamIndex(), "-c:s", "webvtt", "-f", "webvtt", tmp.toString())
                        .redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
                // Reading the stream means reading the whole file; large remuxes take a while.
                if (!p.waitFor(180, TimeUnit.SECONDS)) {
                    p.destroyForcibly();
                    Files.deleteIfExists(tmp);
                    return null;
                }
                if (p.exitValue() != 0 || !Files.exists(tmp)) {
                    Files.deleteIfExists(tmp);
                    return null;
                }
                Files.move(tmp, out, StandardCopyOption.REPLACE_EXISTING);
                return Files.readString(out, StandardCharsets.UTF_8);
            } finally {
                locks.remove(key);
            }
        }
    }

    static String readText(Path p) throws IOException {
        byte[] data = Files.readAllBytes(p);
        String s = new String(data, StandardCharsets.UTF_8);
        // Many older German subtitles are Latin-1/Windows-1252.
        if (s.contains("�")) s = new String(data, Charset.forName("windows-1252"));
        if (s.startsWith("﻿")) s = s.substring(1);
        return s;
    }

    static String srtToVtt(String srt) {
        StringBuilder sb = new StringBuilder("WEBVTT\n\n");
        for (String line : srt.replace("\r\n", "\n").replace('\r', '\n').split("\n", -1)) {
            sb.append(line.contains("-->") ? line.replace(',', '.') : line).append('\n');
        }
        return sb.toString();
    }
}

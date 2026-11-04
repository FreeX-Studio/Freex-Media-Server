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

package de.mirranet.freex.server.scan;

import de.mirranet.freex.server.model.MediaItem;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Semaphore;
import java.util.concurrent.TimeUnit;
import java.util.logging.Logger;

/**
 * Extracts a still frame with {@code ffmpeg} for movies and episodes without artwork.
 * Frames are generated on first request and cached in {@code <data>/thumbs}.
 */
public final class ThumbnailService {

    private static final Logger LOG = Logger.getLogger(ThumbnailService.class.getName());

    private final String ffmpeg;
    private final Path dir;
    private final boolean available;
    private final Map<String, Object> locks = new ConcurrentHashMap<>();
    /** Remembers items where extraction failed so they are not retried on every request. */
    private final Map<String, Long> failed = new ConcurrentHashMap<>();
    /** At most two ffmpeg processes at once, a library page can request dozens of frames. */
    private final Semaphore slots = new Semaphore(2);

    public ThumbnailService(String ffmpeg, Path cacheDir) throws IOException {
        this.ffmpeg = ffmpeg;
        this.dir = cacheDir.resolve("thumbs");
        Files.createDirectories(dir);
        this.available = check();
        if (!available) LOG.warning("ffmpeg not found ('" + ffmpeg + "'): no generated thumbnails");
    }

    public boolean available() {
        return available;
    }

    private boolean check() {
        try {
            Process p = new ProcessBuilder(ffmpeg, "-version").redirectErrorStream(true).start();
            p.getInputStream().readAllBytes();
            return p.waitFor(10, TimeUnit.SECONDS) && p.exitValue() == 0;
        } catch (IOException e) {
            return false;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    /** Path of the cached frame, generating it if needed; null if none can be produced. */
    public Path frame(MediaItem item) {
        if (item.file == null) return null;
        // The file's modification time is part of the name, so a replaced file gets a new frame.
        Path out = dir.resolve(item.id + "-" + Long.toHexString(item.file.modified) + ".jpg");
        if (Files.exists(out)) return out;
        if (!available) return null;
        Long failedAt = failed.get(item.id);
        if (failedAt != null && System.currentTimeMillis() - failedAt < TimeUnit.HOURS.toMillis(6)) return null;

        Object lock = locks.computeIfAbsent(item.id, k -> new Object());
        synchronized (lock) {
            try {
                if (Files.exists(out)) return out;
                slots.acquire();
                try {
                    return extract(item, out) ? out : null;
                } finally {
                    slots.release();
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return null;
            } finally {
                locks.remove(item.id);
            }
        }
    }

    private boolean extract(MediaItem item, Path out) throws InterruptedException {
        long durMs = item.file.durationMs;
        // Skip intros and black frames: 15 % in, but at most 5 minutes; 30 s if the length is unknown.
        long seekMs = durMs > 0 ? Math.min(durMs * 15 / 100, 300_000) : 30_000;
        Path tmp = out.resolveSibling(out.getFileName() + ".tmp.jpg");
        Process p = null;
        try {
            p = new ProcessBuilder(ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
                    "-ss", String.format(Locale.ROOT, "%.3f", seekMs / 1000.0),
                    "-i", item.file.path, "-frames:v", "1", "-vf", "scale=640:-2", "-q:v", "4", tmp.toString())
                    .redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
            boolean done = p.waitFor(60, TimeUnit.SECONDS);
            if (done && p.exitValue() == 0 && Files.exists(tmp) && Files.size(tmp) > 0) {
                Files.move(tmp, out, StandardCopyOption.REPLACE_EXISTING);
                return true;
            }
            failed.put(item.id, System.currentTimeMillis());
            return false;
        } catch (IOException e) {
            failed.put(item.id, System.currentTimeMillis());
            return false;
        } finally {
            if (p != null && p.isAlive()) p.destroyForcibly();
            try {
                Files.deleteIfExists(tmp);
            } catch (IOException ignored) {
                // nothing to clean up
            }
        }
    }
}

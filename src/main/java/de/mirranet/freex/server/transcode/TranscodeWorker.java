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

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.function.LongFunction;
import java.util.logging.Logger;

/**
 * Converts one playback in the background, like the transcoder processes of other media servers.
 * <p>
 * The player gets a playlist of all {@link Segments} at once and asks for whichever it needs.
 * The worker runs ffmpeg from the segment asked for; when the player seeks far ahead or back,
 * ffmpeg is restarted there, and everything converted before is kept and served again. ffmpeg
 * writes one small fragment per keyframe with timestamps of the source (shifted by
 * {@link #TS_OFFSET_MS}), so fragments of different runs fit together exactly; a segment is the
 * fragments between two segment borders, sent one after the other.
 * <p>
 * ffmpeg is paused while it is far ahead of the player and continued when the player catches up.
 */
final class TranscodeWorker {

    private static final Logger LOG = Logger.getLogger(TranscodeWorker.class.getName());

    /** All timestamps are shifted by this, so that no track starts below zero (e.g. AAC priming). */
    static final long TS_OFFSET_MS = 1000;
    /** How far a fragment may start before a segment border and still count as its first one. */
    private static final long TOLERANCE_MS = 200;
    /** A segment at most this far ahead of the running ffmpeg is waited for instead of restarting. */
    private static final long WAIT_AHEAD_MS = 20_000;
    /** Pause ffmpeg when it is this far ahead of the player; continue below the lower mark. */
    private static final long PAUSE_AHEAD_MS = 120_000;
    private static final long RESUME_AHEAD_MS = 60_000;
    /** Folders of earlier runs kept for seeking back. */
    private static final int KEEP_RUNS = 12;

    private record Fragment(long startMs, Path path) {
    }

    /** One ffmpeg process, started at a segment border. */
    private static final class Run {
        final Path dir;
        final Process process;
        final long startMs;
        final List<Fragment> fragments = new ArrayList<>();
        int listed;
        int timescale;
        boolean paused;

        Run(Path dir, Process process, long startMs) {
            this.dir = dir;
            this.process = process;
            this.startMs = startMs;
        }

        long producedUntil() {
            return fragments.isEmpty() ? startMs : fragments.get(fragments.size() - 1).startMs;
        }

        boolean finished() {
            return !process.isAlive() && process.exitValue() == 0;
        }

        boolean failed() {
            return !process.isAlive() && process.exitValue() != 0;
        }
    }

    private final String id;
    private final Path dir;
    private final Segments segments;
    private final LongFunction<List<String>> command;
    private final List<String> priority;
    private final boolean canSignal;
    /** Newest first. */
    private final List<Run> runs = new ArrayList<>();
    private Run active;
    private int runNumber;
    private volatile int lastRequested;
    volatile long lastAccess = System.currentTimeMillis();

    /**
     * @param command  ffmpeg command line that converts from a position (ms) on, writing into the
     *                 working directory
     * @param priority prefix that lowers the priority of ffmpeg, e.g. {@code nice -n 10}, or empty
     */
    TranscodeWorker(String id, Path dir, Segments segments, LongFunction<List<String>> command, List<String> priority,
                    boolean canSignal) {
        this.id = id;
        this.dir = dir;
        this.segments = segments;
        this.command = command;
        this.priority = priority;
        this.canSignal = canSignal;
    }

    String playlist() {
        lastAccess = System.currentTimeMillis();
        return segments.playlist();
    }

    /** Starts converting at the segment that contains a position. */
    synchronized void begin(long positionMs) throws IOException {
        int i = segments.indexAt(positionMs);
        lastRequested = i;
        startRun(i);
    }

    /** The init segment, once ffmpeg has written it; null if that does not happen in time. */
    Path init(long timeoutMs) throws InterruptedException {
        lastAccess = System.currentTimeMillis();
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            synchronized (this) {
                for (Run r : runs) {
                    refresh(r);
                    if (!r.fragments.isEmpty()) return r.dir.resolve("init.mp4");
                }
                if (active == null || active.failed()) return null;
            }
            Thread.sleep(100);
        }
        return null;
    }

    /**
     * The fragment files that make up segment {@code i}, in order, waiting for ffmpeg as needed.
     * Returns null if the segment does not exist or cannot be converted in time.
     */
    List<Path> segment(int i, long timeoutMs) throws InterruptedException, IOException {
        if (i < 0 || i >= segments.count()) return null;
        lastAccess = System.currentTimeMillis();
        lastRequested = i;
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (true) {
            synchronized (this) {
                for (Run r : runs) {
                    refresh(r);
                    List<Path> parts = collect(r, i);
                    if (parts != null) return parts;
                }
                // ffmpeg gave up before writing anything: restarting would only fail again.
                if (active != null && active.failed() && active.fragments.isEmpty()) return null;
                // Restarted exactly here, and still the segment did not come out (it has no keyframe at
                // its border): give up instead of restarting over and over.
                if (active != null && active.startMs == segments.start(i) && passed(active, i)) return null;
                if (active == null || !covers(active, i)) {
                    LOG.fine(() -> "Transcode " + id + ": restarting at segment " + i);
                    startRun(i);
                } else if (active.paused && active.producedUntil() - segments.start(i) < RESUME_AHEAD_MS) {
                    signal(active, "CONT");
                }
            }
            if (System.currentTimeMillis() > deadline) return null;
            Thread.sleep(100);
        }
    }

    /** Whether the running ffmpeg will reach segment {@code i} soon. */
    private boolean covers(Run r, int i) {
        long s = segments.start(i);
        return r.process.isAlive() && r.startMs <= s + TOLERANCE_MS && s <= r.producedUntil() + WAIT_AHEAD_MS
                && !passed(r, i);
    }

    /** Whether a run is already beyond segment {@code i} (and so could have delivered it). */
    private boolean passed(Run r, int i) {
        return r.producedUntil() > segments.end(i) + TOLERANCE_MS || r.finished();
    }

    /** The fragments of segment {@code i} in run {@code r}, if the run has all of them. */
    private List<Path> collect(Run r, int i) {
        long s = segments.start(i);
        long e = segments.end(i);
        List<Path> parts = new ArrayList<>();
        boolean complete = false;
        for (Fragment f : r.fragments) {
            if (f.startMs < s - TOLERANCE_MS) continue;
            if (f.startMs >= e - TOLERANCE_MS) {
                complete = true;
                break;
            }
            // The first fragment has to start at the border, or the run began inside the segment.
            if (parts.isEmpty() && f.startMs > s + TOLERANCE_MS && !(i == 0 && r.startMs == 0)) return null;
            parts.add(f.path);
        }
        if (!complete && r.finished() && !parts.isEmpty()) complete = true; // the end of the title
        return complete && !parts.isEmpty() ? parts : null;
    }

    /** Reads the fragments ffmpeg has finished since the last look (it lists them in its playlist). */
    private void refresh(Run r) {
        Path list = r.dir.resolve("index.m3u8");
        if (!Files.exists(list)) return;
        List<String> names = new ArrayList<>();
        try {
            for (String line : Files.readAllLines(list, StandardCharsets.UTF_8)) {
                if (line.startsWith("seg") && line.endsWith(".m4s")) names.add(line.trim());
            }
            if (r.timescale == 0 && names.size() > r.listed) r.timescale = videoTimescale(r.dir.resolve("init.mp4"));
            for (int k = r.listed; k < names.size(); k++) {
                Path p = r.dir.resolve(names.get(k));
                long start = fragmentStart(p, r.timescale);
                if (start < 0) break; // not readable yet, try again later
                r.fragments.add(new Fragment(start - TS_OFFSET_MS, p));
                r.listed = k + 1;
            }
        } catch (IOException e) {
            // ffmpeg is just rewriting the playlist; next time
        }
    }

    private void startRun(int i) throws IOException {
        if (active != null) kill(active);
        long start = segments.start(i);
        Path runDir = dir.resolve("run" + (++runNumber));
        Files.createDirectories(runDir);
        List<String> cmd = new ArrayList<>(priority);
        cmd.addAll(command.apply(start));
        Process p = new ProcessBuilder(cmd)
                .directory(runDir.toFile())
                .redirectErrorStream(true)
                .redirectOutput(runDir.resolve("ffmpeg.log").toFile())
                .start();
        active = new Run(runDir, p, start);
        runs.add(0, active);
        while (runs.size() > KEEP_RUNS) TranscodeManager.deleteTree(runs.remove(runs.size() - 1).dir);
    }

    /** Pauses ffmpeg while it is far ahead of the player; called regularly by the manager. */
    synchronized void throttle() {
        Run r = active;
        if (r == null || r.paused || !r.process.isAlive()) return;
        refresh(r);
        if (r.producedUntil() - segments.start(lastRequested) > PAUSE_AHEAD_MS) signal(r, "STOP");
    }

    /** The last lines of ffmpeg's output, for error messages. */
    synchronized String log() {
        Run r = active != null ? active : runs.isEmpty() ? null : runs.get(0);
        if (r == null) return "";
        try {
            List<String> lines = Files.readAllLines(r.dir.resolve("ffmpeg.log"), StandardCharsets.UTF_8);
            return String.join("\n", lines.subList(Math.max(0, lines.size() - 5), lines.size()));
        } catch (IOException e) {
            return "";
        }
    }

    synchronized void stop() {
        if (active != null) kill(active);
        active = null;
    }

    private void kill(Run r) {
        if (r.paused) signal(r, "CONT");
        r.process.destroy();
        try {
            if (!r.process.waitFor(3, TimeUnit.SECONDS)) r.process.destroyForcibly();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            r.process.destroyForcibly();
        }
    }

    private void signal(Run r, String sig) {
        if (!canSignal) return;
        try {
            Process k = new ProcessBuilder("kill", "-" + sig, Long.toString(r.process.pid())).redirectErrorStream(true).start();
            k.getInputStream().readAllBytes();
            if (k.waitFor(5, TimeUnit.SECONDS) && k.exitValue() == 0) r.paused = sig.equals("STOP");
        } catch (IOException e) {
            // ignore: ffmpeg just keeps running
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    // ------------------------------------------------------------------ MP4 boxes

    /** Timescale of the video track (the first track) from an init segment, 0 if unknown. */
    static int videoTimescale(Path init) {
        try {
            byte[] b = Files.readAllBytes(init);
            int i = indexOf(b, "mdhd", 0);
            if (i < 4) return 0;
            int version = b[i + 4];
            return readInt(b, i + 4 + 4 + (version == 1 ? 16 : 8));
        } catch (IOException e) {
            return 0;
        }
    }

    /** Start of a fragment in ms: the decode time of its video track (track 1); -1 if unreadable. */
    static long fragmentStart(Path fragment, int timescale) {
        if (timescale <= 0) return -1;
        byte[] b = new byte[64 * 1024];
        int n;
        try (InputStream in = Files.newInputStream(fragment)) {
            n = in.readNBytes(b, 0, b.length);
        } catch (IOException e) {
            return -1;
        }
        // moof > traf > (tfhd with the track id, tfdt with the decode time)
        int traf = indexOf(b, "traf", 0);
        while (traf >= 4 && traf < n) {
            int end = Math.min(n, traf - 4 + readInt(b, traf - 4));
            int tfhd = indexOf(b, "tfhd", traf);
            int tfdt = indexOf(b, "tfdt", traf);
            if (tfhd > 0 && tfdt > 0 && tfhd < end && tfdt < end && readInt(b, tfhd + 8) == 1) {
                int version = b[tfdt + 4];
                long t = version == 1 ? readLong(b, tfdt + 8) : readInt(b, tfdt + 8) & 0xffffffffL;
                return Math.round(t * 1000.0 / timescale);
            }
            traf = indexOf(b, "traf", end);
        }
        return -1;
    }

    private static int indexOf(byte[] b, String type, int from) {
        byte[] t = type.getBytes(StandardCharsets.US_ASCII);
        outer:
        for (int i = Math.max(0, from); i + 4 <= b.length; i++) {
            for (int k = 0; k < 4; k++) if (b[i + k] != t[k]) continue outer;
            return i;
        }
        return -1;
    }

    private static int readInt(byte[] b, int at) {
        return ((b[at] & 0xff) << 24) | ((b[at + 1] & 0xff) << 16) | ((b[at + 2] & 0xff) << 8) | (b[at + 3] & 0xff);
    }

    private static long readLong(byte[] b, int at) {
        return ((long) readInt(b, at) << 32) | (readInt(b, at + 4) & 0xffffffffL);
    }
}

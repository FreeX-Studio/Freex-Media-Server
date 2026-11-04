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

import de.mirranet.freex.server.i18n.Text;
import de.mirranet.freex.server.i18n.Translatable;
import de.mirranet.freex.server.model.MediaFile;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.scan.MediaProbe;
import de.mirranet.freex.server.util.Ids;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.logging.Level;
import java.util.logging.Logger;
import java.util.regex.Pattern;
import java.util.stream.Stream;

/**
 * Runs ffmpeg to produce HLS (fragmented MP4 segments) for one playback.
 * <p>
 * A session starts at a given position; seeking far beyond what has been produced starts a new
 * session there (the player handles the time offset). Sessions nobody requested segments from
 * for a while are stopped and their files deleted. To save CPU and disk, ffmpeg is paused when
 * it is far ahead of the player and continued when the player catches up (Linux/macOS).
 */
public final class TranscodeManager implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(TranscodeManager.class.getName());

    static final int SEGMENT_SECONDS = 4;
    private static final long IDLE_TIMEOUT_MS = 60_000;
    /** Pause ffmpeg when it is this many segments ahead of the player, continue at the lower mark. */
    private static final int THROTTLE_AHEAD = 45;
    private static final int RESUME_AHEAD = 20;
    private static final Pattern SEGMENT_NAME = Pattern.compile("^(index\\.m3u8|init\\.mp4|seg\\d{5}\\.m4s)$");

    public static final class Session {
        public final String id;
        public final String userId;
        public final String itemId;
        public final long offsetMs;
        public final PlaybackPlanner.Mode mode;
        final Path dir;
        final Process process;
        volatile long lastAccess = System.currentTimeMillis();
        volatile int lastRequestedSegment;
        volatile boolean paused;

        Session(String id, String userId, String itemId, long offsetMs, PlaybackPlanner.Mode mode, Path dir, Process p) {
            this.id = id;
            this.userId = userId;
            this.itemId = itemId;
            this.offsetMs = offsetMs;
            this.mode = mode;
            this.dir = dir;
            this.process = p;
        }
    }

    /** Thrown when all transcoding slots are in use. */
    public static final class BusyException extends Exception implements Translatable {
        private static final long serialVersionUID = 1L;

        BusyException() {
            super("All transcoding slots are in use");
        }

        @Override
        public Text text() {
            return new Text("error.transcoderBusy");
        }
    }

    private final String ffmpeg;
    private final Path root;
    private final int maxSessions;
    private final String hwaccel;
    private final String vaapiDevice;
    private final String preset;
    /** Finds keyframes for conversions that copy the video; null without ffprobe. */
    private final MediaProbe probe;
    private final boolean canSignal;
    private final Map<String, Session> sessions = new ConcurrentHashMap<>();
    private final ScheduledExecutorService reaper = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "transcode-reaper");
        t.setDaemon(true);
        return t;
    });

    public TranscodeManager(String ffmpeg, Path root, int maxSessions, String hwaccel, String vaapiDevice, String preset,
                            MediaProbe probe) throws IOException {
        this.ffmpeg = ffmpeg;
        this.probe = probe;
        this.root = root;
        this.maxSessions = maxSessions;
        this.hwaccel = hwaccel;
        this.vaapiDevice = vaapiDevice;
        this.preset = preset.matches("[a-z]+") ? preset : "veryfast";
        this.canSignal = !System.getProperty("os.name", "").toLowerCase(Locale.ROOT).contains("win");
        Files.createDirectories(root);
        cleanDirectory(root); // leftovers from a crash
        reaper.scheduleWithFixedDelay(this::reap, 10, 10, TimeUnit.SECONDS);
        if (!hwaccel.equals("none")) {
            LOG.info("Transcoding uses hardware acceleration: " + hwaccel);
        }
    }

    /** Starts ffmpeg for the plan; stops any previous session of the same user for the same item. */
    public Session start(String userId, MediaItem item, PlaybackPlanner.Plan plan, long offsetMs) throws BusyException, IOException {
        sessions.values().stream()
                .filter(s -> s.userId.equals(userId) && s.itemId.equals(item.id))
                .toList()
                .forEach(s -> stop(s.id));
        if (sessions.size() >= maxSessions) throw new BusyException();

        // A copied video can only start at a keyframe. Starting exactly there makes audio and video
        // begin together; the session's offset is then the keyframe, a little before the requested
        // position, and the player seeks the rest of the way itself.
        if (copiesVideo(plan) && offsetMs > 0 && probe != null) {
            long key = probe.keyframeBefore(Path.of(item.file.path), item.file.videoStreamIndex, offsetMs);
            if (key >= 0) offsetMs = key;
        }

        String id = Ids.random(12);
        Path dir = root.resolve(id);
        Files.createDirectories(dir);
        List<String> cmd = command(item.file, plan, offsetMs, dir);
        LOG.fine(() -> "ffmpeg: " + String.join(" ", cmd));
        Process p = new ProcessBuilder(cmd)
                .directory(dir.toFile())
                .redirectErrorStream(true)
                .redirectOutput(dir.resolve("ffmpeg.log").toFile())
                .start();
        Session s = new Session(id, userId, item.id, offsetMs, plan.mode(), dir, p);
        sessions.put(id, s);
        LOG.info("Transcode " + id + " started: '" + item.title + "' " + plan.mode().name().toLowerCase(Locale.ROOT)
                + " from " + offsetMs / 1000 + " s (" + plan.reason() + ")");
        return s;
    }

    private static boolean copiesVideo(PlaybackPlanner.Plan plan) {
        return plan.mode() == PlaybackPlanner.Mode.REMUX || plan.mode() == PlaybackPlanner.Mode.AUDIO;
    }

    List<String> command(MediaFile f, PlaybackPlanner.Plan plan, long offsetMs, Path dir) {
        List<String> c = new ArrayList<>(List.of(ffmpeg, "-hide_banner", "-loglevel", "warning", "-nostdin", "-y"));
        boolean transcode = plan.mode() == PlaybackPlanner.Mode.TRANSCODE;
        // VAAPI cannot overlay bitmap subtitles in this simple pipeline; burn-ins use the software path.
        String hw = transcode ? (plan.burnSubtitle() >= 0 && hwaccel.equals("vaapi") ? "none" : hwaccel) : "none";
        if (hw.equals("vaapi")) c.addAll(List.of("-vaapi_device", vaapiDevice));
        // When copying the video, offsetMs is a keyframe: aim 1 ms behind it so that rounding cannot
        // make ffmpeg fall back to the keyframe before.
        double ss = offsetMs / 1000.0 + (copiesVideo(plan) ? 0.001 : 0);
        if (offsetMs > 0) c.addAll(List.of("-ss", String.format(Locale.ROOT, "%.3f", ss)));
        c.addAll(List.of("-i", f.path));

        String videoMap = "0:" + (f.videoStreamIndex >= 0 ? f.videoStreamIndex : "v:0");
        if (transcode) {
            List<String> filters = new ArrayList<>();
            String input = "[" + videoMap + "]";
            if (plan.burnSubtitle() >= 0) {
                filters.add(input + "[0:" + plan.burnSubtitle() + "]overlay=eof_action=pass");
                input = "";
            }
            int h = plan.maxHeight();
            String scale = h > 0 ? "scale=-2:'min(" + h + ",ih)'" : "scale=-2:ih";
            if (hw.equals("vaapi")) {
                filters.add(input + "format=nv12,hwupload," + (h > 0 ? "scale_vaapi=w=-2:h='min(" + h + ",ih)'" : "null"));
            } else {
                filters.add(input + scale + ",format=yuv420p");
            }
            c.addAll(List.of("-filter_complex", String.join(",", filters) + "[v]", "-map", "[v]"));
        } else {
            c.addAll(List.of("-map", videoMap));
        }
        if (plan.audioIndex() >= 0) {
            c.addAll(List.of("-map", "0:" + f.audioStreams.get(plan.audioIndex()).streamIndex()));
        }

        if (transcode) {
            long br = plan.videoBitrate();
            String gop = "expr:gte(t,n_forced*" + SEGMENT_SECONDS + ")";
            switch (hw) {
                // Forced keyframes must be IDR frames, or a segment does not start with a picture the
                // browser can decode and the image freezes until the next one.
                case "nvenc" -> c.addAll(List.of("-c:v", "h264_nvenc", "-preset", "p4", "-profile:v", "high", "-forced-idr", "1"));
                case "qsv" -> c.addAll(List.of("-c:v", "h264_qsv", "-preset", "veryfast", "-profile:v", "high", "-forced_idr", "1"));
                case "vaapi" -> c.addAll(List.of("-c:v", "h264_vaapi", "-profile:v", "high"));
                default -> c.addAll(List.of("-c:v", "libx264", "-preset", preset, "-profile:v", "high", "-level:v", "4.1",
                        "-sc_threshold", "0"));
            }
            c.addAll(List.of("-b:v", Long.toString(br), "-maxrate", Long.toString(br),
                    "-bufsize", Long.toString(br * 2), "-force_key_frames", gop));
        } else {
            c.addAll(List.of("-c:v", "copy"));
            if (PlaybackPlanner.codecKind(f.videoCodec).equals("hevc")) c.addAll(List.of("-tag:v", "hvc1"));
        }

        if (plan.audioIndex() >= 0) {
            if (plan.mode() == PlaybackPlanner.Mode.REMUX) {
                c.addAll(List.of("-c:a", "copy"));
            } else {
                // Keep the converted sound on the timestamps of the source: fills gaps and absorbs drift
                // (common in TV recordings), which would otherwise move sound and picture apart over time.
                c.addAll(List.of("-af", "aresample=async=1:min_hard_comp=0.100000:first_pts=0",
                        "-c:a", "aac", "-b:a", "192k", "-ac", "2"));
            }
        }

        c.addAll(List.of("-sn", "-dn", "-map_metadata", "-1", "-map_chapters", "-1",
                "-max_muxing_queue_size", "2048",
                "-f", "hls", "-hls_time", Integer.toString(SEGMENT_SECONDS), "-hls_list_size", "0",
                "-hls_playlist_type", "event", "-hls_segment_type", "fmp4",
                "-hls_fmp4_init_filename", "init.mp4",
                // Without edit lists every track keeps the timestamps it really has. Browsers apply
                // edit lists in fragmented MP4 inconsistently, which shifted sound against picture.
                "-hls_segment_options", "use_editlist=0",
                "-hls_segment_filename", "seg%05d.m4s",
                "-hls_flags", "temp_file+independent_segments",
                "-start_number", "0", "index.m3u8"));
        return c;
    }

    public Session session(String id) {
        return id == null ? null : sessions.get(id);
    }

    /**
     * Path of a session file once it exists, waiting up to {@code timeoutMs} for ffmpeg to write it.
     * Returns null if it does not appear (unknown name, ffmpeg failed, timeout).
     */
    public Path awaitFile(Session s, String name, long timeoutMs) throws InterruptedException {
        if (!SEGMENT_NAME.matcher(name).matches()) return null;
        s.lastAccess = System.currentTimeMillis();
        if (name.startsWith("seg")) {
            s.lastRequestedSegment = Integer.parseInt(name.substring(3, 8));
            resumeIfNeeded(s);
        }
        Path p = s.dir.resolve(name);
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (Files.exists(p) && (!name.equals("index.m3u8") || playlistHasSegment(p))) return p;
            if (!s.process.isAlive() && !Files.exists(p)) return null;
            Thread.sleep(150);
        }
        return Files.exists(p) ? p : null;
    }

    private static boolean playlistHasSegment(Path p) {
        try {
            return Files.readString(p, StandardCharsets.UTF_8).contains("#EXTINF");
        } catch (IOException e) {
            return false;
        }
    }

    /** The last lines of ffmpeg's output, for error messages. */
    public String log(Session s) {
        try {
            List<String> lines = Files.readAllLines(s.dir.resolve("ffmpeg.log"), StandardCharsets.UTF_8);
            return String.join("\n", lines.subList(Math.max(0, lines.size() - 5), lines.size()));
        } catch (IOException e) {
            return "";
        }
    }

    public void stop(String id) {
        Session s = sessions.remove(id);
        if (s == null) return;
        if (s.paused) signal(s, "CONT");
        s.process.destroy();
        try {
            if (!s.process.waitFor(3, TimeUnit.SECONDS)) s.process.destroyForcibly();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            s.process.destroyForcibly();
        }
        deleteTree(s.dir);
        LOG.info("Transcode " + id + " stopped");
    }

    public int activeSessions() {
        return sessions.size();
    }

    private void reap() {
        long now = System.currentTimeMillis();
        for (Session s : List.copyOf(sessions.values())) {
            try {
                if (now - s.lastAccess > IDLE_TIMEOUT_MS) {
                    stop(s.id);
                    continue;
                }
                throttle(s);
            } catch (RuntimeException e) {
                LOG.log(Level.WARNING, "Reaper failed for " + s.id, e);
            }
        }
    }

    private void throttle(Session s) {
        if (!canSignal || !s.process.isAlive()) return;
        int produced = producedSegments(s);
        if (!s.paused && produced - s.lastRequestedSegment > THROTTLE_AHEAD) {
            if (signal(s, "STOP")) s.paused = true;
        }
    }

    private void resumeIfNeeded(Session s) {
        if (s.paused && producedSegments(s) - s.lastRequestedSegment < RESUME_AHEAD) {
            if (signal(s, "CONT")) s.paused = false;
        }
    }

    private static int producedSegments(Session s) {
        try (Stream<Path> files = Files.list(s.dir)) {
            return (int) files.filter(p -> p.getFileName().toString().endsWith(".m4s")).count();
        } catch (IOException e) {
            return 0;
        }
    }

    private boolean signal(Session s, String sig) {
        try {
            Process k = new ProcessBuilder("kill", "-" + sig, Long.toString(s.process.pid()))
                    .redirectErrorStream(true).start();
            k.getInputStream().readAllBytes();
            return k.waitFor(5, TimeUnit.SECONDS) && k.exitValue() == 0;
        } catch (IOException e) {
            return false;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    /**
     * Removes leftover session folders. Only folders named like a session id are touched, so a
     * misconfigured transcoder.directory (e.g. the data directory) can never be wiped.
     */
    private static void cleanDirectory(Path dir) {
        try (Stream<Path> s = Files.list(dir)) {
            s.filter(p -> Files.isDirectory(p) && p.getFileName().toString().matches("[0-9a-f]{24}"))
                    .forEach(TranscodeManager::deleteTree);
        } catch (IOException e) {
            LOG.fine("Cannot clean " + dir + ": " + e.getMessage());
        }
    }

    private static void deleteTree(Path p) {
        try (Stream<Path> s = Files.walk(p)) {
            s.sorted(Comparator.reverseOrder()).forEach(x -> {
                try {
                    Files.deleteIfExists(x);
                } catch (IOException ignored) {
                    // best effort
                }
            });
        } catch (IOException ignored) {
            // already gone
        }
    }

    @Override
    public void close() {
        reaper.shutdownNow();
        for (String id : List.copyOf(sessions.keySet())) stop(id);
    }
}

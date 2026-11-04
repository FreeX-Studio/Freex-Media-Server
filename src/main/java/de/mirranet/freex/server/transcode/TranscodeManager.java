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
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.logging.Level;
import java.util.logging.Logger;
import java.util.stream.Stream;

/**
 * Manages the conversions: one {@link TranscodeWorker} per playback that is not played directly.
 * <p>
 * A session lasts for the whole playback, seeking included: its playlist covers the whole title,
 * and the worker restarts ffmpeg wherever the player asks for a segment. Sessions nobody asked
 * for anything for a while are stopped and their files deleted. ffmpeg runs with lower CPU and
 * I/O priority where the system offers {@code nice}/{@code ionice}, so the server and other
 * playbacks stay responsive.
 */
public final class TranscodeManager implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(TranscodeManager.class.getName());

    /** Length of converted segments; for copied video the minimum, as it is cut at keyframes. */
    static final long SEGMENT_MS = 4000;
    private static final long IDLE_TIMEOUT_MS = 60_000;

    public static final class Session {
        public final String id;
        public final String userId;
        public final String itemId;
        public final PlaybackPlanner.Mode mode;
        final Path dir;
        final TranscodeWorker worker;

        Session(String id, String userId, String itemId, PlaybackPlanner.Mode mode, Path dir, TranscodeWorker worker) {
            this.id = id;
            this.userId = userId;
            this.itemId = itemId;
            this.mode = mode;
            this.dir = dir;
            this.worker = worker;
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
    /** Reads keyframes for conversions that copy the video; null without ffprobe. */
    private final MediaProbe probe;
    private final boolean canSignal;
    private final List<String> priority;
    private final Map<String, Session> sessions = new ConcurrentHashMap<>();
    /** Keyframes of recently played files (path, size and date as key). */
    private final Map<String, List<Long>> keyframes = new LinkedHashMap<>(16, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, List<Long>> eldest) {
            return size() > 32;
        }
    };
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
        this.priority = lowPriority();
        Files.createDirectories(root);
        cleanDirectory(root); // leftovers from a crash
        reaper.scheduleWithFixedDelay(this::reap, 5, 5, TimeUnit.SECONDS);
        if (!hwaccel.equals("none")) {
            LOG.info("Transcoding uses hardware acceleration: " + hwaccel);
        }
    }

    /** {@code nice} and {@code ionice} in front of ffmpeg, where they exist. */
    private static List<String> lowPriority() {
        List<String> p = new ArrayList<>();
        for (String dir : List.of("/usr/bin", "/bin")) {
            if (p.isEmpty() && Files.isExecutable(Path.of(dir, "nice"))) p.addAll(List.of(dir + "/nice", "-n", "10"));
        }
        for (String dir : List.of("/usr/bin", "/bin")) {
            if (Files.isExecutable(Path.of(dir, "ionice"))) {
                p.addAll(List.of(dir + "/ionice", "-c", "2", "-n", "7"));
                break;
            }
        }
        return List.copyOf(p);
    }

    /**
     * Starts a conversion for the plan, converting from {@code startMs} on first; stops any
     * previous session of the same user for the same item.
     */
    public Session start(String userId, MediaItem item, PlaybackPlanner.Plan plan, long startMs) throws BusyException, IOException {
        sessions.values().stream()
                .filter(s -> s.userId.equals(userId) && s.itemId.equals(item.id))
                .toList()
                .forEach(s -> stop(s.id));
        if (sessions.size() >= maxSessions) throw new BusyException();

        MediaFile f = item.file;
        Segments segments;
        if (copiesVideo(plan)) {
            List<Long> keys = keyframesOf(f);
            if (keys.isEmpty()) throw new IOException("The keyframes of the video cannot be read");
            segments = Segments.fromKeyframes(keys, f.durationMs, SEGMENT_MS);
        } else {
            segments = Segments.uniform(f.durationMs, SEGMENT_MS);
        }
        String id = Ids.random(12);
        Path dir = root.resolve(id);
        Files.createDirectories(dir);
        TranscodeWorker worker = new TranscodeWorker(id, dir, segments, at -> command(f, plan, at), priority, canSignal);
        Session s = new Session(id, userId, item.id, plan.mode(), dir, worker);
        sessions.put(id, s);
        worker.begin(startMs);
        LOG.info("Transcode " + id + " started: '" + item.title + "' " + plan.mode().name().toLowerCase(Locale.ROOT)
                + " from " + startMs / 1000 + " s, " + segments.count() + " segments (" + plan.reason() + ")");
        return s;
    }

    private List<Long> keyframesOf(MediaFile f) {
        Path p = Path.of(f.path);
        String key;
        try {
            key = f.path + "|" + Files.size(p) + "|" + Files.getLastModifiedTime(p).toMillis();
        } catch (IOException e) {
            return List.of();
        }
        synchronized (keyframes) {
            List<Long> known = keyframes.get(key);
            if (known != null) return known;
        }
        List<Long> keys = probe == null ? List.of() : probe.keyframes(p, f.videoStreamIndex, f.durationMs);
        if (!keys.isEmpty()) {
            synchronized (keyframes) {
                keyframes.put(key, keys);
            }
        }
        return keys;
    }

    private static boolean copiesVideo(PlaybackPlanner.Plan plan) {
        return plan.mode() == PlaybackPlanner.Mode.REMUX || plan.mode() == PlaybackPlanner.Mode.AUDIO;
    }

    /** The ffmpeg command line that converts from {@code startMs} (a segment border) on. */
    List<String> command(MediaFile f, PlaybackPlanner.Plan plan, long startMs) {
        List<String> c = new ArrayList<>(List.of(ffmpeg, "-hide_banner", "-loglevel", "warning", "-nostdin", "-y"));
        boolean transcode = plan.mode() == PlaybackPlanner.Mode.TRANSCODE;
        // VAAPI cannot overlay bitmap subtitles in this simple pipeline; burn-ins use the software path.
        String hw = transcode ? (plan.burnSubtitle() >= 0 && hwaccel.equals("vaapi") ? "none" : hwaccel) : "none";
        if (hw.equals("vaapi")) c.addAll(List.of("-vaapi_device", vaapiDevice));
        if (startMs > 0) {
            if (transcode) {
                c.addAll(List.of("-ss", seconds(startMs)));
            } else {
                // A copied video starts at the keyframe the segment begins with; audio starts there as
                // well (without -noaccurate_seek ffmpeg drops the sound of the first fragment). 1 ms
                // later, so that rounding cannot make ffmpeg take the keyframe before.
                c.addAll(List.of("-noaccurate_seek", "-ss", seconds(startMs + 1)));
            }
        }
        // Keep the timestamps of the source, so that fragments of different runs fit together.
        c.addAll(List.of("-copyts", "-start_at_zero", "-i", f.path));

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
            // Keyframes exactly on the segment grid. t counts from the start of this run (also with
            // -copyts), and every run starts on the grid.
            String gop = "expr:gte(t,n_forced*" + seconds(SEGMENT_MS) + ")";
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
                c.addAll(List.of("-af", "aresample=async=1:min_hard_comp=0.100000",
                        "-c:a", "aac", "-b:a", "192k", "-ac", "2"));
            }
        }

        c.addAll(List.of("-sn", "-dn", "-map_metadata", "-1", "-map_chapters", "-1",
                "-max_muxing_queue_size", "2048",
                // Real timestamps in every fragment: no shifting per track, no edit lists (browsers apply
                // them inconsistently), and a fixed offset so that nothing starts below zero.
                "-avoid_negative_ts", "disabled", "-output_ts_offset", seconds(TranscodeWorker.TS_OFFSET_MS),
                "-f", "hls", "-hls_time", "0.1", "-hls_list_size", "0", // one fragment per keyframe
                "-hls_playlist_type", "event", "-hls_segment_type", "fmp4",
                "-hls_fmp4_init_filename", "init.mp4",
                "-hls_segment_options", "use_editlist=0:movflags=+frag_discont+negative_cts_offsets",
                "-hls_segment_filename", "seg%05d.m4s",
                "-hls_flags", "temp_file+independent_segments",
                "-start_number", "0", "index.m3u8"));
        return c;
    }

    private static String seconds(long ms) {
        return String.format(Locale.ROOT, "%.3f", ms / 1000.0);
    }

    public Session session(String id) {
        return id == null ? null : sessions.get(id);
    }

    /** The playlist of all segments of a session. */
    public String playlist(Session s) {
        return s.worker.playlist();
    }

    /** The init segment of a session, waiting for ffmpeg to write it; null if it does not. */
    public Path init(Session s, long timeoutMs) throws InterruptedException {
        return s.worker.init(timeoutMs);
    }

    /** The files of a segment in order (see {@link TranscodeWorker#segment}); null if it fails. */
    public List<Path> segment(Session s, int index, long timeoutMs) throws InterruptedException, IOException {
        return s.worker.segment(index, timeoutMs);
    }

    /** The last lines of ffmpeg's output, for error messages. */
    public String log(Session s) {
        return s.worker.log();
    }

    public void stop(String id) {
        Session s = sessions.remove(id);
        if (s == null) return;
        s.worker.stop();
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
                if (now - s.worker.lastAccess > IDLE_TIMEOUT_MS) stop(s.id);
                else s.worker.throttle();
            } catch (RuntimeException e) {
                LOG.log(Level.WARNING, "Reaper failed for " + s.id, e);
            }
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

    static void deleteTree(Path p) {
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

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

import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.MediaFile;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeSet;
import java.util.concurrent.TimeUnit;
import java.util.logging.Logger;

/** Reads duration, codecs and resolution of a video file with {@code ffprobe}. */
public final class MediaProbe {

    private static final Logger LOG = Logger.getLogger(MediaProbe.class.getName());
    private static final long TIMEOUT_SECONDS = 60;

    private final String ffprobe;
    private volatile boolean available;

    public MediaProbe(String ffprobe) {
        this.ffprobe = ffprobe;
        this.available = check();
        if (!available) {
            LOG.warning("ffprobe not found ('" + ffprobe + "'): files are indexed without duration and codec info");
        }
    }

    public boolean available() {
        return available;
    }

    private boolean check() {
        try {
            Process p = new ProcessBuilder(ffprobe, "-version").redirectErrorStream(true).start();
            p.getInputStream().readAllBytes();
            return p.waitFor(10, TimeUnit.SECONDS) && p.exitValue() == 0;
        } catch (IOException e) {
            return false;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    /** Fills the technical fields of {@code file}. Returns false if probing failed. */
    public boolean probe(Path video, MediaFile file) {
        if (!available) return false;
        Process p = null;
        try {
            p = new ProcessBuilder(ffprobe, "-v", "error", "-print_format", "json", "-show_format", "-show_streams",
                    video.toAbsolutePath().toString()).redirectError(ProcessBuilder.Redirect.DISCARD).start();
            String out = readLimited(p.getInputStream(), 8 * 1024 * 1024);
            if (!p.waitFor(TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
                LOG.warning("ffprobe timed out on " + video);
                return false;
            }
            if (p.exitValue() != 0 || out.isBlank()) return false;
            apply(Json.parseObject(out), file);
            file.probed = true;
            return true;
        } catch (IOException | RuntimeException e) {
            LOG.fine("ffprobe failed on " + video + ": " + e.getMessage());
            return false;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        } finally {
            if (p != null && p.isAlive()) p.destroyForcibly();
        }
    }

    /**
     * Times of the video keyframes of a file in ms from its start (as ffmpeg's {@code -ss} counts),
     * sorted; empty if they cannot be read. Conversions that copy the video can only cut there.
     * <p>
     * Instead of reading the whole file, ffprobe seeks to every other second and reports the
     * keyframe it lands on, which uses the file's index and is fast even for large files on a
     * network share. Keyframes closer together than that may be missed, which is fine: segments
     * only need some keyframes as their borders. Files without an index are read completely.
     */
    public List<Long> keyframes(Path video, int streamIndex, long durationMs) {
        if (!available || durationMs <= 0) return List.of();
        double start = startTime(video);
        StringBuilder intervals = new StringBuilder();
        for (long t = 0; t < durationMs; t += 2000) {
            if (!intervals.isEmpty()) intervals.append(',');
            intervals.append(String.format(Locale.ROOT, "%.3f%%+#1", start + t / 1000.0));
        }
        List<Long> keys = keyframePackets(video, streamIndex, intervals.toString(), start);
        // Without an index every seek lands at the start: then the file has to be read once.
        if (keys.size() < Math.max(2, durationMs / 120_000)) keys = keyframePackets(video, streamIndex, null, start);
        return keys;
    }

    private List<Long> keyframePackets(Path video, int streamIndex, String intervals, double startTime) {
        List<String> cmd = new ArrayList<>(List.of(ffprobe, "-v", "error", "-select_streams",
                streamIndex >= 0 ? Integer.toString(streamIndex) : "v:0", "-show_entries", "packet=pts_time,flags", "-of", "csv=p=0"));
        if (intervals != null) cmd.addAll(List.of("-read_intervals", intervals));
        cmd.add(video.toAbsolutePath().toString());
        Process p = null;
        try {
            p = new ProcessBuilder(cmd).redirectError(ProcessBuilder.Redirect.DISCARD).start();
            String out = readLimited(p.getInputStream(), 256 * 1024 * 1024);
            if (!p.waitFor(120, TimeUnit.SECONDS)) return List.of();
            TreeSet<Long> keys = new TreeSet<>();
            for (String line : out.split("\\R")) {
                String[] parts = line.split(",");
                if (parts.length >= 2 && parts[1].startsWith("K") && !parts[0].equals("N/A")) {
                    keys.add(Math.max(0, Math.round((parseDouble(parts[0]) - startTime) * 1000)));
                }
            }
            return new ArrayList<>(keys);
        } catch (IOException | RuntimeException e) {
            LOG.fine("ffprobe keyframes failed on " + video + ": " + e.getMessage());
            return List.of();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return List.of();
        } finally {
            if (p != null && p.isAlive()) p.destroyForcibly();
        }
    }

    /** Start time of the file's timestamps in seconds (often 0, not for MPEG-TS recordings). */
    private double startTime(Path video) {
        Process p = null;
        try {
            p = new ProcessBuilder(ffprobe, "-v", "error", "-show_entries", "format=start_time", "-of", "csv=p=0",
                    video.toAbsolutePath().toString()).redirectError(ProcessBuilder.Redirect.DISCARD).start();
            String st = readLimited(p.getInputStream(), 4096).trim();
            if (!p.waitFor(10, TimeUnit.SECONDS) || st.isEmpty() || st.equals("N/A")) return 0;
            return parseDouble(st);
        } catch (IOException | RuntimeException e) {
            return 0;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return 0;
        } finally {
            if (p != null && p.isAlive()) p.destroyForcibly();
        }
    }

    static void apply(Map<String, Object> json, MediaFile f) {
        Map<String, Object> format = Json.obj(json.get("format"));
        f.durationMs = Math.round(parseDouble(Json.str(format, "duration", "0")) * 1000);
        f.bitrate = Json.num(format, "bit_rate", 0);
        f.formatName = Json.str(format, "format_name", "");
        f.audioStreams = new ArrayList<>();
        f.subtitleStreams = new ArrayList<>();
        List<Object> streams = Json.list(json.get("streams"));
        boolean haveVideo = false;
        for (Object o : streams) {
            Map<String, Object> s = Json.obj(o);
            String type = Json.str(s, "codec_type", "");
            Map<String, Object> disposition = Json.obj(s.get("disposition"));
            Map<String, Object> tags = Json.obj(s.get("tags"));
            int index = (int) Json.num(s, "index", 0);
            String language = normalizeLanguage(Json.str(tags, "language", ""));
            String title = Json.str(tags, "title", "");
            boolean isDefault = Json.num(disposition, "default", 0) == 1;
            boolean coverArt = Json.num(disposition, "attached_pic", 0) == 1;
            switch (type) {
                case "video" -> {
                    if (!haveVideo && !coverArt) {
                        f.videoCodec = Json.str(s, "codec_name", "");
                        f.videoProfile = Json.str(s, "profile", "");
                        f.pixelFormat = Json.str(s, "pix_fmt", "");
                        f.width = (int) Json.num(s, "width", 0);
                        f.height = (int) Json.num(s, "height", 0);
                        f.videoStreamIndex = index;
                        haveVideo = true;
                    }
                }
                case "audio" -> f.audioStreams.add(new MediaFile.AudioStream(index, Json.str(s, "codec_name", ""),
                        (int) Json.num(s, "channels", 0), language, title, isDefault, Json.num(s, "bit_rate", 0)));
                case "subtitle" -> f.subtitleStreams.add(new MediaFile.SubtitleStream(index, Json.str(s, "codec_name", ""),
                        language, title, Json.num(disposition, "forced", 0) == 1 || title.toLowerCase().contains("forced"),
                        isDefault));
                default -> {
                }
            }
        }
        if (!f.audioStreams.isEmpty()) {
            // The browser plays the first audio track in direct play, so that is what we report.
            MediaFile.AudioStream first = f.audioStreams.get(0);
            f.audioCodec = first.codec();
            f.audioChannels = first.channels();
        }
        if (f.durationMs == 0) {
            // Some containers only carry the duration on the stream.
            for (Object o : streams) {
                double d = parseDouble(Json.str(Json.obj(o), "duration", "0"));
                if (d > 0) {
                    f.durationMs = Math.round(d * 1000);
                    break;
                }
            }
        }
        f.probeVersion = MediaFile.PROBE_VERSION;
    }

    /** ffprobe reports ISO 639-2 codes ("ger", "deu", "eng"); "und" means unknown. */
    private static String normalizeLanguage(String lang) {
        String l = lang.trim().toLowerCase();
        return l.equals("und") ? "" : l;
    }

    private static double parseDouble(String s) {
        try {
            return Double.parseDouble(s);
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    private static String readLimited(InputStream in, int max) throws IOException {
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        byte[] b = new byte[8192];
        int n;
        while ((n = in.read(b)) > 0) {
            if (buf.size() + n > max) throw new IOException("ffprobe output too large");
            buf.write(b, 0, n);
        }
        return buf.toString(StandardCharsets.UTF_8);
    }
}

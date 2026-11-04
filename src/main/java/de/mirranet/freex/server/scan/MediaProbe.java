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
     * Time of the last video keyframe at or before {@code ms}, relative to the start of the file
     * (as ffmpeg's {@code -ss} counts), or -1 if it cannot be found within 30 seconds before.
     * A conversion that copies the video must start at a keyframe; starting there exactly keeps
     * picture and sound together.
     */
    public long keyframeBefore(Path video, int streamIndex, long ms) {
        if (!available || ms <= 0) return -1;
        Process p = null;
        try {
            double startTime = 0;
            List<Double> keys = new ArrayList<>();
            // read_intervals takes timestamps of the file, so the file's start time is needed first.
            p = new ProcessBuilder(ffprobe, "-v", "error", "-show_entries", "format=start_time", "-of", "csv=p=0",
                    video.toAbsolutePath().toString()).redirectError(ProcessBuilder.Redirect.DISCARD).start();
            String st = readLimited(p.getInputStream(), 4096).trim();
            if (!p.waitFor(10, TimeUnit.SECONDS)) return -1;
            if (!st.isEmpty() && !st.equals("N/A")) startTime = parseDouble(st);
            double target = startTime + ms / 1000.0;
            String interval = String.format(Locale.ROOT, "%.3f%%%.3f", Math.max(startTime, target - 30), target + 0.001);
            p = new ProcessBuilder(ffprobe, "-v", "error", "-select_streams", streamIndex >= 0 ? Integer.toString(streamIndex) : "v:0",
                    "-show_entries", "packet=pts_time,flags", "-read_intervals", interval, "-of", "csv=p=0",
                    video.toAbsolutePath().toString()).redirectError(ProcessBuilder.Redirect.DISCARD).start();
            String out = readLimited(p.getInputStream(), 4 * 1024 * 1024);
            if (!p.waitFor(10, TimeUnit.SECONDS)) return -1;
            for (String line : out.split("\\R")) {
                String[] parts = line.split(",");
                if (parts.length >= 2 && parts[1].startsWith("K") && !parts[0].equals("N/A")) keys.add(parseDouble(parts[0]));
            }
            double best = -1;
            for (double k : keys) if (k <= target + 0.0005 && k > best) best = k;
            return best < 0 ? -1 : Math.max(0, Math.round((best - startTime) * 1000));
        } catch (IOException | RuntimeException e) {
            LOG.fine("ffprobe keyframe lookup failed on " + video + ": " + e.getMessage());
            return -1;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
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

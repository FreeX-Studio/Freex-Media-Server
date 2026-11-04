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

package de.mirranet.freex.server.model;

import de.mirranet.freex.server.json.Json;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** The video file behind a movie or episode, with the technical info read by ffprobe. */
public final class MediaFile {

    /**
     * Version of the probe data. Files indexed with an older version are probed again on the
     * next scan, so new fields (like the stream lists) get filled for existing libraries.
     */
    public static final int PROBE_VERSION = 2;

    /** Subtitle codecs that can be converted to WebVTT; everything else is a bitmap format. */
    private static final Set<String> TEXT_SUBTITLE_CODECS = Set.of("subrip", "srt", "ass", "ssa", "webvtt",
            "mov_text", "text", "ttml", "microdvd", "subviewer", "subviewer1", "realtext", "sami", "stl", "mpl2", "vplayer");

    /** A subtitle file lying next to the video ({@code Movie.de.srt}, {@code Movie.en.vtt}). */
    public record Subtitle(String path, String language, String format, boolean forced) {

        Map<String, Object> toStorage() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("path", path);
            m.put("language", language);
            m.put("format", format);
            m.put("forced", forced);
            return m;
        }

        static Subtitle fromStorage(Map<String, Object> m) {
            return new Subtitle(Json.str(m, "path", ""), Json.str(m, "language", ""),
                    Json.str(m, "format", "srt"), Json.bool(m, "forced", false));
        }
    }

    /**
     * An audio stream inside the file.
     *
     * @param streamIndex absolute stream index for ffmpeg ({@code -map 0:<streamIndex>})
     */
    public record AudioStream(int streamIndex, String codec, int channels, String language, String title,
                              boolean isDefault, long bitrate) {

        Map<String, Object> toStorage() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("streamIndex", streamIndex);
            m.put("codec", codec);
            m.put("channels", channels);
            m.put("language", language);
            m.put("title", title);
            m.put("default", isDefault);
            m.put("bitrate", bitrate);
            return m;
        }

        static AudioStream fromStorage(Map<String, Object> m) {
            return new AudioStream((int) Json.num(m, "streamIndex", 0), Json.str(m, "codec", ""),
                    (int) Json.num(m, "channels", 0), Json.str(m, "language", ""), Json.str(m, "title", ""),
                    Json.bool(m, "default", false), Json.num(m, "bitrate", 0));
        }
    }

    /** A subtitle stream inside the file. Bitmap subtitles (PGS, VobSub) can only be burned in. */
    public record SubtitleStream(int streamIndex, String codec, String language, String title, boolean forced,
                                 boolean isDefault) {

        public boolean textBased() {
            return TEXT_SUBTITLE_CODECS.contains(codec);
        }

        Map<String, Object> toStorage() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("streamIndex", streamIndex);
            m.put("codec", codec);
            m.put("language", language);
            m.put("title", title);
            m.put("forced", forced);
            m.put("default", isDefault);
            return m;
        }

        static SubtitleStream fromStorage(Map<String, Object> m) {
            return new SubtitleStream((int) Json.num(m, "streamIndex", 0), Json.str(m, "codec", ""),
                    Json.str(m, "language", ""), Json.str(m, "title", ""), Json.bool(m, "forced", false),
                    Json.bool(m, "default", false));
        }
    }

    public String path;
    public long size;
    public long modified;
    public String container = "";
    /** Container names as reported by ffprobe, e.g. "matroska,webm" or "mov,mp4,m4a,3gp,3g2,mj2". */
    public String formatName = "";
    public long durationMs;
    public String videoCodec = "";
    public String videoProfile = "";
    public String pixelFormat = "";
    public int videoStreamIndex = -1;
    public String audioCodec = "";
    public int width;
    public int height;
    public long bitrate;
    public int audioChannels;
    public boolean probed;
    public int probeVersion;
    public List<Subtitle> subtitles = new ArrayList<>();
    public List<AudioStream> audioStreams = new ArrayList<>();
    public List<SubtitleStream> subtitleStreams = new ArrayList<>();

    public boolean probeCurrent() {
        return probed && probeVersion >= PROBE_VERSION;
    }

    /** 10-bit or 4:2:2/4:4:4 video, which browsers usually can't decode for H.264. */
    public boolean highBitDepthOrChroma() {
        String p = pixelFormat == null ? "" : pixelFormat;
        return p.contains("10") || p.contains("12") || p.contains("422") || p.contains("444");
    }

    public Map<String, Object> toStorage() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("path", path);
        m.put("size", size);
        m.put("modified", modified);
        m.put("container", container);
        m.put("formatName", formatName);
        m.put("durationMs", durationMs);
        m.put("videoCodec", videoCodec);
        m.put("videoProfile", videoProfile);
        m.put("pixelFormat", pixelFormat);
        m.put("videoStreamIndex", videoStreamIndex);
        m.put("audioCodec", audioCodec);
        m.put("width", width);
        m.put("height", height);
        m.put("bitrate", bitrate);
        m.put("audioChannels", audioChannels);
        m.put("probed", probed);
        m.put("probeVersion", probeVersion);
        List<Object> subs = new ArrayList<>();
        for (Subtitle s : subtitles) subs.add(s.toStorage());
        m.put("subtitles", subs);
        List<Object> audio = new ArrayList<>();
        for (AudioStream a : audioStreams) audio.add(a.toStorage());
        m.put("audioStreams", audio);
        List<Object> subStreams = new ArrayList<>();
        for (SubtitleStream s : subtitleStreams) subStreams.add(s.toStorage());
        m.put("subtitleStreams", subStreams);
        return m;
    }

    /**
     * API view: no absolute paths. Subtitles are listed with an id: {@code x<n>} for files next to
     * the video, {@code e<streamIndex>} for streams inside it.
     */
    public Map<String, Object> toPublic() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("size", size);
        m.put("container", container);
        m.put("durationMs", durationMs);
        m.put("videoCodec", videoCodec);
        m.put("videoProfile", videoProfile);
        m.put("pixelFormat", pixelFormat);
        m.put("audioCodec", audioCodec);
        m.put("width", width);
        m.put("height", height);
        m.put("bitrate", bitrate);
        m.put("audioChannels", audioChannels);
        m.put("resolution", resolutionLabel());
        m.put("fileName", fileName());
        List<Object> audio = new ArrayList<>();
        for (int i = 0; i < audioStreams.size(); i++) {
            AudioStream a = audioStreams.get(i);
            Map<String, Object> am = new LinkedHashMap<>();
            am.put("index", i);
            am.put("codec", a.codec());
            am.put("channels", a.channels());
            am.put("language", a.language());
            am.put("title", a.title());
            am.put("default", a.isDefault());
            audio.add(am);
        }
        m.put("audio", audio);
        List<Object> subs = new ArrayList<>();
        for (int i = 0; i < subtitles.size(); i++) {
            Subtitle s = subtitles.get(i);
            Map<String, Object> sm = new LinkedHashMap<>();
            sm.put("id", "x" + i);
            sm.put("language", s.language());
            sm.put("title", "");
            sm.put("forced", s.forced());
            sm.put("default", false);
            sm.put("codec", s.format());
            sm.put("textBased", true);
            sm.put("external", true);
            subs.add(sm);
        }
        for (SubtitleStream s : subtitleStreams) {
            Map<String, Object> sm = new LinkedHashMap<>();
            sm.put("id", "e" + s.streamIndex());
            sm.put("language", s.language());
            sm.put("title", s.title());
            sm.put("forced", s.forced());
            sm.put("default", s.isDefault());
            sm.put("codec", s.codec());
            sm.put("textBased", s.textBased());
            sm.put("external", false);
            subs.add(sm);
        }
        m.put("subtitles", subs);
        return m;
    }

    public SubtitleStream subtitleStream(int streamIndex) {
        for (SubtitleStream s : subtitleStreams) if (s.streamIndex() == streamIndex) return s;
        return null;
    }

    public String fileName() {
        int i = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
        return i >= 0 ? path.substring(i + 1) : path;
    }

    /** Short label like "4K", "1080p" or "SD", based on the frame size. */
    public String resolutionLabel() {
        if (width <= 0 && height <= 0) return "";
        if (width >= 3200 || height >= 2000) return "4K";
        if (width >= 1800 || height >= 1000) return "1080p";
        if (width >= 1200 || height >= 700) return "720p";
        if (height >= 560) return "576p";
        return "SD";
    }

    public static MediaFile fromStorage(Map<String, Object> m) {
        MediaFile f = new MediaFile();
        f.path = Json.str(m, "path", "");
        f.size = Json.num(m, "size", 0);
        f.modified = Json.num(m, "modified", 0);
        f.container = Json.str(m, "container", "");
        f.formatName = Json.str(m, "formatName", "");
        f.durationMs = Json.num(m, "durationMs", 0);
        f.videoCodec = Json.str(m, "videoCodec", "");
        f.videoProfile = Json.str(m, "videoProfile", "");
        f.pixelFormat = Json.str(m, "pixelFormat", "");
        f.videoStreamIndex = (int) Json.num(m, "videoStreamIndex", -1);
        f.audioCodec = Json.str(m, "audioCodec", "");
        f.width = (int) Json.num(m, "width", 0);
        f.height = (int) Json.num(m, "height", 0);
        f.bitrate = Json.num(m, "bitrate", 0);
        f.audioChannels = (int) Json.num(m, "audioChannels", 0);
        f.probed = Json.bool(m, "probed", false);
        f.probeVersion = (int) Json.num(m, "probeVersion", 1);
        for (Object o : Json.list(m.get("subtitles"))) f.subtitles.add(Subtitle.fromStorage(Json.obj(o)));
        for (Object o : Json.list(m.get("audioStreams"))) f.audioStreams.add(AudioStream.fromStorage(Json.obj(o)));
        for (Object o : Json.list(m.get("subtitleStreams"))) f.subtitleStreams.add(SubtitleStream.fromStorage(Json.obj(o)));
        return f;
    }
}

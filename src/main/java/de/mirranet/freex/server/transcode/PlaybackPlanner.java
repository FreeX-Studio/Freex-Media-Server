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
import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.MediaFile;

import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * Decides how a file is played, given what the browser reported it can decode.
 * <ul>
 *   <li>{@link Mode#DIRECT} – the file is sent as it is</li>
 *   <li>{@link Mode#REMUX} – video and audio are copied into HLS (only the container changes)</li>
 *   <li>{@link Mode#AUDIO} – video is copied, audio is converted to AAC</li>
 *   <li>{@link Mode#TRANSCODE} – video is converted to H.264 (and audio to AAC)</li>
 * </ul>
 */
public final class PlaybackPlanner {

    public enum Mode { DIRECT, REMUX, AUDIO, TRANSCODE }

    /** A quality step in the player menu: bitrate limit and maximum frame height. */
    public record Quality(String id, String label, long videoBitrate, int maxHeight) {
    }

    public static final List<Quality> QUALITIES = List.of(
            new Quality("original", "Original", 0, 0),
            new Quality("20000", "20 Mbit/s 1080p", 20_000_000, 1080),
            new Quality("12000", "12 Mbit/s 1080p", 12_000_000, 1080),
            new Quality("8000", "8 Mbit/s 1080p", 8_000_000, 1080),
            new Quality("4000", "4 Mbit/s 720p", 4_000_000, 720),
            new Quality("2000", "2 Mbit/s 720p", 2_000_000, 720),
            new Quality("1000", "1 Mbit/s 480p", 1_000_000, 480));

    /** Decoder support reported by the browser. */
    public record Capabilities(Set<String> containers, Set<String> video, Set<String> audio, Set<String> mseVideo,
                               Set<String> mseAudio, boolean directPlayAllowed) {

        public static Capabilities parse(Map<String, Object> m) {
            return new Capabilities(set(m.get("containers")), set(m.get("video")), set(m.get("audio")),
                    set(m.get("mseVideo")), set(m.get("mseAudio")), Json.bool(m, "directPlay", true));
        }

        private static Set<String> set(Object o) {
            Set<String> s = new HashSet<>();
            for (Object v : Json.list(o)) s.add(String.valueOf(v).toLowerCase(Locale.ROOT));
            return s;
        }
    }

    /**
     * The decision.
     *
     * @param audioIndex       position in {@link MediaFile#audioStreams} to play, -1 if the file has no audio
     * @param burnSubtitle     stream index of a bitmap subtitle to burn in, or -1
     * @param videoBitrate     target bitrate when transcoding
     * @param maxHeight        target height when transcoding (0 = keep)
     * @param reason           human readable explanation, shown in the player info
     */
    public record Plan(Mode mode, int audioIndex, int burnSubtitle, long videoBitrate, int maxHeight, Text reason) {
    }

    /** Highest frame height the software encoder produces when the user picked "Original". */
    private static final int SOFTWARE_MAX_HEIGHT = 1080;

    private PlaybackPlanner() {
    }

    public static Quality quality(String id) {
        for (Quality q : QUALITIES) if (q.id().equals(id)) return q;
        return QUALITIES.get(0);
    }

    /** Maps ffprobe container names to the names the browser check uses. */
    static String containerKind(MediaFile f) {
        String fmt = f.formatName == null ? "" : f.formatName;
        String ext = f.container == null ? "" : f.container;
        if (fmt.contains("mp4") || fmt.contains("mov") || ext.equals("mp4") || ext.equals("m4v") || ext.equals("mov")) {
            return "mp4";
        }
        if (ext.equals("webm")) return "webm";
        if (fmt.contains("matroska") || ext.equals("mkv")) return "mkv";
        return ext;
    }

    /** Maps ffprobe codec names to the names the browser check uses. */
    static String codecKind(String codec) {
        String c = codec == null ? "" : codec.toLowerCase(Locale.ROOT);
        return switch (c) {
            case "h265" -> "hevc";
            case "avc" -> "h264";
            case "eac3", "e-ac-3" -> "eac3";
            case "ac-3" -> "ac3";
            default -> c;
        };
    }

    public static Plan plan(MediaFile f, Capabilities caps, Quality quality, int requestedAudio,
                            String subtitleId, boolean transcoderAvailable) {
        int audioIndex = chooseAudio(f, requestedAudio);
        MediaFile.AudioStream audio = audioIndex >= 0 ? f.audioStreams.get(audioIndex) : null;

        int burn = -1;
        if (subtitleId != null && subtitleId.startsWith("e")) {
            try {
                MediaFile.SubtitleStream s = f.subtitleStream(Integer.parseInt(subtitleId.substring(1)));
                if (s != null && !s.textBased()) burn = s.streamIndex();
            } catch (NumberFormatException ignored) {
                // unknown id, ignore
            }
        }

        String video = codecKind(f.videoCodec);
        boolean bitrateOk = quality.videoBitrate() == 0 || (f.bitrate > 0 && f.bitrate <= quality.videoBitrate() * 1.1)
                && (quality.maxHeight() == 0 || f.height <= quality.maxHeight());
        boolean videoDecodable = caps.video().contains(video) && !(video.equals("h264") && f.highBitDepthOrChroma());
        boolean audioDecodable = audio == null || caps.audio().contains(codecKind(audio.codec()));
        boolean firstAudio = audioIndex <= 0;
        boolean containerOk = caps.containers().contains(containerKind(f));

        if (!transcoderAvailable) {
            return new Plan(Mode.DIRECT, audioIndex, -1, 0, 0, new Text("playback.transcoderOff"));
        }
        if (burn < 0 && bitrateOk && videoDecodable && audioDecodable && firstAudio && containerOk
                && caps.directPlayAllowed()) {
            return new Plan(Mode.DIRECT, audioIndex, -1, 0, 0, new Text("playback.direct"));
        }

        // Video can be copied into HLS if the browser's MediaSource accepts it and nothing forces a re-encode.
        boolean videoCopyable = burn < 0 && bitrateOk && caps.mseVideo().contains(video)
                && !(video.equals("h264") && f.highBitDepthOrChroma());
        if (videoCopyable) {
            boolean audioCopyable = audio == null || (caps.mseAudio().contains(codecKind(audio.codec()))
                    && Set.of("aac", "mp3", "ac3", "eac3", "flac", "opus").contains(codecKind(audio.codec())));
            if (audioCopyable) {
                return new Plan(Mode.REMUX, audioIndex, -1, 0, 0, new Text("playback.remux", containerKind(f)));
            }
            return new Plan(Mode.AUDIO, audioIndex, -1, 0, 0,
                    new Text("playback.audio", audio.codec().toUpperCase(Locale.ROOT)));
        }

        long sourceBitrate = f.bitrate > 0 ? f.bitrate : 8_000_000;
        long target = quality.videoBitrate() == 0 ? Math.min(sourceBitrate, 20_000_000) : quality.videoBitrate();
        target = Math.max(target, 500_000);
        int height = quality.maxHeight() == 0 ? SOFTWARE_MAX_HEIGHT : quality.maxHeight();
        Text why;
        if (burn >= 0) why = new Text("playback.burnSubtitles");
        else if (!videoDecodable) why = new Text("playback.video", f.videoCodec.toUpperCase(Locale.ROOT)
                + (f.highBitDepthOrChroma() ? " " + f.pixelFormat : ""));
        else why = new Text("playback.quality");
        return new Plan(Mode.TRANSCODE, audioIndex, burn, target, height, why);
    }

    /** The requested track if valid, otherwise the one flagged as default, otherwise the first. */
    static int chooseAudio(MediaFile f, int requested) {
        if (f.audioStreams.isEmpty()) return -1;
        if (requested >= 0 && requested < f.audioStreams.size()) return requested;
        for (int i = 0; i < f.audioStreams.size(); i++) if (f.audioStreams.get(i).isDefault()) return i;
        return 0;
    }
}

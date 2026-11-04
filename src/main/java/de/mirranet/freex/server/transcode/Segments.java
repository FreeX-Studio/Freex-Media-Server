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

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * The fixed segment layout of a conversion over the whole title, known before anything is
 * converted. The player gets a complete playlist from the start, so seeking is plain seeking in
 * the player; the worker converts whatever segment is asked for.
 * <p>
 * Segments start at video keyframes: every {@code length} ms when the video is converted (the
 * encoder is told to put keyframes there), at the source's keyframes when it is copied.
 */
final class Segments {

    private final long[] starts;
    private final long durationMs;

    private Segments(long[] starts, long durationMs) {
        this.starts = starts;
        this.durationMs = durationMs;
    }

    /** Segments of exactly {@code lengthMs}, for converted video. */
    static Segments uniform(long durationMs, long lengthMs) {
        int n = (int) Math.max(1, (durationMs + lengthMs - 1) / lengthMs);
        long[] s = new long[n];
        for (int i = 0; i < n; i++) s[i] = i * lengthMs;
        return new Segments(s, durationMs);
    }

    /**
     * Segments for copied video: each starts at a keyframe and lasts at least {@code minMs}, so a
     * source with short keyframe intervals does not produce tiny segments.
     */
    static Segments fromKeyframes(List<Long> keyframes, long durationMs, long minMs) {
        List<Long> s = new ArrayList<>();
        s.add(0L);
        long last = 0;
        for (long k : keyframes) {
            if (k - last >= minMs && durationMs - k > 500) {
                s.add(k);
                last = k;
            }
        }
        return new Segments(s.stream().mapToLong(Long::longValue).toArray(), durationMs);
    }

    int count() {
        return starts.length;
    }

    long start(int i) {
        return starts[i];
    }

    long end(int i) {
        return i + 1 < starts.length ? starts[i + 1] : durationMs;
    }

    long durationMs() {
        return durationMs;
    }

    /** The segment that contains a position. */
    int indexAt(long ms) {
        int lo = 0;
        int hi = starts.length - 1;
        while (lo < hi) {
            int mid = (lo + hi + 1) >>> 1;
            if (starts[mid] <= ms) lo = mid; else hi = mid - 1;
        }
        return lo;
    }

    /** HLS playlist (VOD) of all segments; the files are named {@code s<index>.m4s}. */
    String playlist() {
        StringBuilder sb = new StringBuilder();
        long longest = 0;
        for (int i = 0; i < starts.length; i++) longest = Math.max(longest, end(i) - start(i));
        sb.append("#EXTM3U\n#EXT-X-VERSION:7\n");
        sb.append("#EXT-X-TARGETDURATION:").append((longest + 999) / 1000).append('\n');
        sb.append("#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n");
        sb.append("#EXT-X-MAP:URI=\"init.mp4\"\n");
        for (int i = 0; i < starts.length; i++) {
            sb.append(String.format(Locale.ROOT, "#EXTINF:%.6f,\n", (end(i) - start(i)) / 1000.0));
            sb.append('s').append(i).append(".m4s\n");
        }
        sb.append("#EXT-X-ENDLIST\n");
        return sb.toString();
    }
}

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

package de.mirranet.freex.server.markers;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Locale;
import java.util.concurrent.TimeUnit;

/**
 * A compact audio fingerprint in the style of Haitsma/Kalker: the audio is cut into frames of
 * {@value #FRAME_SECONDS} s, each frame becomes a 32-bit value describing how the energy of 33
 * frequency bands changes against its neighbours and the previous frame. The same music in two
 * episodes gives (almost) the same bit patterns, regardless of volume.
 */
public final class AudioFingerprint {

    static final int SAMPLE_RATE = 8000;
    static final int FFT_SIZE = 2048;
    static final int HOP = 512;
    public static final double FRAME_SECONDS = (double) HOP / SAMPLE_RATE;
    private static final int BANDS = 33;
    private static final double MIN_HZ = 300;
    private static final double MAX_HZ = 3000;
    /** Frames quieter than this (mean square of 16-bit samples) carry no information. */
    private static final double SILENCE = 2_000;

    /** Fingerprint values per frame. */
    public final int[] hashes;
    /** False for silent frames, which must never count as a match. */
    public final boolean[] valid;
    /** Position of frame 0 in the file, in seconds. */
    public final double startSeconds;

    AudioFingerprint(int[] hashes, boolean[] valid, double startSeconds) {
        this.hashes = hashes;
        this.valid = valid;
        this.startSeconds = startSeconds;
    }

    public int frames() {
        return hashes.length;
    }

    /** Decodes {@code seconds} of the first audio track from {@code start} with ffmpeg and fingerprints it. */
    public static AudioFingerprint of(String ffmpeg, String file, double start, double seconds)
            throws IOException, InterruptedException {
        Process p = new ProcessBuilder(ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin",
                "-ss", String.format(Locale.ROOT, "%.3f", start), "-t", String.format(Locale.ROOT, "%.3f", seconds),
                "-i", file, "-map", "0:a:0", "-ac", "1", "-ar", Integer.toString(SAMPLE_RATE), "-f", "s16le", "-")
                .redirectError(ProcessBuilder.Redirect.DISCARD).start();
        byte[] pcm;
        try (InputStream in = p.getInputStream()) {
            pcm = readAll(in, (int) (seconds * SAMPLE_RATE * 2) + 65536);
        } finally {
            if (!p.waitFor(120, TimeUnit.SECONDS)) p.destroyForcibly();
        }
        short[] samples = new short[pcm.length / 2];
        ByteBuffer.wrap(pcm).order(ByteOrder.LITTLE_ENDIAN).asShortBuffer().get(samples);
        return compute(samples, start);
    }

    private static byte[] readAll(InputStream in, int max) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream(Math.min(max, 1 << 22));
        byte[] buf = new byte[65536];
        int n;
        while ((n = in.read(buf)) > 0) {
            if (out.size() + n > max) {
                out.write(buf, 0, max - out.size());
                break;
            }
            out.write(buf, 0, n);
        }
        return out.toByteArray();
    }

    static AudioFingerprint compute(short[] samples, double startSeconds) {
        int frames = samples.length < FFT_SIZE ? 0 : (samples.length - FFT_SIZE) / HOP + 1;
        int[] hashes = new int[frames];
        boolean[] valid = new boolean[frames];
        int[] bandEdges = bandEdges();
        double[] window = new double[FFT_SIZE];
        for (int i = 0; i < FFT_SIZE; i++) window[i] = 0.5 - 0.5 * Math.cos(2 * Math.PI * i / (FFT_SIZE - 1));

        double[] re = new double[FFT_SIZE];
        double[] im = new double[FFT_SIZE];
        double[] prev = new double[BANDS];
        double[] cur = new double[BANDS];
        for (int f = 0; f < frames; f++) {
            int off = f * HOP;
            double power = 0;
            for (int i = 0; i < FFT_SIZE; i++) {
                double s = samples[off + i];
                power += s * s;
                re[i] = s * window[i];
                im[i] = 0;
            }
            power /= FFT_SIZE;
            fft(re, im);
            for (int b = 0; b < BANDS; b++) {
                double e = 0;
                for (int k = bandEdges[b]; k < bandEdges[b + 1]; k++) e += re[k] * re[k] + im[k] * im[k];
                cur[b] = e;
            }
            int h = 0;
            if (f > 0) {
                for (int b = 0; b < BANDS - 1; b++) {
                    double d = (cur[b] - cur[b + 1]) - (prev[b] - prev[b + 1]);
                    if (d > 0) h |= 1 << b;
                }
            }
            hashes[f] = h;
            valid[f] = f > 0 && power > SILENCE;
            double[] t = prev;
            prev = cur;
            cur = t;
        }
        return new AudioFingerprint(hashes, valid, startSeconds);
    }

    private static int[] bandEdges() {
        int[] edges = new int[BANDS + 1];
        double binHz = (double) SAMPLE_RATE / FFT_SIZE;
        for (int b = 0; b <= BANDS; b++) {
            double hz = MIN_HZ * Math.pow(MAX_HZ / MIN_HZ, (double) b / BANDS);
            edges[b] = (int) Math.round(hz / binHz);
        }
        for (int b = 1; b <= BANDS; b++) if (edges[b] <= edges[b - 1]) edges[b] = edges[b - 1] + 1;
        return edges;
    }

    /** In-place iterative radix-2 FFT. */
    private static void fft(double[] re, double[] im) {
        int n = re.length;
        for (int i = 1, j = 0; i < n; i++) {
            int bit = n >> 1;
            for (; (j & bit) != 0; bit >>= 1) j ^= bit;
            j ^= bit;
            if (i < j) {
                double t = re[i];
                re[i] = re[j];
                re[j] = t;
                t = im[i];
                im[i] = im[j];
                im[j] = t;
            }
        }
        for (int len = 2; len <= n; len <<= 1) {
            double ang = -2 * Math.PI / len;
            double wr = Math.cos(ang);
            double wi = Math.sin(ang);
            for (int i = 0; i < n; i += len) {
                double cr = 1;
                double ci = 0;
                for (int k = 0; k < len / 2; k++) {
                    int a = i + k;
                    int b = a + len / 2;
                    double xr = re[b] * cr - im[b] * ci;
                    double xi = re[b] * ci + im[b] * cr;
                    re[b] = re[a] - xr;
                    im[b] = im[a] - xi;
                    re[a] += xr;
                    im[a] += xi;
                    double t = cr * wr - ci * wi;
                    ci = cr * wi + ci * wr;
                    cr = t;
                }
            }
        }
    }

    /** A segment present in both fingerprints, in frames of each. */
    public record Match(int startA, int endA, int startB, int endB) {
        public int length() {
            return endA - startA;
        }
    }

    /**
     * Finds the longest stretch that sounds the same in {@code a} and {@code b}, at any time
     * shift. Frames count as equal with at most {@code maxBitErrors} differing bits; short
     * disturbances of up to {@code maxGap} frames inside a stretch are tolerated.
     */
    public static Match longestCommon(AudioFingerprint a, AudioFingerprint b, int minFrames, int maxBitErrors, int maxGap) {
        Match best = null;
        int na = a.frames();
        int nb = b.frames();
        for (int shift = -(nb - minFrames); shift <= na - minFrames; shift++) {
            int iStart = Math.max(0, shift);
            int iEnd = Math.min(na, nb + shift);
            int runStart = -1;
            int lastMatch = -1;
            int matches = 0;
            for (int i = iStart; i < iEnd; i++) {
                int j = i - shift;
                boolean eq = a.valid[i] && b.valid[j] && Integer.bitCount(a.hashes[i] ^ b.hashes[j]) <= maxBitErrors;
                if (eq) {
                    if (runStart < 0) {
                        runStart = i;
                        matches = 0;
                    }
                    lastMatch = i;
                    matches++;
                } else if (runStart >= 0 && i - lastMatch > maxGap) {
                    best = better(best, runStart, lastMatch + 1, shift, matches);
                    runStart = -1;
                }
            }
            if (runStart >= 0) best = better(best, runStart, lastMatch + 1, shift, matches);
        }
        return best != null && best.length() >= minFrames ? best : null;
    }

    private static Match better(Match best, int start, int end, int shift, int matches) {
        // Require that most frames of the stretch really match, not just a few scattered ones.
        if (matches < (end - start) * 0.6) return best;
        if (best == null || end - start > best.length()) return new Match(start, end, start - shift, end - shift);
        return best;
    }
}

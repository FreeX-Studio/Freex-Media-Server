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

import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.Library;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.util.JsonFiles;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Detects intros and end credits of TV episodes, offered as "Skip intro" / "Skip credits".
 * <p>
 * For each season, the beginning and the end of every episode are fingerprinted
 * ({@link AudioFingerprint}). The intro is the longest stretch (15 s to 2.5 min) in the
 * beginning that also occurs in another episode of the season; the credits are such a stretch
 * at the end. Results are stored in {@code markers.json} per episode and file version, so a
 * season is only analysed again when files change.
 */
public final class MarkerService implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(MarkerService.class.getName());

    /** How much of the beginning is searched for the intro. */
    private static final double INTRO_WINDOW = 600;
    /** How much of the end is searched for credits. */
    private static final double CREDITS_WINDOW = 360;
    private static final double MIN_SEGMENT = 15;
    private static final double MAX_INTRO = 150;
    private static final int MAX_BIT_ERRORS = 10;
    private static final int MAX_GAP = 24;

    public record Segment(long startMs, long endMs) {
        Map<String, Object> toJson() {
            return Map.of("startMs", startMs, "endMs", endMs);
        }

        static Segment fromJson(Object o) {
            if (!(o instanceof Map<?, ?>)) return null;
            Map<String, Object> m = Json.obj(o);
            return new Segment(Json.num(m, "startMs", 0), Json.num(m, "endMs", 0));
        }
    }

    /** Markers of one episode; {@code fileModified} ties them to the analysed file. */
    public record Markers(long fileModified, Segment intro, Segment credits, long analyzedAt) {
    }

    private final DataStore store;
    private final String ffmpeg;
    private final Path file;
    private final Map<String, Markers> markers = new ConcurrentHashMap<>();
    private final ExecutorService worker = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "marker-detection");
        t.setDaemon(true);
        t.setPriority(Thread.MIN_PRIORITY);
        return t;
    });
    private final Set<String> queued = ConcurrentHashMap.newKeySet();

    public MarkerService(DataStore store, String ffmpeg, Path dataDir) throws IOException {
        this.store = store;
        this.ffmpeg = ffmpeg;
        this.file = dataDir.resolve("markers.json");
        for (Map.Entry<String, Object> e : Json.obj(JsonFiles.read(file, "{}")).entrySet()) {
            Map<String, Object> m = Json.obj(e.getValue());
            markers.put(e.getKey(), new Markers(Json.num(m, "fileModified", 0), Segment.fromJson(m.get("intro")),
                    Segment.fromJson(m.get("credits")), Json.num(m, "analyzedAt", 0)));
        }
    }

    public Markers get(MediaItem episode) {
        Markers m = markers.get(episode.id);
        return m != null && episode.file != null && m.fileModified == episode.file.modified ? m : null;
    }

    public boolean busy() {
        return !queued.isEmpty();
    }

    /** Queues analysis of all seasons of a show library that have new or changed episodes. */
    public void analyzeLibrary(Library library, boolean force) {
        if (library.type != Library.Type.SHOWS || !queued.add(library.id)) return;
        worker.submit(() -> {
            try {
                int seasons = 0;
                for (MediaItem show : store.topLevel(library.id)) {
                    for (MediaItem season : store.children(show.id)) {
                        if (analyzeSeason(season, force)) seasons++;
                    }
                }
                if (seasons > 0) {
                    save();
                    LOG.info("Intro/credits detection: " + seasons + " seasons analysed in '" + library.name + "'");
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            } catch (Exception e) {
                LOG.log(Level.WARNING, "Intro/credits detection failed for '" + library.name + "'", e);
            } finally {
                queued.remove(library.id);
            }
        });
    }

    private boolean analyzeSeason(MediaItem season, boolean force) throws IOException, InterruptedException {
        List<MediaItem> eps = new ArrayList<>();
        for (MediaItem ep : store.children(season.id)) {
            if (ep.file != null && ep.file.durationMs > 120_000 && !ep.file.audioStreams.isEmpty()) eps.add(ep);
        }
        if (eps.size() < 2) return false;
        boolean needed = force || eps.stream().anyMatch(ep -> get(ep) == null);
        if (!needed) return false;

        List<AudioFingerprint> heads = new ArrayList<>();
        List<AudioFingerprint> tails = new ArrayList<>();
        for (MediaItem ep : eps) {
            double dur = ep.file.durationMs / 1000.0;
            double head = Math.min(INTRO_WINDOW, dur * 0.4);
            double tail = Math.min(CREDITS_WINDOW, dur * 0.3);
            heads.add(AudioFingerprint.of(ffmpeg, ep.file.path, 0, head));
            tails.add(AudioFingerprint.of(ffmpeg, ep.file.path, dur - tail, tail));
        }

        int minFrames = (int) Math.ceil(MIN_SEGMENT / AudioFingerprint.FRAME_SECONDS);
        int maxIntro = (int) Math.ceil(MAX_INTRO / AudioFingerprint.FRAME_SECONDS);
        for (int i = 0; i < eps.size(); i++) {
            MediaItem ep = eps.get(i);
            Segment intro = best(heads, i, minFrames, maxIntro, false, ep.file.durationMs);
            Segment credits = best(tails, i, minFrames, Integer.MAX_VALUE, true, ep.file.durationMs);
            markers.put(ep.id, new Markers(ep.file.modified, intro, credits, System.currentTimeMillis()));
        }
        return true;
    }

    /**
     * Compares fingerprint {@code i} with its neighbours in the season and returns the longest
     * common stretch as a segment of episode {@code i}.
     */
    private static Segment best(List<AudioFingerprint> fps, int i, int minFrames, int maxFrames, boolean credits, long durationMs) {
        AudioFingerprint a = fps.get(i);
        // Compare with the neighbours. With two or more matches only the part common to all of
        // them is used: a stretch that happens to sound alike in just one pair (similar music
        // under a scene) must not be skipped.
        int start = -1;
        int end = -1;
        int found = 0;
        AudioFingerprint.Match longest = null;
        for (int j : new int[]{i + 1, i - 1, i + 2, i - 2}) {
            if (j < 0 || j >= fps.size() || found >= 2) continue;
            AudioFingerprint.Match m = AudioFingerprint.longestCommon(a, fps.get(j), minFrames, MAX_BIT_ERRORS, MAX_GAP);
            if (m == null || m.length() > maxFrames) continue;
            found++;
            if (longest == null || m.length() > longest.length()) longest = m;
            start = start < 0 ? m.startA() : Math.max(start, m.startA());
            end = end < 0 ? m.endA() : Math.min(end, m.endA());
        }
        if (found == 0) return null;
        if (found >= 2 && end - start < minFrames) {
            // The matches disagree; fall back to the longest one.
            start = longest.startA();
            end = longest.endA();
        }
        long startMs = Math.round((a.startSeconds + start * AudioFingerprint.FRAME_SECONDS) * 1000);
        long endMs = Math.round((a.startSeconds + end * AudioFingerprint.FRAME_SECONDS) * 1000);
        if (credits && durationMs - endMs < 20_000) {
            // Credits run until the end of the file, even if the music stops a bit earlier.
            endMs = durationMs;
        }
        return new Segment(startMs, Math.min(endMs, durationMs));
    }

    private void save() {
        Map<String, Object> out = new LinkedHashMap<>();
        for (Map.Entry<String, Markers> e : markers.entrySet()) {
            Markers m = e.getValue();
            Map<String, Object> mm = new LinkedHashMap<>();
            mm.put("fileModified", m.fileModified());
            mm.put("intro", m.intro() == null ? null : m.intro().toJson());
            mm.put("credits", m.credits() == null ? null : m.credits().toJson());
            mm.put("analyzedAt", m.analyzedAt());
            out.put(e.getKey(), mm);
        }
        try {
            JsonFiles.write(file, out);
        } catch (IOException e) {
            LOG.log(Level.WARNING, "Saving markers failed", e);
        }
    }

    @Override
    public void close() {
        worker.shutdownNow();
        // Wait for running work, so nothing writes to the store after it is closed (matters for restarts).
        try {
            worker.awaitTermination(10, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }
}

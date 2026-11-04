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

package de.mirranet.freex.server.http;

import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.MediaFile;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.transcode.PlaybackPlanner;
import de.mirranet.freex.server.transcode.TranscodeManager;
import de.mirranet.freex.server.util.Ids;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static de.mirranet.freex.server.http.WebServer.Access.PUBLIC;
import static de.mirranet.freex.server.http.WebServer.Access.USER;

/**
 * Playback on a Chromecast. The browser is the remote control (Google Cast sender), the
 * Chromecast runs Google's Default Media Receiver and loads the media from this server itself.
 * <p>
 * The Chromecast has no login, so every cast gets a <em>grant</em>: a random 256-bit token in
 * the URL that unlocks exactly one item (its stream or conversion, subtitles and artwork) for a
 * limited time. URLs under {@code /api/cast/} are the only ones served to other origins (CORS),
 * because the receiver runs on Google's origin.
 * <p>
 * The Default Media Receiver plays H.264 with AAC/MP3 in MP4 directly; anything else is
 * converted to HLS like for a browser that cannot play the file.
 */
public final class CastApi {

    public static final String PREFIX = "/api/cast/";

    /** What the receiver can decode (first-generation Chromecasts included). */
    private static final PlaybackPlanner.Capabilities CHROMECAST = new PlaybackPlanner.Capabilities(
            Set.of("mp4"), Set.of("h264"), Set.of("aac", "mp3"), Set.of("h264"), Set.of("aac", "mp3"), true);

    private static final long EXTRA_VALIDITY_MS = TimeUnit.HOURS.toMillis(6);
    private static final Pattern VTT_TIME = Pattern.compile("(?:(\\d+):)?(\\d{2}):(\\d{2})\\.(\\d{3})");

    /** Access of one Chromecast to one item. */
    private record Grant(String userId, String itemId, String session, long offsetMs, long expires) {
    }

    private final Map<String, Grant> grants = new ConcurrentHashMap<>();
    private final DataStore store;
    private final MediaApi media;

    public CastApi(DataStore store, MediaApi media) {
        this.store = store;
        this.media = media;
    }

    public void register(WebServer web) {
        web.route("POST", "/api/items/{id}/cast", USER, this::start);
        web.route("GET", PREFIX + "{token}/stream", PUBLIC, this::stream);
        web.route("GET", PREFIX + "{token}/hls/{file}", PUBLIC, this::hls);
        web.route("GET", PREFIX + "{token}/subtitles/{sid}", PUBLIC, this::subtitle);
        web.route("GET", PREFIX + "{token}/image/{kind}", PUBLIC, this::image);
    }

    /**
     * Plans a cast. Body as for {@code /playback} (quality, audio, subtitle, startMs, client,
     * device), without capabilities. Answers with URLs relative to this server.
     */
    private void start(Exchange ex) throws Exception {
        MediaItem it = store.item(ex.user(), ex.param("id")).orElseThrow(ApiException::notFound);
        if (!it.type.playable() || it.file == null) throw ApiException.badRequest("error.notPlayable");
        Map<String, Object> body = ex.body();
        PlaybackPlanner.Quality quality = PlaybackPlanner.quality(Json.str(body, "quality", "original"));
        int audio = (int) Json.num(body, "audio", -1);
        String subtitle = Json.str(body, "subtitle", null);
        long start = Math.max(0, Json.num(body, "startMs", 0));
        if (it.file.durationMs > 0 && start >= it.file.durationMs) start = 0;
        TranscodeManager transcoder = media.transcoder();
        PlaybackPlanner.Plan plan = PlaybackPlanner.plan(it.file, CHROMECAST, quality, audio, subtitle, transcoder != null);

        expire();
        String token = Ids.token();
        String session = null;
        long offset = 0;
        Map<String, Object> m = new LinkedHashMap<>();
        if (plan.mode() == PlaybackPlanner.Mode.DIRECT) {
            m.put("url", PREFIX + token + "/stream");
            m.put("contentType", "video/mp4");
            m.put("hls", false);
        } else {
            try {
                TranscodeManager.Session s = transcoder.start(ex.user().id, it, plan, start);
                session = s.id;
                offset = start;
            } catch (TranscodeManager.BusyException e) {
                throw ApiException.of(503, e);
            }
            m.put("url", PREFIX + token + "/hls/index.m3u8");
            m.put("contentType", "application/x-mpegurl");
            m.put("hls", true);
        }
        long validity = (it.file.durationMs > 0 ? it.file.durationMs : TimeUnit.HOURS.toMillis(4)) + EXTRA_VALIDITY_MS;
        grants.put(token, new Grant(ex.user().id, it.id, session, offset, System.currentTimeMillis() + validity));

        m.put("mode", plan.mode().name().toLowerCase(Locale.ROOT));
        m.put("reason", plan.reason());
        m.put("audio", plan.audioIndex());
        m.put("offsetMs", offset);
        m.put("startMs", start);
        m.put("durationMs", it.file.durationMs);
        m.put("image", PREFIX + token + "/image/" + (it.type == MediaItem.Type.EPISODE ? "thumb" : "poster"));
        String burned = plan.burnSubtitle() >= 0 ? "e" + plan.burnSubtitle() : null;
        m.put("burnedSubtitle", burned);
        List<Object> subs = new ArrayList<>();
        for (String id : textSubtitleIds(it.file)) {
            if (id.equals(burned)) continue;
            Map<String, Object> s = new LinkedHashMap<>();
            s.put("id", id);
            s.put("url", PREFIX + token + "/subtitles/" + id + ".vtt");
            subs.add(s);
        }
        m.put("subtitles", subs);

        ActivityApi activity = media.activity();
        if (activity != null) {
            String device = Json.str(body, "device", "Chromecast");
            activity.started(ex, it, ActivityApi.client(Json.str(body, "client", null)), device,
                    plan.mode().name().toLowerCase(Locale.ROOT), plan.reason(), quality.label(), session, start);
        }
        ex.ok(m);
    }

    /** Ids of the subtitles a receiver can show as text track (external files and text streams). */
    private static List<String> textSubtitleIds(MediaFile f) {
        List<String> ids = new ArrayList<>();
        for (int i = 0; i < f.subtitles.size(); i++) ids.add("x" + i);
        for (MediaFile.SubtitleStream s : f.subtitleStreams) {
            if (s.textBased()) ids.add("e" + s.streamIndex());
        }
        return ids;
    }

    private void expire() {
        long now = System.currentTimeMillis();
        grants.values().removeIf(g -> g.expires < now);
    }

    private Grant grant(Exchange ex) {
        Grant g = grants.get(ex.param("token"));
        if (g == null || g.expires < System.currentTimeMillis() || store.user(g.userId).isEmpty()) {
            throw ApiException.notFound();
        }
        return g;
    }

    private MediaItem item(Grant g) {
        MediaItem it = store.item(g.itemId).orElseThrow(ApiException::notFound);
        if (it.file == null) throw ApiException.notFound();
        return it;
    }

    private void stream(Exchange ex) throws IOException {
        MediaItem it = item(grant(ex));
        Path p = Path.of(it.file.path);
        if (!Files.isRegularFile(p)) throw ApiException.notFound();
        FileSender.send(ex, p, 0);
    }

    private void hls(Exchange ex) throws Exception {
        Grant g = grant(ex);
        TranscodeManager transcoder = media.transcoder();
        TranscodeManager.Session s = g.session == null || transcoder == null ? null : transcoder.session(g.session);
        if (s == null) throw ApiException.notFound();
        media.sendTranscodeFile(ex, s, ex.param("file"));
    }

    private void subtitle(Exchange ex) throws Exception {
        Grant g = grant(ex);
        String text = media.subtitleText(item(g), ex.param("sid").replace(".vtt", ""));
        if (text == null) throw ApiException.notFound();
        // A conversion starts at the offset, so the cues have to move with it.
        if (g.offsetMs > 0) text = shift(text, -g.offsetMs);
        ex.send(200, "text/vtt; charset=utf-8", text.getBytes(StandardCharsets.UTF_8));
    }

    private void image(Exchange ex) throws IOException {
        Path p = media.imagePath(item(grant(ex)), ex.param("kind"));
        if (p == null) throw ApiException.notFound();
        FileSender.send(ex, p, 3600);
    }

    /** Moves all cue times of a WebVTT text; cues that end before 0 are dropped. */
    static String shift(String vtt, long deltaMs) {
        StringBuilder out = new StringBuilder();
        String[] blocks = vtt.replace("\r", "").split("\n\n");
        for (String block : blocks) {
            int arrow = block.indexOf("-->");
            if (arrow < 0) {
                out.append(block).append("\n\n");
                continue;
            }
            int lineStart = block.lastIndexOf('\n', arrow) + 1;
            int lineEnd = block.indexOf('\n', arrow);
            String line = block.substring(lineStart, lineEnd < 0 ? block.length() : lineEnd);
            Matcher m = VTT_TIME.matcher(line);
            List<long[]> times = new ArrayList<>();
            while (m.find()) times.add(new long[]{m.start(), m.end(), parse(m) + deltaMs});
            if (times.size() < 2 || times.get(1)[2] <= 0) continue;
            StringBuilder l = new StringBuilder(line);
            for (int i = times.size() - 1; i >= 0; i--) {
                long[] t = times.get(i);
                l.replace((int) t[0], (int) t[1], format(Math.max(0, t[2])));
            }
            out.append(block, 0, lineStart).append(l).append(lineEnd < 0 ? "" : block.substring(lineEnd)).append("\n\n");
        }
        return out.toString();
    }

    private static long parse(Matcher m) {
        long h = m.group(1) == null ? 0 : Long.parseLong(m.group(1));
        return ((h * 60 + Long.parseLong(m.group(2))) * 60 + Long.parseLong(m.group(3))) * 1000 + Long.parseLong(m.group(4));
    }

    private static String format(long ms) {
        return String.format(Locale.ROOT, "%02d:%02d:%02d.%03d", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    }
}

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

import de.mirranet.freex.server.model.Library;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.transcode.TranscodeManager;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;

import static de.mirranet.freex.server.http.WebServer.Access.ADMIN;

/**
 * What the server is doing right now, for the activity menu of administrators: running
 * playbacks (who watches what, on which device, direct or converted, where in the file) and
 * library scans.
 * <p>
 * Players announce a playback when they start it and then report position and state every few
 * seconds, also while paused; a playback that has not reported for {@link #TIMEOUT_MS} is gone
 * (closed tab, lost connection).
 */
public final class ActivityApi {

    private static final long TIMEOUT_MS = TimeUnit.SECONDS.toMillis(45);

    /** One running playback of one client. */
    static final class Playback {
        final String userId;
        final String itemId;
        final String client;
        final long startedAt = System.currentTimeMillis();
        volatile String mode;
        volatile String reason;
        volatile String quality;
        volatile String session;
        volatile String ip;
        volatile String userAgent;
        volatile String device;
        volatile long offsetMs;
        volatile String state = "playing";
        volatile long lastSeen = System.currentTimeMillis();

        Playback(String userId, String itemId, String client) {
            this.userId = userId;
            this.itemId = itemId;
            this.client = client;
        }
    }

    private final Map<String, Playback> playbacks = new ConcurrentHashMap<>();
    private final DataStore store;
    private final TranscodeManager transcoder;
    private MediaApi media;

    public ActivityApi(DataStore store, TranscodeManager transcoder) {
        this.store = store;
        this.transcoder = transcoder;
    }

    public void register(WebServer web, MediaApi media) {
        this.media = media;
        web.route("GET", "/api/admin/activity", ADMIN, ex -> ex.ok(snapshot(ex.user())));
    }

    private static String key(String userId, String itemId, String client) {
        return userId + "|" + itemId + "|" + client;
    }

    /** Client ids are chosen by the browser; anything else than a short token is ignored. */
    static String client(String raw) {
        return raw != null && raw.matches("[A-Za-z0-9_-]{1,64}") ? raw : null;
    }

    /** A player started (or restarted, e.g. after a seek or quality change) a playback. */
    void started(Exchange ex, MediaItem it, String client, String device, String mode, String reason, String quality,
                 String session, long offsetMs) {
        if (client == null) return;
        Playback p = playbacks.computeIfAbsent(key(ex.user().id, it.id, client), k -> new Playback(ex.user().id, it.id, client));
        p.mode = mode;
        p.reason = reason;
        p.quality = quality;
        p.session = session;
        p.ip = ex.remoteAddress();
        p.userAgent = ex.userAgent();
        p.device = device;
        p.offsetMs = offsetMs;
        p.state = "playing";
        p.lastSeen = System.currentTimeMillis();
    }

    /** Position report; {@code stopped} ends the playback. */
    void progress(User u, MediaItem it, String client, long offsetMs, String state, boolean stopped) {
        if (client == null) return;
        String key = key(u.id, it.id, client);
        if (stopped) {
            playbacks.remove(key);
            return;
        }
        Playback p = playbacks.get(key);
        if (p == null) return; // not announced (e.g. the server restarted); the next start announces it
        p.offsetMs = offsetMs;
        if ("paused".equals(state) || "playing".equals(state) || "buffering".equals(state)) p.state = state;
        p.lastSeen = System.currentTimeMillis();
    }

    /** Number of running playbacks, for the badge in the header. */
    public int count() {
        expire();
        return playbacks.size();
    }

    private void expire() {
        long cutoff = System.currentTimeMillis() - TIMEOUT_MS;
        playbacks.values().removeIf(p -> p.lastSeen < cutoff);
    }

    private Map<String, Object> snapshot(User viewer) {
        expire();
        List<Object> sessions = new ArrayList<>();
        List<Playback> list = new ArrayList<>(playbacks.values());
        list.sort(Comparator.comparingLong(p -> p.startedAt));
        for (Playback p : list) {
            MediaItem it = store.item(p.itemId).orElse(null);
            if (it == null) continue;
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("client", p.client);
            store.user(p.userId).ifPresent(u -> {
                Map<String, Object> um = new LinkedHashMap<>();
                um.put("id", u.id);
                um.put("username", u.username);
                um.put("avatar", u.avatarVersion);
                m.put("user", um);
            });
            m.put("item", media.view(it, viewer, false));
            m.put("mode", p.mode);
            m.put("reason", p.reason);
            m.put("quality", p.quality);
            m.put("state", p.state);
            m.put("offsetMs", p.offsetMs);
            m.put("durationMs", it.file == null ? 0 : it.file.durationMs);
            m.put("device", p.device);
            m.put("userAgent", p.userAgent);
            m.put("ip", p.ip);
            m.put("startedAt", p.startedAt);
            m.put("transcoding", p.session != null && transcoder != null && transcoder.session(p.session) != null);
            sessions.add(m);
        }
        List<Object> tasks = new ArrayList<>();
        for (Library l : store.libraries()) {
            if (!l.scanning) continue;
            Map<String, Object> t = new LinkedHashMap<>();
            t.put("type", "scan");
            t.put("title", l.name + " wird gescannt");
            tasks.add(t);
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("sessions", sessions);
        m.put("tasks", tasks);
        m.put("transcodes", transcoder == null ? 0 : transcoder.activeSessions());
        return m;
    }
}

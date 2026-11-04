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

package de.mirranet.freex.server.together;

import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.User;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.Executors;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.function.ToLongFunction;
import java.util.logging.Logger;

/**
 * Watch Together: several users watch the same item in sync. Each member streams on their own;
 * the room only shares the playback state (item, playing/paused, position). Any member can
 * play, pause, seek or switch to the next episode, and everyone follows.
 * <p>
 * Members receive changes as Server-Sent Events. The state's position is extrapolated from the
 * time of the last change, so late joiners land at the right spot.
 * <p>
 * A member whose player is loading (starting, seeking, buffering) reports it, and the room holds
 * its position until everyone is ready again, so nobody falls behind. A report that is not
 * cleared expires after {@link #WAIT_LIMIT_MS}, so one stuck player cannot stop the room.
 */
public final class TogetherService implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(TogetherService.class.getName());
    private static final String CODE_CHARS = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    private static final long EMPTY_ROOM_TTL_MS = TimeUnit.MINUTES.toMillis(10);
    private static final long IDLE_ROOM_TTL_MS = TimeUnit.HOURS.toMillis(8);
    private static final SecureRandom RANDOM = new SecureRandom();
    static final long WAIT_LIMIT_MS = TimeUnit.SECONDS.toMillis(20);

    public static final class Room {
        public final String code;
        public final String hostId;
        final long createdAt = System.currentTimeMillis();
        volatile long lastActivity = createdAt;
        volatile String itemId;
        volatile boolean playing;
        volatile long positionMs;
        volatile long changedAt = System.currentTimeMillis();
        volatile String changedBy = "";
        /** Counts every change, so clients can drop states that are older than one they know. */
        volatile long version;
        /** userId → display name, for everyone who joined. */
        final Map<String, String> members = new ConcurrentHashMap<>();
        /** Members whose player is loading: userId → since when. Guarded by the room's lock. */
        final Map<String, Long> waiting = new LinkedHashMap<>();
        final List<Subscriber> subscribers = new CopyOnWriteArrayList<>();

        /** The title currently played in the room. */
        public String itemId() {
            return itemId;
        }

        Room(String code, String hostId, String itemId) {
            this.code = code;
            this.hostId = hostId;
            this.itemId = itemId;
        }

        /** Whether the position advances: playing, and nobody is loading. */
        synchronized boolean running() {
            return playing && waiting.isEmpty();
        }

        /** Position at {@code now}, counting the time since the last change while running. */
        synchronized long positionAt(long now) {
            return running() ? positionMs + Math.max(0, now - changedAt) : positionMs;
        }

        /** Fixes the position at now; called before anything that starts or stops the clock. */
        private void freeze(long now) {
            positionMs = positionAt(now);
            changedAt = now;
        }

        synchronized Map<String, Object> toJson(ToLongFunction<String> avatars) {
            long now = System.currentTimeMillis();
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("code", code);
            m.put("hostId", hostId);
            m.put("itemId", itemId);
            // playing is what the members chose; running says whether the position moves right now
            m.put("playing", playing);
            m.put("running", running());
            m.put("positionMs", positionAt(now));
            m.put("changedBy", changedBy);
            m.put("version", version);
            m.put("serverTime", now);
            List<Object> loading = new ArrayList<>();
            for (String id : waiting.keySet()) loading.add(Map.of("id", id, "name", members.getOrDefault(id, "")));
            m.put("waiting", loading);
            List<Object> online = new ArrayList<>();
            Set<String> connected = new HashSet<>();
            for (Subscriber s : subscribers) connected.add(s.userId);
            for (Map.Entry<String, String> e : members.entrySet()) {
                online.add(Map.of("id", e.getKey(), "name", e.getValue(), "online", connected.contains(e.getKey()),
                        "avatar", avatars.applyAsLong(e.getKey())));
            }
            m.put("members", online);
            return m;
        }
    }

    /** One open event stream. */
    public static final class Subscriber {
        final String userId;
        final BlockingQueue<String> queue = new LinkedBlockingQueue<>(200);

        Subscriber(String userId) {
            this.userId = userId;
        }

        /** Next event ("event: …\ndata: …\n\n"), or null after the timeout (send a heartbeat then). */
        public String next(long timeoutMs) throws InterruptedException {
            return queue.poll(timeoutMs, TimeUnit.MILLISECONDS);
        }
    }

    private final Map<String, Room> rooms = new ConcurrentHashMap<>();
    private final ScheduledExecutorService cleaner = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "together-cleaner");
        t.setDaemon(true);
        return t;
    });

    /** Profile picture version of a user (0 = none), shown next to the members. */
    private final ToLongFunction<String> avatars;

    public TogetherService(ToLongFunction<String> avatars) {
        this.avatars = avatars;
        cleaner.scheduleWithFixedDelay(this::cleanup, 1, 1, TimeUnit.MINUTES);
        cleaner.scheduleWithFixedDelay(this::expireWaiting, 2, 2, TimeUnit.SECONDS);
    }

    /** The state of a room as sent to its members. */
    public Map<String, Object> state(Room r) {
        return r.toJson(avatars);
    }

    public Room create(User host, String itemId, long positionMs) {
        String code;
        do {
            StringBuilder sb = new StringBuilder();
            for (int i = 0; i < 6; i++) sb.append(CODE_CHARS.charAt(RANDOM.nextInt(CODE_CHARS.length())));
            code = sb.toString();
        } while (rooms.containsKey(code));
        Room r = new Room(code, host.id, itemId);
        r.positionMs = Math.max(0, positionMs);
        r.playing = true; // the host pressed "watch together": start right away
        r.members.put(host.id, host.username);
        rooms.put(code, r);
        LOG.info("Watch Together room " + code + " created by '" + host.username + "'");
        return r;
    }

    public Room room(String code) {
        return code == null ? null : rooms.get(code.trim().toUpperCase());
    }

    public void join(Room r, User u) {
        if (r.members.putIfAbsent(u.id, u.username) == null) broadcast(r, "members");
        r.lastActivity = System.currentTimeMillis();
    }

    /**
     * Applies a change from a member. {@code action} is play, pause, seek, item (switch to another
     * item, e.g. the next episode), or buffering / ready, which a player reports while it loads.
     *
     * @return the room's new version
     */
    public long update(Room r, User u, String action, long positionMs, String itemId) {
        if (!r.members.containsKey(u.id)) throw new IllegalStateException("not a member");
        if (action.equals("buffering") || action.equals("ready")) {
            if (setWaiting(r, u.id, action.equals("buffering"))) broadcast(r, "state");
            return r.version;
        }
        synchronized (r) {
            switch (action) {
                case "play" -> r.playing = true;
                case "pause" -> r.playing = false;
                case "seek" -> {
                    // keep play/pause state
                }
                case "item" -> {
                    if (itemId == null || itemId.isBlank()) throw new IllegalArgumentException("itemId missing");
                    r.itemId = itemId;
                    r.playing = true;
                    r.waiting.clear(); // everyone loads the new title and reports again
                }
                default -> throw new IllegalArgumentException("unknown action " + action);
            }
            r.positionMs = Math.max(0, positionMs);
            r.changedAt = System.currentTimeMillis();
            r.changedBy = u.id;
            r.lastActivity = r.changedAt;
            r.version++;
        }
        broadcast(r, "state");
        return r.version;
    }

    /** @return whether the room's state changed */
    private boolean setWaiting(Room r, String userId, boolean on) {
        synchronized (r) {
            if (on == r.waiting.containsKey(userId)) return false;
            long now = System.currentTimeMillis();
            r.freeze(now);
            if (on) r.waiting.put(userId, now); else r.waiting.remove(userId);
            r.lastActivity = now;
            r.version++;
            return true;
        }
    }

    private void expireWaiting() {
        long now = System.currentTimeMillis();
        for (Room r : rooms.values()) {
            List<String> expired = new ArrayList<>();
            synchronized (r) {
                r.waiting.forEach((id, since) -> {
                    if (now - since > WAIT_LIMIT_MS) expired.add(id);
                });
            }
            boolean changed = false;
            for (String id : expired) changed |= setWaiting(r, id, false);
            if (changed) broadcast(r, "state");
        }
    }

    public Subscriber subscribe(Room r, User u) {
        join(r, u);
        Subscriber s = new Subscriber(u.id);
        r.subscribers.add(s);
        s.queue.offer(event("state", r));
        broadcast(r, "members");
        return s;
    }

    public void unsubscribe(Room r, Subscriber s) {
        r.subscribers.remove(s);
        r.lastActivity = System.currentTimeMillis();
        // Whoever left no longer holds up the others.
        boolean stillThere = r.subscribers.stream().anyMatch(x -> x.userId.equals(s.userId));
        broadcast(r, !stillThere && setWaiting(r, s.userId, false) ? "state" : "members");
    }

    public void leave(Room r, User u) {
        setWaiting(r, u.id, false);
        r.members.remove(u.id);
        r.subscribers.removeIf(s -> s.userId.equals(u.id));
        broadcast(r, "members");
        if (r.members.isEmpty()) rooms.remove(r.code);
    }

    private void broadcast(Room r, String type) {
        String ev = event(type, r);
        for (Subscriber s : r.subscribers) {
            if (!s.queue.offer(ev)) {
                // A client that stopped reading; drop its stream.
                r.subscribers.remove(s);
            }
        }
    }

    private String event(String type, Room r) {
        return "event: " + type + "\ndata: " + Json.write(state(r)) + "\n\n";
    }

    private void cleanup() {
        long now = System.currentTimeMillis();
        rooms.values().removeIf(r -> (r.subscribers.isEmpty() && now - r.lastActivity > EMPTY_ROOM_TTL_MS)
                || now - r.lastActivity > IDLE_ROOM_TTL_MS);
    }

    @Override
    public void close() {
        cleaner.shutdownNow();
    }
}

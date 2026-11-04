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

package de.mirranet.freex.server.store;

import de.mirranet.freex.server.model.Progress;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.util.Ids;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;

/**
 * Accounts, login sessions, login history and watch state in SQLite.
 * <p>
 * Users, sessions and watch state are also kept in memory, because nearly every request needs
 * them (token check, watch state of every listed item). Every change is written to the
 * database right away, so nothing is lost on a crash.
 */
public final class UserStore {

    /** A login on one device. Stored under the SHA-256 hash of its token. */
    public record Session(String userId, long createdAt, long lastSeen, String ip, String userAgent) {
    }

    /** One login attempt for the account's security overview. */
    public record LoginEvent(long time, String ip, String userAgent, boolean success, String detail) {
    }

    private static final int MAX_LOGIN_EVENTS = 25;

    private final Database db;
    private volatile long sessionTtlMs = TimeUnit.DAYS.toMillis(30);
    private final Map<String, User> users = new ConcurrentHashMap<>();
    private final Map<String, Session> sessions = new ConcurrentHashMap<>();
    /** userId → itemId → progress */
    private final Map<String, Map<String, Progress>> progress = new ConcurrentHashMap<>();

    public UserStore(Database db) {
        this.db = db;
        load();
    }

    private void load() {
        for (User u : db.query("SELECT * FROM users", rs -> {
            User user = new User(rs.getString("id"), rs.getString("username"), rs.getString("password_hash"),
                    rs.getInt("admin") == 1, rs.getLong("created_at"));
            user.passwordChangedAt = rs.getLong("password_changed_at");
            user.totpSecret = rs.getString("totp_secret");
            user.totpEnabled = rs.getInt("totp_enabled") == 1 && user.totpSecret != null;
            user.totpLastStep = rs.getLong("totp_last_step");
            user.avatarVersion = rs.getLong("avatar_version");
            user.audioLanguage = rs.getString("audio_language");
            user.subtitleMode = rs.getString("subtitle_mode");
            user.subtitleLanguage = rs.getString("subtitle_language");
            user.allLibraries = rs.getInt("all_libraries") == 1;
            return user;
        })) {
            users.put(u.id, u);
        }
        for (User u : users.values()) {
            u.recoveryCodes = new ArrayList<>(db.query("SELECT code_hash FROM recovery_codes WHERE user_id = ?",
                    rs -> rs.getString(1), u.id));
            u.libraryIds = Set.copyOf(db.query("SELECT library_id FROM user_libraries WHERE user_id = ?",
                    rs -> rs.getString(1), u.id));
        }
        db.query("SELECT * FROM sessions", rs -> {
            sessions.put(rs.getString("token_hash"), new Session(rs.getString("user_id"), rs.getLong("created_at"),
                    rs.getLong("last_seen"), rs.getString("ip"), rs.getString("user_agent")));
            return null;
        });
        db.query("SELECT * FROM progress", rs -> {
            progress.computeIfAbsent(rs.getString("user_id"), k -> new ConcurrentHashMap<>()).put(rs.getString("item_id"),
                    new Progress(rs.getLong("offset_ms"), rs.getInt("watched") == 1, rs.getInt("view_count"), rs.getLong("updated_at")));
            return null;
        });
        long cutoff = System.currentTimeMillis() - sessionTtlMs;
        db.update("DELETE FROM sessions WHERE last_seen < ?", cutoff);
        sessions.values().removeIf(s -> s.lastSeen < cutoff);
    }

    public int userCount() {
        return users.size();
    }

    public void setSessionTtlDays(int days) {
        this.sessionTtlMs = TimeUnit.DAYS.toMillis(days);
    }

    // ================================================================== users

    public boolean hasUsers() {
        return !users.isEmpty();
    }

    public List<User> users() {
        List<User> list = new ArrayList<>(users.values());
        list.sort(Comparator.comparing(u -> u.username.toLowerCase()));
        return list;
    }

    public Optional<User> user(String id) {
        return Optional.ofNullable(id == null ? null : users.get(id));
    }

    public Optional<User> userByName(String username) {
        if (username == null) return Optional.empty();
        String n = username.trim();
        return users.values().stream().filter(u -> u.username.equalsIgnoreCase(n)).findFirst();
    }

    public synchronized User addUser(String username, String passwordHash, boolean admin) {
        if (userByName(username).isPresent()) throw new IllegalArgumentException("Benutzername ist bereits vergeben");
        User u = new User(Ids.random(8), username.trim(), passwordHash, admin, System.currentTimeMillis());
        insert(u);
        users.put(u.id, u);
        return u;
    }

    private void insert(User u) {
        db.update("INSERT INTO users (id, username, password_hash, admin, created_at, password_changed_at, totp_secret, "
                        + "totp_enabled, totp_last_step) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                u.id, u.username, u.passwordHash, u.admin, u.createdAt, u.passwordChangedAt, u.totpSecret, u.totpEnabled,
                u.totpLastStep);
    }

    /** Writes all fields of the user, including the recovery codes. */
    public void updateUser(User u) {
        db.transaction(d -> {
            d.update("UPDATE users SET password_hash = ?, admin = ?, password_changed_at = ?, totp_secret = ?, "
                            + "totp_enabled = ?, totp_last_step = ?, avatar_version = ?, audio_language = ?, "
                            + "subtitle_mode = ?, subtitle_language = ?, all_libraries = ? WHERE id = ?",
                    u.passwordHash, u.admin, u.passwordChangedAt, u.totpSecret, u.totpEnabled, u.totpLastStep,
                    u.avatarVersion, u.audioLanguage, u.subtitleMode, u.subtitleLanguage, u.allLibraries, u.id);
            d.update("DELETE FROM user_libraries WHERE user_id = ?", u.id);
            for (String lib : u.libraryIds) d.update("INSERT INTO user_libraries (user_id, library_id) VALUES (?, ?)", u.id, lib);
            d.update("DELETE FROM recovery_codes WHERE user_id = ?", u.id);
            for (String code : u.recoveryCodes) d.update("INSERT INTO recovery_codes (user_id, code_hash) VALUES (?, ?)", u.id, code);
        });
        users.put(u.id, u);
    }

    /** Removes the user; sessions, history, watch state and lists go with it (foreign keys). */
    public void removeUser(String id) {
        db.update("DELETE FROM users WHERE id = ?", id);
        users.remove(id);
        sessions.values().removeIf(s -> s.userId.equals(id));
        progress.remove(id);
    }

    /** A deleted library disappears from all user assignments. */
    public void removeLibraryAccess(String libraryId) {
        db.update("DELETE FROM user_libraries WHERE library_id = ?", libraryId);
        for (User u : users.values()) {
            if (u.libraryIds.contains(libraryId)) {
                Set<String> s = new HashSet<>(u.libraryIds);
                s.remove(libraryId);
                u.libraryIds = Set.copyOf(s);
            }
        }
    }

    // ================================================================== sessions

    /** Creates a login session and returns the token (the only time it exists in plain text). */
    public String createSession(String userId, String ip, String userAgent) {
        String token = Ids.token();
        long now = System.currentTimeMillis();
        Session s = new Session(userId, now, now, ip == null ? "" : ip, shorten(userAgent));
        String key = Ids.sha256(token);
        db.update("INSERT INTO sessions (token_hash, user_id, created_at, last_seen, ip, user_agent) VALUES (?, ?, ?, ?, ?, ?)",
                key, userId, now, now, s.ip, s.userAgent);
        sessions.put(key, s);
        return token;
    }

    private static String shorten(String s) {
        if (s == null) return "";
        return s.length() > 300 ? s.substring(0, 300) : s;
    }

    public Optional<User> userForToken(String token) {
        if (token == null || token.isBlank() || token.length() > 128) return Optional.empty();
        String key = Ids.sha256(token);
        Session s = sessions.get(key);
        if (s == null) return Optional.empty();
        long now = System.currentTimeMillis();
        if (now - s.lastSeen > sessionTtlMs) {
            removeByKey(key);
            return Optional.empty();
        }
        // Only persist the "last seen" bump occasionally, it is not worth a write per request.
        if (now - s.lastSeen > TimeUnit.MINUTES.toMillis(10)) {
            sessions.put(key, new Session(s.userId, s.createdAt, now, s.ip, s.userAgent));
            db.update("UPDATE sessions SET last_seen = ? WHERE token_hash = ?", now, key);
        }
        return user(s.userId);
    }

    private void removeByKey(String key) {
        sessions.remove(key);
        db.update("DELETE FROM sessions WHERE token_hash = ?", key);
    }

    /** Sessions of a user, keyed by public id (hash prefix), most recently used first. */
    public Map<String, Session> sessionsOf(String userId) {
        Map<String, Session> out = new LinkedHashMap<>();
        sessions.entrySet().stream()
                .filter(e -> e.getValue().userId.equals(userId))
                .sorted((a, b) -> Long.compare(b.getValue().lastSeen, a.getValue().lastSeen))
                .forEach(e -> out.put(e.getKey().substring(0, 16), e.getValue()));
        return out;
    }

    public boolean removeSessionById(String userId, String publicId) {
        if (publicId == null || publicId.length() != 16) return false;
        boolean removed = false;
        for (Map.Entry<String, Session> e : List.copyOf(sessions.entrySet())) {
            if (e.getKey().startsWith(publicId) && e.getValue().userId.equals(userId)) {
                removeByKey(e.getKey());
                removed = true;
            }
        }
        return removed;
    }

    /** Ends all sessions of a user except the one with the given token (null = all). */
    public int removeSessionsOf(String userId, String keepToken) {
        String keep = keepToken == null ? null : Ids.sha256(keepToken);
        int n = 0;
        for (Map.Entry<String, Session> e : List.copyOf(sessions.entrySet())) {
            if (e.getValue().userId.equals(userId) && !e.getKey().equals(keep)) {
                removeByKey(e.getKey());
                n++;
            }
        }
        return n;
    }

    public void removeSession(String token) {
        if (token != null) removeByKey(Ids.sha256(token));
    }

    // ================================================================== login history

    public void addLoginEvent(String userId, LoginEvent e) {
        db.transaction(d -> {
            d.update("INSERT INTO login_events (user_id, time, ip, user_agent, success, detail) VALUES (?, ?, ?, ?, ?, ?)",
                    userId, e.time, e.ip == null ? "" : e.ip, shorten(e.userAgent), e.success, e.detail == null ? "" : e.detail);
            // Keep only the newest entries per user.
            d.update("DELETE FROM login_events WHERE user_id = ? AND id NOT IN "
                    + "(SELECT id FROM login_events WHERE user_id = ? ORDER BY time DESC, id DESC LIMIT ?)", userId, userId, MAX_LOGIN_EVENTS);
        });
    }

    public List<LoginEvent> loginEvents(String userId) {
        return db.query("SELECT * FROM login_events WHERE user_id = ? ORDER BY time DESC, id DESC LIMIT ?",
                rs -> new LoginEvent(rs.getLong("time"), rs.getString("ip"), rs.getString("user_agent"),
                        rs.getInt("success") == 1, rs.getString("detail")), userId, MAX_LOGIN_EVENTS);
    }

    // ================================================================== watch state

    public Progress progress(String userId, String itemId) {
        Map<String, Progress> m = progress.get(userId);
        return m == null ? Progress.NONE : m.getOrDefault(itemId, Progress.NONE);
    }

    public Map<String, Progress> progressOf(String userId) {
        return progress.getOrDefault(userId, Map.of());
    }

    public void setProgress(String userId, String itemId, Progress p) {
        if (!users.containsKey(userId)) return;
        db.update("INSERT INTO progress (user_id, item_id, offset_ms, watched, view_count, updated_at) VALUES (?, ?, ?, ?, ?, ?) "
                        + "ON CONFLICT(user_id, item_id) DO UPDATE SET offset_ms = excluded.offset_ms, watched = excluded.watched, "
                        + "view_count = excluded.view_count, updated_at = excluded.updated_at",
                userId, itemId, p.offsetMs(), p.watched(), p.viewCount(), p.updatedAt());
        progress.computeIfAbsent(userId, k -> new ConcurrentHashMap<>()).put(itemId, p);
    }

    // ================================================================== watch history

    /**
     * One finished playback. {@code title}/{@code parentTitle} are a snapshot, so the entry stays
     * readable when the item later disappears from the library.
     */
    public record HistoryEntry(long id, String userId, String itemId, long watchedAt, String title, String parentTitle) {
    }

    public void addHistory(String userId, String itemId, long time, String title, String parentTitle) {
        if (!users.containsKey(userId)) return;
        db.update("INSERT INTO watch_history (user_id, item_id, watched_at, title, parent_title) VALUES (?, ?, ?, ?, ?)",
                userId, itemId, time, title == null ? "" : title, parentTitle == null ? "" : parentTitle);
    }

    /** Newest first; {@code userId} null = all users. */
    public List<HistoryEntry> history(String userId, int offset, int limit) {
        String sql = "SELECT * FROM watch_history WHERE (? IS NULL OR user_id = ?) "
                + "ORDER BY watched_at DESC, id DESC LIMIT ? OFFSET ?";
        return db.query(sql, rs -> new HistoryEntry(rs.getLong("id"), rs.getString("user_id"), rs.getString("item_id"),
                        rs.getLong("watched_at"), rs.getString("title"), rs.getString("parent_title")),
                userId, userId, limit, offset);
    }

    /** Deletes one entry of the user; false if there is none with that id. */
    public boolean removeHistory(String userId, long id) {
        return db.update("DELETE FROM watch_history WHERE id = ? AND user_id = ?", id, userId) > 0;
    }

    public int clearHistory(String userId) {
        return db.update("DELETE FROM watch_history WHERE user_id = ?", userId);
    }
}

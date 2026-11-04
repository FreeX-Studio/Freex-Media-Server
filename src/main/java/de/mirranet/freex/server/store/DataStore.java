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

import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.Library;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.model.Progress;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.util.Ids;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Entry point to the server's state.
 * <p>
 * Users and everything that belongs to them (sessions, login history, watch state) live in the
 * SQLite database and are reached through {@link UserStore}. Libraries and the scanned index are
 * held in memory and written to JSON files in the data directory, because they can be rebuilt
 * by a scan at any time: changes mark a file dirty, a background task writes dirty files every few
 * seconds (atomically, via a temp file) and {@link #close()} flushes everything on shutdown.
 * <ul>
 *   <li>{@code libraries.json} – library definitions</li>
 *   <li>{@code items/<libraryId>.json} – scanned index of one library</li>
 * </ul>
 */
public final class DataStore implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(DataStore.class.getName());

    private final Path dir;
    private final UserStore userStore;
    private final Map<String, Library> libraries = new ConcurrentHashMap<>();
    private final Map<String, MediaItem> items = new ConcurrentHashMap<>();

    /** parentId → child ids, rebuilt whenever a library index is replaced. */
    private volatile Map<String, List<String>> childIndex = Map.of();

    private final Object writeLock = new Object();
    private final java.util.Set<String> dirty = ConcurrentHashMap.newKeySet();
    private final ScheduledExecutorService saver = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "datastore-saver");
        t.setDaemon(true);
        return t;
    });

    public DataStore(Path dir, UserStore userStore) throws IOException {
        this.dir = dir;
        this.userStore = userStore;
        Files.createDirectories(dir.resolve("items"));
        try {
            Files.setPosixFilePermissions(dir, java.nio.file.attribute.PosixFilePermissions.fromString("rwx------"));
        } catch (UnsupportedOperationException | IOException ignored) {
            // Windows or a file system without POSIX permissions
        }
        load();
        saver.scheduleWithFixedDelay(this::flushQuietly, 5, 5, TimeUnit.SECONDS);
    }

    public Path directory() {
        return dir;
    }

    public void setSessionTtlDays(int days) {
        userStore.setSessionTtlDays(days);
    }

    // ================================================================== users (SQLite, see UserStore)

    public boolean hasUsers() {
        return userStore.hasUsers();
    }

    public List<User> users() {
        return userStore.users();
    }

    public Optional<User> user(String id) {
        return userStore.user(id);
    }

    public Optional<User> userByName(String username) {
        return userStore.userByName(username);
    }

    public User addUser(String username, String passwordHash, boolean admin) {
        return userStore.addUser(username, passwordHash, admin);
    }

    public void updateUser(User u) {
        userStore.updateUser(u);
    }

    public void removeUser(String id) {
        userStore.removeUser(id);
    }

    public String createSession(String userId, String ip, String userAgent) {
        return userStore.createSession(userId, ip, userAgent);
    }

    public Optional<User> userForToken(String token) {
        return userStore.userForToken(token);
    }

    /** Public id of a session: a prefix of its hash, enough to address it, useless as a token. */
    public static String sessionId(String token) {
        return token == null ? "" : Ids.sha256(token).substring(0, 16);
    }

    public Map<String, UserStore.Session> sessionsOf(String userId) {
        return userStore.sessionsOf(userId);
    }

    public boolean removeSessionById(String userId, String publicId) {
        return userStore.removeSessionById(userId, publicId);
    }

    public int removeSessionsOf(String userId, String keepToken) {
        return userStore.removeSessionsOf(userId, keepToken);
    }

    public void removeSession(String token) {
        userStore.removeSession(token);
    }

    public void addLoginEvent(String userId, UserStore.LoginEvent e) {
        userStore.addLoginEvent(userId, e);
    }

    public List<UserStore.LoginEvent> loginEvents(String userId) {
        return userStore.loginEvents(userId);
    }

    // ================================================================== libraries

    public List<Library> libraries() {
        List<Library> list = new ArrayList<>(libraries.values());
        list.sort(Comparator.comparingLong((Library l) -> l.createdAt).thenComparing(l -> l.name));
        return list;
    }

    public Optional<Library> library(String id) {
        return Optional.ofNullable(id == null ? null : libraries.get(id));
    }

    public Library addLibrary(String name, Library.Type type, List<String> paths) {
        Library l = new Library(Ids.random(6), name, type, paths, System.currentTimeMillis(), 0);
        libraries.put(l.id, l);
        dirty.add("libraries");
        return l;
    }

    public void saveLibrary(Library l) {
        libraries.put(l.id, l);
        dirty.add("libraries");
    }

    public void removeLibrary(String id) {
        synchronized (writeLock) {
            libraries.remove(id);
            items.values().removeIf(i -> i.libraryId.equals(id));
            rebuildIndex();
            dirty.add("libraries");
            try {
                Files.deleteIfExists(dir.resolve("items").resolve(id + ".json"));
            } catch (IOException e) {
                LOG.log(Level.WARNING, "Could not delete index of library " + id, e);
            }
        }
    }

    // ================================================================== items

    public Optional<MediaItem> item(String id) {
        return Optional.ofNullable(id == null ? null : items.get(id));
    }

    public List<MediaItem> children(String parentId) {
        List<MediaItem> out = new ArrayList<>();
        for (String id : childIndex.getOrDefault(parentId, List.of())) {
            MediaItem it = items.get(id);
            if (it != null) out.add(it);
        }
        return out;
    }

    public List<MediaItem> itemsOfLibrary(String libraryId) {
        List<MediaItem> out = new ArrayList<>();
        for (MediaItem it : items.values()) if (it.libraryId.equals(libraryId)) out.add(it);
        return out;
    }

    public Collection<MediaItem> allItems() {
        return items.values();
    }

    /** Top-level entries of a library: movies, or shows. */
    public List<MediaItem> topLevel(String libraryId) {
        List<MediaItem> out = new ArrayList<>();
        for (MediaItem it : items.values()) {
            if (it.libraryId.equals(libraryId) && it.parentId == null) out.add(it);
        }
        return out;
    }

    public int countTopLevel(String libraryId) {
        int n = 0;
        for (MediaItem it : items.values()) if (it.libraryId.equals(libraryId) && it.parentId == null) n++;
        return n;
    }

    /** Replaces the whole index of one library with the result of a scan. */
    public void replaceLibraryItems(String libraryId, Collection<MediaItem> scanned) {
        synchronized (writeLock) {
            items.values().removeIf(i -> i.libraryId.equals(libraryId));
            for (MediaItem it : scanned) items.put(it.id, it);
            rebuildIndex();
            dirty.add("items:" + libraryId);
        }
    }

    /** Marks a library index for saving after an item inside it was changed in place. */
    public void touchLibraryItems(String libraryId) {
        dirty.add("items:" + libraryId);
    }

    private void rebuildIndex() {
        Map<String, List<String>> idx = new HashMap<>();
        List<MediaItem> sorted = new ArrayList<>(items.values());
        sorted.sort(Comparator.comparingInt((MediaItem i) -> i.index).thenComparing(i -> i.sortTitle));
        for (MediaItem it : sorted) {
            if (it.parentId != null) idx.computeIfAbsent(it.parentId, k -> new ArrayList<>()).add(it.id);
        }
        childIndex = idx;
    }

    // ================================================================== progress (SQLite, see UserStore)

    public Progress progress(String userId, String itemId) {
        return userStore.progress(userId, itemId);
    }

    public Map<String, Progress> progressOf(String userId) {
        return userStore.progressOf(userId);
    }

    public void setProgress(String userId, String itemId, Progress p) {
        userStore.setProgress(userId, itemId, p);
    }

    /** Users, preferences and watch history in the database. */
    public UserStore userStore() {
        return userStore;
    }

    // ================================================================== persistence

    private void load() throws IOException {
        for (Object o : Json.list(readJson("libraries.json", "[]"))) {
            try {
                Library l = Library.fromStorage(Json.obj(o));
                libraries.put(l.id, l);
            } catch (IllegalArgumentException ex) {
                LOG.warning("Skipping invalid library entry: " + ex.getMessage());
            }
        }
        for (Library l : libraries.values()) {
            for (Object o : Json.list(readJson("items/" + l.id + ".json", "[]"))) {
                MediaItem it = MediaItem.fromStorage(Json.obj(o));
                items.put(it.id, it);
            }
        }
        rebuildIndex();
        LOG.info("Loaded " + userStore.userCount() + " users, " + libraries.size() + " libraries, " + items.size() + " items");
    }

    private Object readJson(String name, String fallback) throws IOException {
        Path p = dir.resolve(name);
        if (!Files.exists(p)) return Json.parse(fallback);
        try {
            return Json.parse(Files.readString(p, StandardCharsets.UTF_8));
        } catch (Json.JsonException e) {
            // Keep the broken file for inspection instead of silently overwriting it later.
            Path backup = p.resolveSibling(p.getFileName() + ".broken-" + System.currentTimeMillis());
            Files.copy(p, backup);
            LOG.severe("Could not parse " + p + " (" + e.getMessage() + "), saved a copy as " + backup);
            return Json.parse(fallback);
        }
    }

    private void flushQuietly() {
        try {
            flush();
        } catch (Exception e) {
            LOG.log(Level.WARNING, "Saving data failed", e);
        }
    }

    /** Writes every dirty file now. */
    public void flush() throws IOException {
        if (dirty.isEmpty()) return;
        List<String> todo = new ArrayList<>(dirty);
        for (String key : todo) {
            dirty.remove(key);
            try {
                writeOne(key);
            } catch (IOException e) {
                dirty.add(key); // retry on the next round
                throw e;
            }
        }
    }

    private void writeOne(String key) throws IOException {
        switch (key) {
            case "libraries" -> {
                List<Object> list = new ArrayList<>();
                for (Library l : libraries()) list.add(l.toStorage());
                writeAtomic("libraries.json", list);
            }
            default -> {
                if (key.startsWith("items:")) {
                    String libId = key.substring("items:".length());
                    if (!libraries.containsKey(libId)) return;
                    List<Object> list = new ArrayList<>();
                    for (MediaItem it : itemsOfLibrary(libId)) list.add(it.toStorage());
                    writeAtomic("items/" + libId + ".json", list);
                }
            }
        }
    }

    private void writeAtomic(String name, Object value) throws IOException {
        Path target = dir.resolve(name);
        Path tmp = target.resolveSibling(target.getFileName() + ".tmp");
        Files.writeString(tmp, Json.write(value), StandardCharsets.UTF_8);
        restrictPermissions(tmp);
        try {
            Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        } catch (AtomicMoveNotSupportedException e) {
            Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING);
        }
    }

    /** Data files contain password hashes and 2FA secrets: readable for the server user only (POSIX). */
    private static void restrictPermissions(Path p) {
        try {
            Files.setPosixFilePermissions(p, java.nio.file.attribute.PosixFilePermissions.fromString("rw-------"));
        } catch (UnsupportedOperationException | IOException ignored) {
            // Windows or a file system without POSIX permissions
        }
    }

    @Override
    public void close() {
        saver.shutdown();
        flushQuietly();
    }
}

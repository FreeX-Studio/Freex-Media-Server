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

import de.mirranet.freex.server.util.Ids;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * User-made lists in SQLite: collections (per library, made by admins), playlists and the
 * watchlist (per user). The lists are kept in memory as well; after changing one, callers pass
 * it to {@link #changedCollection} / {@link #changedPlaylist}, which write it in one transaction.
 */
public final class ListStore implements AutoCloseable {

    /** A hand-made collection. Collections from TMDB movie series are built on the fly elsewhere. */
    public static final class Collection {
        public final String id;
        public final String libraryId;
        public volatile String name;
        public volatile String summary;
        public final List<String> itemIds;
        public final long createdAt;

        Collection(String id, String libraryId, String name, String summary, List<String> itemIds, long createdAt) {
            this.id = id;
            this.libraryId = libraryId;
            this.name = name;
            this.summary = summary;
            this.itemIds = new CopyOnWriteArrayList<>(itemIds);
            this.createdAt = createdAt;
        }
    }

    /** A user's playlist of movies and episodes, in play order. */
    public static final class Playlist {
        public final String id;
        public final String userId;
        public volatile String name;
        public final List<String> itemIds;
        public final long createdAt;
        public volatile long updatedAt;

        Playlist(String id, String userId, String name, List<String> itemIds, long createdAt, long updatedAt) {
            this.id = id;
            this.userId = userId;
            this.name = name;
            this.itemIds = new CopyOnWriteArrayList<>(itemIds);
            this.createdAt = createdAt;
            this.updatedAt = updatedAt;
        }
    }

    private final Database db;
    private final Map<String, Collection> collections = new ConcurrentHashMap<>();
    private final Map<String, Playlist> playlists = new ConcurrentHashMap<>();
    /** userId → item ids, newest first */
    private final Map<String, List<String>> watchlists = new ConcurrentHashMap<>();

    public ListStore(Database db) {
        this.db = db;
        load();
    }

    private void load() {
        db.query("SELECT * FROM collections", rs -> {
            String id = rs.getString("id");
            collections.put(id, new Collection(id, rs.getString("library_id"), rs.getString("name"), rs.getString("summary"),
                    List.of(), rs.getLong("created_at")));
            return null;
        });
        db.query("SELECT collection_id, item_id FROM collection_items ORDER BY collection_id, position", rs -> {
            Collection c = collections.get(rs.getString(1));
            if (c != null) c.itemIds.add(rs.getString(2));
            return null;
        });
        db.query("SELECT * FROM playlists", rs -> {
            String id = rs.getString("id");
            playlists.put(id, new Playlist(id, rs.getString("user_id"), rs.getString("name"), List.of(),
                    rs.getLong("created_at"), rs.getLong("updated_at")));
            return null;
        });
        db.query("SELECT playlist_id, item_id FROM playlist_items ORDER BY playlist_id, position", rs -> {
            Playlist p = playlists.get(rs.getString(1));
            if (p != null) p.itemIds.add(rs.getString(2));
            return null;
        });
        db.query("SELECT user_id, item_id FROM watchlist ORDER BY added_at DESC", rs -> {
            watchlists.computeIfAbsent(rs.getString(1), k -> new CopyOnWriteArrayList<>()).add(rs.getString(2));
            return null;
        });
    }

    // ------------------------------------------------------------------ collections

    public List<Collection> collectionsOf(String libraryId) {
        List<Collection> out = new ArrayList<>();
        for (Collection c : collections.values()) if (c.libraryId.equals(libraryId)) out.add(c);
        out.sort(Comparator.comparing(c -> c.name.toLowerCase()));
        return out;
    }

    public Optional<Collection> collection(String id) {
        return Optional.ofNullable(id == null ? null : collections.get(id));
    }

    public Collection createCollection(String libraryId, String name) {
        Collection c = new Collection(Ids.random(6), libraryId, name, "", List.of(), System.currentTimeMillis());
        db.update("INSERT INTO collections (id, library_id, name, summary, created_at) VALUES (?, ?, ?, ?, ?)",
                c.id, c.libraryId, c.name, c.summary, c.createdAt);
        collections.put(c.id, c);
        return c;
    }

    /** Writes name, summary and item order of a collection. */
    public void changedCollection(Collection c) {
        db.transaction(d -> {
            d.update("UPDATE collections SET name = ?, summary = ? WHERE id = ?", c.name, c.summary, c.id);
            d.update("DELETE FROM collection_items WHERE collection_id = ?", c.id);
            int pos = 0;
            for (String itemId : c.itemIds) {
                d.update("INSERT INTO collection_items (collection_id, position, item_id) VALUES (?, ?, ?)", c.id, pos++, itemId);
            }
        });
    }

    public void deleteCollection(String id) {
        db.update("DELETE FROM collections WHERE id = ?", id);
        collections.remove(id);
    }

    /** Hand-made collections containing the item. */
    public List<Collection> collectionsWith(String itemId) {
        List<Collection> out = new ArrayList<>();
        for (Collection c : collections.values()) if (c.itemIds.contains(itemId)) out.add(c);
        return out;
    }

    // ------------------------------------------------------------------ playlists

    public List<Playlist> playlistsOf(String userId) {
        List<Playlist> out = new ArrayList<>();
        for (Playlist p : playlists.values()) if (p.userId.equals(userId)) out.add(p);
        out.sort(Comparator.comparingLong((Playlist p) -> p.updatedAt).reversed());
        return out;
    }

    public Optional<Playlist> playlist(String userId, String id) {
        Playlist p = id == null ? null : playlists.get(id);
        return Optional.ofNullable(p != null && p.userId.equals(userId) ? p : null);
    }

    public Playlist createPlaylist(String userId, String name, List<String> itemIds) {
        long now = System.currentTimeMillis();
        Playlist p = new Playlist(Ids.random(6), userId, name, itemIds, now, now);
        db.update("INSERT INTO playlists (id, user_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
                p.id, p.userId, p.name, p.createdAt, p.updatedAt);
        playlists.put(p.id, p);
        if (!itemIds.isEmpty()) changedPlaylist(p);
        return p;
    }

    /** Writes name and item order of a playlist. */
    public void changedPlaylist(Playlist p) {
        p.updatedAt = System.currentTimeMillis();
        db.transaction(d -> {
            d.update("UPDATE playlists SET name = ?, updated_at = ? WHERE id = ?", p.name, p.updatedAt, p.id);
            d.update("DELETE FROM playlist_items WHERE playlist_id = ?", p.id);
            int pos = 0;
            for (String itemId : p.itemIds) {
                d.update("INSERT INTO playlist_items (playlist_id, position, item_id) VALUES (?, ?, ?)", p.id, pos++, itemId);
            }
        });
    }

    public void deletePlaylist(String id) {
        db.update("DELETE FROM playlists WHERE id = ?", id);
        playlists.remove(id);
    }

    // ------------------------------------------------------------------ watchlist

    public List<String> watchlist(String userId) {
        return List.copyOf(watchlists.getOrDefault(userId, List.of()));
    }

    public boolean onWatchlist(String userId, String itemId) {
        return watchlists.getOrDefault(userId, List.of()).contains(itemId);
    }

    public void setWatchlist(String userId, String itemId, boolean on) {
        if (on) {
            db.update("INSERT INTO watchlist (user_id, item_id, added_at) VALUES (?, ?, ?) "
                    + "ON CONFLICT(user_id, item_id) DO UPDATE SET added_at = excluded.added_at", userId, itemId, System.currentTimeMillis());
        } else {
            db.update("DELETE FROM watchlist WHERE user_id = ? AND item_id = ?", userId, itemId);
        }
        watchlists.compute(userId, (k, list) -> {
            List<String> l = list == null ? new CopyOnWriteArrayList<>() : list;
            l.remove(itemId);
            if (on) l.add(0, itemId);
            return l;
        });
    }

    /** Forgets the lists of a deleted user (the database rows go with the user's foreign key). */
    public void removeUser(String userId) {
        playlists.values().removeIf(p -> p.userId.equals(userId));
        watchlists.remove(userId);
    }

    @Override
    public void close() {
        // Everything is written immediately; nothing to flush.
    }
}

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
import de.mirranet.freex.server.markers.MarkerService;
import de.mirranet.freex.server.metadata.Metadata;
import de.mirranet.freex.server.metadata.MetadataService;
import de.mirranet.freex.server.model.Library;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.store.ListStore;
import de.mirranet.freex.server.together.TogetherService;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

import static de.mirranet.freex.server.http.WebServer.Access.ADMIN;
import static de.mirranet.freex.server.http.WebServer.Access.USER;

/**
 * Endpoints for online metadata (fix match, people), collections, playlists, the watchlist,
 * intro/credits markers and Watch Together.
 */
public final class ExtrasApi implements MediaApi.Extension {

    private static final int IMAGE_CACHE_SECONDS = 86_400;

    private final DataStore store;
    private final MediaApi media;
    private final MetadataService metadata;
    private final ListStore lists;
    private final MarkerService markers;
    private final TogetherService together;

    public ExtrasApi(DataStore store, MediaApi media, MetadataService metadata, ListStore lists,
                     MarkerService markers, TogetherService together) {
        this.store = store;
        this.media = media;
        this.metadata = metadata;
        this.lists = lists;
        this.markers = markers;
        this.together = together;
        media.addExtension(this);
    }

    public void register(WebServer web) {
        // metadata
        web.route("GET", "/api/items/{id}/metadata/search", ADMIN, this::metadataSearch);
        web.route("GET", "/api/metadata/image", ADMIN, this::metadataImage);
        web.route("POST", "/api/items/{id}/metadata/match", ADMIN, this::metadataMatch);
        web.route("POST", "/api/items/{id}/metadata/refresh", ADMIN, this::metadataRefresh);
        web.route("POST", "/api/items/{id}/metadata/unmatch", ADMIN, this::metadataUnmatch);
        web.route("POST", "/api/libraries/{id}/metadata", ADMIN, ex -> {
            Library l = library(ex);
            metadata.refreshLibrary(l, true);
            ex.json(202, Map.of("queued", metadata.enabled()));
        });
        web.route("POST", "/api/libraries/{id}/markers", ADMIN, ex -> {
            Library l = library(ex);
            if (markers != null) markers.analyzeLibrary(l, true);
            ex.json(202, Map.of("queued", markers != null && l.type == Library.Type.SHOWS));
        });
        web.route("GET", "/api/extras/status", ADMIN, ex -> {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("tmdb", metadata.enabled());
            m.put("metadataBusy", metadata.busy());
            m.put("metadataError", metadata.lastError());
            m.put("markers", markers != null);
            m.put("markersBusy", markers != null && markers.busy());
            ex.ok(m);
        });
        web.route("GET", "/api/people/{pid}", USER, this::person);
        web.route("GET", "/api/people/{pid}/image", USER, this::personImage);

        // collections
        web.route("GET", "/api/libraries/{id}/collections", USER, ex -> ex.ok(collectionsOf(library(ex), ex.user())));
        web.route("GET", "/api/collections/{cid}", USER, this::collection);
        web.route("GET", "/api/collections/{cid}/image/{kind}", USER, this::collectionImage);
        web.route("POST", "/api/collections", ADMIN, this::createCollection);
        web.route("PUT", "/api/collections/{cid}", ADMIN, this::updateCollection);
        web.route("DELETE", "/api/collections/{cid}", ADMIN, ex -> {
            lists.deleteCollection(ownCollection(ex).id);
            ex.noContent();
        });
        web.route("POST", "/api/collections/{cid}/items", ADMIN, this::addToCollection);
        web.route("DELETE", "/api/collections/{cid}/items/{itemId}", ADMIN, ex -> {
            ListStore.Collection c = ownCollection(ex);
            c.itemIds.remove(ex.param("itemId"));
            lists.changedCollection(c);
            ex.noContent();
        });

        // playlists
        web.route("GET", "/api/playlists", USER, this::playlists);
        web.route("POST", "/api/playlists", USER, this::createPlaylist);
        web.route("GET", "/api/playlists/{pid}", USER, this::playlist);
        web.route("PUT", "/api/playlists/{pid}", USER, this::updatePlaylist);
        web.route("DELETE", "/api/playlists/{pid}", USER, ex -> {
            lists.deletePlaylist(ownPlaylist(ex).id);
            ex.noContent();
        });
        web.route("POST", "/api/playlists/{pid}/items", USER, this::addToPlaylist);
        web.route("DELETE", "/api/playlists/{pid}/items/{index}", USER, this::removeFromPlaylist);

        // watchlist
        web.route("GET", "/api/watchlist", USER, ex -> ex.ok(watchlistItems(ex.user())));
        web.route("PUT", "/api/watchlist/{itemId}", USER, ex -> setWatchlist(ex, true));
        web.route("DELETE", "/api/watchlist/{itemId}", USER, ex -> setWatchlist(ex, false));

        // watch together
        web.route("POST", "/api/together", USER, this::createRoom);
        web.route("GET", "/api/together/{code}", USER, this::joinRoom);
        web.route("GET", "/api/together/{code}/events", USER, this::roomEvents);
        web.route("POST", "/api/together/{code}/state", USER, this::roomState);
        web.route("DELETE", "/api/together/{code}", USER, ex -> {
            TogetherService.Room r = room(ex);
            together.leave(r, ex.user());
            ex.noContent();
        });
    }

    /** Library or item of the path, only if shared with the user (404 otherwise). */
    private Library library(Exchange ex) {
        return store.library(ex.param("id")).filter(l -> store.canSee(ex.user(), l)).orElseThrow(ApiException::notFound);
    }

    private MediaItem item(Exchange ex, String param) {
        return store.item(ex.user(), ex.param(param)).orElseThrow(ApiException::notFound);
    }

    /** A collection of a library shared with the user, with only the items the user may see. */
    private Resolved visibleCollection(Exchange ex) {
        Resolved r = resolve(ex.param("cid")).orElseThrow(ApiException::notFound);
        if (!ex.user().mayAccess(r.libraryId)) throw ApiException.notFound();
        List<MediaItem> items = r.items.stream().filter(it -> store.canSee(ex.user(), it)).toList();
        return new Resolved(r.id, r.libraryId, r.name, r.summary, r.auto, items, r.poster, r.backdrop);
    }

    // ================================================================== item extras

    @Override
    public void decorate(Map<String, Object> m, MediaItem it, User u, boolean full) {
        if (it.type == MediaItem.Type.MOVIE || it.type == MediaItem.Type.SHOW) {
            m.put("onWatchlist", lists.onWatchlist(u.id, it.id));
        }
        if (it.type == MediaItem.Type.EPISODE && markers != null && full) {
            MarkerService.Markers mk = markers.get(it);
            if (mk != null) {
                Map<String, Object> mm = new LinkedHashMap<>();
                if (mk.intro() != null) mm.put("intro", Map.of("startMs", mk.intro().startMs(), "endMs", mk.intro().endMs()));
                if (mk.credits() != null) mm.put("credits", Map.of("startMs", mk.credits().startMs(), "endMs", mk.credits().endMs()));
                m.put("markers", mm);
            }
        }
        if (!full) return;
        Metadata md = metadata.get(it.id);
        if (md == null && it.type == MediaItem.Type.EPISODE) {
            // Episode without its own metadata: show the show's cast.
            MediaItem season = store.item(it.parentId).orElse(null);
            md = season == null ? null : metadata.get(season.parentId);
            if (md != null) {
                Map<String, Object> e = new LinkedHashMap<>();
                e.put("cast", castOf(md));
                m.put("metadata", e);
            }
            return;
        }
        if (md == null) return;
        Map<String, Object> e = new LinkedHashMap<>();
        e.put("tmdbId", md.tmdbId);
        e.put("matched", md.matched());
        e.put("locked", md.locked);
        e.put("contentRating", md.contentRating);
        e.put("studio", md.studio);
        e.put("airDate", md.airDate);
        e.put("trailer", md.trailerKey);
        e.put("directors", md.directors);
        e.put("writers", md.writers);
        List<Map<String, Object>> cast = castOf(md);
        if (cast.isEmpty() && it.type == MediaItem.Type.EPISODE) {
            MediaItem season = store.item(it.parentId).orElse(null);
            Metadata showMd = season == null ? null : metadata.get(season.parentId);
            if (showMd != null) cast = castOf(showMd);
        }
        e.put("cast", cast);
        m.put("metadata", e);

        List<Object> colls = new ArrayList<>();
        if (md.collectionId > 0) {
            colls.add(Map.of("id", autoId(md.collectionId, it.libraryId), "name", collectionName(md)));
        }
        for (ListStore.Collection c : lists.collectionsWith(it.id)) colls.add(Map.of("id", c.id, "name", c.name));
        m.put("collections", colls);
    }

    private List<Map<String, Object>> castOf(Metadata md) {
        List<Map<String, Object>> out = new ArrayList<>();
        for (Metadata.Person p : md.cast) {
            Map<String, Object> pm = new LinkedHashMap<>();
            pm.put("id", p.id());
            pm.put("name", p.name());
            pm.put("role", p.role());
            pm.put("hasImage", p.image() != null);
            out.add(pm);
        }
        return out;
    }

    private String collectionName(Metadata md) {
        Map<String, Object> c = metadata.collection(md.collectionId);
        String n = c == null ? md.collectionName : Json.str(c, "name", md.collectionName);
        return n.isBlank() ? md.collectionName : n;
    }

    @Override
    public List<Map<String, Object>> homeHubs(User u) {
        List<Object> items = new ArrayList<>();
        for (String id : lists.watchlist(u.id)) {
            store.item(u, id).ifPresent(it -> items.add(media.view(it, u, false)));
            if (items.size() >= 24) break;
        }
        if (items.isEmpty()) return List.of();
        Map<String, Object> hub = MediaApi.hubOf("watchlist", "Merkliste", null, "poster", items);
        hub.put("href", "/watchlist");
        return List.of(hub);
    }

    @Override
    public List<Map<String, Object>> libraryHubs(Library l, User u) {
        List<Map<String, Object>> colls = collectionsOf(l, u);
        if (colls.isEmpty()) return List.of();
        Map<String, Object> hub = MediaApi.hubOf("collections", "Sammlungen", l.id, "collection", new ArrayList<>(colls));
        hub.put("href", "/library/" + l.id + "/collections");
        return List.of(hub);
    }

    // ================================================================== metadata

    private void metadataSearch(Exchange ex) throws Exception {
        MediaItem it = item(ex, "id");
        String type = it.type == MediaItem.Type.MOVIE ? "movie" : "tv";
        String q = ex.query("q", it.title).trim();
        if (q.isEmpty()) throw ApiException.badRequest("Bitte einen Suchbegriff angeben");
        try {
            ex.ok(metadata.search(type, q, ex.queryInt("year", 0)));
        } catch (IOException e) {
            throw new ApiException(502, e.getMessage());
        }
    }

    private void metadataImage(Exchange ex) throws Exception {
        Path p = metadata.searchImage(ex.query("path"));
        if (p == null) throw ApiException.notFound();
        FileSender.send(ex, p, IMAGE_CACHE_SECONDS);
    }

    private void metadataMatch(Exchange ex) throws Exception {
        MediaItem it = item(ex, "id");
        int tmdbId = (int) Json.num(ex.body(), "tmdbId", 0);
        if (tmdbId <= 0) throw ApiException.badRequest("tmdbId fehlt");
        try {
            metadata.match(it, tmdbId);
        } catch (IOException e) {
            throw new ApiException(502, e.getMessage());
        }
        ex.ok(media.view(store.item(it.id).orElse(it), ex.user(), true));
    }

    private void metadataRefresh(Exchange ex) throws Exception {
        MediaItem it = item(ex, "id");
        try {
            metadata.refreshItem(it);
        } catch (IOException e) {
            throw new ApiException(502, e.getMessage());
        }
        ex.ok(media.view(store.item(it.id).orElse(it), ex.user(), true));
    }

    private void metadataUnmatch(Exchange ex) throws IOException {
        MediaItem it = item(ex, "id");
        metadata.unmatch(it);
        ex.json(202, Map.of("ok", true, "hint", "Wird beim nächsten Scan zurückgesetzt"));
    }

    private void person(Exchange ex) throws IOException {
        int pid = parseInt(ex.param("pid"));
        Metadata.Person p = metadata.person(pid);
        if (p == null) throw ApiException.notFound();
        List<Object> items = new ArrayList<>();
        List<MediaItem> found = new ArrayList<>();
        for (String id : metadata.itemsWithPerson(pid)) {
            store.item(ex.user(), id).ifPresent(it -> {
                // Guest roles in episodes link to the show.
                MediaItem top = it;
                while (top.parentId != null) top = store.item(top.parentId).orElse(top);
                if (!found.contains(top)) found.add(top);
            });
        }
        found.sort(Comparator.comparingInt((MediaItem i) -> i.year).reversed());
        for (MediaItem it : found) items.add(media.view(it, ex.user(), false));
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", p.id());
        m.put("name", p.name());
        m.put("hasImage", p.image() != null);
        m.put("items", items);
        ex.ok(m);
    }

    private void personImage(Exchange ex) throws IOException {
        Metadata.Person p = metadata.person(parseInt(ex.param("pid")));
        if (p == null || p.image() == null) throw ApiException.notFound();
        FileSender.send(ex, Path.of(p.image()), IMAGE_CACHE_SECONDS);
    }

    private static int parseInt(String s) {
        try {
            return Integer.parseInt(s);
        } catch (NumberFormatException e) {
            throw ApiException.notFound();
        }
    }

    // ================================================================== collections

    private static String autoId(int tmdbCollection, String libraryId) {
        return "t" + tmdbCollection + "-" + libraryId;
    }

    /** Collections of a library: TMDB movie series with at least two movies here, and hand-made ones. */
    private List<Map<String, Object>> collectionsOf(Library l, User u) {
        Map<Integer, List<MediaItem>> auto = new LinkedHashMap<>();
        for (MediaItem it : store.topLevel(l.id)) {
            Metadata md = metadata.get(it.id);
            if (md != null && md.collectionId > 0) auto.computeIfAbsent(md.collectionId, k -> new ArrayList<>()).add(it);
        }
        List<Map<String, Object>> out = new ArrayList<>();
        for (Map.Entry<Integer, List<MediaItem>> e : auto.entrySet()) {
            if (e.getValue().size() < 2) continue;
            Metadata md = metadata.get(e.getValue().get(0).id);
            out.add(collectionView(autoId(e.getKey(), l.id), l.id, collectionName(md), e.getValue().size(), true));
        }
        for (ListStore.Collection c : lists.collectionsOf(l.id)) {
            out.add(collectionView(c.id, l.id, c.name, (int) c.itemIds.stream().filter(id -> store.item(u, id).isPresent()).count(), false));
        }
        out.sort(Comparator.comparing(m -> String.valueOf(m.get("title")).toLowerCase()));
        return out;
    }

    private static Map<String, Object> collectionView(String id, String libraryId, String name, int count, boolean auto) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", id);
        m.put("type", "collection");
        m.put("libraryId", libraryId);
        m.put("title", name);
        m.put("childCount", count);
        m.put("auto", auto);
        return m;
    }

    /** Resolved collection: name, summary, ordered items, images. */
    private record Resolved(String id, String libraryId, String name, String summary, boolean auto,
                            List<MediaItem> items, String poster, String backdrop) {
    }

    private Optional<Resolved> resolve(String cid) {
        if (cid.startsWith("t") && cid.contains("-")) {
            int tmdb;
            try {
                tmdb = Integer.parseInt(cid.substring(1, cid.indexOf('-')));
            } catch (NumberFormatException e) {
                return Optional.empty();
            }
            String libId = cid.substring(cid.indexOf('-') + 1);
            List<MediaItem> items = new ArrayList<>();
            Metadata first = null;
            for (MediaItem it : store.topLevel(libId)) {
                Metadata md = metadata.get(it.id);
                if (md != null && md.collectionId == tmdb) {
                    items.add(it);
                    if (first == null) first = md;
                }
            }
            if (items.isEmpty()) return Optional.empty();
            items.sort(Comparator.comparingInt((MediaItem i) -> i.year).thenComparing(i -> i.sortTitle));
            Map<String, Object> c = metadata.collection(tmdb);
            return Optional.of(new Resolved(cid, libId, collectionName(first), c == null ? "" : Json.str(c, "summary", ""), true,
                    items, c == null ? null : Json.str(c, "poster", null), c == null ? null : Json.str(c, "backdrop", null)));
        }
        return lists.collection(cid).map(c -> {
            List<MediaItem> items = new ArrayList<>();
            for (String id : c.itemIds) store.item(id).ifPresent(items::add);
            return new Resolved(c.id, c.libraryId, c.name, c.summary, false, items, null, null);
        });
    }

    private void collection(Exchange ex) throws IOException {
        Resolved r = visibleCollection(ex);
        Map<String, Object> m = collectionView(r.id, r.libraryId, r.name, r.items.size(), r.auto);
        m.put("summary", r.summary);
        List<Object> items = new ArrayList<>();
        int watched = 0;
        for (MediaItem it : r.items) {
            Map<String, Object> v = media.view(it, ex.user(), false);
            items.add(v);
            if (Json.bool(Json.obj(v.get("progress")), "watched", false)) watched++;
        }
        m.put("items", items);
        m.put("watchedCount", watched);
        ex.ok(m);
    }

    private void collectionImage(Exchange ex) throws IOException {
        Resolved r = visibleCollection(ex);
        String kind = ex.param("kind");
        String own = kind.equals("backdrop") ? r.backdrop : r.poster;
        if (own != null && Files.isRegularFile(Path.of(own))) {
            FileSender.send(ex, Path.of(own), IMAGE_CACHE_SECONDS);
            return;
        }
        // Fall back to the artwork of the first item.
        for (MediaItem it : r.items) {
            String p = kind.equals("backdrop") ? it.backdropPath : it.posterPath;
            if (p != null && Files.isRegularFile(Path.of(p))) {
                FileSender.send(ex, Path.of(p), IMAGE_CACHE_SECONDS);
                return;
            }
        }
        throw ApiException.notFound();
    }

    private ListStore.Collection ownCollection(Exchange ex) {
        return lists.collection(ex.param("cid")).orElseThrow(() ->
                new ApiException(404, "Sammlungen aus TMDB-Filmreihen lassen sich nicht bearbeiten"));
    }

    private void createCollection(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        Library l = store.library(Json.str(body, "libraryId", "")).orElseThrow(() -> ApiException.badRequest("Bibliothek fehlt"));
        String name = Json.str(body, "name", "").trim();
        if (name.isEmpty() || name.length() > 80) throw ApiException.badRequest("Bitte einen Namen angeben (max. 80 Zeichen)");
        ListStore.Collection c = lists.createCollection(l.id, name);
        String itemId = Json.str(body, "itemId", null);
        if (itemId != null) addItem(c, itemId);
        ex.json(201, Map.of("id", c.id, "name", c.name));
    }

    private void updateCollection(Exchange ex) throws IOException {
        ListStore.Collection c = ownCollection(ex);
        Map<String, Object> body = ex.body();
        if (body.containsKey("name")) {
            String n = Json.str(body, "name", "").trim();
            if (n.isEmpty() || n.length() > 80) throw ApiException.badRequest("Ungültiger Name");
            c.name = n;
        }
        if (body.containsKey("summary")) c.summary = Json.str(body, "summary", "").trim();
        lists.changedCollection(c);
        ex.ok(Map.of("id", c.id, "name", c.name));
    }

    private void addToCollection(Exchange ex) throws IOException {
        ListStore.Collection c = ownCollection(ex);
        addItem(c, Json.str(ex.body(), "itemId", ""));
        ex.noContent();
    }

    private void addItem(ListStore.Collection c, String itemId) {
        MediaItem it = store.item(itemId).orElseThrow(() -> ApiException.badRequest("Unbekannter Eintrag"));
        if (!it.libraryId.equals(c.libraryId) || it.parentId != null) {
            throw ApiException.badRequest("Nur Filme oder Serien aus derselben Bibliothek");
        }
        if (!c.itemIds.contains(it.id)) c.itemIds.add(it.id);
        lists.changedCollection(c);
    }

    // ================================================================== playlists

    private ListStore.Playlist ownPlaylist(Exchange ex) {
        return lists.playlist(ex.user().id, ex.param("pid")).orElseThrow(ApiException::notFound);
    }

    private Map<String, Object> playlistView(ListStore.Playlist p, User u, boolean withItems) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", p.id);
        m.put("type", "playlist");
        m.put("title", p.name);
        long duration = 0;
        List<Object> items = new ArrayList<>();
        String first = null;
        for (String id : p.itemIds) {
            MediaItem it = store.item(u, id).orElse(null); // items of libraries no longer shared are left out
            if (it == null) continue;
            if (first == null) first = it.id;
            if (it.file != null) duration += it.file.durationMs;
            if (withItems) items.add(media.view(it, u, false));
        }
        m.put("childCount", p.itemIds.size());
        m.put("durationMs", duration);
        m.put("firstItemId", first);
        m.put("updatedAt", p.updatedAt);
        if (withItems) m.put("items", items);
        return m;
    }

    private void playlists(Exchange ex) throws IOException {
        List<Object> out = new ArrayList<>();
        for (ListStore.Playlist p : lists.playlistsOf(ex.user().id)) out.add(playlistView(p, ex.user(), false));
        ex.ok(out);
    }

    private void playlist(Exchange ex) throws IOException {
        ex.ok(playlistView(ownPlaylist(ex), ex.user(), true));
    }

    private void createPlaylist(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        String name = Json.str(body, "name", "").trim();
        if (name.isEmpty() || name.length() > 80) throw ApiException.badRequest("Bitte einen Namen angeben (max. 80 Zeichen)");
        if (lists.playlistsOf(ex.user().id).size() >= 200) throw ApiException.badRequest("Zu viele Playlists");
        List<String> ids = new ArrayList<>();
        String itemId = Json.str(body, "itemId", null);
        if (itemId != null) ids.addAll(playableIds(store.item(ex.user(), itemId).orElseThrow(() -> ApiException.badRequest("Unbekannter Eintrag"))));
        ListStore.Playlist p = lists.createPlaylist(ex.user().id, name, ids);
        ex.json(201, playlistView(p, ex.user(), false));
    }

    private void updatePlaylist(Exchange ex) throws IOException {
        ListStore.Playlist p = ownPlaylist(ex);
        Map<String, Object> body = ex.body();
        if (body.containsKey("name")) {
            String n = Json.str(body, "name", "").trim();
            if (n.isEmpty() || n.length() > 80) throw ApiException.badRequest("Ungültiger Name");
            p.name = n;
        }
        if (body.get("itemIds") instanceof List<?> order) {
            // Reorder: only ids already in the playlist are accepted.
            List<String> next = new ArrayList<>();
            for (Object o : order) if (p.itemIds.contains(String.valueOf(o))) next.add(String.valueOf(o));
            if (next.size() == p.itemIds.size()) {
                p.itemIds.clear();
                p.itemIds.addAll(next);
            }
        }
        lists.changedPlaylist(p);
        ex.ok(playlistView(p, ex.user(), false));
    }

    private void addToPlaylist(Exchange ex) throws IOException {
        ListStore.Playlist p = ownPlaylist(ex);
        MediaItem it = store.item(ex.user(), Json.str(ex.body(), "itemId", "")).orElseThrow(() -> ApiException.badRequest("Unbekannter Eintrag"));
        List<String> ids = playableIds(it);
        if (p.itemIds.size() + ids.size() > 5000) throw ApiException.badRequest("Die Playlist ist zu lang");
        p.itemIds.addAll(ids);
        lists.changedPlaylist(p);
        ex.ok(Map.of("added", ids.size()));
    }

    /** A show or season adds all its episodes in order. */
    private List<String> playableIds(MediaItem it) {
        if (it.type.playable()) return List.of(it.id);
        List<String> out = new ArrayList<>();
        if (it.type == MediaItem.Type.SHOW) {
            for (MediaItem ep : media.episodesOf(it)) out.add(ep.id);
        } else {
            for (MediaItem ep : store.children(it.id)) out.add(ep.id);
        }
        return out;
    }

    private void removeFromPlaylist(Exchange ex) throws IOException {
        ListStore.Playlist p = ownPlaylist(ex);
        int idx = parseInt(ex.param("index"));
        if (idx < 0 || idx >= p.itemIds.size()) throw ApiException.notFound();
        p.itemIds.remove(idx);
        lists.changedPlaylist(p);
        ex.noContent();
    }

    // ================================================================== watchlist

    private List<Object> watchlistItems(User u) {
        List<Object> out = new ArrayList<>();
        for (String id : lists.watchlist(u.id)) store.item(u, id).ifPresent(it -> out.add(media.view(it, u, false)));
        return out;
    }

    private void setWatchlist(Exchange ex, boolean on) throws IOException {
        MediaItem it = item(ex, "itemId");
        if (it.type != MediaItem.Type.MOVIE && it.type != MediaItem.Type.SHOW) {
            throw ApiException.badRequest("Nur Filme und Serien");
        }
        lists.setWatchlist(ex.user().id, it.id, on);
        ex.ok(Map.of("onWatchlist", on));
    }

    // ================================================================== watch together

    private TogetherService.Room room(Exchange ex) {
        TogetherService.Room r = together.room(ex.param("code"));
        if (r == null) throw new ApiException(404, "Diesen Raum gibt es nicht (mehr)");
        return r;
    }

    private void createRoom(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        MediaItem it = store.item(ex.user(), Json.str(body, "itemId", "")).orElseThrow(() -> ApiException.badRequest("Unbekannter Eintrag"));
        if (!it.type.playable()) throw ApiException.badRequest("Nur Filme und Episoden");
        TogetherService.Room r = together.create(ex.user(), it.id, Json.num(body, "positionMs", 0));
        ex.json(201, r.toJson());
    }

    private void joinRoom(Exchange ex) throws IOException {
        TogetherService.Room r = room(ex);
        // Only who may see the title can join; the invite link alone is not enough.
        if (store.item(ex.user(), r.itemId()).isEmpty()) throw new ApiException(403, "Dieser Titel ist für dich nicht freigegeben");
        together.join(r, ex.user());
        ex.ok(r.toJson());
    }

    private void roomState(Exchange ex) throws IOException {
        TogetherService.Room r = room(ex);
        Map<String, Object> body = ex.body();
        String itemId = Json.str(body, "itemId", null);
        if (itemId != null && store.item(ex.user(), itemId).filter(i -> i.type.playable()).isEmpty()) {
            throw ApiException.badRequest("Unbekannter Eintrag");
        }
        try {
            long version = together.update(r, ex.user(), Json.str(body, "action", ""), Json.num(body, "positionMs", 0), itemId);
            ex.ok(Map.of("version", version));
        } catch (IllegalStateException e) {
            throw new ApiException(403, "Du bist nicht in diesem Raum");
        } catch (IllegalArgumentException e) {
            throw ApiException.badRequest(e.getMessage());
        }
    }

    /** Server-Sent Events stream of the room; stays open until the browser closes it. */
    private void roomEvents(Exchange ex) throws Exception {
        TogetherService.Room r = room(ex);
        if (store.item(ex.user(), r.itemId()).isEmpty()) throw new ApiException(403, "Dieser Titel ist für dich nicht freigegeben");
        var h = ex.raw.getResponseHeaders();
        h.set("Content-Type", "text/event-stream; charset=utf-8");
        h.set("Cache-Control", "no-cache");
        h.set("X-Accel-Buffering", "no");
        ex.markResponded();
        ex.raw.sendResponseHeaders(200, 0);
        TogetherService.Subscriber sub = together.subscribe(r, ex.user());
        try (OutputStream out = ex.raw.getResponseBody()) {
            out.write("retry: 3000\n\n".getBytes(StandardCharsets.UTF_8));
            out.flush();
            while (true) {
                String ev = sub.next(15_000);
                out.write((ev == null ? ": ping\n\n" : ev).getBytes(StandardCharsets.UTF_8));
                out.flush();
                if (together.room(r.code) == null) break;
            }
        } catch (IOException e) {
            // browser closed the stream
        } finally {
            together.unsubscribe(r, sub);
            ex.raw.close();
        }
    }
}

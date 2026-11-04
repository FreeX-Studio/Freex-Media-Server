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

import de.mirranet.freex.server.i18n.Text;
import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.Library;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.model.Progress;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.scan.LibraryScanner;
import de.mirranet.freex.server.scan.MediaProbe;
import de.mirranet.freex.server.scan.ThumbnailService;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.transcode.PlaybackPlanner;
import de.mirranet.freex.server.transcode.SubtitleService;
import de.mirranet.freex.server.transcode.TranscodeManager;

import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.TreeSet;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.function.Predicate;
import java.util.stream.Stream;

import static de.mirranet.freex.server.http.WebServer.Access.ADMIN;
import static de.mirranet.freex.server.http.WebServer.Access.PUBLIC;
import static de.mirranet.freex.server.http.WebServer.Access.USER;

/** All {@code /api} endpoints used by the web interface. */
public final class MediaApi {

    /** Played this far (share of the duration) counts as watched. */
    private static final double WATCHED_THRESHOLD = 0.90;
    /** Positions before this are not worth resuming. */
    private static final long MIN_RESUME_MS = 30_000;
    private static final int HUB_SIZE = 24;
    private static final int IMAGE_CACHE_SECONDS = 86_400;

    private final String serverName;
    private final String version;

    /**
     * Name and version of the bundled web client (FreeX Web), from {@code web/version.json}.
     * It has its own version: the server and the web client are separate products of FreeX.
     */
    private static final Map<String, Object> WEB_CLIENT = readWebClientVersion();

    /** Name and version of the bundled web client. */
    public static Map<String, Object> webClient() {
        return WEB_CLIENT;
    }

    private static Map<String, Object> readWebClientVersion() {
        try (InputStream in = MediaApi.class.getResourceAsStream("/web/version.json")) {
            if (in != null) {
                Map<String, Object> m = Json.parseObject(new String(in.readAllBytes(), StandardCharsets.UTF_8));
                return Map.of("name", Json.str(m, "name", "FreeX Web"), "version", Json.str(m, "version", "dev"));
            }
        } catch (IOException | RuntimeException e) {
            // fall through
        }
        return Map.of("name", "FreeX Web", "version", "dev");
    }
    private final DataStore store;
    private final LibraryScanner scanner;
    private final ThumbnailService thumbs;
    private final MediaProbe probe;
    private final boolean ffmpegAvailable;
    /** Null when transcoding is switched off or ffmpeg is missing. */
    private final TranscodeManager transcoder;
    private final SubtitleService subtitles;

    public MediaApi(String serverName, String version, DataStore store, LibraryScanner scanner,
                    ThumbnailService thumbs, MediaProbe probe, boolean ffmpegAvailable,
                    TranscodeManager transcoder, SubtitleService subtitles) {
        this.serverName = serverName;
        this.version = version;
        this.store = store;
        this.scanner = scanner;
        this.thumbs = thumbs;
        this.probe = probe;
        this.ffmpegAvailable = ffmpegAvailable;
        this.transcoder = transcoder;
        this.subtitles = subtitles;
    }

    public void register(WebServer web) {
        web.route("GET", "/api/status", PUBLIC, this::status);

        // browsing
        web.route("GET", "/api/libraries", USER, this::libraries);
        web.route("GET", "/api/libraries/{id}/items", USER, this::libraryItems);
        web.route("GET", "/api/libraries/{id}/hubs", USER, this::libraryHubs);
        web.route("GET", "/api/hubs", USER, this::hubs);
        web.route("GET", "/api/search", USER, this::search);
        web.route("GET", "/api/items/{id}", USER, this::item);
        web.route("GET", "/api/items/{id}/children", USER, this::children);
        web.route("GET", "/api/items/{id}/image/{kind}", USER, this::image);

        // playback
        web.route("POST", "/api/items/{id}/playback", USER, this::playback);
        web.route("GET", "/api/items/{id}/stream", USER, this::stream);
        web.route("GET", "/api/items/{id}/subtitles/{sid}", USER, this::subtitle);
        web.route("GET", "/api/transcode/{session}/{file}", USER, this::transcodeFile);
        web.route("DELETE", "/api/transcode/{session}", USER, this::stopTranscode);
        web.route("PUT", "/api/items/{id}/progress", USER, this::progress);
        web.route("POST", "/api/items/{id}/watched", USER, ex -> markWatched(ex, true));
        web.route("DELETE", "/api/items/{id}/watched", USER, ex -> markWatched(ex, false));

        // administration
        web.route("POST", "/api/libraries", ADMIN, this::createLibrary);
        web.route("PUT", "/api/libraries/{id}", ADMIN, this::updateLibrary);
        web.route("DELETE", "/api/libraries/{id}", ADMIN, this::deleteLibrary);
        web.route("POST", "/api/libraries/{id}/scan", ADMIN, this::scanLibrary);
        web.route("POST", "/api/scan", ADMIN, ex -> {
            scanner.scanAll();
            ex.json(202, Map.of("queued", true));
        });
        web.route("GET", "/api/fs", ADMIN, this::browseFolders);
    }

    // ================================================================== status

    private void status(Exchange ex) throws IOException {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", serverName);
        // Versions only for signed-in users: an anonymous visitor does not need to know what runs here.
        if (ex.user() != null) {
            m.put("version", version);
            m.put("server", Map.of("name", "FreeX Media Server", "version", version));
            m.put("web", WEB_CLIENT);
        }
        m.put("setupRequired", !store.hasUsers());
        m.put("loggedIn", ex.user() != null);
        if (ex.user() != null && ex.user().admin) {
            m.put("ffprobe", probe.available());
            m.put("ffmpeg", ffmpegAvailable);
            m.put("transcoder", transcoder != null);
            m.put("activeTranscodes", transcoder == null ? 0 : transcoder.activeSessions());
            m.put("tls", ex.secure());
            m.put("playing", activity == null ? 0 : activity.count());
        }
        ex.ok(m);
    }

    // ================================================================== libraries

    private void libraries(Exchange ex) throws IOException {
        List<Object> out = new ArrayList<>();
        for (Library l : store.librariesOf(ex.user())) {
            Map<String, Object> m = l.toPublic(store.countTopLevel(l.id));
            if (!ex.user().admin) m.remove("paths");
            out.add(m);
        }
        ex.ok(out);
    }

    private void createLibrary(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        String name = Json.str(body, "name", "").trim();
        if (name.isEmpty() || name.length() > 64) throw ApiException.badRequest("error.nameRequired", 64);
        Library.Type type;
        try {
            type = Library.Type.parse(Json.str(body, "type", ""));
        } catch (IllegalArgumentException e) {
            throw ApiException.badRequest("error.libraryType");
        }
        Library l = store.addLibrary(name, type, validPaths(body.get("paths")));
        scanner.scan(l);
        ex.json(201, l.toPublic(0));
    }

    private void updateLibrary(Exchange ex) throws IOException {
        Library l = store.library(ex.param("id")).orElseThrow(ApiException::notFound);
        Map<String, Object> body = ex.body();
        if (body.containsKey("name")) {
            String name = Json.str(body, "name", "").trim();
            if (name.isEmpty() || name.length() > 64) throw ApiException.badRequest("error.invalidName");
            l.name = name;
        }
        boolean rescan = false;
        if (body.containsKey("paths")) {
            l.paths = validPaths(body.get("paths"));
            rescan = true;
        }
        store.saveLibrary(l);
        if (rescan) scanner.scan(l);
        ex.ok(l.toPublic(store.countTopLevel(l.id)));
    }

    private static List<String> validPaths(Object raw) {
        List<String> paths = new ArrayList<>();
        for (Object o : Json.list(raw)) {
            String s = String.valueOf(o).trim();
            if (s.isEmpty()) continue;
            Path p = Path.of(s).toAbsolutePath().normalize();
            if (!Files.isDirectory(p)) throw ApiException.badRequest("error.folderNotFound", s);
            if (!paths.contains(p.toString())) paths.add(p.toString());
        }
        if (paths.isEmpty()) throw ApiException.badRequest("error.folderRequired");
        return paths;
    }

    private void deleteLibrary(Exchange ex) throws IOException {
        Library l = store.library(ex.param("id")).orElseThrow(ApiException::notFound);
        store.removeLibrary(l.id);
        ex.noContent();
    }

    private void scanLibrary(Exchange ex) throws IOException {
        Library l = store.library(ex.param("id")).orElseThrow(ApiException::notFound);
        scanner.scan(l);
        ex.json(202, Map.of("queued", true));
    }

    /** Lists sub folders, for the folder picker when adding a library. */
    private void browseFolders(Exchange ex) throws IOException {
        String requested = ex.query("path", "");
        List<Object> dirs = new ArrayList<>();
        Map<String, Object> out = new LinkedHashMap<>();
        if (requested.isEmpty()) {
            Set<String> seen = new HashSet<>();
            for (File root : File.listRoots()) {
                if (seen.add(root.getAbsolutePath())) dirs.add(Map.of("name", root.getAbsolutePath(), "path", root.getAbsolutePath()));
            }
            for (String common : new String[]{System.getProperty("user.home"), "/mnt", "/media", "/srv", "/data"}) {
                if (common != null && Files.isDirectory(Path.of(common)) && seen.add(common)) {
                    dirs.add(Map.of("name", common, "path", common));
                }
            }
            out.put("path", "");
            out.put("parent", null);
        } else {
            Path p = Path.of(requested).toAbsolutePath().normalize();
            if (!Files.isDirectory(p)) throw ApiException.notFound();
            try (Stream<Path> s = Files.list(p)) {
                s.filter(Files::isDirectory)
                        .filter(d -> !d.getFileName().toString().startsWith("."))
                        .sorted(Comparator.comparing(d -> d.getFileName().toString().toLowerCase(Locale.ROOT)))
                        .limit(2000)
                        .forEach(d -> dirs.add(Map.of("name", d.getFileName().toString(), "path", d.toString())));
            } catch (IOException e) {
                throw new ApiException(403, "error.folderUnreadable");
            }
            out.put("path", p.toString());
            out.put("parent", p.getParent() == null ? "" : p.getParent().toString());
        }
        out.put("folders", dirs);
        ex.ok(out);
    }

    // ================================================================== browsing

    private void libraryItems(Exchange ex) throws IOException {
        Library l = visibleLibrary(ex);
        User u = ex.user();
        String sort = ex.query("sort", "title");
        boolean desc = ex.query("order", sort.equals("title") ? "asc" : "desc").equals("desc");
        boolean unwatchedOnly = "1".equals(ex.query("unwatched"));
        String genre = ex.query("genre", "");

        List<MediaItem> items = store.topLevel(l.id);
        Comparator<MediaItem> cmp = switch (sort) {
            case "added" -> Comparator.comparingLong(i -> i.addedAt);
            case "year" -> Comparator.comparingInt((MediaItem i) -> i.year).thenComparing(i -> i.sortTitle);
            case "rating" -> Comparator.comparingDouble((MediaItem i) -> i.rating).thenComparing(i -> i.sortTitle);
            case "duration" -> Comparator.comparingLong(i -> i.file == null ? 0 : i.file.durationMs);
            default -> Comparator.comparing(i -> i.sortTitle);
        };
        items.sort(desc ? cmp.reversed() : cmp);

        Set<String> genres = new TreeSet<>();
        List<Object> out = new ArrayList<>();
        for (MediaItem it : items) {
            for (String g : it.genres.split(",")) if (!g.isBlank()) genres.add(g.trim());
            if (!genre.isEmpty() && !containsGenre(it, genre)) continue;
            Map<String, Object> m = decorate(it, u, false);
            if (unwatchedOnly && isFullyWatched(m)) continue;
            out.add(m);
        }
        Map<String, Object> res = new LinkedHashMap<>();
        res.put("library", l.toPublic(items.size()));
        if (!u.admin) Json.obj(res.get("library")).remove("paths");
        res.put("genres", genres);
        res.put("items", out);
        ex.ok(res);
    }

    private static boolean containsGenre(MediaItem it, String genre) {
        for (String g : it.genres.split(",")) if (g.trim().equalsIgnoreCase(genre)) return true;
        return false;
    }

    private static boolean isFullyWatched(Map<String, Object> m) {
        Object leaf = m.get("leafCount");
        if (leaf instanceof Number n) return n.intValue() > 0 && n.equals(m.get("viewedLeafCount"));
        return Json.bool(Json.obj(m.get("progress")), "watched", false);
    }

    private void item(Exchange ex) throws IOException {
        MediaItem it = store.item(ex.user(), ex.param("id")).orElseThrow(ApiException::notFound);
        Map<String, Object> m = decorate(it, ex.user(), true);
        if (it.type == MediaItem.Type.SHOW || it.type == MediaItem.Type.SEASON) {
            List<Object> kids = new ArrayList<>();
            for (MediaItem c : store.children(it.id)) kids.add(decorate(c, ex.user(), false));
            m.put("children", kids);
            if (it.type == MediaItem.Type.SHOW) {
                MediaItem next = nextEpisode(it, ex.user());
                if (next != null) m.put("onDeck", decorate(next, ex.user(), false));
            }
        }
        if (it.type.playable()) {
            MediaItem next = followingEpisode(it);
            if (next != null) m.put("next", decorate(next, ex.user(), false));
        }
        ex.ok(m);
    }

    private void children(Exchange ex) throws IOException {
        MediaItem it = store.item(ex.user(), ex.param("id")).orElseThrow(ApiException::notFound);
        List<Object> kids = new ArrayList<>();
        for (MediaItem c : store.children(it.id)) kids.add(decorate(c, ex.user(), false));
        ex.ok(kids);
    }

    /**
     * Item as JSON plus watch state, parent titles for episodes and episode counts for shows/seasons.
     */
    Map<String, Object> decorate(MediaItem it, User u, boolean full) {
        Map<String, Object> m = it.toPublic();
        m.put("artVersion", artVersion(it));
        if (!full && it.type != MediaItem.Type.MOVIE) m.remove("summary");
        if (it.type.playable()) {
            m.put("progress", store.progress(u.id, it.id).toJson());
        }
        if (it.type == MediaItem.Type.EPISODE) {
            store.item(it.parentId).ifPresent(season -> {
                m.put("seasonIndex", season.index);
                m.put("seasonId", season.id);
                store.item(season.parentId).ifPresent(show -> {
                    m.put("showTitle", show.title);
                    m.put("showId", show.id);
                });
            });
        }
        if (it.type == MediaItem.Type.SEASON) {
            store.item(it.parentId).ifPresent(show -> {
                m.put("showTitle", show.title);
                m.put("showId", show.id);
            });
        }
        if (it.type == MediaItem.Type.SHOW || it.type == MediaItem.Type.SEASON) {
            int[] counts = new int[2];
            countLeaves(it, u, counts);
            m.put("leafCount", counts[0]);
            m.put("viewedLeafCount", counts[1]);
            m.put("childCount", store.children(it.id).size());
        }
        for (Extension e : extensions) e.decorate(m, it, u, full);
        return m;
    }

    /** Adds data from other features (metadata, collections, markers) to item views and hubs. */
    public interface Extension {
        void decorate(Map<String, Object> view, MediaItem item, User user, boolean full);

        default List<Map<String, Object>> homeHubs(User user) {
            return List.of();
        }

        default List<Map<String, Object>> libraryHubs(Library library, User user) {
            return List.of();
        }
    }

    private final List<Extension> extensions = new CopyOnWriteArrayList<>();

    /** Running playbacks for the activity menu; set during wiring. */
    private volatile ActivityApi activity;

    public void setActivity(ActivityApi activity) {
        this.activity = activity;
    }

    public void addExtension(Extension e) {
        extensions.add(e);
    }

    /**
     * The JSON view of an item as this user sees it: watch state, show and season of episodes,
     * data of the extensions; {@code full} adds the description for shows, seasons and episodes.
     */
    public Map<String, Object> view(MediaItem it, User u, boolean full) {
        return decorate(it, u, full);
    }

    /** A row of the home screen; {@code title} is a {@link Text} or, for names from the library, a String. */
    static Map<String, Object> hubOf(String key, Object title, String libraryId, String style, List<Object> items) {
        return hub(key, title, libraryId, style, items);
    }

    private void countLeaves(MediaItem it, User u, int[] counts) {
        for (MediaItem c : store.children(it.id)) {
            if (c.type == MediaItem.Type.EPISODE) {
                counts[0]++;
                if (store.progress(u.id, c.id).watched()) counts[1]++;
            } else {
                countLeaves(c, u, counts);
            }
        }
    }

    /** All episodes of a show in viewing order. */
    List<MediaItem> episodesOf(MediaItem show) {
        List<MediaItem> eps = new ArrayList<>();
        List<MediaItem> seasons = new ArrayList<>(store.children(show.id));
        // Specials last, regular seasons in order.
        seasons.sort(Comparator.comparingInt(s -> s.index == 0 ? Integer.MAX_VALUE : s.index));
        for (MediaItem s : seasons) eps.addAll(store.children(s.id));
        return eps;
    }

    /** First unwatched episode after the last watched one ("On Deck"). */
    private MediaItem nextEpisode(MediaItem show, User u) {
        List<MediaItem> eps = episodesOf(show);
        int lastWatched = -1;
        for (int i = 0; i < eps.size(); i++) {
            if (store.progress(u.id, eps.get(i).id).watched()) lastWatched = i;
        }
        for (int i = lastWatched + 1; i < eps.size(); i++) {
            if (!store.progress(u.id, eps.get(i).id).watched()) return eps.get(i);
        }
        return null;
    }

    /** The episode after this one, for "play next". */
    MediaItem followingEpisode(MediaItem episode) {
        if (episode.type != MediaItem.Type.EPISODE) return null;
        MediaItem season = store.item(episode.parentId).orElse(null);
        if (season == null) return null;
        MediaItem show = store.item(season.parentId).orElse(null);
        if (show == null) return null;
        List<MediaItem> eps = episodesOf(show);
        for (int i = 0; i < eps.size() - 1; i++) {
            if (eps.get(i).id.equals(episode.id)) return eps.get(i + 1);
        }
        return null;
    }

    private void hubs(Exchange ex) throws IOException {
        User u = ex.user();
        List<Map<String, Object>> hubs = new ArrayList<>();

        // Continue watching: started but not finished, newest first.
        List<Map.Entry<String, Progress>> started = new ArrayList<>();
        for (Map.Entry<String, Progress> e : store.progressOf(u.id).entrySet()) {
            Progress p = e.getValue();
            if (!p.watched() && p.offsetMs() > 0 && store.item(u, e.getKey()).isPresent()) started.add(e);
        }
        started.sort(Comparator.comparingLong((Map.Entry<String, Progress> e) -> e.getValue().updatedAt()).reversed());
        List<Object> cont = new ArrayList<>();
        Set<String> showsInProgress = new HashSet<>();
        for (Map.Entry<String, Progress> e : started) {
            if (cont.size() >= HUB_SIZE) break;
            MediaItem it = store.item(e.getKey()).orElseThrow();
            Map<String, Object> m = decorate(it, u, false);
            if (m.get("showId") != null) showsInProgress.add((String) m.get("showId"));
            cont.add(m);
        }
        if (!cont.isEmpty()) hubs.add(hub("continue", new Text("hub.continue"), null, "landscape", cont));

        // Up next: shows with watch history, next episode, ordered by last activity.
        Map<String, Long> showActivity = new HashMap<>();
        for (Map.Entry<String, Progress> e : store.progressOf(u.id).entrySet()) {
            MediaItem it = store.item(u, e.getKey()).orElse(null);
            if (it == null || it.type != MediaItem.Type.EPISODE || !e.getValue().watched()) continue;
            MediaItem season = store.item(it.parentId).orElse(null);
            if (season == null || season.parentId == null) continue;
            showActivity.merge(season.parentId, e.getValue().updatedAt(), Math::max);
        }
        List<Map.Entry<String, Long>> shows = new ArrayList<>(showActivity.entrySet());
        shows.sort(Map.Entry.<String, Long>comparingByValue().reversed());
        List<Object> deck = new ArrayList<>();
        for (Map.Entry<String, Long> e : shows) {
            if (deck.size() >= HUB_SIZE) break;
            if (showsInProgress.contains(e.getKey())) continue;
            MediaItem show = store.item(e.getKey()).orElse(null);
            if (show == null) continue;
            MediaItem next = nextEpisode(show, u);
            if (next != null) deck.add(decorate(next, u, false));
        }
        if (!deck.isEmpty()) hubs.add(hub("ondeck", new Text("hub.upNext"), null, "landscape", deck));
        for (Extension e : extensions) hubs.addAll(e.homeHubs(u));

        // Recently added per library.
        for (Library l : store.librariesOf(u)) {
            List<MediaItem> top = store.topLevel(l.id);
            top.sort(Comparator.comparingLong((MediaItem i) -> i.addedAt).reversed());
            List<Object> recent = new ArrayList<>();
            for (MediaItem it : top) {
                if (recent.size() >= HUB_SIZE) break;
                recent.add(decorate(it, u, false));
            }
            if (!recent.isEmpty()) {
                hubs.add(hub("recent-" + l.id, new Text("hub.recentlyAddedIn", l.name), l.id, "poster", recent));
            }
        }
        ex.ok(hubs);
    }

    /** The "Recommended" tab of a library: rows built from the library's own content. */
    private void libraryHubs(Exchange ex) throws IOException {
        Library l = visibleLibrary(ex);
        User u = ex.user();
        List<Map<String, Object>> hubs = new ArrayList<>();
        List<MediaItem> top = store.topLevel(l.id);

        // Continue watching inside this library.
        List<Map.Entry<String, Progress>> started = new ArrayList<>();
        for (Map.Entry<String, Progress> e : store.progressOf(u.id).entrySet()) {
            Progress p = e.getValue();
            MediaItem it = store.item(e.getKey()).orElse(null);
            if (it != null && it.libraryId.equals(l.id) && !p.watched() && p.offsetMs() > 0) started.add(e);
        }
        started.sort(Comparator.comparingLong((Map.Entry<String, Progress> e) -> e.getValue().updatedAt()).reversed());
        List<Object> cont = new ArrayList<>();
        for (Map.Entry<String, Progress> e : started) {
            if (cont.size() >= HUB_SIZE) break;
            cont.add(decorate(store.item(e.getKey()).orElseThrow(), u, false));
        }
        if (!cont.isEmpty()) hubs.add(hub("continue", new Text("hub.continue"), l.id, "landscape", cont));

        if (l.type == Library.Type.SHOWS) {
            List<Object> deck = new ArrayList<>();
            for (MediaItem show : top) {
                if (deck.size() >= HUB_SIZE) break;
                int[] c = new int[2];
                countLeaves(show, u, c);
                if (c[1] == 0 || c[1] == c[0]) continue; // not started or finished
                MediaItem next = nextEpisode(show, u);
                if (next != null) deck.add(decorate(next, u, false));
            }
            if (!deck.isEmpty()) hubs.add(hub("ondeck", new Text("hub.upNext"), l.id, "landscape", deck));
        }

        addSortedHub(hubs, l, u, top, "recent", new Text("hub.recentlyAdded"), "added",
                Comparator.comparingLong((MediaItem i) -> i.addedAt).reversed(), i -> true);
        addSortedHub(hubs, l, u, top, "released", new Text("hub.recentlyReleased"), "year",
                Comparator.comparingInt((MediaItem i) -> i.year).reversed().thenComparing(i -> i.sortTitle), i -> i.year > 0);
        addSortedHub(hubs, l, u, top, "unwatched", new Text(l.type == Library.Type.SHOWS ? "hub.notStarted" : "hub.unwatched"),
                null, Comparator.comparingLong((MediaItem i) -> i.addedAt).reversed(), i -> !anyWatched(i, u));
        addSortedHub(hubs, l, u, top, "rated", new Text("hub.topRated"), "rating",
                Comparator.comparingDouble((MediaItem i) -> i.rating).reversed(), i -> i.rating > 0);

        for (Extension e : extensions) hubs.addAll(e.libraryHubs(l, u));

        // The three most common genres get their own row.
        Map<String, Integer> genreCount = new HashMap<>();
        for (MediaItem it : top) {
            for (String g : it.genres.split(",")) if (!g.isBlank()) genreCount.merge(g.trim(), 1, Integer::sum);
        }
        genreCount.entrySet().stream()
                .filter(e -> e.getValue() >= 3)
                .sorted(Map.Entry.<String, Integer>comparingByValue().reversed())
                .limit(3)
                .forEach(e -> addSortedHub(hubs, l, u, top, "genre-" + e.getKey(), e.getKey(), null,
                        Comparator.comparing((MediaItem i) -> i.sortTitle), i -> containsGenre(i, e.getKey())));
        ex.ok(hubs);
    }

    private void addSortedHub(List<Map<String, Object>> hubs, Library l, User u, List<MediaItem> source, String key,
                              Object title, String sort, Comparator<MediaItem> order,
                              Predicate<MediaItem> filter) {
        List<MediaItem> list = new ArrayList<>(source.stream().filter(filter).toList());
        if (list.size() < 2) return;
        list.sort(order);
        List<Object> out = new ArrayList<>();
        for (MediaItem it : list) {
            if (out.size() >= HUB_SIZE) break;
            out.add(decorate(it, u, false));
        }
        Map<String, Object> h = hub(key, title, l.id, "poster", out);
        h.put("sort", sort);
        hubs.add(h);
    }

    private boolean anyWatched(MediaItem it, User u) {
        if (it.type.playable()) {
            Progress p = store.progress(u.id, it.id);
            return p.watched() || p.offsetMs() > 0;
        }
        int[] c = new int[2];
        countLeaves(it, u, c);
        return c[1] > 0;
    }

    private static Map<String, Object> hub(String key, Object title, String libraryId, String style, List<Object> items) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("key", key);
        m.put("title", title);
        m.put("libraryId", libraryId);
        m.put("style", style);
        m.put("items", items);
        return m;
    }

    private void search(Exchange ex) throws IOException {
        String q = ex.query("q", "").trim().toLowerCase(Locale.ROOT);
        Map<String, Object> out = new LinkedHashMap<>();
        List<Object> movies = new ArrayList<>();
        List<Object> shows = new ArrayList<>();
        List<Object> episodes = new ArrayList<>();
        if (q.length() >= 2) {
            List<MediaItem> all = new ArrayList<>(store.allItems());
            all.sort(Comparator.comparing(i -> i.sortTitle));
            for (MediaItem it : all) {
                if (!store.canSee(ex.user(), it)) continue;
                boolean hit = it.title.toLowerCase(Locale.ROOT).contains(q)
                        || it.originalTitle.toLowerCase(Locale.ROOT).contains(q);
                if (!hit) continue;
                switch (it.type) {
                    case MOVIE -> {
                        if (movies.size() < 40) movies.add(decorate(it, ex.user(), false));
                    }
                    case SHOW -> {
                        if (shows.size() < 40) shows.add(decorate(it, ex.user(), false));
                    }
                    case EPISODE -> {
                        if (episodes.size() < 40) episodes.add(decorate(it, ex.user(), false));
                    }
                    default -> {
                    }
                }
            }
        }
        out.put("movies", movies);
        out.put("shows", shows);
        out.put("episodes", episodes);
        ex.ok(out);
    }

    /**
     * Artwork. {@code poster} is portrait, {@code backdrop} the large background, {@code thumb}
     * a landscape still (episodes, continue watching). Missing art falls back to the parent's art
     * or a frame from the video.
     */
    private void image(Exchange ex) throws IOException {
        MediaItem it = store.item(ex.user(), ex.param("id")).orElseThrow(ApiException::notFound);
        Path p = imagePath(it, ex.param("kind"));
        if (p == null) throw ApiException.notFound();
        FileSender.send(ex, p, IMAGE_CACHE_SECONDS);
    }

    /** Artwork file of an item: {@code poster}, {@code backdrop} or {@code thumb}; null if there is none. */
    Path imagePath(MediaItem it, String kind) {
        return switch (kind) {
            case "poster" -> poster(it);
            case "backdrop" -> backdrop(it);
            case "thumb" -> thumb(it);
            default -> throw ApiException.notFound();
        };
    }

    private Path poster(MediaItem it) {
        Path own = existing(it.posterPath);
        if (own != null && it.type != MediaItem.Type.EPISODE) return own;
        return switch (it.type) {
            case SEASON -> store.item(it.parentId).map(this::poster).orElse(null);
            case EPISODE -> store.item(it.parentId).map(this::poster).orElse(null);
            case MOVIE -> thumbs.frame(it);
            case SHOW -> {
                List<MediaItem> eps = episodesOf(it);
                yield eps.isEmpty() ? null : thumb(eps.get(0));
            }
        };
    }

    private Path backdrop(MediaItem it) {
        Path own = existing(it.backdropPath);
        if (own != null) return own;
        return switch (it.type) {
            case SEASON, EPISODE -> store.item(it.parentId).map(this::backdrop).orElse(null);
            case MOVIE -> thumbs.frame(it);
            case SHOW -> {
                List<MediaItem> eps = episodesOf(it);
                yield eps.isEmpty() ? null : thumb(eps.get(0));
            }
        };
    }

    private Path thumb(MediaItem it) {
        return switch (it.type) {
            case EPISODE -> {
                Path own = existing(it.posterPath);
                yield own != null ? own : thumbs.frame(it);
            }
            case MOVIE -> {
                Path b = existing(it.backdropPath);
                yield b != null ? b : thumbs.frame(it);
            }
            default -> backdrop(it);
        };
    }

    /**
     * Changes whenever the artwork of an item changes, for example when online metadata replaces a
     * frame from the video. The web client adds it to image URLs, so browsers do not keep showing
     * an old picture they have cached.
     */
    private String artVersion(MediaItem it) {
        int h = 17;
        MediaItem cur = it;
        for (int depth = 0; cur != null && depth < 3; depth++) {
            h = 31 * h + Objects.hash(cur.posterPath, cur.backdropPath);
            cur = cur.parentId == null ? null : store.item(cur.parentId).orElse(null);
        }
        return Integer.toUnsignedString(h, 36);
    }

    private static Path existing(String path) {
        if (path == null) return null;
        Path p = Path.of(path);
        return Files.isRegularFile(p) ? p : null;
    }

    // ================================================================== playback

    /** The library of the path parameter {id}, if shared with the user; 404 otherwise. */
    private Library visibleLibrary(Exchange ex) {
        Library l = store.library(ex.param("id")).orElseThrow(ApiException::notFound);
        if (!store.canSee(ex.user(), l)) throw ApiException.notFound();
        return l;
    }

    private MediaItem playable(Exchange ex) {
        MediaItem it = store.item(ex.user(), ex.param("id")).orElseThrow(ApiException::notFound);
        if (!it.type.playable() || it.file == null) throw ApiException.badRequest("error.notPlayable");
        return it;
    }

    private void stream(Exchange ex) throws IOException {
        MediaItem it = playable(ex);
        Path p = Path.of(it.file.path);
        if (!Files.isRegularFile(p)) throw new ApiException(404, "error.fileGone");
        if ("1".equals(ex.query("download"))) {
            String name = it.file.fileName().replaceAll("[\"\\\\\\r\\n]", "_");
            ex.raw.getResponseHeaders().set("Content-Disposition", "attachment; filename=\"" + name + "\"");
        }
        FileSender.send(ex, p, 0);
    }

    /** Subtitles are always delivered as WebVTT; the player renders them itself. */
    private void subtitle(Exchange ex) throws Exception {
        MediaItem it = playable(ex);
        String id = ex.param("sid").replace(".vtt", "");
        String text = subtitleText(it, id);
        if (text == null) throw ApiException.notFound();
        ex.raw.getResponseHeaders().set("Cache-Control", "private, max-age=3600");
        ex.send(200, "text/vtt; charset=utf-8", text.getBytes(StandardCharsets.UTF_8));
    }

    /** A subtitle as WebVTT, or null. */
    String subtitleText(MediaItem it, String id) throws Exception {
        return subtitles.vtt(it, id);
    }

    TranscodeManager transcoder() {
        return transcoder;
    }

    ActivityApi activity() {
        return activity;
    }

    // ================================================================== transcoding

    /**
     * Decides how to play an item for this browser and, if needed, starts ffmpeg.
     * Body: {@code {caps, quality, audio, subtitle, startMs}}.
     */
    private void playback(Exchange ex) throws Exception {
        MediaItem it = playable(ex);
        Map<String, Object> body = ex.body();
        PlaybackPlanner.Capabilities caps = PlaybackPlanner.Capabilities.parse(Json.obj(body.get("caps")));
        PlaybackPlanner.Quality quality = PlaybackPlanner.quality(Json.str(body, "quality", "original"));
        int audio = (int) Json.num(body, "audio", -1);
        String subtitle = Json.str(body, "subtitle", null);
        long start = Math.max(0, Json.num(body, "startMs", 0));
        if (it.file.durationMs > 0 && start >= it.file.durationMs) start = 0;

        PlaybackPlanner.Plan plan = PlaybackPlanner.plan(it.file, caps, quality, audio, subtitle, transcoder != null);
        String client = ActivityApi.client(Json.str(body, "client", null));
        String device = Json.str(body, "device", "");
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("mode", plan.mode());
        m.put("reason", plan.reason());
        m.put("audio", plan.audioIndex());
        m.put("burnedSubtitle", plan.burnSubtitle() >= 0 ? "e" + plan.burnSubtitle() : null);
        m.put("durationMs", it.file.durationMs);
        m.put("qualities", PlaybackPlanner.QUALITIES.stream().map(q -> Map.of("id", q.id(), "label", q.label())).toList());
        if (plan.mode() == PlaybackPlanner.Mode.DIRECT) {
            m.put("url", "/api/items/" + it.id + "/stream");
            m.put("offsetMs", 0);
            m.put("startMs", start);
            if (activity != null) activity.started(ex, it, client, device, "direct", plan.reason(), quality.label(), null, start);
        } else {
            try {
                TranscodeManager.Session s = transcoder.start(ex.user().id, it, plan, start);
                m.put("session", s.id);
                m.put("url", "/api/transcode/" + s.id + "/index.m3u8");
                m.put("offsetMs", s.offsetMs); // where the stream begins; the player seeks on to startMs
                m.put("startMs", start);
                if (activity != null) {
                    activity.started(ex, it, client, device, plan.mode().name().toLowerCase(Locale.ROOT),
                            plan.reason(), quality.label(), s.id, start);
                }
            } catch (TranscodeManager.BusyException e) {
                throw ApiException.of(503, e);
            }
        }
        ex.ok(m);
    }

    private TranscodeManager.Session ownSession(Exchange ex) {
        if (transcoder == null) throw ApiException.notFound();
        TranscodeManager.Session s = transcoder.session(ex.param("session"));
        if (s == null || !s.userId.equals(ex.user().id)) throw ApiException.notFound();
        return s;
    }

    private void transcodeFile(Exchange ex) throws Exception {
        sendTranscodeFile(ex, ownSession(ex), ex.param("file"));
    }

    /** Sends the playlist or a segment of a conversion, waiting until ffmpeg has written it. */
    void sendTranscodeFile(Exchange ex, TranscodeManager.Session s, String name) throws Exception {
        if (!name.matches("index\\.m3u8|init\\.mp4|seg\\d{1,8}\\.m4s")) throw ApiException.notFound();
        // Segments far ahead of ffmpeg can take a while; the playlist needs the first segment.
        Path p = transcoder.awaitFile(s, name, name.endsWith(".m4s") ? 60_000 : 30_000);
        if (p == null) {
            String log = transcoder.log(s);
            String last = log.isBlank() ? "" : log.lines().reduce((x, y) -> y).orElse("");
            throw new ApiException(name.equals("index.m3u8") ? 500 : 404, last.isEmpty() ? "error.transcodeFailed" : "error.transcodeFailedWith", last);
        }
        if (name.endsWith(".m3u8")) {
            ex.raw.getResponseHeaders().set("Cache-Control", "no-cache");
            ex.send(200, "application/vnd.apple.mpegurl", Files.readAllBytes(p));
        } else {
            ex.raw.getResponseHeaders().set("Cache-Control", "private, max-age=3600");
            ex.send(200, name.endsWith(".mp4") ? "video/mp4" : "video/iso.segment", Files.readAllBytes(p));
        }
    }

    private void stopTranscode(Exchange ex) throws IOException {
        TranscodeManager.Session s = ownSession(ex);
        transcoder.stop(s.id);
        ex.noContent();
    }

    private void progress(Exchange ex) throws IOException {
        MediaItem it = playable(ex);
        Map<String, Object> body = ex.body();
        long offset = Math.max(0, Json.num(body, "offsetMs", 0));
        long duration = it.file.durationMs > 0 ? it.file.durationMs : Json.num(body, "durationMs", 0);
        if (it.file.durationMs == 0 && duration > 0) {
            // ffprobe was not available: remember the length the browser reported.
            it.file.durationMs = duration;
            store.touchLibraryItems(it.libraryId);
        }
        User u = ex.user();
        if (activity != null) {
            activity.progress(u, it, ActivityApi.client(Json.str(body, "client", null)), offset,
                    Json.str(body, "state", "playing"), Json.bool(body, "stopped", false));
        }
        Progress old = store.progress(u.id, it.id);
        long now = System.currentTimeMillis();
        Progress p;
        if (duration > 0 && offset >= duration * WATCHED_THRESHOLD) {
            // Count the view once per playthrough, not on every report near the end.
            boolean newView = !old.watched() || old.offsetMs() > 0;
            p = new Progress(0, true, old.viewCount() + (newView ? 1 : 0), now);
            if (newView) recordHistory(u, it, now);
        } else if (offset < MIN_RESUME_MS) {
            p = new Progress(0, old.watched(), old.viewCount(), old.updatedAt());
        } else {
            p = new Progress(offset, old.watched(), old.viewCount(), now);
        }
        store.setProgress(u.id, it.id, p);
        ex.ok(p.toJson());
    }

    /** A finished playback goes into the watch history, with the title as it is shown now. */
    private void recordHistory(User u, MediaItem it, long now) {
        Map<String, Object> v = decorate(it, u, false);
        String title = String.valueOf(v.getOrDefault("title", it.title));
        String parent;
        if (it.type == MediaItem.Type.EPISODE) {
            Object season = v.get("seasonIndex");
            parent = v.getOrDefault("showTitle", "") + " · "
                    + (Integer.valueOf(0).equals(season) ? "Special" : "S" + season) + " E" + it.index;
        } else {
            parent = it.year > 0 ? Integer.toString(it.year) : "";
        }
        store.userStore().addHistory(u.id, it.id, now, title, parent);
    }

    private void markWatched(Exchange ex, boolean watched) throws IOException {
        MediaItem it = store.item(ex.user(), ex.param("id")).orElseThrow(ApiException::notFound);
        User u = ex.user();
        long now = System.currentTimeMillis();
        for (MediaItem leaf : leaves(it)) {
            Progress old = store.progress(u.id, leaf.id);
            store.setProgress(u.id, leaf.id, watched
                    ? new Progress(0, true, Math.max(1, old.viewCount()), now)
                    : new Progress(0, false, old.viewCount(), now));
        }
        ex.ok(decorate(it, u, false));
    }

    private List<MediaItem> leaves(MediaItem it) {
        if (it.type.playable()) return List.of(it);
        List<MediaItem> out = new ArrayList<>();
        for (MediaItem c : store.children(it.id)) out.addAll(leaves(c));
        return out;
    }
}

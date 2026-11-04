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

package de.mirranet.freex.server.metadata;

import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.Library;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.scan.NameParser;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.util.JsonFiles;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.text.Normalizer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Fetches metadata from TMDB for movies, shows, seasons and episodes, keeps it in
 * {@code <data>/metadata.json} and merges it into the scanned items after every scan.
 * <p>
 * Precedence: local files win. A {@code .nfo} file keeps its values and TMDB only fills what is
 * missing; local artwork is used before downloaded artwork.
 */
public final class MetadataService implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(MetadataService.class.getName());
    private static final long REFRESH_AFTER_MS = TimeUnit.DAYS.toMillis(30);
    private static final long RETRY_FAILED_AFTER_MS = TimeUnit.DAYS.toMillis(3);
    private static final int CAST_SIZE = 20;

    private final DataStore store;
    private final TmdbClient tmdb;
    private final String region;
    private final Path dir;
    private final Path file;
    private final Map<String, Metadata> byItem = new ConcurrentHashMap<>();
    /** TMDB collection id → {name, summary, poster, backdrop} */
    private final Map<Integer, Map<String, Object>> collections = new ConcurrentHashMap<>();
    private final ExecutorService worker = Executors.newSingleThreadExecutor(r -> daemon(r, "metadata"));
    private final ScheduledExecutorService saver = Executors.newSingleThreadScheduledExecutor(r -> daemon(r, "metadata-saver"));
    private final Set<String> queuedLibraries = ConcurrentHashMap.newKeySet();
    private volatile boolean dirty;
    private volatile String lastError = "";
    private volatile Consumer<Library> afterRefresh = l -> { };

    /** @param tmdb null when no API key is configured: then only stored metadata is applied */
    public MetadataService(DataStore store, TmdbClient tmdb, String region, Path dataDir) throws IOException {
        this.store = store;
        this.tmdb = tmdb;
        this.region = region.toUpperCase(Locale.ROOT);
        this.dir = dataDir.resolve("metadata");
        this.file = dataDir.resolve("metadata.json");
        Files.createDirectories(dir);
        load();
        saver.scheduleWithFixedDelay(this::saveQuietly, 10, 10, TimeUnit.SECONDS);
    }

    private static Thread daemon(Runnable r, String name) {
        Thread t = new Thread(r, name);
        t.setDaemon(true);
        return t;
    }

    public boolean enabled() {
        return tmdb != null;
    }

    public String lastError() {
        return lastError;
    }

    public void onRefreshed(Consumer<Library> callback) {
        this.afterRefresh = callback;
    }

    public Metadata get(String itemId) {
        return itemId == null ? null : byItem.get(itemId);
    }

    public Map<String, Object> collection(int tmdbCollectionId) {
        return collections.get(tmdbCollectionId);
    }

    // ================================================================== merging into items

    /** Copies stored metadata into the freshly scanned items of a library. Fast, no network. */
    public void applyAll(Library library) {
        boolean changed = false;
        for (MediaItem it : store.itemsOfLibrary(library.id)) {
            Metadata md = byItem.get(it.id);
            if (md != null && apply(it, md)) changed = true;
        }
        if (changed) store.touchLibraryItems(library.id);
    }

    /** Merges one item; returns true (the item was touched and its index should be saved). */
    static boolean apply(MediaItem it, Metadata md) {
        boolean fillOnly = it.hasNfo;
        if (!md.title.isBlank() && (!fillOnly || it.title.isBlank() || it.type == MediaItem.Type.EPISODE && it.title.startsWith("Episode "))) {
            it.title = md.title;
            it.sortTitle = it.type == MediaItem.Type.EPISODE ? String.format("%04d", it.index) + " " + NameParser.sortTitle(it.title)
                    : it.type == MediaItem.Type.SEASON ? it.sortTitle : NameParser.sortTitle(it.title);
        }
        if (!md.originalTitle.isBlank() && !md.originalTitle.equals(it.title) && (!fillOnly || it.originalTitle.isBlank())) {
            it.originalTitle = md.originalTitle;
        }
        if (!md.summary.isBlank() && (!fillOnly || it.summary.isBlank())) it.summary = md.summary;
        if (!md.tagline.isBlank() && (!fillOnly || it.tagline.isBlank())) it.tagline = md.tagline;
        if (md.year > 0 && (it.year == 0 || !fillOnly && it.type != MediaItem.Type.EPISODE)) it.year = md.year;
        if (!md.genres.isBlank() && (!fillOnly || it.genres.isBlank())) it.genres = md.genres;
        if (md.rating > 0 && (!fillOnly || it.rating == 0)) it.rating = md.rating;
        if (it.posterPath == null && md.poster != null && Files.exists(Path.of(md.poster))) it.posterPath = md.poster;
        if (it.backdropPath == null && md.backdrop != null && Files.exists(Path.of(md.backdrop))) it.backdropPath = md.backdrop;
        return true;
    }

    // ================================================================== refreshing

    /** Queues fetching metadata for the items of a library that have none (or old or forced). */
    public void refreshLibrary(Library library, boolean force) {
        if (tmdb == null || !queuedLibraries.add(library.id)) return;
        worker.submit(() -> {
            try {
                refreshNow(library, force);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            } catch (Exception e) {
                LOG.log(Level.WARNING, "Metadata refresh of '" + library.name + "' failed", e);
                lastError = e.getMessage();
            } finally {
                queuedLibraries.remove(library.id);
            }
        });
    }

    public boolean busy() {
        return !queuedLibraries.isEmpty();
    }

    private void refreshNow(Library library, boolean force) throws InterruptedException {
        long now = System.currentTimeMillis();
        int done = 0;
        for (MediaItem it : store.topLevel(library.id)) {
            Metadata md = byItem.get(it.id);
            boolean due = force || md == null
                    || (md.matched() && now - md.updatedAt > REFRESH_AFTER_MS)
                    || (!md.matched() && now - md.failedAt > RETRY_FAILED_AFTER_MS);
            if (!due) continue;
            try {
                if (it.type == MediaItem.Type.MOVIE) refreshMovie(it, md == null ? 0 : md.locked ? md.tmdbId : 0);
                else if (it.type == MediaItem.Type.SHOW) refreshShow(it, md == null ? 0 : md.locked ? md.tmdbId : 0);
                done++;
                lastError = "";
            } catch (IOException e) {
                lastError = e.getMessage();
                LOG.warning("Metadata for '" + it.title + "': " + e.getMessage());
                if (e.getMessage() != null && e.getMessage().contains("401")) break; // wrong key, stop here
            }
        }
        if (done > 0) {
            LOG.info("Metadata refreshed for " + done + " entries in '" + library.name + "'");
            applyAll(library);
            afterRefresh.accept(library);
        }
    }

    /** Re-fetches one item now (after "fix match" or "refresh metadata"). */
    public void refreshItem(MediaItem it) throws IOException, InterruptedException {
        if (tmdb == null) throw new IOException("Kein TMDB-Schlüssel eingetragen (tmdb.apiKey in config.yml)");
        MediaItem top = topOf(it);
        Metadata md = byItem.get(top.id);
        int id = md != null && md.locked ? md.tmdbId : 0;
        if (top.type == MediaItem.Type.MOVIE) refreshMovie(top, id);
        else refreshShow(top, id);
        store.library(top.libraryId).ifPresent(l -> {
            applyAll(l);
            afterRefresh.accept(l);
        });
    }

    /** Pins an item to a TMDB id and fetches it. */
    public void match(MediaItem it, int tmdbId) throws IOException, InterruptedException {
        MediaItem top = topOf(it);
        Metadata md = new Metadata();
        md.tmdbId = tmdbId;
        md.locked = true;
        byItem.put(top.id, md);
        dirty = true;
        refreshItem(top);
    }

    /** Removes online metadata of an item (and its seasons/episodes). */
    public void unmatch(MediaItem it) {
        MediaItem top = topOf(it);
        Metadata md = new Metadata();
        md.failedAt = Long.MAX_VALUE / 2; // never retried automatically
        md.locked = true;
        byItem.put(top.id, md);
        for (MediaItem c : descendants(top)) byItem.remove(c.id);
        dirty = true;
    }

    private MediaItem topOf(MediaItem it) {
        MediaItem cur = it;
        while (cur.parentId != null) {
            MediaItem p = store.item(cur.parentId).orElse(null);
            if (p == null) break;
            cur = p;
        }
        return cur;
    }

    private List<MediaItem> descendants(MediaItem it) {
        List<MediaItem> out = new ArrayList<>();
        for (MediaItem c : store.children(it.id)) {
            out.add(c);
            out.addAll(descendants(c));
        }
        return out;
    }

    /** Search for "fix match". {@code type} is movie or tv. */
    public List<Map<String, Object>> search(String type, String query, int year) throws IOException, InterruptedException {
        if (tmdb == null) throw new IOException("Kein TMDB-Schlüssel eingetragen (tmdb.apiKey in config.yml)");
        Map<String, String> p = new LinkedHashMap<>();
        p.put("query", query);
        p.put("language", tmdb.language());
        if (year > 0) p.put(type.equals("tv") ? "first_air_date_year" : "year", Integer.toString(year));
        Map<String, Object> res = tmdb.get("/search/" + type, p);
        List<Map<String, Object>> out = new ArrayList<>();
        if (res == null) return out;
        for (Object o : Json.list(res.get("results"))) {
            Map<String, Object> r = Json.obj(o);
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", Json.num(r, "id", 0));
            m.put("title", Json.str(r, type.equals("tv") ? "name" : "title", ""));
            m.put("originalTitle", Json.str(r, type.equals("tv") ? "original_name" : "original_title", ""));
            m.put("year", yearOf(Json.str(r, type.equals("tv") ? "first_air_date" : "release_date", "")));
            m.put("summary", Json.str(r, "overview", ""));
            m.put("poster", Json.str(r, "poster_path", null));
            out.add(m);
            if (out.size() >= 12) break;
        }
        return out;
    }

    /** Downloads a TMDB image for the search results into a small cache and returns its path. */
    public Path searchImage(String tmdbPath) throws IOException, InterruptedException {
        if (tmdb == null || tmdbPath == null || !tmdbPath.matches("/[A-Za-z0-9_.-]+\\.(jpg|png)")) return null;
        Path target = dir.resolve("search").resolve(tmdbPath.substring(1));
        return tmdb.download(tmdbPath, "w154", target) ? target : null;
    }

    // ------------------------------------------------------------------ movies

    private void refreshMovie(MediaItem it, int pinnedId) throws IOException, InterruptedException {
        int id = pinnedId > 0 ? pinnedId : it.tmdbHint > 0 ? it.tmdbHint : findId("movie", it.title, it.year, it.originalTitle);
        Metadata md = byItem.getOrDefault(it.id, new Metadata());
        if (id <= 0) {
            md.failedAt = System.currentTimeMillis();
            byItem.put(it.id, md);
            dirty = true;
            return;
        }
        Map<String, String> p = new LinkedHashMap<>();
        p.put("language", tmdb.language());
        p.put("append_to_response", "credits,videos,release_dates");
        p.put("include_video_language", lang2() + ",en,null");
        Map<String, Object> m = tmdb.get("/movie/" + id, p);
        if (m == null) {
            md.failedAt = System.currentTimeMillis();
            md.tmdbId = 0;
            byItem.put(it.id, md);
            dirty = true;
            return;
        }
        Metadata n = new Metadata();
        n.locked = pinnedId > 0;
        n.tmdbId = id;
        n.title = Json.str(m, "title", "");
        n.originalTitle = Json.str(m, "original_title", "");
        n.summary = Json.str(m, "overview", "");
        if (n.summary.isBlank()) n.summary = englishOverview("/movie/" + id);
        n.tagline = Json.str(m, "tagline", "");
        n.year = yearOf(Json.str(m, "release_date", ""));
        n.airDate = Json.str(m, "release_date", "");
        n.genres = names(m.get("genres"));
        n.rating = round1(m.get("vote_average"));
        List<Object> companies = Json.list(m.get("production_companies"));
        n.studio = companies.isEmpty() ? "" : Json.str(Json.obj(companies.get(0)), "name", "");
        n.contentRating = movieCertification(Json.obj(m.get("release_dates")));
        n.trailerKey = trailer(Json.obj(m.get("videos")));
        Map<String, Object> coll = Json.obj(m.get("belongs_to_collection"));
        if (!coll.isEmpty()) {
            n.collectionId = (int) Json.num(coll, "id", 0);
            n.collectionName = Json.str(coll, "name", "");
            fetchCollection(n.collectionId);
        }
        credits(Json.obj(m.get("credits")), n);
        Path itemDir = dir.resolve("items").resolve(it.id);
        n.poster = image(Json.str(m, "poster_path", null), "w500", itemDir.resolve("poster.jpg"));
        n.backdrop = image(Json.str(m, "backdrop_path", null), "w1280", itemDir.resolve("backdrop.jpg"));
        n.updatedAt = System.currentTimeMillis();
        byItem.put(it.id, n);
        dirty = true;
    }

    private void fetchCollection(int id) throws IOException, InterruptedException {
        if (id <= 0 || collections.containsKey(id)) return;
        Map<String, Object> c = tmdb.get("/collection/" + id);
        if (c == null) return;
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", Json.str(c, "name", ""));
        m.put("summary", Json.str(c, "overview", ""));
        Path cdir = dir.resolve("collections").resolve(Integer.toString(id));
        m.put("poster", image(Json.str(c, "poster_path", null), "w500", cdir.resolve("poster.jpg")));
        m.put("backdrop", image(Json.str(c, "backdrop_path", null), "w1280", cdir.resolve("backdrop.jpg")));
        collections.put(id, m);
        dirty = true;
    }

    private String movieCertification(Map<String, Object> releaseDates) {
        String fallback = "";
        for (Object o : Json.list(releaseDates.get("results"))) {
            Map<String, Object> r = Json.obj(o);
            String country = Json.str(r, "iso_3166_1", "");
            for (Object d : Json.list(r.get("release_dates"))) {
                String cert = Json.str(Json.obj(d), "certification", "").trim();
                if (cert.isEmpty()) continue;
                if (country.equals(region)) return certLabel(cert);
                if (country.equals("US") && fallback.isEmpty()) fallback = cert;
            }
        }
        return fallback;
    }

    private String certLabel(String cert) {
        return region.equals("DE") && cert.matches("\\d+") ? "FSK " + cert : cert;
    }

    // ------------------------------------------------------------------ shows

    private void refreshShow(MediaItem show, int pinnedId) throws IOException, InterruptedException {
        int id = pinnedId > 0 ? pinnedId : show.tmdbHint > 0 ? show.tmdbHint : findId("tv", show.title, show.year, show.originalTitle);
        Metadata md = byItem.getOrDefault(show.id, new Metadata());
        if (id <= 0) {
            md.failedAt = System.currentTimeMillis();
            byItem.put(show.id, md);
            dirty = true;
            return;
        }
        Map<String, String> p = new LinkedHashMap<>();
        p.put("language", tmdb.language());
        p.put("append_to_response", "credits,videos,content_ratings");
        p.put("include_video_language", lang2() + ",en,null");
        Map<String, Object> m = tmdb.get("/tv/" + id, p);
        if (m == null) {
            md.failedAt = System.currentTimeMillis();
            md.tmdbId = 0;
            byItem.put(show.id, md);
            dirty = true;
            return;
        }
        Metadata n = new Metadata();
        n.locked = pinnedId > 0;
        n.tmdbId = id;
        n.title = Json.str(m, "name", "");
        n.originalTitle = Json.str(m, "original_name", "");
        n.summary = Json.str(m, "overview", "");
        if (n.summary.isBlank()) n.summary = englishOverview("/tv/" + id);
        n.tagline = Json.str(m, "tagline", "");
        n.year = yearOf(Json.str(m, "first_air_date", ""));
        n.airDate = Json.str(m, "first_air_date", "");
        n.genres = names(m.get("genres"));
        n.rating = round1(m.get("vote_average"));
        List<Object> networks = Json.list(m.get("networks"));
        n.studio = networks.isEmpty() ? "" : Json.str(Json.obj(networks.get(0)), "name", "");
        for (Object o : Json.list(Json.obj(m.get("content_ratings")).get("results"))) {
            Map<String, Object> r = Json.obj(o);
            if (Json.str(r, "iso_3166_1", "").equals(region)) n.contentRating = certLabel(Json.str(r, "rating", ""));
        }
        n.trailerKey = trailer(Json.obj(m.get("videos")));
        credits(Json.obj(m.get("credits")), n);
        for (Object o : Json.list(m.get("created_by"))) n.directors.add(Json.str(Json.obj(o), "name", ""));
        Path showDir = dir.resolve("items").resolve(show.id);
        n.poster = image(Json.str(m, "poster_path", null), "w500", showDir.resolve("poster.jpg"));
        n.backdrop = image(Json.str(m, "backdrop_path", null), "w1280", showDir.resolve("backdrop.jpg"));
        n.updatedAt = System.currentTimeMillis();
        byItem.put(show.id, n);
        dirty = true;

        // Seasons and their episodes, only those that exist in the library.
        for (MediaItem season : store.children(show.id)) {
            Map<String, Object> s = tmdb.get("/tv/" + id + "/season/" + season.index);
            if (s == null) continue;
            Metadata sm = new Metadata();
            sm.tmdbId = (int) Json.num(s, "id", 0);
            sm.title = season.index == 0 ? "Specials" : Json.str(s, "name", "");
            if (sm.title.matches("(?i)(season|staffel) \\d+")) sm.title = "Staffel " + season.index;
            sm.summary = Json.str(s, "overview", "");
            sm.year = yearOf(Json.str(s, "air_date", ""));
            sm.airDate = Json.str(s, "air_date", "");
            sm.poster = image(Json.str(s, "poster_path", null), "w500", dir.resolve("items").resolve(season.id).resolve("poster.jpg"));
            sm.updatedAt = System.currentTimeMillis();
            byItem.put(season.id, sm);

            Map<Integer, Map<String, Object>> eps = new HashMap<>();
            for (Object o : Json.list(s.get("episodes"))) {
                Map<String, Object> e = Json.obj(o);
                eps.put((int) Json.num(e, "episode_number", -1), e);
            }
            for (MediaItem ep : store.children(season.id)) {
                Map<String, Object> e = eps.get(ep.index);
                if (e == null) continue;
                Metadata em = new Metadata();
                em.tmdbId = (int) Json.num(e, "id", 0);
                em.title = Json.str(e, "name", "");
                em.summary = Json.str(e, "overview", "");
                em.airDate = Json.str(e, "air_date", "");
                em.year = yearOf(em.airDate);
                em.rating = round1(e.get("vote_average"));
                for (Object c : Json.list(e.get("crew"))) {
                    Map<String, Object> cm = Json.obj(c);
                    if ("Director".equals(Json.str(cm, "job", ""))) em.directors.add(Json.str(cm, "name", ""));
                    if ("Writing".equals(Json.str(cm, "department", "")) && em.writers.size() < 3) {
                        em.writers.add(Json.str(cm, "name", ""));
                    }
                }
                for (Object c : Json.list(e.get("guest_stars"))) {
                    Map<String, Object> cm = Json.obj(c);
                    if (em.cast.size() >= 10) break;
                    em.cast.add(person(cm, Json.str(cm, "character", "")));
                }
                em.poster = image(Json.str(e, "still_path", null), "w500", dir.resolve("items").resolve(ep.id).resolve("still.jpg"));
                em.updatedAt = System.currentTimeMillis();
                byItem.put(ep.id, em);
            }
            dirty = true;
        }
    }

    // ------------------------------------------------------------------ helpers

    /** Best search result for title and year, or 0. */
    int findId(String type, String title, int year, String originalTitle) throws IOException, InterruptedException {
        List<Map<String, Object>> results = search(type, title, year);
        if (results.isEmpty() && year > 0) results = search(type, title, 0);
        if (results.isEmpty() && originalTitle != null && !originalTitle.isBlank()) results = search(type, originalTitle, year);
        int best = 0;
        double bestScore = Double.NEGATIVE_INFINITY;
        String wanted = normalize(title);
        for (int i = 0; i < results.size(); i++) {
            Map<String, Object> r = results.get(i);
            double score = -i * 0.3; // TMDB orders by relevance/popularity already
            String t = normalize(Json.str(r, "title", ""));
            String o = normalize(Json.str(r, "originalTitle", ""));
            if (t.equals(wanted) || o.equals(wanted)) score += 4;
            else if (t.contains(wanted) || wanted.contains(t) || o.contains(wanted)) score += 1.5;
            int y = (int) Json.num(r, "year", 0);
            if (year > 0 && y > 0) {
                if (y == year) score += 3;
                else if (Math.abs(y - year) == 1) score += 1.5;
                else score -= 2;
            }
            if (score > bestScore) {
                bestScore = score;
                best = (int) Json.num(r, "id", 0);
            }
        }
        // Without a reasonable title match it is safer to leave the item unmatched.
        return bestScore >= 1 ? best : 0;
    }

    static String normalize(String s) {
        String n = Normalizer.normalize(s.toLowerCase(Locale.ROOT), Normalizer.Form.NFD).replaceAll("\\p{M}", "");
        n = n.replace("ß", "ss").replace("&", "and");
        return n.replaceAll("[^a-z0-9]+", " ").replaceAll("^(the|a|an|der|die|das) ", "").trim();
    }

    private void credits(Map<String, Object> credits, Metadata n) throws IOException, InterruptedException {
        for (Object o : Json.list(credits.get("cast"))) {
            if (n.cast.size() >= CAST_SIZE) break;
            Map<String, Object> c = Json.obj(o);
            n.cast.add(person(c, Json.str(c, "character", "")));
        }
        for (Object o : Json.list(credits.get("crew"))) {
            Map<String, Object> c = Json.obj(o);
            String job = Json.str(c, "job", "");
            String name = Json.str(c, "name", "");
            if (job.equals("Director") && !n.directors.contains(name)) n.directors.add(name);
            if ((job.equals("Screenplay") || job.equals("Writer") || job.equals("Novel")) && n.writers.size() < 4
                    && !n.writers.contains(name)) n.writers.add(name);
        }
    }

    private Metadata.Person person(Map<String, Object> c, String role) throws IOException, InterruptedException {
        int pid = (int) Json.num(c, "id", 0);
        String img = image(Json.str(c, "profile_path", null), "w185", dir.resolve("people").resolve(pid + ".jpg"));
        return new Metadata.Person(pid, Json.str(c, "name", ""), role, img);
    }

    private String image(String tmdbPath, String size, Path target) throws InterruptedException {
        try {
            return tmdb.download(tmdbPath, size, target) ? target.toString() : null;
        } catch (IOException e) {
            return null;
        }
    }

    private String englishOverview(String path) throws IOException, InterruptedException {
        if (tmdb.language().startsWith("en")) return "";
        Map<String, Object> en = tmdb.get(path, Map.of("language", "en-US"));
        return en == null ? "" : Json.str(en, "overview", "");
    }

    private String trailer(Map<String, Object> videos) {
        String best = "";
        int bestScore = -1;
        for (Object o : Json.list(videos.get("results"))) {
            Map<String, Object> v = Json.obj(o);
            if (!"YouTube".equals(Json.str(v, "site", ""))) continue;
            String type = Json.str(v, "type", "");
            int score = type.equals("Trailer") ? 4 : type.equals("Teaser") ? 2 : 0;
            if (Json.str(v, "iso_639_1", "").equals(lang2())) score += 3;
            if (Json.bool(v, "official", false)) score += 1;
            if (score > bestScore && Json.str(v, "key", "").matches("[A-Za-z0-9_-]{6,20}")) {
                bestScore = score;
                best = Json.str(v, "key", "");
            }
        }
        return best;
    }

    private String lang2() {
        return tmdb.language().length() >= 2 ? tmdb.language().substring(0, 2) : "en";
    }

    private static String names(Object list) {
        List<String> out = new ArrayList<>();
        for (Object o : Json.list(list)) out.add(Json.str(Json.obj(o), "name", ""));
        return String.join(", ", out);
    }

    private static double round1(Object v) {
        return v instanceof Number n ? Math.round(n.doubleValue() * 10) / 10.0 : 0;
    }

    static int yearOf(String date) {
        return date != null && date.length() >= 4 && date.substring(0, 4).matches("\\d{4}") ? Integer.parseInt(date.substring(0, 4)) : 0;
    }

    /** Items whose cast contains the person, for the person page. */
    public List<String> itemsWithPerson(int personId) {
        List<String> out = new ArrayList<>();
        for (Map.Entry<String, Metadata> e : byItem.entrySet()) {
            for (Metadata.Person p : e.getValue().cast) {
                if (p.id() == personId) {
                    out.add(e.getKey());
                    break;
                }
            }
        }
        return out;
    }

    /** Name and image of a person, from any item that lists them. */
    public Metadata.Person person(int personId) {
        for (Metadata md : byItem.values()) {
            for (Metadata.Person p : md.cast) if (p.id() == personId) return p;
        }
        return null;
    }

    // ------------------------------------------------------------------ persistence

    private void load() throws IOException {
        Map<String, Object> root = Json.obj(JsonFiles.read(file, "{}"));
        for (Map.Entry<String, Object> e : Json.obj(root.get("items")).entrySet()) {
            byItem.put(e.getKey(), Metadata.fromStorage(Json.obj(e.getValue())));
        }
        for (Map.Entry<String, Object> e : Json.obj(root.get("collections")).entrySet()) {
            try {
                collections.put(Integer.parseInt(e.getKey()), Json.obj(e.getValue()));
            } catch (NumberFormatException ignored) {
                // skip
            }
        }
    }

    private void saveQuietly() {
        if (!dirty) return;
        dirty = false;
        try {
            Map<String, Object> items = new LinkedHashMap<>();
            for (Map.Entry<String, Metadata> e : byItem.entrySet()) items.put(e.getKey(), e.getValue().toStorage());
            Map<String, Object> colls = new LinkedHashMap<>();
            for (Map.Entry<Integer, Map<String, Object>> e : collections.entrySet()) colls.put(e.getKey().toString(), e.getValue());
            JsonFiles.write(file, Map.of("items", items, "collections", colls));
        } catch (IOException e) {
            dirty = true;
            LOG.log(Level.WARNING, "Saving metadata failed", e);
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
        saver.shutdown();
        saveQuietly();
    }
}

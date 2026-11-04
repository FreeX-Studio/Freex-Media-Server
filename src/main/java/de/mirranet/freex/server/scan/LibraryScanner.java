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

package de.mirranet.freex.server.scan;

import de.mirranet.freex.server.model.Library;
import de.mirranet.freex.server.model.MediaFile;
import de.mirranet.freex.server.model.MediaItem;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.util.Ids;

import java.io.IOException;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Scans the folders of a library and rebuilds its index.
 * <p>
 * Expected layouts (the ones common to the well-known media servers):
 * <pre>
 * Movies/Title (Year)/Title (Year).mkv     or   Movies/Title (Year).mkv
 * Shows/Show (Year)/Season 01/Show - S01E01 - Episode.mkv
 * </pre>
 * Local artwork ({@code poster.jpg}, {@code fanart.jpg}, {@code <name>-poster.jpg}, ...), Kodi
 * {@code .nfo} files and subtitle files next to the video ({@code Title.de.srt}) are picked up.
 * Files whose size and modification time did not change keep their probe results, so a rescan
 * only runs ffprobe for new or changed files.
 */
public final class LibraryScanner implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(LibraryScanner.class.getName());

    static final Set<String> VIDEO_EXTENSIONS = Set.of("mkv", "mp4", "m4v", "webm", "avi", "mov", "ts", "m2ts",
            "mts", "wmv", "mpg", "mpeg", "flv", "ogv", "3gp");
    private static final Set<String> IMAGE_EXTENSIONS = Set.of("jpg", "jpeg", "png", "webp");
    private static final Set<String> SUBTITLE_EXTENSIONS = Set.of("srt", "vtt");
    /** Folders that hold bonus material, not main content. */
    private static final Set<String> SKIP_FOLDERS = Set.of("extras", "featurettes", "trailers", "behind the scenes",
            "deleted scenes", "interviews", "scenes", "shorts", "samples", "sample", "other", "@eadir", "#recycle",
            "$recycle.bin", "lost+found", ".actors", "metadata");
    private static final long SAMPLE_MAX_BYTES = 300L * 1024 * 1024;

    private final DataStore store;
    private final MediaProbe probe;
    private final int threads;
    private final ScheduledExecutorService scheduler;
    private final ExecutorService scanExecutor;
    private final List<Consumer<Library>> listeners = new CopyOnWriteArrayList<>();

    public LibraryScanner(DataStore store, MediaProbe probe, int threads, int intervalMinutes) {
        this.store = store;
        this.probe = probe;
        this.threads = threads;
        this.scanExecutor = Executors.newSingleThreadExecutor(r -> daemon(r, "library-scanner"));
        this.scheduler = Executors.newSingleThreadScheduledExecutor(r -> daemon(r, "library-scan-timer"));
        if (intervalMinutes > 0) {
            scheduler.scheduleWithFixedDelay(this::scanAll, intervalMinutes, intervalMinutes, TimeUnit.MINUTES);
        }
    }

    private static Thread daemon(Runnable r, String name) {
        Thread t = new Thread(r, name);
        t.setDaemon(true);
        return t;
    }

    /** Called after every finished scan of a library (metadata, intro detection, ...). */
    public void onScanned(Consumer<Library> listener) {
        listeners.add(listener);
    }

    /** Queues a scan of every library. */
    public void scanAll() {
        for (Library l : store.libraries()) scan(l);
    }

    /** Queues a scan of one library; does nothing if one is already queued or running. */
    public synchronized void scan(Library library) {
        if (library.scanning) return;
        library.scanning = true;
        scanExecutor.submit(() -> {
            try {
                scanNow(library);
            } catch (Exception e) {
                LOG.log(Level.SEVERE, "Scan of library '" + library.name + "' failed", e);
            } finally {
                library.scanning = false;
            }
        });
    }

    /** Scans synchronously in the calling thread. */
    void scanNow(Library library) throws InterruptedException {
        long start = System.currentTimeMillis();
        Sidecars.clearCache();
        Map<String, MediaItem> previous = new HashMap<>();
        for (MediaItem it : store.itemsOfLibrary(library.id)) previous.put(it.id, it);
        boolean firstScan = library.lastScan == 0;

        Map<String, MediaItem> result = new LinkedHashMap<>();
        List<Runnable> probes = new ArrayList<>();
        for (String p : library.paths) {
            Path root = Path.of(p);
            if (!Files.isDirectory(root)) {
                LOG.warning("Library '" + library.name + "': folder not found: " + root);
                // A missing folder (unmounted disk) must not wipe the index: keep its old items.
                for (MediaItem it : previous.values()) {
                    if (belongsTo(it, root, previous)) result.put(it.id, it);
                }
                continue;
            }
            List<Path> videos = findVideos(root);
            for (Path video : videos) {
                try {
                    if (library.type == Library.Type.MOVIES) {
                        addMovie(library, root, video, previous, result, probes, firstScan);
                    } else {
                        addEpisode(library, root, video, previous, result, probes, firstScan);
                    }
                } catch (IOException | RuntimeException e) {
                    LOG.log(Level.WARNING, "Skipping " + video + ": " + e.getMessage(), e);
                }
            }
        }

        runProbes(probes);
        Sidecars.clearCache();
        finishShows(library, result);
        store.replaceLibraryItems(library.id, result.values());
        library.lastScan = System.currentTimeMillis();
        store.saveLibrary(library);
        for (var l : listeners) {
            try {
                l.accept(library);
            } catch (RuntimeException e) {
                LOG.log(Level.WARNING, "After-scan task failed for '" + library.name + "'", e);
            }
        }
        LOG.info("Scanned library '" + library.name + "': " + result.size() + " entries, " + probes.size()
                + " files probed, " + (System.currentTimeMillis() - start) + " ms");
    }

    private static boolean belongsTo(MediaItem it, Path root, Map<String, MediaItem> all) {
        String r = root.toAbsolutePath().toString();
        if (it.file != null) return it.file.path.startsWith(r);
        if (it.folder != null) return it.folder.startsWith(r);
        return false;
    }

    // ------------------------------------------------------------------ walking

    private static List<Path> findVideos(Path root) {
        List<Path> out = new ArrayList<>();
        try {
            Files.walkFileTree(root, new SimpleFileVisitor<>() {
                @Override
                public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) {
                    if (!dir.equals(root)) {
                        String name = dir.getFileName().toString();
                        if (name.startsWith(".") || SKIP_FOLDERS.contains(name.toLowerCase(Locale.ROOT))
                                || Files.exists(dir.resolve(".ignore"))) {
                            return FileVisitResult.SKIP_SUBTREE;
                        }
                    }
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) {
                    String name = file.getFileName().toString();
                    if (name.startsWith(".") || !attrs.isRegularFile()) return FileVisitResult.CONTINUE;
                    if (!VIDEO_EXTENSIONS.contains(NameParser.extension(name))) return FileVisitResult.CONTINUE;
                    String lower = name.toLowerCase(Locale.ROOT);
                    if (attrs.size() < SAMPLE_MAX_BYTES && (lower.contains("sample") || lower.endsWith("-trailer."
                            + NameParser.extension(name)))) {
                        return FileVisitResult.CONTINUE;
                    }
                    out.add(file);
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult visitFileFailed(Path file, IOException exc) {
                    LOG.fine("Cannot read " + file + ": " + exc.getMessage());
                    return FileVisitResult.CONTINUE;
                }
            });
        } catch (IOException e) {
            LOG.log(Level.WARNING, "Cannot walk " + root, e);
        }
        out.sort(null);
        return out;
    }

    // ------------------------------------------------------------------ movies

    private void addMovie(Library lib, Path root, Path video, Map<String, MediaItem> previous,
                          Map<String, MediaItem> result, List<Runnable> probes, boolean firstScan) throws IOException {
        String abs = video.toAbsolutePath().toString();
        String id = Ids.stable(lib.id, "movie", abs);
        MediaItem item = new MediaItem(id, lib.id, MediaItem.Type.MOVIE);
        MediaItem old = previous.get(id);
        Path dir = video.getParent();
        String fileName = video.getFileName().toString();
        String base = NameParser.stripExtension(fileName);

        NameParser.TitleYear ty = NameParser.titleYear(base);
        // Generic file names ("movie.mkv", "1080p.mkv") get their title from the folder.
        if (!dir.equals(root) && (ty.year() == 0 || ty.title().length() <= 2)) {
            NameParser.TitleYear folderTy = NameParser.titleYear(dir.getFileName().toString());
            if (folderTy.year() != 0 || ty.title().length() <= 2) ty = folderTy;
        }
        item.title = ty.title();
        item.year = ty.year();
        item.sortTitle = NameParser.sortTitle(item.title);
        item.folder = dir.toAbsolutePath().toString();

        Sidecars sc = Sidecars.of(dir);
        boolean ownFolder = !dir.equals(root) && sc.videoCount <= 2;
        Path nfo = sc.find(base + ".nfo");
        if (nfo == null && ownFolder) nfo = sc.find("movie.nfo");
        if (nfo != null) NfoReader.apply(nfo, item);
        if (item.tmdbHint == 0) {
            item.tmdbHint = NameParser.tmdbTag(fileName);
            if (item.tmdbHint == 0 && ownFolder) item.tmdbHint = NameParser.tmdbTag(dir.getFileName().toString());
        }

        item.posterPath = path(sc.image(base + "-poster", base + ".poster", base));
        if (item.posterPath == null && ownFolder) item.posterPath = path(sc.image("poster", "folder", "cover", "movie"));
        item.backdropPath = path(sc.image(base + "-fanart", base + ".fanart", base + "-backdrop"));
        if (item.backdropPath == null && ownFolder) item.backdropPath = path(sc.image("fanart", "backdrop", "background", "art"));

        item.file = fileInfo(video, sc, base, old, probes);
        item.addedAt = addedAt(old, video, firstScan);
        result.put(id, item);
    }

    // ------------------------------------------------------------------ shows

    private void addEpisode(Library lib, Path root, Path video, Map<String, MediaItem> previous,
                            Map<String, MediaItem> result, List<Runnable> probes, boolean firstScan) throws IOException {
        Path rel = root.relativize(video);
        String fileName = video.getFileName().toString();
        String base = NameParser.stripExtension(fileName);
        Path dir = video.getParent();

        // The show is the first folder below the library root, the season optionally the next one.
        Path showDir;
        int folderSeason = -1;
        if (rel.getNameCount() >= 2) {
            showDir = root.resolve(rel.getName(0));
            if (rel.getNameCount() >= 3) {
                folderSeason = NameParser.seasonFromFolder(rel.getName(1).toString());
            }
        } else {
            showDir = null;
        }
        NameParser.EpisodeInfo ep = NameParser.episode(fileName, folderSeason);

        String showKey;
        String showTitleRaw;
        if (showDir != null) {
            showKey = showDir.toAbsolutePath().toString();
            showTitleRaw = showDir.getFileName().toString();
        } else {
            // Episode directly in the root: the show name is the part before SxxEyy.
            showTitleRaw = ep.showPart().isEmpty() ? base : ep.showPart();
            showKey = root.toAbsolutePath() + "/" + NameParser.sortTitle(showTitleRaw);
        }
        int season = ep.season() >= 0 ? ep.season() : Math.max(folderSeason, 1);
        int episode = ep.episode();

        MediaItem show = result.get(Ids.stable(lib.id, "show", showKey));
        if (show == null) {
            show = new MediaItem(Ids.stable(lib.id, "show", showKey), lib.id, MediaItem.Type.SHOW);
            NameParser.TitleYear ty = NameParser.titleYear(showTitleRaw);
            show.title = ty.title();
            show.year = ty.year();
            show.sortTitle = NameParser.sortTitle(show.title);
            show.folder = showDir != null ? showDir.toAbsolutePath().toString() : null;
            if (showDir != null) {
                Sidecars ssc = Sidecars.of(showDir);
                Path nfo = ssc.find("tvshow.nfo");
                if (nfo != null) NfoReader.apply(nfo, show);
                if (show.tmdbHint == 0) show.tmdbHint = NameParser.tmdbTag(showTitleRaw);
                show.posterPath = path(ssc.image("poster", "folder", "cover", "show"));
                show.backdropPath = path(ssc.image("fanart", "backdrop", "background", "art"));
            }
            MediaItem oldShow = previous.get(show.id);
            show.addedAt = oldShow != null ? oldShow.addedAt : 0;
            result.put(show.id, show);
        }

        String seasonId = Ids.stable(lib.id, "season", showKey, Integer.toString(season));
        MediaItem seasonItem = result.get(seasonId);
        if (seasonItem == null) {
            seasonItem = new MediaItem(seasonId, lib.id, MediaItem.Type.SEASON);
            seasonItem.parentId = show.id;
            seasonItem.index = season;
            seasonItem.title = season == 0 ? "Specials" : "Staffel " + season;
            seasonItem.sortTitle = String.format("%04d", season);
            seasonItem.year = show.year;
            if (folderSeason >= 0 && !dir.equals(showDir)) {
                seasonItem.folder = dir.toAbsolutePath().toString();
                seasonItem.posterPath = path(Sidecars.of(dir).image("poster", "folder", "cover"));
            }
            if (seasonItem.posterPath == null && showDir != null) {
                String key = season == 0 ? "season-specials" : String.format("season%02d", season);
                seasonItem.posterPath = path(Sidecars.of(showDir).image(key + "-poster", key));
            }
            MediaItem oldSeason = previous.get(seasonId);
            seasonItem.addedAt = oldSeason != null ? oldSeason.addedAt : 0;
            result.put(seasonId, seasonItem);
        }

        String abs = video.toAbsolutePath().toString();
        String id = Ids.stable(lib.id, "episode", abs);
        MediaItem item = new MediaItem(id, lib.id, MediaItem.Type.EPISODE);
        MediaItem old = previous.get(id);
        item.parentId = seasonId;
        item.index = episode >= 0 ? episode : 0;
        String label = episode >= 0 ? "Episode " + episode + (ep.episodeEnd() > 0 ? "–" + ep.episodeEnd() : "") : base;
        item.title = ep.title().isEmpty() || ep.title().equalsIgnoreCase(show.title) ? label : ep.title();
        item.sortTitle = String.format("%04d", item.index) + " " + NameParser.sortTitle(item.title);
        item.year = show.year;

        Sidecars sc = Sidecars.of(dir);
        Path nfo = sc.find(base + ".nfo");
        if (nfo != null) {
            NfoReader.apply(nfo, item);
            item.sortTitle = String.format("%04d", item.index) + " " + NameParser.sortTitle(item.title);
        }
        item.posterPath = path(sc.image(base + "-thumb", base));
        item.file = fileInfo(video, sc, base, old, probes);
        item.addedAt = addedAt(old, video, firstScan);
        result.put(id, item);
    }

    /** Shows and seasons count as "added" when their newest episode was added. */
    private static void finishShows(Library lib, Map<String, MediaItem> result) {
        if (lib.type != Library.Type.SHOWS) return;
        Map<String, Long> newest = new HashMap<>();
        for (MediaItem it : result.values()) {
            if (it.type == MediaItem.Type.EPISODE) {
                newest.merge(it.parentId, it.addedAt, Math::max);
            }
        }
        for (MediaItem it : result.values()) {
            if (it.type == MediaItem.Type.SEASON) {
                it.addedAt = newest.getOrDefault(it.id, it.addedAt);
                newest.merge(it.parentId, it.addedAt, Math::max);
            }
        }
        for (MediaItem it : result.values()) {
            if (it.type == MediaItem.Type.SHOW) it.addedAt = newest.getOrDefault(it.id, it.addedAt);
        }
    }

    // ------------------------------------------------------------------ shared

    private MediaFile fileInfo(Path video, Sidecars sc, String base, MediaItem old, List<Runnable> probes)
            throws IOException {
        BasicFileAttributes attrs = Files.readAttributes(video, BasicFileAttributes.class);
        MediaFile f;
        if (old != null && old.file != null && old.file.size == attrs.size()
                && old.file.modified == attrs.lastModifiedTime().toMillis() && (old.file.probeCurrent() || !probe.available())) {
            f = old.file;
        } else {
            f = new MediaFile();
            f.path = video.toAbsolutePath().toString();
            f.size = attrs.size();
            f.modified = attrs.lastModifiedTime().toMillis();
            f.container = NameParser.extension(video.getFileName().toString());
            MediaFile target = f;
            probes.add(() -> probe.probe(video, target));
        }
        f.subtitles = sc.subtitles(base);
        return f;
    }

    private void runProbes(List<Runnable> probes) throws InterruptedException {
        if (probes.isEmpty() || !probe.available()) return;
        ExecutorService pool = Executors.newFixedThreadPool(threads, r -> daemon(r, "ffprobe"));
        try {
            List<Future<?>> futures = new ArrayList<>();
            for (Runnable r : probes) futures.add(pool.submit(r));
            for (Future<?> f : futures) {
                try {
                    f.get();
                } catch (ExecutionException e) {
                    LOG.log(Level.FINE, "Probe failed", e.getCause());
                }
            }
        } finally {
            pool.shutdownNow();
        }
    }

    /** Keeps the original "added" time; on the first scan the file date is used, so the order makes sense. */
    private static long addedAt(MediaItem old, Path video, boolean firstScan) throws IOException {
        if (old != null && old.addedAt > 0) return old.addedAt;
        long now = System.currentTimeMillis();
        if (firstScan) return Math.min(now, Files.getLastModifiedTime(video).toMillis());
        return now;
    }

    private static String path(Path p) {
        return p == null ? null : p.toAbsolutePath().toString();
    }

    /** Directory listing of one folder, used to find artwork, NFOs and subtitles next to a video. */
    private static final class Sidecars {
        private static final Map<Path, Sidecars> CACHE = new ConcurrentHashMap<>();

        final Map<String, Path> byLowerName = new HashMap<>();
        int videoCount;

        static Sidecars of(Path dir) {
            // Short-lived cache: a scan looks at the same folder many times (one per episode).
            if (CACHE.size() > 2000) CACHE.clear();
            return CACHE.computeIfAbsent(dir, Sidecars::read);
        }

        static void clearCache() {
            CACHE.clear();
        }

        private static Sidecars read(Path dir) {
            Sidecars s = new Sidecars();
            try (var stream = Files.list(dir)) {
                stream.forEach(p -> {
                    String n = p.getFileName().toString();
                    s.byLowerName.put(n.toLowerCase(Locale.ROOT), p);
                    if (VIDEO_EXTENSIONS.contains(NameParser.extension(n))) s.videoCount++;
                });
            } catch (IOException e) {
                LOG.fine("Cannot list " + dir + ": " + e.getMessage());
            }
            return s;
        }

        Path find(String name) {
            return byLowerName.get(name.toLowerCase(Locale.ROOT));
        }

        /** First existing image among the given base names (any supported image extension). */
        Path image(String... baseNames) {
            for (String b : baseNames) {
                for (String ext : IMAGE_EXTENSIONS) {
                    Path p = find(b + "." + ext);
                    if (p != null) return p;
                }
            }
            return null;
        }

        /** Subtitles named {@code <base>.srt}, {@code <base>.<lang>.srt}, {@code <base>.<lang>.forced.srt}. */
        List<MediaFile.Subtitle> subtitles(String base) {
            List<MediaFile.Subtitle> out = new ArrayList<>();
            String prefix = base.toLowerCase(Locale.ROOT);
            for (Map.Entry<String, Path> e : byLowerName.entrySet()) {
                String n = e.getKey();
                String ext = NameParser.extension(n);
                if (!SUBTITLE_EXTENSIONS.contains(ext) || !n.startsWith(prefix)) continue;
                String middle = n.substring(prefix.length(), n.length() - ext.length() - 1);
                if (!middle.isEmpty() && !middle.startsWith(".")) continue; // belongs to another video
                boolean forced = false;
                String lang = "";
                for (String part : middle.split("\\.")) {
                    if (part.isEmpty()) continue;
                    if (part.equals("forced")) forced = true;
                    else if (!part.equals("sdh") && !part.equals("cc") && !part.equals("default")) lang = part;
                }
                out.add(new MediaFile.Subtitle(e.getValue().toAbsolutePath().toString(), lang, ext, forced));
            }
            out.sort((a, b) -> a.language().compareTo(b.language()));
            return out;
        }
    }

    @Override
    public void close() {
        scheduler.shutdownNow();
        scanExecutor.shutdownNow();
        // Wait for running work, so nothing writes to the store after it is closed (matters for restarts).
        try {
            scanExecutor.awaitTermination(10, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }
}

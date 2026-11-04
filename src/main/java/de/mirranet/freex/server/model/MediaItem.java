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

package de.mirranet.freex.server.model;

import de.mirranet.freex.server.json.Json;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * One entry of a library. Movies and episodes have a {@link MediaFile}; shows and seasons are
 * containers. The hierarchy is show → season → episode, linked by {@link #parentId}.
 * <p>
 * Ids are derived from the library id and the relative path, so they stay the same across
 * rescans and watch state survives.
 */
public final class MediaItem {

    public enum Type {
        MOVIE, SHOW, SEASON, EPISODE;

        public static Type parse(String s) {
            return valueOf(s.trim().toUpperCase());
        }

        public boolean playable() {
            return this == MOVIE || this == EPISODE;
        }
    }

    public final String id;
    public final String libraryId;
    public final Type type;
    public String parentId;
    public String title = "";
    public String sortTitle = "";
    public String originalTitle = "";
    public int year;
    /** Season number for seasons, episode number for episodes. */
    public int index;
    public String summary = "";
    public String tagline = "";
    public String genres = "";
    public double rating;
    public long addedAt;
    /** Folder of a show or season, used for poster lookups. */
    public String folder;
    public String posterPath;
    public String backdropPath;
    public MediaFile file;
    /** True if a .nfo file supplied the metadata; online metadata then only fills gaps. */
    public boolean hasNfo;
    /** TMDB id from the .nfo or the file/folder name ({@code {tmdb-603}}), 0 if unknown. */
    public int tmdbHint;

    public MediaItem(String id, String libraryId, Type type) {
        this.id = id;
        this.libraryId = libraryId;
        this.type = type;
    }

    public Map<String, Object> toStorage() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", id);
        m.put("libraryId", libraryId);
        m.put("type", type);
        m.put("parentId", parentId);
        m.put("title", title);
        m.put("sortTitle", sortTitle);
        m.put("originalTitle", originalTitle);
        m.put("year", year);
        m.put("index", index);
        m.put("summary", summary);
        m.put("tagline", tagline);
        m.put("genres", genres);
        m.put("rating", rating);
        m.put("addedAt", addedAt);
        m.put("folder", folder);
        m.put("posterPath", posterPath);
        m.put("backdropPath", backdropPath);
        m.put("file", file == null ? null : file.toStorage());
        m.put("hasNfo", hasNfo);
        m.put("tmdbHint", tmdbHint);
        return m;
    }

    /** API view without file system paths. Watch state is added by the caller. */
    public Map<String, Object> toPublic() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", id);
        m.put("libraryId", libraryId);
        m.put("type", type);
        m.put("parentId", parentId);
        m.put("title", title);
        m.put("originalTitle", originalTitle);
        m.put("year", year);
        m.put("index", index);
        m.put("summary", summary);
        m.put("tagline", tagline);
        m.put("genres", genres);
        m.put("rating", rating);
        m.put("addedAt", addedAt);
        m.put("hasPoster", posterPath != null);
        m.put("hasBackdrop", backdropPath != null);
        if (file != null) m.put("media", file.toPublic());
        return m;
    }

    public static MediaItem fromStorage(Map<String, Object> m) {
        MediaItem it = new MediaItem(Json.str(m, "id", ""), Json.str(m, "libraryId", ""),
                Type.parse(Json.str(m, "type", "movie")));
        it.parentId = Json.str(m, "parentId", null);
        it.title = Json.str(m, "title", "");
        it.sortTitle = Json.str(m, "sortTitle", it.title);
        it.originalTitle = Json.str(m, "originalTitle", "");
        it.year = (int) Json.num(m, "year", 0);
        it.index = (int) Json.num(m, "index", 0);
        it.summary = Json.str(m, "summary", "");
        it.tagline = Json.str(m, "tagline", "");
        it.genres = Json.str(m, "genres", "");
        Object r = m.get("rating");
        it.rating = r instanceof Number n ? n.doubleValue() : 0;
        it.addedAt = Json.num(m, "addedAt", 0);
        it.folder = Json.str(m, "folder", null);
        it.posterPath = Json.str(m, "posterPath", null);
        it.backdropPath = Json.str(m, "backdropPath", null);
        Object f = m.get("file");
        it.file = f instanceof Map<?, ?> ? MediaFile.fromStorage(Json.obj(f)) : null;
        it.hasNfo = Json.bool(m, "hasNfo", false);
        it.tmdbHint = (int) Json.num(m, "tmdbHint", 0);
        return it;
    }
}

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

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Online metadata for one library item, kept separately from the scanned index so that rescans
 * never lose it. Images are downloaded and referenced by local path.
 */
public final class Metadata {

    /** One cast member. {@code image} is a local file path or null. */
    public record Person(int id, String name, String role, String image) {

        Map<String, Object> toStorage() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", id);
            m.put("name", name);
            m.put("role", role);
            m.put("image", image);
            return m;
        }

        static Person fromStorage(Map<String, Object> m) {
            return new Person((int) Json.num(m, "id", 0), Json.str(m, "name", ""), Json.str(m, "role", ""),
                    Json.str(m, "image", null));
        }
    }

    public int tmdbId;
    public String title = "";
    public String originalTitle = "";
    public String summary = "";
    public String tagline = "";
    public int year;
    public String genres = "";
    public double rating;
    public String contentRating = "";
    public String studio = "";
    public String airDate = "";
    public String trailerKey = "";
    public int collectionId;
    public String collectionName = "";
    public List<Person> cast = new ArrayList<>();
    public List<String> directors = new ArrayList<>();
    public List<String> writers = new ArrayList<>();
    public String poster;
    public String backdrop;
    public long updatedAt;
    /** Set by "fix match": the id stays even if a search would find something else. */
    public boolean locked;
    /** Last time no match was found; retried after a while. */
    public long failedAt;

    public boolean matched() {
        return tmdbId > 0;
    }

    public Map<String, Object> toStorage() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("tmdbId", tmdbId);
        m.put("title", title);
        m.put("originalTitle", originalTitle);
        m.put("summary", summary);
        m.put("tagline", tagline);
        m.put("year", year);
        m.put("genres", genres);
        m.put("rating", rating);
        m.put("contentRating", contentRating);
        m.put("studio", studio);
        m.put("airDate", airDate);
        m.put("trailerKey", trailerKey);
        m.put("collectionId", collectionId);
        m.put("collectionName", collectionName);
        List<Object> c = new ArrayList<>();
        for (Person p : cast) c.add(p.toStorage());
        m.put("cast", c);
        m.put("directors", directors);
        m.put("writers", writers);
        m.put("poster", poster);
        m.put("backdrop", backdrop);
        m.put("updatedAt", updatedAt);
        m.put("locked", locked);
        m.put("failedAt", failedAt);
        return m;
    }

    public static Metadata fromStorage(Map<String, Object> m) {
        Metadata md = new Metadata();
        md.tmdbId = (int) Json.num(m, "tmdbId", 0);
        md.title = Json.str(m, "title", "");
        md.originalTitle = Json.str(m, "originalTitle", "");
        md.summary = Json.str(m, "summary", "");
        md.tagline = Json.str(m, "tagline", "");
        md.year = (int) Json.num(m, "year", 0);
        md.genres = Json.str(m, "genres", "");
        Object r = m.get("rating");
        md.rating = r instanceof Number n ? n.doubleValue() : 0;
        md.contentRating = Json.str(m, "contentRating", "");
        md.studio = Json.str(m, "studio", "");
        md.airDate = Json.str(m, "airDate", "");
        md.trailerKey = Json.str(m, "trailerKey", "");
        md.collectionId = (int) Json.num(m, "collectionId", 0);
        md.collectionName = Json.str(m, "collectionName", "");
        for (Object o : Json.list(m.get("cast"))) md.cast.add(Person.fromStorage(Json.obj(o)));
        for (Object o : Json.list(m.get("directors"))) md.directors.add(String.valueOf(o));
        for (Object o : Json.list(m.get("writers"))) md.writers.add(String.valueOf(o));
        md.poster = Json.str(m, "poster", null);
        md.backdrop = Json.str(m, "backdrop", null);
        md.updatedAt = Json.num(m, "updatedAt", 0);
        md.locked = Json.bool(m, "locked", false);
        md.failedAt = Json.num(m, "failedAt", 0);
        return md;
    }
}

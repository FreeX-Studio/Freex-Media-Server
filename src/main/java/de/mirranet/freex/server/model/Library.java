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

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** A library: one or more folders scanned as either movies or TV shows. */
public final class Library {

    public enum Type {
        MOVIES, SHOWS;

        public static Type parse(String s) {
            return switch (s == null ? "" : s.trim().toLowerCase()) {
                case "movies", "movie", "filme" -> MOVIES;
                case "shows", "show", "series", "serien", "tv" -> SHOWS;
                default -> throw new IllegalArgumentException("Unknown library type: " + s);
            };
        }
    }

    public final String id;
    public volatile String name;
    public final Type type;
    public volatile List<String> paths;
    public final long createdAt;
    public volatile long lastScan;

    /** Runtime only: true while a scan of this library is running. */
    public volatile boolean scanning;

    public Library(String id, String name, Type type, List<String> paths, long createdAt, long lastScan) {
        this.id = id;
        this.name = name;
        this.type = type;
        this.paths = List.copyOf(paths);
        this.createdAt = createdAt;
        this.lastScan = lastScan;
    }

    public Map<String, Object> toStorage() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", id);
        m.put("name", name);
        m.put("type", type);
        m.put("paths", paths);
        m.put("createdAt", createdAt);
        m.put("lastScan", lastScan);
        return m;
    }

    public Map<String, Object> toPublic(int itemCount) {
        Map<String, Object> m = toStorage();
        m.put("scanning", scanning);
        m.put("itemCount", itemCount);
        return m;
    }

    public static Library fromStorage(Map<String, Object> m) {
        List<String> paths = new ArrayList<>();
        for (Object o : Json.list(m.get("paths"))) paths.add(String.valueOf(o));
        return new Library(Json.str(m, "id", ""), Json.str(m, "name", ""), Type.parse(Json.str(m, "type", "movies")),
                paths, Json.num(m, "createdAt", 0), Json.num(m, "lastScan", 0));
    }
}

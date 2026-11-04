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
 * Watch state of one user for one playable item.
 *
 * @param offsetMs  resume position; 0 when not started or finished
 * @param watched   true once the item was played to (almost) the end or marked as watched
 * @param viewCount how often it was finished
 * @param updatedAt last change, used for "continue watching" ordering
 */
public record Progress(long offsetMs, boolean watched, int viewCount, long updatedAt) {

    public static final Progress NONE = new Progress(0, false, 0, 0);

    public Map<String, Object> toJson() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("offsetMs", offsetMs);
        m.put("watched", watched);
        m.put("viewCount", viewCount);
        m.put("updatedAt", updatedAt);
        return m;
    }

    public static Progress fromJson(Map<String, Object> m) {
        return new Progress(Json.num(m, "offsetMs", 0), Json.bool(m, "watched", false),
                (int) Json.num(m, "viewCount", 0), Json.num(m, "updatedAt", 0));
    }
}

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
import de.mirranet.freex.server.model.Progress;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.util.JsonFiles;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.logging.Logger;

/**
 * One-time move of the user data from the JSON files of version 0.3 and earlier into the
 * database. Runs only while the database has no users; the old files are kept, renamed to
 * {@code *.json.imported}, so nothing is lost if something goes wrong.
 */
public final class JsonImport {

    private static final Logger LOG = Logger.getLogger(JsonImport.class.getName());
    private static final String[] FILES = {"users.json", "sessions.json", "logins.json", "progress.json",
            "collections.json", "playlists.json", "watchlists.json"};

    private JsonImport() {
    }

    public static void run(Path dataDir, Database db, UserStore users, ListStore lists) throws IOException {
        Path usersFile = dataDir.resolve("users.json");
        if (users.hasUsers() || !Files.exists(usersFile)) return;
        LOG.info("Importing user data from the JSON files of an older version into " + db.file().getFileName());

        db.transaction(d -> {
            try {
                importAll(dataDir, users, lists);
            } catch (IOException e) {
                throw new IllegalStateException("Import failed: " + e.getMessage(), e);
            }
        });
        for (String name : FILES) {
            Path p = dataDir.resolve(name);
            if (Files.exists(p)) Files.move(p, dataDir.resolve(name + ".imported"), StandardCopyOption.REPLACE_EXISTING);
        }
        LOG.info("Imported " + users.userCount() + " users; the old files were renamed to *.json.imported");
    }

    private static void importAll(Path dir, UserStore users, ListStore lists) throws IOException {
        for (Object o : Json.list(JsonFiles.read(dir.resolve("users.json"), "[]"))) {
            users.importUser(User.fromStorage(Json.obj(o)));
        }
        for (Map.Entry<String, Object> e : Json.obj(JsonFiles.read(dir.resolve("sessions.json"), "{}")).entrySet()) {
            Map<String, Object> m = Json.obj(e.getValue());
            users.importSession(e.getKey(), new UserStore.Session(Json.str(m, "userId", ""), Json.num(m, "createdAt", 0),
                    Json.num(m, "lastSeen", 0), Json.str(m, "ip", ""), Json.str(m, "userAgent", "")));
        }
        for (Map.Entry<String, Object> e : Json.obj(JsonFiles.read(dir.resolve("logins.json"), "{}")).entrySet()) {
            List<Object> events = new ArrayList<>(Json.list(e.getValue()));
            java.util.Collections.reverse(events); // stored newest first; insert oldest first
            for (Object o : events) {
                Map<String, Object> m = Json.obj(o);
                users.importLoginEvent(e.getKey(), new UserStore.LoginEvent(Json.num(m, "time", 0), Json.str(m, "ip", ""),
                        Json.str(m, "userAgent", ""), Json.bool(m, "success", false), Json.str(m, "detail", "")));
            }
        }
        for (Map.Entry<String, Object> e : Json.obj(JsonFiles.read(dir.resolve("progress.json"), "{}")).entrySet()) {
            for (Map.Entry<String, Object> pe : Json.obj(e.getValue()).entrySet()) {
                users.setProgress(e.getKey(), pe.getKey(), Progress.fromJson(Json.obj(pe.getValue())));
            }
        }
        for (Object o : Json.list(JsonFiles.read(dir.resolve("collections.json"), "[]"))) {
            Map<String, Object> m = Json.obj(o);
            lists.importCollection(Json.str(m, "id", ""), Json.str(m, "libraryId", ""), Json.str(m, "name", ""),
                    Json.str(m, "summary", ""), strings(m.get("itemIds")), Json.num(m, "createdAt", 0));
        }
        for (Object o : Json.list(JsonFiles.read(dir.resolve("playlists.json"), "[]"))) {
            Map<String, Object> m = Json.obj(o);
            String userId = Json.str(m, "userId", "");
            if (users.user(userId).isEmpty()) continue;
            lists.importPlaylist(Json.str(m, "id", ""), userId, Json.str(m, "name", ""), strings(m.get("itemIds")),
                    Json.num(m, "createdAt", 0), Json.num(m, "updatedAt", 0));
        }
        for (Map.Entry<String, Object> e : Json.obj(JsonFiles.read(dir.resolve("watchlists.json"), "{}")).entrySet()) {
            if (users.user(e.getKey()).isEmpty()) continue;
            lists.importWatchlist(e.getKey(), strings(e.getValue()));
        }
    }

    private static List<String> strings(Object o) {
        List<String> out = new ArrayList<>();
        for (Object v : Json.list(o)) out.add(String.valueOf(v));
        return out;
    }
}

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
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.store.UserStore;

import javax.imageio.ImageIO;
import javax.imageio.ImageReadParam;
import javax.imageio.ImageReader;
import javax.imageio.stream.ImageInputStream;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.image.BufferedImage;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.logging.Logger;

import static de.mirranet.freex.server.http.WebServer.Access.ADMIN;
import static de.mirranet.freex.server.http.WebServer.Access.USER;

/**
 * The user's own profile: profile picture, language preferences for playback and the watch
 * history.
 * <p>
 * Uploaded pictures are never stored as sent: they are decoded, cropped to a square, scaled to
 * {@value #AVATAR_SIZE} px and written as a fresh PNG. That drops metadata (EXIF, GPS) and
 * anything else hidden in the file, and the server only ever serves an image it made itself.
 */
public final class ProfileApi {

    private static final Logger LOG = Logger.getLogger(ProfileApi.class.getName());
    private static final int AVATAR_SIZE = 256;
    private static final int MAX_UPLOAD = 10 * 1024 * 1024;
    private static final long MAX_PIXELS = 50_000_000L;

    /** Languages offered for audio and subtitles (ISO 639-1); the web interface has the names. */
    public static final Set<String> LANGUAGES = Set.of("de", "en", "fr", "es", "it", "nl", "pt", "pl", "ru", "tr",
            "sv", "da", "no", "fi", "cs", "hu", "el", "ja", "ko", "zh", "ar", "hi");

    private final DataStore store;
    private final UserStore users;
    private final MediaApi media;
    private final Path avatarDir;

    public ProfileApi(DataStore store, MediaApi media, Path dataDirectory) throws IOException {
        this.store = store;
        this.users = store.userStore();
        this.media = media;
        this.avatarDir = dataDirectory.resolve("avatars");
        Files.createDirectories(avatarDir);
    }

    public void register(WebServer web) {
        web.route("PUT", "/api/me/preferences", USER, this::preferences);
        web.route("PUT", "/api/me/avatar", USER, this::uploadAvatar);
        web.route("DELETE", "/api/me/avatar", USER, this::deleteAvatar);
        web.route("GET", "/api/users/{id}/avatar", USER, this::avatar);
        web.route("GET", "/api/me/history", USER, ex -> ex.ok(history(ex, ex.user().id)));
        web.route("DELETE", "/api/me/history/{hid}", USER, this::removeHistory);
        web.route("DELETE", "/api/me/history", USER, this::clearHistory);
        web.route("GET", "/api/history", ADMIN, ex -> {
            String user = ex.query("user", "");
            ex.ok(history(ex, user.isEmpty() ? null : user));
        });
    }

    /** Called when an account is deleted. */
    public void removeUser(String userId) {
        try {
            Files.deleteIfExists(avatarFile(userId));
        } catch (IOException e) {
            LOG.fine("Cannot delete avatar of " + userId + ": " + e.getMessage());
        }
    }

    private Path avatarFile(String userId) {
        if (!userId.matches("[0-9a-f]{1,64}")) throw ApiException.notFound();
        return avatarDir.resolve(userId + ".png");
    }

    // ================================================================== preferences

    private void preferences(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        User u = ex.user();
        String audio = Json.str(body, "audioLanguage", u.audioLanguage);
        String mode = Json.str(body, "subtitleMode", u.subtitleMode);
        String subs = Json.str(body, "subtitleLanguage", u.subtitleLanguage);
        if (!audio.isEmpty() && !LANGUAGES.contains(audio)) throw ApiException.badRequest("Unbekannte Sprache " + audio);
        if (!subs.isEmpty() && !LANGUAGES.contains(subs)) throw ApiException.badRequest("Unbekannte Sprache " + subs);
        if (!User.SUBTITLE_MODES.contains(mode)) throw ApiException.badRequest("Unbekannte Untertitel-Einstellung " + mode);
        u.audioLanguage = audio;
        u.subtitleMode = mode;
        u.subtitleLanguage = subs;
        store.updateUser(u);
        ex.ok(u.toPublic());
    }

    // ================================================================== profile picture

    private void uploadAvatar(Exchange ex) throws IOException {
        byte[] png = toAvatar(ex.bytes(MAX_UPLOAD));
        User u = ex.user();
        Path file = avatarFile(u.id);
        Path tmp = file.resolveSibling(file.getFileName() + ".tmp");
        Files.write(tmp, png);
        try {
            Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        } catch (AtomicMoveNotSupportedException e) {
            Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING);
        }
        u.avatarVersion = System.currentTimeMillis();
        store.updateUser(u);
        ex.ok(u.toPublic());
    }

    private void deleteAvatar(Exchange ex) throws IOException {
        User u = ex.user();
        Files.deleteIfExists(avatarFile(u.id));
        u.avatarVersion = 0;
        store.updateUser(u);
        ex.ok(u.toPublic());
    }

    private void avatar(Exchange ex) throws IOException {
        User u = store.user(ex.param("id")).orElseThrow(ApiException::notFound);
        if (u.avatarVersion == 0) throw ApiException.notFound();
        // The URL carries ?v=<version>, so the picture can be cached for long.
        FileSender.send(ex, avatarFile(u.id), 30 * 24 * 3600);
    }

    /** Decodes JPEG/PNG/GIF/BMP, crops the centre square and returns a new PNG. */
    static byte[] toAvatar(byte[] data) {
        try (ImageInputStream in = ImageIO.createImageInputStream(new ByteArrayInputStream(data))) {
            Iterator<ImageReader> readers = in == null ? null : ImageIO.getImageReaders(in);
            if (readers == null || !readers.hasNext()) throw ApiException.badRequest("Bitte ein JPEG-, PNG- oder GIF-Bild hochladen");
            ImageReader reader = readers.next();
            try {
                reader.setInput(in, true, true);
                int w = reader.getWidth(0);
                int h = reader.getHeight(0);
                if (w < 32 || h < 32) throw ApiException.badRequest("Das Bild ist zu klein (mindestens 32 × 32 Pixel)");
                // Checked before decoding, so a tiny file claiming huge dimensions cannot exhaust memory.
                if ((long) w * h > MAX_PIXELS) throw ApiException.badRequest("Das Bild hat zu viele Pixel");
                ImageReadParam param = reader.getDefaultReadParam();
                int step = Math.max(1, Math.min(w, h) / (AVATAR_SIZE * 2));
                param.setSourceSubsampling(step, step, 0, 0);
                BufferedImage src = reader.read(0, param);
                int side = Math.min(src.getWidth(), src.getHeight());
                int x = (src.getWidth() - side) / 2;
                int y = (src.getHeight() - side) / 2;
                BufferedImage out = new BufferedImage(AVATAR_SIZE, AVATAR_SIZE, BufferedImage.TYPE_INT_ARGB);
                Graphics2D g = out.createGraphics();
                try {
                    g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC);
                    g.setRenderingHint(RenderingHints.KEY_RENDERING, RenderingHints.VALUE_RENDER_QUALITY);
                    g.drawImage(src, 0, 0, AVATAR_SIZE, AVATAR_SIZE, x, y, x + side, y + side, null);
                } finally {
                    g.dispose();
                }
                ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                ImageIO.write(out, "png", bytes);
                return bytes.toByteArray();
            } finally {
                reader.dispose();
            }
        } catch (ApiException e) {
            throw e;
        } catch (IOException | RuntimeException e) {
            throw ApiException.badRequest("Das Bild lässt sich nicht lesen");
        }
    }

    // ================================================================== watch history

    private Map<String, Object> history(Exchange ex, String userId) {
        int offset = Math.max(0, ex.queryInt("offset", 0));
        int limit = Math.min(100, Math.max(1, ex.queryInt("limit", 50)));
        User viewer = ex.user();
        List<UserStore.HistoryEntry> entries = users.history(userId, offset, limit + 1);
        List<Object> out = new ArrayList<>();
        for (UserStore.HistoryEntry e : entries.subList(0, Math.min(limit, entries.size()))) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", e.id());
            m.put("itemId", e.itemId());
            m.put("watchedAt", e.watchedAt());
            m.put("title", e.title());
            m.put("parentTitle", e.parentTitle());
            // The item as it is now (title from metadata, artwork); null once it left the library.
            m.put("item", store.item(viewer, e.itemId()).map(it -> (Object) media.view(it, viewer, false)).orElse(null));
            if (userId == null || !userId.equals(viewer.id)) {
                store.user(e.userId()).ifPresent(u -> {
                    Map<String, Object> um = new LinkedHashMap<>();
                    um.put("id", u.id);
                    um.put("username", u.username);
                    um.put("avatar", u.avatarVersion);
                    m.put("user", um);
                });
            }
            out.add(m);
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("entries", out);
        m.put("more", entries.size() > limit);
        return m;
    }

    private void removeHistory(Exchange ex) throws IOException {
        long id;
        try {
            id = Long.parseLong(ex.param("hid"));
        } catch (NumberFormatException e) {
            throw ApiException.notFound();
        }
        if (!users.removeHistory(ex.user().id, id)) throw ApiException.notFound();
        ex.noContent();
    }

    private void clearHistory(Exchange ex) throws IOException {
        int n = users.clearHistory(ex.user().id);
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("removed", n);
        ex.ok(m);
    }
}

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

import java.io.IOException;
import java.net.ProxySelector;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.Semaphore;
import java.util.logging.Logger;

/**
 * Minimal client for The Movie Database API v3 (https://developer.themoviedb.org/docs).
 * Accepts either a v3 API key (sent as {@code api_key}) or a v4 read access token (sent as
 * Bearer header). Requests are limited to a few at a time and retried on HTTP 429.
 */
public final class TmdbClient {

    private static final Logger LOG = Logger.getLogger(TmdbClient.class.getName());
    private static final int MAX_PARALLEL = 4;

    private final String apiBase;
    private final String imageBase;
    private final String key;
    private final boolean bearer;
    private final String language;
    private final HttpClient http;
    private final Semaphore slots = new Semaphore(MAX_PARALLEL);

    /**
     * @param apiBase   normally {@code https://api.themoviedb.org/3}
     * @param imageBase normally {@code https://image.tmdb.org/t/p}
     * @param language  e.g. {@code de-DE}
     */
    public TmdbClient(String apiBase, String imageBase, String key, String language) {
        this.apiBase = apiBase.replaceAll("/+$", "");
        this.imageBase = imageBase.replaceAll("/+$", "");
        this.key = key.trim();
        this.bearer = this.key.startsWith("eyJ"); // v4 tokens are JWTs
        this.language = language;
        // Honour -Dhttps.proxyHost etc., for servers that reach the internet only through a proxy.
        this.http = HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(10))
                .proxy(ProxySelector.getDefault())
                .followRedirects(HttpClient.Redirect.NORMAL).build();
    }

    public String language() {
        return language;
    }

    /** GET {@code path} with query parameters; returns the parsed object, or null on 404. */
    public Map<String, Object> get(String path, Map<String, String> params) throws IOException, InterruptedException {
        Map<String, String> q = new LinkedHashMap<>(params);
        if (!bearer) q.put("api_key", key);
        StringBuilder url = new StringBuilder(apiBase).append(path);
        char sep = '?';
        for (Map.Entry<String, String> e : q.entrySet()) {
            if (e.getValue() == null) continue;
            url.append(sep).append(URLEncoder.encode(e.getKey(), StandardCharsets.UTF_8)).append('=')
                    .append(URLEncoder.encode(e.getValue(), StandardCharsets.UTF_8));
            sep = '&';
        }
        HttpRequest.Builder rb = HttpRequest.newBuilder(URI.create(url.toString()))
                .timeout(Duration.ofSeconds(20)).header("Accept", "application/json")
                .header("User-Agent", "FreeX-Media-Server");
        if (bearer) rb.header("Authorization", "Bearer " + key);
        HttpRequest req = rb.GET().build();

        for (int attempt = 0; attempt < 4; attempt++) {
            slots.acquire();
            HttpResponse<String> res;
            try {
                res = http.send(req, HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
            } finally {
                slots.release();
            }
            int code = res.statusCode();
            if (code == 200) return Json.parseObject(res.body());
            if (code == 404) return null;
            if (code == 401) throw new TmdbException("error.tmdbKeyRejected");
            if (code == 429 || code >= 500) {
                long wait = res.headers().firstValue("Retry-After").map(TmdbClient::seconds).orElse(2L);
                Thread.sleep(Math.min(10, Math.max(1, wait)) * 1000L * (attempt + 1));
                continue;
            }
            throw new TmdbException("error.tmdbHttp", code, path);
        }
        throw new TmdbException("error.tmdbBusy");
    }

    private static long seconds(String v) {
        try {
            return Long.parseLong(v.trim());
        } catch (NumberFormatException e) {
            return 2;
        }
    }

    public Map<String, Object> get(String path) throws IOException, InterruptedException {
        return get(path, Map.of("language", language));
    }

    /**
     * Downloads an image ({@code /abc.jpg} from TMDB) in the given size to {@code target},
     * unless it already exists. Returns false if TMDB has no such image.
     */
    public boolean download(String imagePath, String size, Path target) throws IOException, InterruptedException {
        if (imagePath == null || imagePath.isBlank() || !imagePath.matches("/[A-Za-z0-9_.-]+")) return false;
        if (Files.exists(target)) return true;
        Files.createDirectories(target.getParent());
        HttpRequest req = HttpRequest.newBuilder(URI.create(imageBase + "/" + size + imagePath))
                .timeout(Duration.ofSeconds(30)).header("User-Agent", "FreeX-Media-Server").GET().build();
        Path tmp = target.resolveSibling(target.getFileName() + ".part");
        slots.acquire();
        try {
            HttpResponse<Path> res = http.send(req, HttpResponse.BodyHandlers.ofFile(tmp));
            if (res.statusCode() != 200) {
                Files.deleteIfExists(tmp);
                return false;
            }
            Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING);
            return true;
        } catch (IOException e) {
            Files.deleteIfExists(tmp);
            LOG.fine("Image download failed: " + imagePath + ": " + e.getMessage());
            throw e;
        } finally {
            slots.release();
        }
    }
}

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

import com.sun.net.httpserver.HttpExchange;
import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.User;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;

/** Wraps {@link HttpExchange} with the helpers the API handlers need. */
public final class Exchange {

    /** Request bodies above this size are rejected; the API only takes small JSON documents. */
    private static final int MAX_BODY = 1024 * 1024;

    public final HttpExchange raw;
    private final Map<String, String> query;
    private final Map<String, String> pathParams;
    private User user;
    private String token;
    private boolean responded;
    private String clientIp = "?";
    private boolean secure;

    Exchange(HttpExchange raw, Map<String, String> pathParams) {
        this.raw = raw;
        this.pathParams = pathParams;
        this.query = parseQuery(raw.getRequestURI().getRawQuery());
    }

    public String method() {
        return raw.getRequestMethod().toUpperCase();
    }

    public String path() {
        return raw.getRequestURI().getPath();
    }

    public String param(String name) {
        return pathParams.get(name);
    }

    public String query(String name) {
        return query.get(name);
    }

    public String query(String name, String def) {
        String v = query.get(name);
        return v == null || v.isEmpty() ? def : v;
    }

    public int queryInt(String name, int def) {
        try {
            return Integer.parseInt(query(name, Integer.toString(def)));
        } catch (NumberFormatException e) {
            return def;
        }
    }

    public String header(String name) {
        return raw.getRequestHeaders().getFirst(name);
    }

    public String cookie(String name) {
        String all = header("Cookie");
        if (all == null) return null;
        for (String part : all.split(";")) {
            int eq = part.indexOf('=');
            if (eq > 0 && part.substring(0, eq).trim().equals(name)) return part.substring(eq + 1).trim();
        }
        return null;
    }

    /** Client address; behind a trusted proxy taken from X-Forwarded-For. */
    public String remoteAddress() {
        return clientIp;
    }

    /** True if the browser reached us via HTTPS (directly or through a trusted proxy). */
    public boolean secure() {
        return secure;
    }

    void setConnection(String clientIp, boolean secure) {
        this.clientIp = clientIp;
        this.secure = secure;
    }

    public String userAgent() {
        String ua = header("User-Agent");
        return ua == null ? "" : ua;
    }

    public User user() {
        return user;
    }

    public String token() {
        return token;
    }

    void authenticate(User user, String token) {
        this.user = user;
        this.token = token;
    }

    /** Reads the raw request body; {@code 413} if it is larger than {@code max} bytes. */
    public byte[] bytes(int max) throws IOException {
        try (InputStream in = raw.getRequestBody()) {
            byte[] data = in.readNBytes(max + 1);
            if (data.length > max) throw new ApiException(413, "Datei zu groß");
            return data;
        }
    }

    /** Reads the request body as a JSON object (empty map for an empty body). */
    public Map<String, Object> body() throws IOException {
        try (InputStream in = raw.getRequestBody()) {
            byte[] data = in.readNBytes(MAX_BODY + 1);
            if (data.length > MAX_BODY) throw new ApiException(413, "Anfrage zu groß");
            String s = new String(data, StandardCharsets.UTF_8).trim();
            if (s.isEmpty()) return new LinkedHashMap<>();
            try {
                return Json.parseObject(s);
            } catch (Json.JsonException e) {
                throw new ApiException(400, "Ungültiges JSON: " + e.getMessage());
            }
        }
    }

    public boolean responded() {
        return responded;
    }

    public void json(int status, Object value) throws IOException {
        byte[] data = Json.write(value).getBytes(StandardCharsets.UTF_8);
        send(status, "application/json; charset=utf-8", data);
    }

    public void ok(Object value) throws IOException {
        json(200, value);
    }

    public void noContent() throws IOException {
        responded = true;
        raw.getResponseHeaders().set("Cache-Control", "no-store");
        raw.sendResponseHeaders(204, -1);
        raw.close();
    }

    public void error(int status, String message) throws IOException {
        Map<String, Object> m = new HashMap<>();
        m.put("error", message);
        m.put("status", status);
        json(status, m);
    }

    public void send(int status, String contentType, byte[] data) throws IOException {
        responded = true;
        raw.getResponseHeaders().set("Content-Type", contentType);
        if (!raw.getResponseHeaders().containsKey("Cache-Control")) {
            raw.getResponseHeaders().set("Cache-Control", "no-store");
        }
        boolean head = method().equals("HEAD");
        raw.sendResponseHeaders(status, head ? -1 : (data.length == 0 ? -1 : data.length));
        if (!head && data.length > 0) {
            try (OutputStream out = raw.getResponseBody()) {
                out.write(data);
            }
        }
        raw.close();
    }

    void markResponded() {
        responded = true;
    }

    private static Map<String, String> parseQuery(String raw) {
        Map<String, String> m = new HashMap<>();
        if (raw == null || raw.isEmpty()) return m;
        for (String pair : raw.split("&")) {
            int eq = pair.indexOf('=');
            String k = eq >= 0 ? pair.substring(0, eq) : pair;
            String v = eq >= 0 ? pair.substring(eq + 1) : "";
            try {
                m.put(URLDecoder.decode(k, StandardCharsets.UTF_8), URLDecoder.decode(v, StandardCharsets.UTF_8));
            } catch (IllegalArgumentException ignored) {
                // malformed escape, skip this parameter
            }
        }
        return m;
    }
}

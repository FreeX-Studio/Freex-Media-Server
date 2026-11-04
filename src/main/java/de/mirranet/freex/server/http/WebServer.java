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
import com.sun.net.httpserver.HttpServer;
import com.sun.net.httpserver.HttpsConfigurator;
import com.sun.net.httpserver.HttpsServer;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.store.DataStore;

import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import java.io.IOException;
import java.io.InputStream;
import java.net.InetSocketAddress;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.logging.Level;
import java.util.logging.Logger;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * The HTTP server: routes {@code /api/...} to handlers with an access check and serves the web
 * interface from the jar ({@code /web}), falling back to {@code index.html} for app routes.
 * Every request runs on its own virtual thread, so long video streams block nothing.
 */
public final class WebServer implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(WebServer.class.getName());
    public static final String TOKEN_COOKIE = "freex_token";
    public static final String TOKEN_HEADER = "X-FreeX-Token";

    public enum Access { PUBLIC, USER, ADMIN }

    @FunctionalInterface
    public interface Handler {
        void handle(Exchange ex) throws Exception;
    }

    private record Route(String method, Pattern pattern, List<String> names, Access access, Handler handler) {
    }

    private final List<Route> routes = new ArrayList<>();
    private final DataStore store;
    private final HttpServer server;
    private final ExecutorService executor = Executors.newVirtualThreadPerTaskExecutor();
    private final Map<String, byte[]> staticCache = new ConcurrentHashMap<>();

    private final boolean tls;
    private final boolean trustProxy;

    /**
     * @param tlsKeystore PKCS#12 keystore for HTTPS, or empty for plain HTTP
     * @param trustProxy  trust X-Forwarded-* headers (only behind your own reverse proxy)
     */
    public WebServer(String bindAddress, int port, DataStore store, String tlsKeystore, String tlsPassword,
                     boolean trustProxy) throws IOException {
        this.store = store;
        this.trustProxy = trustProxy;
        InetSocketAddress address = new InetSocketAddress(bindAddress, port);
        if (tlsKeystore != null && !tlsKeystore.isEmpty()) {
            // Load the keystore before binding: create() takes the port right away, and a failed
            // load must not leave it occupied (the restart falls back to the previous settings).
            SSLContext ctx = sslContext(tlsKeystore, tlsPassword);
            HttpsServer https = HttpsServer.create(address, 128);
            https.setHttpsConfigurator(new HttpsConfigurator(ctx));
            this.server = https;
            this.tls = true;
        } else {
            this.server = HttpServer.create(address, 128);
            this.tls = false;
        }
        server.setExecutor(executor);
        server.createContext("/", this::dispatch);
    }

    public boolean tls() {
        return tls;
    }

    static SSLContext sslContext(String keystore, String password) throws IOException {
        try (InputStream in = Files.newInputStream(Path.of(keystore))) {
            KeyStore ks = KeyStore.getInstance("PKCS12");
            char[] pw = password == null ? new char[0] : password.toCharArray();
            ks.load(in, pw);
            KeyManagerFactory kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
            kmf.init(ks, pw);
            SSLContext ctx = SSLContext.getInstance("TLS");
            ctx.init(kmf.getKeyManagers(), null, null);
            return ctx;
        } catch (GeneralSecurityException e) {
            throw new IOException("Cannot load TLS keystore " + keystore + ": " + e.getMessage(), e);
        }
    }

    private Exchange newExchange(HttpExchange raw, Map<String, String> params) {
        Exchange ex = new Exchange(raw, params);
        String ip = raw.getRemoteAddress() == null ? "?" : raw.getRemoteAddress().getAddress().getHostAddress();
        boolean secure = tls;
        if (trustProxy) {
            String xff = raw.getRequestHeaders().getFirst("X-Forwarded-For");
            if (xff != null && !xff.isBlank()) ip = xff.split(",")[0].trim();
            String proto = raw.getRequestHeaders().getFirst("X-Forwarded-Proto");
            if ("https".equalsIgnoreCase(proto)) secure = true;
        }
        ex.setConnection(ip, secure);
        return ex;
    }

    /**
     * Rejects state-changing requests whose Origin header names a different site. Together with
     * the header-only token for such requests this blocks cross-site request forgery.
     */
    private boolean sameOrigin(HttpExchange raw) {
        String origin = raw.getRequestHeaders().getFirst("Origin");
        if (origin == null || origin.equals("null")) return origin == null;
        String host = trustProxy && raw.getRequestHeaders().getFirst("X-Forwarded-Host") != null
                ? raw.getRequestHeaders().getFirst("X-Forwarded-Host") : raw.getRequestHeaders().getFirst("Host");
        if (host == null) return false;
        try {
            java.net.URI o = java.net.URI.create(origin);
            String oh = o.getHost() + (o.getPort() > 0 ? ":" + o.getPort() : "");
            return oh.equalsIgnoreCase(host.trim()) || o.getHost().equalsIgnoreCase(host.trim());
        } catch (IllegalArgumentException e) {
            return false;
        }
    }

    public void start() {
        server.start();
    }

    /**
     * Registers a route. Path segments in braces are parameters: {@code /api/items/{id}}.
     */
    public void route(String method, String path, Access access, Handler handler) {
        List<String> names = new ArrayList<>();
        StringBuilder regex = new StringBuilder("^");
        Matcher m = Pattern.compile("\\{(\\w+)}").matcher(path);
        int last = 0;
        while (m.find()) {
            regex.append(Pattern.quote(path.substring(last, m.start()))).append("([^/]+)");
            names.add(m.group(1));
            last = m.end();
        }
        regex.append(Pattern.quote(path.substring(last))).append("$");
        routes.add(new Route(method, Pattern.compile(regex.toString()), names, access, handler));
    }

    private void dispatch(HttpExchange raw) {
        Exchange ex = null;
        try {
            String path = raw.getRequestURI().getPath();
            String method = raw.getRequestMethod().toUpperCase();
            addSecurityHeaders(raw);
            boolean cast = path.startsWith(CastApi.PREFIX);
            if (cast) {
                // The Chromecast receiver runs on Google's origin and loads media from here.
                var h = raw.getResponseHeaders();
                h.set("Cross-Origin-Resource-Policy", "cross-origin");
                h.set("Access-Control-Allow-Origin", "*");
                h.set("Access-Control-Allow-Headers", "Range, Content-Type");
                h.set("Access-Control-Expose-Headers", "Content-Length, Content-Range, Accept-Ranges");
                if (method.equals("OPTIONS")) {
                    h.set("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS");
                    raw.sendResponseHeaders(204, -1);
                    raw.close();
                    return;
                }
            }
            boolean unsafe = !method.equals("GET") && !method.equals("HEAD");
            if (unsafe && !sameOrigin(raw)) {
                ex = newExchange(raw, Map.of());
                ex.error(403, "Anfrage von fremder Seite abgelehnt");
                return;
            }
            if (path.startsWith("/api/")) {
                boolean pathMatched = false;
                for (Route r : routes) {
                    Matcher m = r.pattern.matcher(path);
                    if (!m.matches()) continue;
                    pathMatched = true;
                    boolean methodOk = r.method.equals(method) || (r.method.equals("GET") && method.equals("HEAD"));
                    if (!methodOk) continue;
                    Map<String, String> params = new HashMap<>();
                    for (int i = 0; i < r.names.size(); i++) params.put(r.names.get(i), m.group(i + 1));
                    ex = newExchange(raw, params);
                    if (!authorize(ex, r.access)) return;
                    r.handler.handle(ex);
                    if (!ex.responded()) ex.noContent();
                    return;
                }
                ex = newExchange(raw, Map.of());
                ex.error(pathMatched ? 405 : 404, pathMatched ? "Methode nicht erlaubt" : "Unbekannter Endpunkt");
                return;
            }
            ex = newExchange(raw, Map.of());
            serveStatic(ex, path);
        } catch (ApiException e) {
            respondError(ex, raw, e.status, e.getMessage());
        } catch (IOException e) {
            // Client went away mid-response.
            raw.close();
        } catch (Exception e) {
            LOG.log(Level.WARNING, "Request " + raw.getRequestMethod() + " " + raw.getRequestURI() + " failed", e);
            respondError(ex, raw, 500, "Interner Fehler");
        }
    }

    private static void respondError(Exchange ex, HttpExchange raw, int status, String message) {
        try {
            if (ex == null) {
                ex = new Exchange(raw, Map.of());
            }
            if (!ex.responded()) ex.error(status, message);
        } catch (IOException ignored) {
            raw.close();
        }
    }

    private void addSecurityHeaders(HttpExchange raw) {
        var h = raw.getResponseHeaders();
        h.set("X-Content-Type-Options", "nosniff");
        h.set("Referrer-Policy", "same-origin");
        h.set("X-Frame-Options", "DENY");
        h.set("Cross-Origin-Opener-Policy", "same-origin");
        h.set("Cross-Origin-Resource-Policy", "same-origin");
        h.set("Permissions-Policy", "camera=(), microphone=(), geolocation=(), payment=()");
        // Only our own scripts; blob: is needed for the HLS player's worker and media buffers.
        // gstatic.com: the Google Cast sender SDK, only loaded in Chrome over HTTPS.
        h.set("Content-Security-Policy", "default-src 'self'; script-src 'self' https://www.gstatic.com; style-src 'self'; "
                + "img-src 'self' data: blob:; media-src 'self' blob:; worker-src 'self' blob:; connect-src 'self'; "
                + "font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; "
                // Trailers are embedded from YouTube's privacy-enhanced domain only.
                + "frame-src https://www.youtube-nocookie.com");
        boolean secure = tls || (trustProxy && "https".equalsIgnoreCase(raw.getRequestHeaders().getFirst("X-Forwarded-Proto")));
        if (secure) h.set("Strict-Transport-Security", "max-age=31536000");
    }

    /**
     * Resolves the user from the token header or cookie. The cookie is only accepted for GET/HEAD
     * (images, video, subtitles); anything that changes state needs the header, which a foreign
     * site cannot set, so cross-site requests cannot act on behalf of a logged-in user.
     */
    private boolean authorize(Exchange ex, Access access) throws IOException {
        String token = ex.header(TOKEN_HEADER);
        if (token == null) {
            String auth = ex.header("Authorization");
            if (auth != null && auth.regionMatches(true, 0, "Bearer ", 0, 7)) token = auth.substring(7).trim();
        }
        boolean safeMethod = ex.method().equals("GET") || ex.method().equals("HEAD");
        if (token == null && safeMethod) token = ex.cookie(TOKEN_COOKIE);
        Optional<User> user = store.userForToken(token);
        if (user.isPresent()) ex.authenticate(user.get(), token);
        switch (access) {
            case PUBLIC -> {
                return true;
            }
            case USER -> {
                if (user.isEmpty()) {
                    ex.error(401, "Nicht angemeldet");
                    return false;
                }
                return true;
            }
            case ADMIN -> {
                if (user.isEmpty()) {
                    ex.error(401, "Nicht angemeldet");
                    return false;
                }
                if (!user.get().admin) {
                    ex.error(403, "Nur für Administratoren");
                    return false;
                }
                return true;
            }
        }
        return false;
    }

    // ------------------------------------------------------------------ web interface

    private void serveStatic(Exchange ex, String path) throws IOException {
        if (!ex.method().equals("GET") && !ex.method().equals("HEAD")) {
            ex.error(405, "Methode nicht erlaubt");
            return;
        }
        String file = path.equals("/") ? "index.html" : path.substring(1);
        // Only plain names from the bundled web folder, no traversal.
        if (file.contains("..") || file.contains("\\") || file.contains("//")) {
            ex.error(404, "Nicht gefunden");
            return;
        }
        byte[] data = resource(file);
        if (data == null) {
            // App routes like /library/abc are handled in the browser.
            if (file.contains(".")) {
                ex.error(404, "Nicht gefunden");
                return;
            }
            file = "index.html";
            data = resource(file);
            if (data == null) {
                ex.error(500, "Web-Oberfläche fehlt im Jar");
                return;
            }
        }
        ex.raw.getResponseHeaders().set("Cache-Control", file.equals("index.html") ? "no-cache" : "public, max-age=300");
        ex.send(200, FileSender.contentType(file), data);
    }

    private byte[] resource(String file) throws IOException {
        byte[] cached = staticCache.get(file);
        if (cached != null) return cached;
        try (InputStream in = WebServer.class.getResourceAsStream("/web/" + file)) {
            if (in == null) return null;
            byte[] data = in.readAllBytes();
            staticCache.put(file, data);
            return data;
        }
    }

    @Override
    public void close() {
        server.stop(1);
        executor.shutdownNow();
    }
}

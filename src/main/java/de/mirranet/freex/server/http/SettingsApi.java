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

import de.mirranet.freex.server.auth.PasswordHasher;
import de.mirranet.freex.server.config.Configuration;
import de.mirranet.freex.server.config.Settings;
import de.mirranet.freex.server.json.Json;

import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.NetworkInterface;
import java.net.ServerSocket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.logging.Logger;

import static de.mirranet.freex.server.http.WebServer.Access.ADMIN;

/**
 * Server settings for administrators: shows the values of {@code config.yml}, writes changes
 * back to it and restarts the server so they take effect.
 * <p>
 * Secrets (TMDB key, keystore password) are never sent to the browser, only whether they are
 * set. Saving needs the current password. Before a change is written, a new port is test-bound
 * and a new keystore is test-loaded, so a typo does not leave the server unreachable; should
 * the restart fail anyway, the previous file is restored (see {@code FreeXMediaServer}).
 */
public final class SettingsApi {

    private static final Logger LOG = Logger.getLogger(SettingsApi.class.getName());

    private final Configuration running;
    private final Runnable restart;

    /**
     * @param running the configuration the server currently runs with
     * @param restart asks the server to restart with the configuration file as it is now
     */
    public SettingsApi(Configuration running, Runnable restart) {
        this.running = running;
        this.restart = restart;
    }

    public void register(WebServer web) {
        web.route("GET", "/api/admin/settings", ADMIN, ex -> ex.ok(describe()));
        web.route("PUT", "/api/admin/settings", ADMIN, this::save);
        web.route("POST", "/api/admin/restart", ADMIN, this::restart);
    }

    private String fileText() throws IOException {
        return Files.exists(running.file) ? Files.readString(running.file, StandardCharsets.UTF_8) : "";
    }

    private boolean writable() {
        Path dir = running.file.getParent();
        return Files.isWritable(running.file) && dir != null && Files.isWritable(dir);
    }

    private Map<String, Object> describe() throws IOException {
        Map<String, String> saved = Settings.effective(fileText());
        Map<String, Object> values = new LinkedHashMap<>();
        Map<String, Object> secrets = new LinkedHashMap<>();
        List<Object> fields = new ArrayList<>();
        boolean pending = false;
        for (Settings.Field f : Settings.FIELDS) {
            fields.add(f.toPublic());
            String v = saved.getOrDefault(f.key(), "");
            if (!v.equals(running.effective(f.key()))) pending = true;
            switch (f.type()) {
                case SECRET -> secrets.put(f.key(), !v.isEmpty());
                case BOOL -> values.put(f.key(), !"false".equalsIgnoreCase(v.trim()) && !v.isBlank());
                case NUMBER -> {
                    try {
                        values.put(f.key(), Long.parseLong(v.trim()));
                    } catch (NumberFormatException e) {
                        values.put(f.key(), v);
                    }
                }
                default -> values.put(f.key(), v);
            }
        }
        // trustProxy is only on when explicitly true, unlike the other switches
        values.put("server.trustProxy", "true".equalsIgnoreCase(saved.getOrDefault("server.trustProxy", "").trim()));
        Map<String, Object> paths = new LinkedHashMap<>();
        paths.put("data.directory", running.dataDirectory.toAbsolutePath().toString());
        paths.put("data.cacheDirectory", running.cacheDirectory.toAbsolutePath().toString());
        paths.put("transcoder.directory", running.transcodeDirectory.toAbsolutePath().toString());

        Map<String, Object> m = new LinkedHashMap<>();
        m.put("file", running.file.toString());
        m.put("writable", writable());
        m.put("pendingRestart", pending);
        m.put("fields", fields);
        m.put("values", values);
        m.put("secrets", secrets);
        m.put("paths", paths);
        Map<String, Object> now = new LinkedHashMap<>();
        now.put("port", running.port);
        now.put("tls", !running.tlsKeystore.isEmpty());
        m.put("running", now);
        return m;
    }

    private void save(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        if (!PasswordHasher.verify(Json.str(body, "currentPassword", ""), ex.user().passwordHash)) {
            throw new ApiException(403, "Das aktuelle Passwort ist falsch");
        }
        if (!writable()) {
            throw new ApiException(409, "Der Server darf " + running.file + " nicht ändern. Passe die Datei direkt an.");
        }
        Map<String, String> saved = Settings.effective(fileText());
        Map<String, String> next = new LinkedHashMap<>(saved);
        Map<String, String> changes = new LinkedHashMap<>();
        for (Map.Entry<String, Object> e : Json.obj(body.get("values")).entrySet()) {
            Settings.Field f = Settings.field(e.getKey());
            if (f == null) throw ApiException.badRequest("Unbekannte Einstellung " + e.getKey());
            if (f.type() == Settings.Type.SECRET && e.getValue() == null) continue; // unchanged
            String value;
            try {
                value = Settings.validate(f, e.getValue());
            } catch (IllegalArgumentException iae) {
                throw ApiException.badRequest(iae.getMessage());
            }
            if (!value.equals(saved.get(f.key()))) changes.put(f.key(), value);
            next.put(f.key(), value);
        }
        if (changes.isEmpty()) {
            ex.ok(describe());
            return;
        }
        checkNetwork(next);
        checkKeystore(next, changes);
        if (changes.containsKey("transcoder.directory")) checkTranscodeDirectory(next.get("transcoder.directory"));

        Settings.write(running.file, changes);
        LOG.info("Settings changed by '" + ex.user().username + "': " + String.join(", ", changes.keySet()));
        ex.ok(describe());
    }

    /** A changed address must belong to this machine, a changed port must be free. */
    private void checkNetwork(Map<String, String> next) {
        String address = next.get("server.bindAddress").trim();
        int port = Integer.parseInt(next.get("server.port").trim());
        boolean addressChanged = !address.equals(running.bindAddress);
        if (addressChanged) {
            try {
                InetAddress a = InetAddress.getByName(address);
                if (!a.isAnyLocalAddress() && !a.isLoopbackAddress() && NetworkInterface.getByInetAddress(a) == null) {
                    throw ApiException.badRequest("Die Adresse " + address + " gehört zu keiner Netzwerkschnittstelle dieses Servers");
                }
            } catch (IOException e) {
                throw ApiException.badRequest("Die Adresse " + address + " ist ungültig");
            }
        }
        // The current port is held by this server itself, so only a different port can be tested.
        if (port != running.port) {
            try (ServerSocket s = new ServerSocket()) {
                s.setReuseAddress(true);
                s.bind(new InetSocketAddress(address, port));
            } catch (IOException e) {
                throw ApiException.badRequest("Port " + port + " ist nicht verfügbar (" + e.getMessage() + ")"
                        + (port < 1024 ? ". Ports unter 1024 brauchen besondere Rechte." : ""));
            }
        }
    }

    /**
     * The transcoding directory must exist (or be creatable) and writable for the server. Under
     * the hardened systemd service only a few paths are writable; for others a drop-in is needed.
     */
    private void checkTranscodeDirectory(String value) {
        Path dir = running.cacheDirectory.resolve(value.trim()).normalize();
        Path probe = dir.resolve(".freex-write-test");
        try {
            Files.createDirectories(dir);
            Files.writeString(probe, "ok");
            Files.delete(probe);
        } catch (IOException | SecurityException e) {
            String why = e instanceof java.nio.file.AccessDeniedException ? "keine Berechtigung"
                    : e instanceof java.nio.file.FileSystemException fse && fse.getReason() != null ? fse.getReason()
                    : e instanceof java.nio.file.NoSuchFileException ? "Pfad nicht vorhanden" : e.getClass().getSimpleName();
            throw ApiException.badRequest("Der Server kann nicht in " + dir + " schreiben (" + why + "). "
                    + "Prüfe die Rechte für den Benutzer des Servers. Läuft er als systemd-Dienst, gib den Ordner dort frei: "
                    + "sudo systemctl edit freex-media-server und unter [Service] ReadWritePaths=" + dir + " eintragen.");
        }
    }

    /** A new keystore or password must open, otherwise HTTPS would not start. */
    private void checkKeystore(Map<String, String> next, Map<String, String> changes) {
        if (!changes.containsKey("server.tlsKeystore") && !changes.containsKey("server.tlsKeystorePassword")) return;
        String keystore = next.get("server.tlsKeystore").trim();
        if (keystore.isEmpty()) return;
        Path p = running.file.getParent().resolve(keystore).normalize();
        if (!Files.isReadable(p)) throw ApiException.badRequest("Keystore " + p + " nicht gefunden oder nicht lesbar");
        try {
            WebServer.sslContext(p.toString(), next.get("server.tlsKeystorePassword"));
        } catch (IOException e) {
            throw ApiException.badRequest("Keystore lässt sich nicht öffnen: falsches Passwort oder kein PKCS#12-Format");
        }
    }

    private void restart(Exchange ex) throws IOException {
        LOG.info("Restart requested by '" + ex.user().username + "'");
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("restarting", true);
        ex.ok(m);
        // Answer first; the restart closes the web server this request runs on.
        Thread.ofPlatform().name("restart").start(() -> {
            try {
                Thread.sleep(300);
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            }
            restart.run();
        });
    }
}

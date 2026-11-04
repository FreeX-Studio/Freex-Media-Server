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

import de.mirranet.freex.server.auth.LoginGuard;
import de.mirranet.freex.server.auth.PasswordHasher;
import de.mirranet.freex.server.auth.Totp;
import de.mirranet.freex.server.json.Json;
import de.mirranet.freex.server.model.User;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.store.UserStore;
import de.mirranet.freex.server.util.Ids;

import java.io.IOException;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.logging.Logger;

import static de.mirranet.freex.server.http.WebServer.Access.ADMIN;
import static de.mirranet.freex.server.http.WebServer.Access.PUBLIC;
import static de.mirranet.freex.server.http.WebServer.Access.USER;

/**
 * Accounts and sign-in: setup, login with optional two-factor step, sessions per device, login
 * history, password changes and user management.
 * <p>
 * Hardening: unknown and known account names take the same time and give the same answer,
 * guessing is slowed down by {@link LoginGuard}, sensitive changes need the current password,
 * changing the password or 2FA settings signs out all other devices.
 */
public final class AuthApi {

    private static final Logger LOG = Logger.getLogger(AuthApi.class.getName());
    private static final long CHALLENGE_TTL_MS = TimeUnit.MINUTES.toMillis(5);
    private static final int CHALLENGE_ATTEMPTS = 5;
    private static final int RECOVERY_CODES = 10;
    private static final String WRONG_LOGIN = "Benutzername oder Passwort falsch";
    private static final SecureRandom RANDOM = new SecureRandom();

    /** Password check passed, waiting for the second factor. */
    private record Challenge(String userId, long expires, int attempts) {
    }

    private final String serverName;
    private final DataStore store;
    private final LoginGuard guard = new LoginGuard();
    private final int minPasswordLength;
    private final int sessionDays;
    private final Map<String, Challenge> challenges = new ConcurrentHashMap<>();
    /** Compared against when the account does not exist, so the response time is the same. */
    private final String dummyHash = PasswordHasher.hash(Ids.token());

    private final java.util.function.Consumer<String> onUserDeleted;

    public AuthApi(String serverName, DataStore store, int minPasswordLength, int sessionDays,
                   java.util.function.Consumer<String> onUserDeleted) {
        this.onUserDeleted = onUserDeleted;
        this.serverName = serverName;
        this.store = store;
        this.minPasswordLength = minPasswordLength;
        this.sessionDays = sessionDays;
    }

    public void register(WebServer web) {
        web.route("POST", "/api/setup", PUBLIC, this::setup);
        web.route("POST", "/api/login", PUBLIC, this::login);
        web.route("POST", "/api/login/verify", PUBLIC, this::verifySecondFactor);
        web.route("POST", "/api/logout", USER, this::logout);

        web.route("GET", "/api/me", USER, ex -> ex.ok(ex.user().toPublic()));
        web.route("PUT", "/api/me/password", USER, this::changePassword);
        web.route("GET", "/api/me/sessions", USER, this::sessions);
        web.route("DELETE", "/api/me/sessions/{sid}", USER, this::endSession);
        web.route("POST", "/api/me/sessions/logout-others", USER, ex -> {
            int n = store.removeSessionsOf(ex.user().id, ex.token());
            ex.ok(Map.of("ended", n));
        });
        web.route("GET", "/api/me/logins", USER, this::logins);
        web.route("POST", "/api/me/2fa/setup", USER, this::twoFactorSetup);
        web.route("POST", "/api/me/2fa/enable", USER, this::twoFactorEnable);
        web.route("POST", "/api/me/2fa/disable", USER, this::twoFactorDisable);
        web.route("POST", "/api/me/2fa/recovery", USER, this::newRecoveryCodes);

        web.route("GET", "/api/users", ADMIN, ex -> ex.ok(store.users().stream().map(User::toPublic).toList()));
        web.route("POST", "/api/users", ADMIN, this::createUser);
        web.route("PUT", "/api/users/{id}", ADMIN, this::updateUser);
        web.route("DELETE", "/api/users/{id}", ADMIN, this::deleteUser);
    }

    // ================================================================== sign in

    private void setup(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        synchronized (store) {
            if (store.hasUsers()) throw new ApiException(409, "Der Server ist bereits eingerichtet");
            String username = validUsername(Json.str(body, "username", ""));
            String password = validPassword(Json.str(body, "password", ""), username);
            User u = store.addUser(username, PasswordHasher.hash(password), true);
            LOG.info("Admin account '" + username + "' created from " + ex.remoteAddress());
            startSession(ex, u, "Einrichtung");
        }
    }

    private void login(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        String username = Json.str(body, "username", "").trim();
        String password = Json.str(body, "password", "");
        String ip = ex.remoteAddress();
        throttle(ex, ip, username);

        Optional<User> user = store.userByName(username);
        // Always run the hash, so a missing account takes as long as a wrong password.
        boolean ok = PasswordHasher.verify(password, user.map(u -> u.passwordHash).orElse(dummyHash)) && user.isPresent();
        if (!ok) {
            guard.failure(ip, username);
            user.ifPresent(u -> store.addLoginEvent(u.id, event(ex, false, "Falsches Passwort")));
            LOG.warning("Failed login for '" + username + "' from " + ip);
            throw new ApiException(401, WRONG_LOGIN);
        }
        User u = user.get();
        if (u.totpEnabled) {
            String challenge = Ids.token();
            challenges.put(Ids.sha256(challenge), new Challenge(u.id, System.currentTimeMillis() + CHALLENGE_TTL_MS, 0));
            challenges.values().removeIf(c -> c.expires < System.currentTimeMillis());
            ex.ok(Map.of("twoFactorRequired", true, "challenge", challenge));
            return;
        }
        guard.success(ip, username);
        startSession(ex, u, "Passwort");
    }

    private void verifySecondFactor(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        String key = Ids.sha256(Json.str(body, "challenge", ""));
        String code = Json.str(body, "code", "").trim();
        Challenge c = challenges.get(key);
        if (c == null || c.expires < System.currentTimeMillis()) {
            challenges.remove(key);
            throw new ApiException(401, "Die Anmeldung ist abgelaufen. Bitte erneut mit dem Passwort anmelden.");
        }
        User u = store.user(c.userId).orElseThrow(() -> new ApiException(401, WRONG_LOGIN));
        String ip = ex.remoteAddress();
        throttle(ex, ip, u.username);

        String method = null;
        synchronized (u) {
            long step = Totp.verify(u.totpSecret, code, u.totpLastStep);
            if (step >= 0) {
                u.totpLastStep = step;
                method = "Passwort + Authenticator";
            } else if (useRecoveryCode(u, code)) {
                method = "Passwort + Wiederherstellungscode";
            }
            if (method != null) store.updateUser(u);
        }
        if (method == null) {
            guard.failure(ip, u.username);
            store.addLoginEvent(u.id, event(ex, false, "Falscher Bestätigungscode"));
            LOG.warning("Failed second factor for '" + u.username + "' from " + ip);
            if (c.attempts + 1 >= CHALLENGE_ATTEMPTS) {
                challenges.remove(key);
                throw new ApiException(401, "Zu viele falsche Codes. Bitte erneut mit dem Passwort anmelden.");
            }
            challenges.put(key, new Challenge(c.userId, c.expires, c.attempts + 1));
            throw new ApiException(401, "Der Code stimmt nicht");
        }
        challenges.remove(key);
        guard.success(ip, u.username);
        startSession(ex, u, method);
    }

    private void throttle(Exchange ex, String ip, String account) {
        long wait = guard.retryAfterSeconds(ip, account);
        if (wait > 0) {
            ex.raw.getResponseHeaders().set("Retry-After", Long.toString(wait));
            throw new ApiException(429, "Zu viele Fehlversuche. Bitte in " + formatWait(wait) + " erneut versuchen.");
        }
    }

    private static String formatWait(long seconds) {
        if (seconds < 90) return seconds + " Sekunden";
        return ((seconds + 59) / 60) + " Minuten";
    }

    private boolean useRecoveryCode(User u, String code) {
        String normalized = code.replaceAll("[\\s-]", "").toLowerCase(Locale.ROOT);
        if (!normalized.matches("[a-z0-9]{10}")) return false;
        String hash = Ids.sha256(normalized);
        List<String> codes = new ArrayList<>(u.recoveryCodes);
        if (!codes.remove(hash)) return false;
        u.recoveryCodes = codes;
        return true;
    }

    private void startSession(Exchange ex, User u, String method) throws IOException {
        String token = store.createSession(u.id, ex.remoteAddress(), ex.userAgent());
        store.addLoginEvent(u.id, event(ex, true, method));
        LOG.info("Login '" + u.username + "' from " + ex.remoteAddress() + " (" + method + ")");
        ex.raw.getResponseHeaders().add("Set-Cookie", WebServer.TOKEN_COOKIE + "=" + token
                + "; Path=/; HttpOnly; SameSite=Strict; Max-Age=" + TimeUnit.DAYS.toSeconds(sessionDays)
                + (ex.secure() ? "; Secure" : ""));
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("token", token);
        m.put("user", u.toPublic());
        ex.ok(m);
    }

    private static UserStore.LoginEvent event(Exchange ex, boolean success, String detail) {
        return new UserStore.LoginEvent(System.currentTimeMillis(), ex.remoteAddress(), ex.userAgent(), success, detail);
    }

    private void logout(Exchange ex) throws IOException {
        store.removeSession(ex.token());
        ex.raw.getResponseHeaders().add("Set-Cookie", WebServer.TOKEN_COOKIE
                + "=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0" + (ex.secure() ? "; Secure" : ""));
        ex.noContent();
    }

    // ================================================================== own account

    private void requirePassword(Exchange ex, Map<String, Object> body) {
        User u = ex.user();
        throttle(ex, ex.remoteAddress(), u.username);
        if (!PasswordHasher.verify(Json.str(body, "currentPassword", ""), u.passwordHash)) {
            guard.failure(ex.remoteAddress(), u.username);
            throw new ApiException(403, "Das aktuelle Passwort ist falsch");
        }
    }

    private void changePassword(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        requirePassword(ex, body);
        User u = ex.user();
        u.passwordHash = PasswordHasher.hash(validPassword(Json.str(body, "password", ""), u.username));
        u.passwordChangedAt = System.currentTimeMillis();
        store.updateUser(u);
        int ended = store.removeSessionsOf(u.id, ex.token());
        LOG.info("Password of '" + u.username + "' changed, " + ended + " other sessions ended");
        ex.ok(Map.of("endedSessions", ended));
    }

    private void sessions(Exchange ex) throws IOException {
        String current = DataStore.sessionId(ex.token());
        List<Object> out = new ArrayList<>();
        for (Map.Entry<String, UserStore.Session> e : store.sessionsOf(ex.user().id).entrySet()) {
            UserStore.Session s = e.getValue();
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", e.getKey());
            m.put("current", e.getKey().equals(current));
            m.put("createdAt", s.createdAt());
            m.put("lastSeen", s.lastSeen());
            m.put("ip", s.ip());
            m.put("userAgent", s.userAgent());
            out.add(m);
        }
        ex.ok(out);
    }

    private void endSession(Exchange ex) throws IOException {
        if (!store.removeSessionById(ex.user().id, ex.param("sid"))) throw ApiException.notFound();
        ex.noContent();
    }

    private void logins(Exchange ex) throws IOException {
        List<Object> out = new ArrayList<>();
        for (UserStore.LoginEvent e : store.loginEvents(ex.user().id)) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("time", e.time());
            m.put("ip", e.ip());
            m.put("userAgent", e.userAgent());
            m.put("success", e.success());
            m.put("detail", e.detail());
            out.add(m);
        }
        ex.ok(out);
    }

    // ================================================================== two-factor

    private void twoFactorSetup(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        requirePassword(ex, body);
        User u = ex.user();
        if (u.totpEnabled) throw ApiException.badRequest("Die Zwei-Faktor-Anmeldung ist bereits aktiv");
        u.totpSecret = Totp.newSecret();
        u.totpLastStep = 0;
        store.updateUser(u);
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("secret", u.totpSecret);
        m.put("uri", Totp.uri(serverName, u.username, u.totpSecret));
        ex.ok(m);
    }

    private void twoFactorEnable(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        User u = ex.user();
        if (u.totpEnabled) throw ApiException.badRequest("Die Zwei-Faktor-Anmeldung ist bereits aktiv");
        if (u.totpSecret == null) throw ApiException.badRequest("Bitte die Einrichtung neu starten");
        long step = Totp.verify(u.totpSecret, Json.str(body, "code", ""), 0);
        if (step < 0) throw ApiException.badRequest("Der Code stimmt nicht. Uhrzeit von Handy und Server prüfen.");
        List<String> codes = new ArrayList<>();
        u.recoveryCodes = generateRecoveryCodes(codes);
        u.totpLastStep = step;
        u.totpEnabled = true;
        store.updateUser(u);
        int ended = store.removeSessionsOf(u.id, ex.token());
        LOG.info("Two-factor sign-in enabled for '" + u.username + "', " + ended + " other sessions ended");
        ex.ok(Map.of("recoveryCodes", codes));
    }

    private void twoFactorDisable(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        requirePassword(ex, body);
        User u = ex.user();
        u.totpEnabled = false;
        u.totpSecret = null;
        u.recoveryCodes = new ArrayList<>();
        store.updateUser(u);
        LOG.info("Two-factor sign-in disabled for '" + u.username + "'");
        ex.ok(u.toPublic());
    }

    private void newRecoveryCodes(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        requirePassword(ex, body);
        User u = ex.user();
        if (!u.totpEnabled) throw ApiException.badRequest("Die Zwei-Faktor-Anmeldung ist nicht aktiv");
        List<String> codes = new ArrayList<>();
        u.recoveryCodes = generateRecoveryCodes(codes);
        store.updateUser(u);
        ex.ok(Map.of("recoveryCodes", codes));
    }

    /** Fills {@code plain} with new codes ("abcde-12345") and returns their hashes. */
    private static List<String> generateRecoveryCodes(List<String> plain) {
        String alphabet = "abcdefghjkmnpqrstuvwxyz23456789";
        List<String> hashes = new ArrayList<>();
        for (int i = 0; i < RECOVERY_CODES; i++) {
            StringBuilder sb = new StringBuilder();
            for (int j = 0; j < 10; j++) sb.append(alphabet.charAt(RANDOM.nextInt(alphabet.length())));
            String code = sb.toString();
            plain.add(code.substring(0, 5) + "-" + code.substring(5));
            hashes.add(Ids.sha256(code));
        }
        return hashes;
    }

    // ================================================================== user management

    private void createUser(Exchange ex) throws IOException {
        Map<String, Object> body = ex.body();
        String username = validUsername(Json.str(body, "username", ""));
        String password = validPassword(Json.str(body, "password", ""), username);
        try {
            User u = store.addUser(username, PasswordHasher.hash(password), Json.bool(body, "admin", false));
            if (body.containsKey("libraries")) {
                applyLibraries(u, body.get("libraries"));
                store.updateUser(u);
            }
            LOG.info("User '" + username + "' created by '" + ex.user().username + "'");
            ex.json(201, u.toPublic());
        } catch (IllegalArgumentException e) {
            throw new ApiException(409, e.getMessage());
        }
    }

    private void updateUser(Exchange ex) throws IOException {
        User u = store.user(ex.param("id")).orElseThrow(ApiException::notFound);
        Map<String, Object> body = ex.body();
        boolean endSessions = false;
        if (body.containsKey("admin")) {
            boolean admin = Json.bool(body, "admin", u.admin);
            if (!admin && u.admin && store.users().stream().filter(x -> x.admin).count() <= 1) {
                throw ApiException.badRequest("Es muss mindestens einen Administrator geben");
            }
            u.admin = admin;
        }
        if (body.containsKey("libraries")) {
            applyLibraries(u, body.get("libraries"));
            LOG.info("Libraries of '" + u.username + "' set by '" + ex.user().username + "': "
                    + (u.allLibraries ? "all" : String.join(", ", u.libraryIds)));
        }
        String password = Json.str(body, "password", "");
        if (!password.isEmpty()) {
            u.passwordHash = PasswordHasher.hash(validPassword(password, u.username));
            u.passwordChangedAt = System.currentTimeMillis();
            endSessions = true;
        }
        if (Json.bool(body, "resetTwoFactor", false)) {
            u.totpEnabled = false;
            u.totpSecret = null;
            u.recoveryCodes = new ArrayList<>();
            endSessions = true;
        }
        store.updateUser(u);
        if (endSessions) store.removeSessionsOf(u.id, u.id.equals(ex.user().id) ? ex.token() : null);
        LOG.info("User '" + u.username + "' changed by '" + ex.user().username + "'");
        ex.ok(u.toPublic());
    }

    private void deleteUser(Exchange ex) throws IOException {
        String id = ex.param("id");
        if (id.equals(ex.user().id)) throw ApiException.badRequest("Du kannst dich nicht selbst löschen");
        User u = store.user(id).orElseThrow(ApiException::notFound);
        store.removeUser(id);
        onUserDeleted.accept(id);
        LOG.info("User '" + u.username + "' deleted by '" + ex.user().username + "'");
        ex.noContent();
    }

    /**
     * Sets which libraries a user may see: {@code "all"} (also libraries added later) or a list of
     * library ids. Admins always see everything, whatever is stored here.
     */
    private void applyLibraries(User u, Object value) {
        if ("all".equals(value)) {
            u.allLibraries = true;
            u.libraryIds = java.util.Set.of();
            return;
        }
        if (!(value instanceof List<?> list)) throw ApiException.badRequest("libraries: \"all\" oder eine Liste von Bibliotheken erwartet");
        java.util.Set<String> ids = new java.util.LinkedHashSet<>();
        for (Object o : list) {
            String id = String.valueOf(o);
            if (store.library(id).isEmpty()) throw ApiException.badRequest("Unbekannte Bibliothek " + id);
            ids.add(id);
        }
        u.allLibraries = false;
        u.libraryIds = java.util.Set.copyOf(ids);
    }

    // ================================================================== validation

    static String validUsername(String s) {
        String u = s.trim();
        if (!u.matches("[\\p{L}\\p{N}._ -]{2,32}")) {
            throw ApiException.badRequest("Benutzername: 2–32 Zeichen, Buchstaben, Ziffern, Punkt, Strich, Leerzeichen");
        }
        return u;
    }

    String validPassword(String s, String username) {
        if (s.length() < minPasswordLength) {
            throw ApiException.badRequest("Das Passwort muss mindestens " + minPasswordLength + " Zeichen lang sein");
        }
        if (s.length() > 256) throw ApiException.badRequest("Das Passwort ist zu lang");
        if (s.equalsIgnoreCase(username)) throw ApiException.badRequest("Das Passwort darf nicht der Benutzername sein");
        if (s.chars().distinct().count() < 4) throw ApiException.badRequest("Das Passwort ist zu einfach");
        return s;
    }
}

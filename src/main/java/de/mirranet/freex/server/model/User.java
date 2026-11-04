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

/**
 * A server account. The password is only stored as a PBKDF2 hash, recovery codes as SHA-256
 * hashes. The TOTP secret has to be stored in a usable form; the data files are therefore
 * written readable for the server user only.
 */
public final class User {

    public final String id;
    public final String username;
    public volatile String passwordHash;
    public volatile boolean admin;
    public final long createdAt;
    public volatile long passwordChangedAt;

    /** Base32 TOTP secret; set while two-factor setup is pending or active. */
    public volatile String totpSecret;
    public volatile boolean totpEnabled;
    /** Time step of the last accepted code, to reject replays. */
    public volatile long totpLastStep;
    /** SHA-256 hashes of unused recovery codes. */
    public volatile List<String> recoveryCodes = new ArrayList<>();

    /** Changes with every new profile picture (cache key); 0 = none. */
    public volatile long avatarVersion;
    /** Preferred audio language as ISO 639-1 code; empty = the file's default track. */
    public volatile String audioLanguage = "";
    /** When subtitles are switched on by default, see {@link #SUBTITLE_MODES}. */
    public volatile String subtitleMode = "forced";
    /** Preferred subtitle language as ISO 639-1 code; empty = same as the audio preference. */
    public volatile String subtitleLanguage = "";

    /**
     * off: never; forced: only forced subtitles (foreign-language parts); foreign: when the audio
     * is not in the subtitle language; always: whenever subtitles in that language exist.
     */
    public static final List<String> SUBTITLE_MODES = List.of("off", "forced", "foreign", "always");

    public User(String id, String username, String passwordHash, boolean admin, long createdAt) {
        this.id = id;
        this.username = username;
        this.passwordHash = passwordHash;
        this.admin = admin;
        this.createdAt = createdAt;
        this.passwordChangedAt = createdAt;
    }

    /** Full record for persistence, including secrets. */
    public Map<String, Object> toStorage() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", id);
        m.put("username", username);
        m.put("passwordHash", passwordHash);
        m.put("admin", admin);
        m.put("createdAt", createdAt);
        m.put("passwordChangedAt", passwordChangedAt);
        m.put("totpSecret", totpSecret);
        m.put("totpEnabled", totpEnabled);
        m.put("totpLastStep", totpLastStep);
        m.put("recoveryCodes", recoveryCodes);
        return m;
    }

    /** Public view for the API, without any secret. */
    public Map<String, Object> toPublic() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", id);
        m.put("username", username);
        m.put("admin", admin);
        m.put("createdAt", createdAt);
        m.put("twoFactor", totpEnabled);
        m.put("recoveryCodesLeft", totpEnabled ? recoveryCodes.size() : 0);
        m.put("passwordChangedAt", passwordChangedAt);
        m.put("avatar", avatarVersion);
        m.put("preferences", preferences());
        return m;
    }

    public Map<String, Object> preferences() {
        Map<String, Object> p = new LinkedHashMap<>();
        p.put("audioLanguage", audioLanguage);
        p.put("subtitleMode", subtitleMode);
        p.put("subtitleLanguage", subtitleLanguage);
        return p;
    }

    public static User fromStorage(Map<String, Object> m) {
        User u = new User(Json.str(m, "id", ""), Json.str(m, "username", ""), Json.str(m, "passwordHash", ""),
                Json.bool(m, "admin", false), Json.num(m, "createdAt", 0));
        u.passwordChangedAt = Json.num(m, "passwordChangedAt", u.createdAt);
        u.totpSecret = Json.str(m, "totpSecret", null);
        u.totpEnabled = Json.bool(m, "totpEnabled", false) && u.totpSecret != null;
        u.totpLastStep = Json.num(m, "totpLastStep", 0);
        List<String> codes = new ArrayList<>();
        for (Object o : Json.list(m.get("recoveryCodes"))) codes.add(String.valueOf(o));
        u.recoveryCodes = codes;
        return u;
    }
}

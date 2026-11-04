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


import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * A server account. The password is only stored as a PBKDF2 hash, recovery codes as SHA-256
 * hashes. The TOTP secret has to be stored in a usable form, which is why the database is
 * readable for the server user only.
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
    /** Display language of the web interface and server messages; empty = the browser's language. */
    public volatile String uiLanguage = "";
    /** Sidebar entries the user removed from the pinned ones, such as {@code history} or {@code library:<id>}. */
    public volatile List<String> unpinned = List.of();

    /** True: may see all libraries, including ones added later. Admins always may. */
    public volatile boolean allLibraries = true;
    /** The libraries this user may see when {@link #allLibraries} is false. */
    public volatile Set<String> libraryIds = Set.of();

    /** Whether this user may see the library with the given id. */
    public boolean mayAccess(String libraryId) {
        return admin || allLibraries || libraryIds.contains(libraryId);
    }

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
        m.put("libraries", admin || allLibraries ? "all" : new ArrayList<>(libraryIds));
        m.put("preferences", preferences());
        return m;
    }

    public Map<String, Object> preferences() {
        Map<String, Object> p = new LinkedHashMap<>();
        p.put("audioLanguage", audioLanguage);
        p.put("subtitleMode", subtitleMode);
        p.put("subtitleLanguage", subtitleLanguage);
        p.put("uiLanguage", uiLanguage);
        p.put("unpinned", unpinned);
        return p;
    }
}

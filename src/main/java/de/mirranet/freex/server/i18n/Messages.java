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

package de.mirranet.freex.server.i18n;

import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Properties;

/**
 * The message catalog of the server: texts it sends to users (errors, row titles of the home
 * screen, playback details), in {@code i18n/messages_<language>.properties}. A text missing in a
 * language falls back to English. Placeholders are written {@code {0}}, {@code {1}}, …
 */
public final class Messages {

    public static final String DEFAULT = "en";

    /** Display languages users can choose, ISO 639-1. */
    public static final List<String> LANGUAGES = List.of("en", "de");

    private static final Map<String, Properties> CATALOGS = new HashMap<>();

    static {
        for (String lang : LANGUAGES) CATALOGS.put(lang, load(lang));
    }

    private Messages() {
    }

    private static Properties load(String lang) {
        String name = "/i18n/messages_" + lang + ".properties";
        Properties p = new Properties();
        try (InputStream in = Messages.class.getResourceAsStream(name)) {
            if (in == null) throw new IllegalStateException("Message catalog missing: " + name);
            p.load(new InputStreamReader(in, StandardCharsets.UTF_8));
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return p;
    }

    public static boolean supported(String lang) {
        return LANGUAGES.contains(lang);
    }

    public static String get(String lang, Text text) {
        return get(lang, text.key(), text.args());
    }

    public static String get(String lang, String key, Object... args) {
        String pattern = CATALOGS.getOrDefault(lang, CATALOGS.get(DEFAULT)).getProperty(key);
        if (pattern == null) pattern = CATALOGS.get(DEFAULT).getProperty(key, key);
        if (args.length == 0) return pattern;
        StringBuilder sb = new StringBuilder(pattern.length() + 32);
        for (int i = 0; i < pattern.length(); i++) {
            char c = pattern.charAt(i);
            int end = c == '{' ? pattern.indexOf('}', i) : -1;
            if (end > i + 1 && end - i <= 3 && Character.isDigit(pattern.charAt(i + 1))) {
                int n = Integer.parseInt(pattern.substring(i + 1, end));
                Object a = n < args.length ? args[n] : "";
                sb.append(a instanceof Text t ? get(lang, t) : String.valueOf(a));
                i = end;
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }

    /**
     * The best supported language for an {@code Accept-Language} header, such as
     * {@code de-DE,de;q=0.9,en;q=0.8}; English if none matches.
     */
    public static String negotiate(String acceptLanguage) {
        if (acceptLanguage == null) return DEFAULT;
        String best = DEFAULT;
        double bestQ = -1;
        for (String part : acceptLanguage.split(",")) {
            String[] p = part.trim().split(";");
            String lang = p[0].trim().toLowerCase(Locale.ROOT).split("-")[0];
            double q = 1;
            for (int i = 1; i < p.length; i++) {
                String s = p[i].trim();
                if (s.startsWith("q=")) {
                    try {
                        q = Double.parseDouble(s.substring(2));
                    } catch (NumberFormatException e) {
                        q = 0;
                    }
                }
            }
            if (supported(lang) && q > bestQ) {
                best = lang;
                bestQ = q;
            }
        }
        return best;
    }
}

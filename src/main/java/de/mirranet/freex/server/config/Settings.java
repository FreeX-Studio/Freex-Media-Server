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

package de.mirranet.freex.server.config;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.PosixFilePermission;
import java.nio.file.attribute.PosixFilePermissions;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

/**
 * The settings an administrator can change in the web interface, with their validation, and
 * the code that writes changed values back into {@code config.yml}.
 * <p>
 * Writing changes only the value of the affected lines; comments, order and keys the web
 * interface does not know stay as they are. Keys missing in the file are added to their section.
 * <p>
 * Paths the server executes (ffmpeg, ffprobe) or keeps its state in (data and cache directory)
 * are deliberately read-only here: a stolen admin session must not be able to make the server
 * run another program. They can only be changed in the file itself. The transcoding directory
 * is editable (it is often moved to a fast disk): the server only ever deletes its own session
 * folders in it, and {@code SettingsApi} checks that it is writable before saving.
 */
public final class Settings {

    public enum Type { TEXT, NUMBER, BOOL, CHOICE, SECRET, READONLY }

    /** One setting. {@code min}/{@code max} apply to numbers, {@code pattern} to text. */
    public record Field(String key, Type type, int min, int max, List<String> choices, Pattern pattern,
                        String patternMessage, int maxLength) {

        public Map<String, Object> toPublic() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("key", key);
            m.put("type", type.name().toLowerCase());
            if (type == Type.NUMBER) {
                m.put("min", min);
                m.put("max", max);
            }
            if (type == Type.CHOICE) m.put("choices", choices);
            return m;
        }
    }

    private static Field text(String key, int maxLength, String pattern, String patternMessage) {
        return new Field(key, Type.TEXT, 0, 0, List.of(), pattern == null ? null : Pattern.compile(pattern),
                patternMessage, maxLength);
    }

    private static Field number(String key, int min, int max) {
        return new Field(key, Type.NUMBER, min, max, List.of(), null, null, 0);
    }

    private static Field of(String key, Type type) {
        return new Field(key, type, 0, 0, List.of(), null, null, type == Type.SECRET ? 512 : 0);
    }

    private static Field choice(String key, String... choices) {
        return new Field(key, Type.CHOICE, 0, 0, List.of(choices), null, null, 0);
    }

    /** All settings in the order of the web interface. */
    public static final List<Field> FIELDS = List.of(
            text("server.name", 60, null, null),
            text("server.bindAddress", 100, "[0-9A-Za-z.:%-]+", "Ungültige Adresse"),
            number("server.port", 1, 65535),
            text("server.tlsKeystore", 500, null, null),
            of("server.tlsKeystorePassword", Type.SECRET),
            of("server.trustProxy", Type.BOOL),
            of("data.directory", Type.READONLY),
            of("data.cacheDirectory", Type.READONLY),
            of("ffmpeg.ffprobe", Type.READONLY),
            of("ffmpeg.ffmpeg", Type.READONLY),
            number("security.sessionDays", 1, 365),
            number("security.minPasswordLength", 8, 128),
            of("tmdb.apiKey", Type.SECRET),
            text("tmdb.language", 10, "[a-z]{2}(-[A-Z]{2})?", "Sprache im Format de-DE oder en"),
            text("tmdb.region", 2, "[A-Z]{2}", "Land als Kürzel mit zwei Großbuchstaben, z. B. DE"),
            of("markers.enabled", Type.BOOL),
            of("transcoder.enabled", Type.BOOL),
            text("transcoder.directory", 500, null, null),
            number("transcoder.maxSessions", 1, 32),
            choice("transcoder.hardwareAcceleration", "none", "nvenc", "qsv", "vaapi"),
            text("transcoder.vaapiDevice", 100, "/dev/dri/[A-Za-z0-9]+", "Pfad unter /dev/dri, z. B. /dev/dri/renderD128"),
            choice("transcoder.preset", "ultrafast", "superfast", "veryfast", "faster", "fast", "medium"),
            number("scanner.intervalMinutes", 0, 10080),
            number("scanner.threads", 1, 32));

    private static final Map<String, Field> BY_KEY = new HashMap<>();

    static {
        for (Field f : FIELDS) BY_KEY.put(f.key, f);
    }

    private Settings() {
    }

    public static Field field(String key) {
        return BY_KEY.get(key);
    }

    /**
     * Checks one submitted value and returns it in its normalized form.
     *
     * @throws IllegalArgumentException with a message for the user
     */
    public static String validate(Field f, Object value) {
        switch (f.type) {
            case READONLY -> throw new IllegalArgumentException(f.key + " kann nur in der config.yml geändert werden");
            case BOOL -> {
                if (value instanceof Boolean b) return b.toString();
                throw new IllegalArgumentException(f.key + ": Ja oder Nein erwartet");
            }
            case NUMBER -> {
                long n;
                if (value instanceof Number num && num.doubleValue() == Math.rint(num.doubleValue())) {
                    n = num.longValue();
                } else {
                    try {
                        n = Long.parseLong(String.valueOf(value).trim());
                    } catch (NumberFormatException e) {
                        throw new IllegalArgumentException(f.key + ": ganze Zahl erwartet");
                    }
                }
                if (n < f.min || n > f.max) {
                    throw new IllegalArgumentException(f.key + ": erlaubt sind " + f.min + " bis " + f.max);
                }
                return Long.toString(n);
            }
            case CHOICE -> {
                String s = String.valueOf(value);
                if (!f.choices.contains(s)) throw new IllegalArgumentException(f.key + ": erlaubt sind " + String.join(", ", f.choices));
                return s;
            }
            default -> {
                if (!(value instanceof String s)) throw new IllegalArgumentException(f.key + ": Text erwartet");
                if (f.type == Type.TEXT) s = s.trim();
                for (int i = 0; i < s.length(); i++) {
                    if (Character.isISOControl(s.charAt(i))) throw new IllegalArgumentException(f.key + ": Steuerzeichen sind nicht erlaubt");
                }
                if (f.maxLength > 0 && s.length() > f.maxLength) {
                    throw new IllegalArgumentException(f.key + ": höchstens " + f.maxLength + " Zeichen");
                }
                if (f.pattern != null && !s.isEmpty() && !f.pattern.matcher(s).matches()) {
                    throw new IllegalArgumentException(f.key + ": " + f.patternMessage);
                }
                if (f.key.equals("server.name") && s.isEmpty()) throw new IllegalArgumentException("Der Servername darf nicht leer sein");
                if (f.key.equals("server.bindAddress") && s.isEmpty()) throw new IllegalArgumentException("Die Adresse darf nicht leer sein");
                if (f.key.equals("transcoder.directory") && s.isEmpty()) throw new IllegalArgumentException("Der Umwandlungsordner darf nicht leer sein");
                return s;
            }
        }
    }

    /** The value as it is written into the file. */
    static String scalar(Field f, String value) {
        return switch (f.type) {
            case BOOL, NUMBER -> value;
            default -> Configuration.quote(value);
        };
    }

    /** The values in a configuration text, with defaults for missing keys. */
    public static Map<String, String> effective(String text) {
        Map<String, String> out = new LinkedHashMap<>(Configuration.defaults());
        out.putAll(Configuration.parse(text));
        return out;
    }

    /** Reads the file, applies the changes (dotted key to normalized value) and replaces it atomically. */
    public static void write(Path file, Map<String, String> changes) throws IOException {
        String text = Files.exists(file) ? Files.readString(file, StandardCharsets.UTF_8) : "";
        String updated = apply(text, changes);
        Path tmp = file.resolveSibling(file.getFileName() + ".tmp");
        Files.deleteIfExists(tmp);
        // The file holds secrets: the new copy starts private and then gets the old file's permissions.
        Set<PosixFilePermission> perms = null;
        try {
            if (Files.exists(file)) perms = Files.getPosixFilePermissions(file);
            Files.createFile(tmp, PosixFilePermissions.asFileAttribute(PosixFilePermissions.fromString("rw-------")));
        } catch (UnsupportedOperationException e) {
            perms = null; // Windows: no POSIX permissions
        }
        Files.writeString(tmp, updated, StandardCharsets.UTF_8);
        if (perms != null) Files.setPosixFilePermissions(tmp, perms);
        try {
            Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        } catch (AtomicMoveNotSupportedException e) {
            Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING);
        }
    }

    /**
     * Returns the text with the given values set. Existing lines keep their indentation and
     * trailing comment; missing keys are added at the end of their section (or a new section).
     */
    static String apply(String text, Map<String, String> changes) {
        String nl = text.contains("\r\n") ? "\r\n" : "\n";
        List<String> lines = new ArrayList<>(List.of(text.split("\r?\n", -1)));
        if (!lines.isEmpty() && lines.get(lines.size() - 1).isEmpty()) lines.remove(lines.size() - 1);

        Map<String, Integer> keyLine = new HashMap<>();
        Map<String, Integer> sectionEnd = new HashMap<>();
        Map<String, Integer> sectionIndent = new HashMap<>();
        Deque<Map.Entry<Integer, String>> path = new ArrayDeque<>();
        for (int i = 0; i < lines.size(); i++) {
            String content = Configuration.stripComment(lines.get(i));
            if (content.isBlank()) continue;
            int indent = 0;
            while (indent < content.length() && content.charAt(indent) == ' ') indent++;
            String trimmed = content.trim();
            int colon = trimmed.indexOf(':');
            if (colon <= 0) continue;
            while (!path.isEmpty() && path.peek().getKey() >= indent) path.pop();
            StringBuilder prefix = new StringBuilder();
            path.descendingIterator().forEachRemaining(e -> prefix.append(e.getValue()).append('.'));
            String key = trimmed.substring(0, colon).trim();
            String full = prefix + key;
            for (Map.Entry<Integer, String> e : path) {
                // every enclosing section now ends at this line at the earliest
                String section = sectionName(path, e);
                sectionEnd.put(section, i);
            }
            if (trimmed.substring(colon + 1).isBlank()) {
                path.push(Map.entry(indent, key));
                sectionEnd.put(full, i);
                sectionIndent.put(full, indent);
            } else {
                keyLine.put(full, i);
            }
        }

        // Replace existing lines.
        Map<String, List<String>> missing = new LinkedHashMap<>();
        for (Map.Entry<String, String> c : changes.entrySet()) {
            Field f = BY_KEY.get(c.getKey());
            String value = f == null ? Configuration.quote(c.getValue()) : scalar(f, c.getValue());
            Integer at = keyLine.get(c.getKey());
            int dot = c.getKey().lastIndexOf('.');
            String name = c.getKey().substring(dot + 1);
            if (at != null) {
                String raw = lines.get(at);
                String content = Configuration.stripComment(raw);
                String comment = raw.substring(content.length());
                int indent = 0;
                while (indent < raw.length() && raw.charAt(indent) == ' ') indent++;
                lines.set(at, " ".repeat(indent) + name + ": " + value + (comment.isBlank() ? "" : " " + comment.trim()));
            } else {
                missing.computeIfAbsent(dot < 0 ? "" : c.getKey().substring(0, dot), k -> new ArrayList<>())
                        .add(name + ": " + value);
            }
        }

        // Insert missing keys, bottom-up so earlier line numbers stay valid.
        List<Map.Entry<String, List<String>>> inserts = new ArrayList<>(missing.entrySet());
        inserts.sort((a, b) -> Integer.compare(sectionEnd.getOrDefault(b.getKey(), Integer.MAX_VALUE),
                sectionEnd.getOrDefault(a.getKey(), Integer.MAX_VALUE)));
        List<String> appended = new ArrayList<>();
        for (Map.Entry<String, List<String>> e : inserts) {
            Integer end = sectionEnd.get(e.getKey());
            if (end == null || e.getKey().isEmpty()) {
                if (!e.getKey().isEmpty()) {
                    appended.add("");
                    appended.add(e.getKey() + ":");
                    for (String l : e.getValue()) appended.add("  " + l);
                } else {
                    appended.addAll(e.getValue());
                }
                continue;
            }
            String pad = " ".repeat(sectionIndent.getOrDefault(e.getKey(), 0) + 2);
            List<String> add = e.getValue().stream().map(l -> pad + l).toList();
            lines.addAll(end + 1, add);
        }
        lines.addAll(appended);
        return String.join(nl, lines) + nl;
    }

    private static String sectionName(Deque<Map.Entry<Integer, String>> path, Map.Entry<Integer, String> upTo) {
        List<String> parts = new ArrayList<>();
        for (var it = path.descendingIterator(); it.hasNext(); ) {
            Map.Entry<Integer, String> e = it.next();
            parts.add(e.getValue());
            if (e == upTo) break;
        }
        return String.join(".", parts);
    }
}

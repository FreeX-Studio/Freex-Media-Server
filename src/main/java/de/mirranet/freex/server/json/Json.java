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

package de.mirranet.freex.server.json;

import java.util.ArrayList;
import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Minimal JSON reader and writer, so the server needs no external library.
 * <p>
 * Parsing maps JSON objects to {@link LinkedHashMap}, arrays to {@link ArrayList}, numbers to
 * {@link Long} (integers) or {@link Double}, plus {@link String}, {@link Boolean} and {@code null}.
 * Writing accepts the same types, any {@link Map}, {@link Collection}, {@link Number},
 * {@link Enum} and objects implementing {@link JsonWritable}.
 */
public final class Json {

    private Json() {
    }

    /** Objects that know how to turn themselves into a JSON-compatible value. */
    public interface JsonWritable {
        Object toJson();
    }

    // ------------------------------------------------------------------ writing

    public static String write(Object value) {
        StringBuilder sb = new StringBuilder();
        writeValue(sb, value);
        return sb.toString();
    }

    private static void writeValue(StringBuilder sb, Object value) {
        if (value == null) {
            sb.append("null");
        } else if (value instanceof JsonWritable w) {
            writeValue(sb, w.toJson());
        } else if (value instanceof String s) {
            writeString(sb, s);
        } else if (value instanceof Boolean b) {
            sb.append(b);
        } else if (value instanceof Double d) {
            sb.append(d.isNaN() || d.isInfinite() ? "null" : stripZero(d));
        } else if (value instanceof Float f) {
            writeValue(sb, f.doubleValue());
        } else if (value instanceof Number n) {
            sb.append(n);
        } else if (value instanceof Enum<?> e) {
            writeString(sb, e.name().toLowerCase());
        } else if (value instanceof Map<?, ?> map) {
            sb.append('{');
            boolean first = true;
            for (Map.Entry<?, ?> e : map.entrySet()) {
                if (!first) sb.append(',');
                first = false;
                writeString(sb, String.valueOf(e.getKey()));
                sb.append(':');
                writeValue(sb, e.getValue());
            }
            sb.append('}');
        } else if (value instanceof Collection<?> list) {
            sb.append('[');
            boolean first = true;
            for (Object o : list) {
                if (!first) sb.append(',');
                first = false;
                writeValue(sb, o);
            }
            sb.append(']');
        } else if (value instanceof Object[] arr) {
            writeValue(sb, List.of(arr));
        } else {
            writeString(sb, value.toString());
        }
    }

    private static String stripZero(double d) {
        if (d == Math.rint(d) && Math.abs(d) < 1e15) {
            return Long.toString((long) d);
        }
        return Double.toString(d);
    }

    private static void writeString(StringBuilder sb, String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> sb.append("\\\"");
                case '\\' -> sb.append("\\\\");
                case '\n' -> sb.append("\\n");
                case '\r' -> sb.append("\\r");
                case '\t' -> sb.append("\\t");
                case '\b' -> sb.append("\\b");
                case '\f' -> sb.append("\\f");
                default -> {
                    // Escape control chars and the line separators that break JavaScript.
                    if (c < 0x20 || c == ' ' || c == ' ') {
                        sb.append(String.format("\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
                }
            }
        }
        sb.append('"');
    }

    // ------------------------------------------------------------------ reading

    public static Object parse(String text) {
        Parser p = new Parser(text);
        p.skipWs();
        Object v = p.readValue(0);
        p.skipWs();
        if (p.pos != text.length()) {
            throw p.error("Unexpected trailing content");
        }
        return v;
    }

    /** Parses text that must be a JSON object. */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> parseObject(String text) {
        Object v = parse(text);
        if (!(v instanceof Map<?, ?>)) {
            throw new JsonException("Expected a JSON object");
        }
        return (Map<String, Object>) v;
    }

    public static final class JsonException extends RuntimeException {

        private static final long serialVersionUID = 1L;
        public JsonException(String message) {
            super(message);
        }
    }

    private static final class Parser {
        private static final int MAX_DEPTH = 256;
        private final String s;
        private int pos;

        Parser(String s) {
            this.s = s;
        }

        JsonException error(String msg) {
            return new JsonException(msg + " at position " + pos);
        }

        void skipWs() {
            while (pos < s.length()) {
                char c = s.charAt(pos);
                if (c == ' ' || c == '\n' || c == '\r' || c == '\t') pos++;
                else break;
            }
        }

        Object readValue(int depth) {
            if (depth > MAX_DEPTH) throw error("Nesting too deep");
            if (pos >= s.length()) throw error("Unexpected end");
            char c = s.charAt(pos);
            return switch (c) {
                case '{' -> readObject(depth);
                case '[' -> readArray(depth);
                case '"' -> readString();
                case 't' -> literal("true", Boolean.TRUE);
                case 'f' -> literal("false", Boolean.FALSE);
                case 'n' -> literal("null", null);
                default -> {
                    if (c == '-' || (c >= '0' && c <= '9')) yield readNumber();
                    throw error("Unexpected character '" + c + "'");
                }
            };
        }

        Object literal(String word, Object value) {
            if (!s.startsWith(word, pos)) throw error("Invalid literal");
            pos += word.length();
            return value;
        }

        Map<String, Object> readObject(int depth) {
            Map<String, Object> map = new LinkedHashMap<>();
            pos++; // {
            skipWs();
            if (pos < s.length() && s.charAt(pos) == '}') {
                pos++;
                return map;
            }
            while (true) {
                skipWs();
                if (pos >= s.length() || s.charAt(pos) != '"') throw error("Expected key");
                String key = readString();
                skipWs();
                if (pos >= s.length() || s.charAt(pos) != ':') throw error("Expected ':'");
                pos++;
                skipWs();
                map.put(key, readValue(depth + 1));
                skipWs();
                if (pos >= s.length()) throw error("Unexpected end");
                char c = s.charAt(pos++);
                if (c == '}') return map;
                if (c != ',') throw error("Expected ',' or '}'");
            }
        }

        List<Object> readArray(int depth) {
            List<Object> list = new ArrayList<>();
            pos++; // [
            skipWs();
            if (pos < s.length() && s.charAt(pos) == ']') {
                pos++;
                return list;
            }
            while (true) {
                skipWs();
                list.add(readValue(depth + 1));
                skipWs();
                if (pos >= s.length()) throw error("Unexpected end");
                char c = s.charAt(pos++);
                if (c == ']') return list;
                if (c != ',') throw error("Expected ',' or ']'");
            }
        }

        String readString() {
            pos++; // opening quote
            StringBuilder sb = new StringBuilder();
            while (true) {
                if (pos >= s.length()) throw error("Unterminated string");
                char c = s.charAt(pos++);
                if (c == '"') return sb.toString();
                if (c == '\\') {
                    if (pos >= s.length()) throw error("Bad escape");
                    char e = s.charAt(pos++);
                    switch (e) {
                        case '"', '\\', '/' -> sb.append(e);
                        case 'n' -> sb.append('\n');
                        case 'r' -> sb.append('\r');
                        case 't' -> sb.append('\t');
                        case 'b' -> sb.append('\b');
                        case 'f' -> sb.append('\f');
                        case 'u' -> {
                            if (pos + 4 > s.length()) throw error("Bad unicode escape");
                            try {
                                sb.append((char) Integer.parseInt(s.substring(pos, pos + 4), 16));
                            } catch (NumberFormatException ex) {
                                throw error("Bad unicode escape");
                            }
                            pos += 4;
                        }
                        default -> throw error("Bad escape");
                    }
                } else {
                    sb.append(c);
                }
            }
        }

        Number readNumber() {
            int start = pos;
            if (s.charAt(pos) == '-') pos++;
            boolean decimal = false;
            while (pos < s.length()) {
                char c = s.charAt(pos);
                if (c >= '0' && c <= '9') {
                    pos++;
                } else if (c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-') {
                    decimal = true;
                    pos++;
                } else {
                    break;
                }
            }
            String num = s.substring(start, pos);
            try {
                if (!decimal) {
                    return Long.parseLong(num);
                }
                return Double.parseDouble(num);
            } catch (NumberFormatException ex) {
                try {
                    return Double.parseDouble(num);
                } catch (NumberFormatException ex2) {
                    throw error("Bad number '" + num + "'");
                }
            }
        }
    }

    // ------------------------------------------------------------------ helpers for reading maps

    public static String str(Map<String, Object> m, String key, String def) {
        Object v = m.get(key);
        return v == null ? def : v.toString();
    }

    public static long num(Map<String, Object> m, String key, long def) {
        Object v = m.get(key);
        if (v instanceof Number n) return n.longValue();
        if (v instanceof String s) {
            try {
                return Long.parseLong(s.trim());
            } catch (NumberFormatException ignored) {
                // fall through
            }
        }
        return def;
    }

    public static boolean bool(Map<String, Object> m, String key, boolean def) {
        Object v = m.get(key);
        if (v instanceof Boolean b) return b;
        if (v instanceof String s) return Boolean.parseBoolean(s);
        return def;
    }

    @SuppressWarnings("unchecked")
    public static Map<String, Object> obj(Object v) {
        return v instanceof Map<?, ?> ? (Map<String, Object>) v : new LinkedHashMap<>();
    }

    @SuppressWarnings("unchecked")
    public static List<Object> list(Object v) {
        return v instanceof List<?> ? (List<Object>) v : new ArrayList<>();
    }
}

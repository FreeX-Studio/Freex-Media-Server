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

package de.mirranet.freex.server.scan;

import de.mirranet.freex.server.model.MediaItem;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.logging.Logger;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Reads local metadata from Kodi-style {@code .nfo} files ({@code movie.nfo}, {@code <name>.nfo},
 * {@code tvshow.nfo}). This gives titles, plots and genres without any online service.
 * <p>
 * Only simple element text is read with regular expressions; this is deliberately lenient so that
 * hand-written or slightly broken files still work. A file that is just a URL is ignored.
 */
final class NfoReader {

    private static final Logger LOG = Logger.getLogger(NfoReader.class.getName());
    private static final long MAX_SIZE = 2 * 1024 * 1024;

    private NfoReader() {
    }

    /** Applies the values found in the file to the item; missing elements leave fields unchanged. */
    static void apply(Path nfo, MediaItem item) {
        String xml;
        try {
            if (Files.size(nfo) > MAX_SIZE) return;
            xml = Files.readString(nfo, StandardCharsets.UTF_8);
        } catch (IOException | java.io.UncheckedIOException e) {
            try {
                xml = Files.readString(nfo, StandardCharsets.ISO_8859_1);
            } catch (IOException ex) {
                LOG.fine("Cannot read " + nfo + ": " + ex.getMessage());
                return;
            }
        }
        if (!xml.contains("<")) return;
        item.hasNfo = true;
        String tmdb = element(xml, "tmdbid");
        if (tmdb.isEmpty()) {
            Matcher um = Pattern.compile("<uniqueid[^>]*type=\"tmdb\"[^>]*>(\\d+)</uniqueid>", Pattern.CASE_INSENSITIVE).matcher(xml);
            if (um.find()) tmdb = um.group(1);
        }
        try {
            if (!tmdb.isEmpty()) item.tmdbHint = Integer.parseInt(tmdb.trim());
        } catch (NumberFormatException ignored) {
            // not a number
        }

        String title = element(xml, "title");
        if (!title.isEmpty()) {
            item.title = title;
            item.sortTitle = NameParser.sortTitle(title);
        }
        String sort = element(xml, "sorttitle");
        if (!sort.isEmpty()) item.sortTitle = sort.toLowerCase();
        String original = element(xml, "originaltitle");
        if (!original.isEmpty() && !original.equals(item.title)) item.originalTitle = original;
        String plot = element(xml, "plot");
        if (plot.isEmpty()) plot = element(xml, "outline");
        if (!plot.isEmpty()) item.summary = plot;
        String tagline = element(xml, "tagline");
        if (!tagline.isEmpty()) item.tagline = tagline;
        String year = element(xml, "year");
        if (year.isEmpty()) {
            String premiered = element(xml, "premiered");
            if (premiered.length() >= 4) year = premiered.substring(0, 4);
        }
        try {
            if (!year.isEmpty()) item.year = Integer.parseInt(year.trim());
        } catch (NumberFormatException ignored) {
            // keep the year from the file name
        }
        List<String> genres = elements(xml, "genre");
        if (!genres.isEmpty()) item.genres = String.join(", ", genres);
        // New Kodi format: <ratings><rating name=".."><value>7.8</value>..., old format: <rating>7.8</rating>
        String rating = element(xml, "value");
        if (rating.isEmpty()) rating = element(xml, "rating");
        try {
            if (!rating.isEmpty()) item.rating = Math.round(Double.parseDouble(rating.replace(',', '.')) * 10) / 10.0;
        } catch (NumberFormatException ignored) {
            // no usable rating
        }
    }

    private static String element(String xml, String name) {
        Matcher m = pattern(name).matcher(xml);
        return m.find() ? decode(m.group(1)) : "";
    }

    private static List<String> elements(String xml, String name) {
        List<String> out = new ArrayList<>();
        Matcher m = pattern(name).matcher(xml);
        while (m.find()) {
            String v = decode(m.group(1));
            if (!v.isEmpty() && !out.contains(v)) out.add(v);
        }
        return out;
    }

    private static Pattern pattern(String name) {
        return Pattern.compile("<" + name + "(?:\\s[^>]*)?>(.*?)</" + name + ">", Pattern.DOTALL | Pattern.CASE_INSENSITIVE);
    }

    private static String decode(String s) {
        String r = s.trim();
        if (r.startsWith("<![CDATA[") && r.endsWith("]]>")) r = r.substring(9, r.length() - 3);
        r = r.replaceAll("<[^>]+>", "");
        return r.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"")
                .replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&").trim();
    }
}

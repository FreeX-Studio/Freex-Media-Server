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

import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Turns file and folder names into titles, years and episode numbers.
 * <p>
 * Understands the common naming schemes: {@code Title (Year)}, scene names like
 * {@code Title.2010.1080p.BluRay.x264}, episodes as {@code S01E02}, {@code S01E02-E03},
 * {@code 1x02}, and season folders named {@code Season 1}, {@code Staffel 1} or {@code Specials}.
 */
public final class NameParser {

    private NameParser() {
    }

    /** Title and optional year (0 if none). */
    public record TitleYear(String title, int year) {
    }

    /** Season and episode numbers (-1 if unknown) plus the episode title, if any. */
    public record EpisodeInfo(int season, int episode, int episodeEnd, String showPart, String title) {
    }

    private static final Pattern YEAR = Pattern.compile("(?:^|[\\s(\\[._-])((?:19|20)\\d{2})(?=$|[\\s)\\]._-])");

    /** Tokens that start the "technical" part of a scene release name. */
    private static final Pattern RELEASE_TAG = Pattern.compile(
            "(?i)(?:^|[\\s._\\[(-])(2160p|1080[pi]|720p|576p|480p|4k|uhd|hdr10?\\+?|dv|bluray|blu-ray|bdrip|brrip|"
                    + "web-?dl|web-?rip|webrip|hdtv|dvdrip|dvd|remux|x264|x265|h\\.?264|h\\.?265|hevc|avc|xvid|divx|"
                    + "aac|ac3|dts(?:-hd)?|truehd|atmos|ddp?5\\.1|10bit|proper|repack|extended|unrated|"
                    + "german|english|multi|dl|ml|subbed|dubbed)(?=$|[\\s._\\])-])");

    private static final Pattern BRACKETS = Pattern.compile("\\[[^\\]]*]|\\{[^}]*}");

    private static final Pattern SXXEYY = Pattern.compile(
            "(?i)s(\\d{1,2})[\\s._-]*e(\\d{1,3})(?:[\\s._-]*-?[\\s._-]*e(\\d{1,3}))?");
    private static final Pattern NXNN = Pattern.compile("(?i)(?:^|[\\s._\\[(-])(\\d{1,2})x(\\d{2,3})(?=$|[\\s._\\])-])");
    private static final Pattern SEASON_FOLDER = Pattern.compile("(?i)^(?:season|staffel|series|serie|saison|s)[\\s._-]*(\\d{1,3})$");
    private static final Pattern LEADING_NUMBER = Pattern.compile("^(\\d{1,3})(?=[\\s._-]|$)");

    private static final Pattern TMDB_TAG = Pattern.compile("(?i)[\\[{(]\\s*tmdb(?:id)?\\s*[-=:]\\s*(\\d+)\\s*[\\]})]");

    /** TMDB id written into a name, as other media servers support it: {@code Movie (2010) {tmdb-27205}}. */
    public static int tmdbTag(String name) {
        Matcher m = TMDB_TAG.matcher(name);
        return m.find() ? Integer.parseInt(m.group(1)) : 0;
    }

    public static String stripExtension(String fileName) {
        int dot = fileName.lastIndexOf('.');
        return dot > 0 ? fileName.substring(0, dot) : fileName;
    }

    public static String extension(String fileName) {
        int dot = fileName.lastIndexOf('.');
        return dot > 0 ? fileName.substring(dot + 1).toLowerCase(Locale.ROOT) : "";
    }

    /** Parses a movie or show name ({@code The Matrix (1999)}, {@code The.Matrix.1999.1080p}). */
    public static TitleYear titleYear(String name) {
        // Drop bracketed tags, but keep a year written as "[2010]".
        String s = BRACKETS.matcher(TMDB_TAG.matcher(name).replaceAll(" ")).replaceAll(r -> r.group().matches("\\[(?:19|20)\\d{2}]")
                ? " " + r.group().substring(1, 5) + " " : " ");
        s = normalizeSeparators(s);

        int year = 0;
        int cut = -1;
        Matcher ym = YEAR.matcher(s);
        while (ym.find()) {
            // A year at the very start is part of the title ("2001 A Space Odyssey"), keep looking.
            if (ym.start(1) == 0) continue;
            year = Integer.parseInt(ym.group(1));
            cut = ym.start(1);
        }
        Matcher tm = RELEASE_TAG.matcher(s);
        if (tm.find() && tm.start() > 0 && (cut < 0 || tm.start() < cut)) {
            cut = tm.start();
        }
        String title = cut > 0 ? s.substring(0, cut) : s;
        title = cleanup(title);
        if (title.isEmpty()) title = cleanup(s);
        return new TitleYear(title, year);
    }

    /** Season number from a folder name, or -1 when the folder is not a season folder. */
    public static int seasonFromFolder(String folder) {
        String f = folder.trim();
        if (f.equalsIgnoreCase("specials") || f.equalsIgnoreCase("special") || f.equalsIgnoreCase("extras")) {
            return 0;
        }
        Matcher m = SEASON_FOLDER.matcher(f);
        return m.matches() ? Integer.parseInt(m.group(1)) : -1;
    }

    /**
     * Reads season/episode from an episode file name. {@code folderSeason} is used when the file
     * name only carries an episode number ({@code 03 - Title.mkv} inside {@code Season 2}).
     */
    public static EpisodeInfo episode(String fileName, int folderSeason) {
        String base = stripExtension(fileName);
        Matcher m = SXXEYY.matcher(base);
        if (m.find()) {
            int end = m.group(3) != null ? Integer.parseInt(m.group(3)) : -1;
            return new EpisodeInfo(Integer.parseInt(m.group(1)), Integer.parseInt(m.group(2)), end,
                    cleanup(normalizeSeparators(base.substring(0, m.start()))), episodeTitle(base.substring(m.end())));
        }
        m = NXNN.matcher(base);
        if (m.find()) {
            return new EpisodeInfo(Integer.parseInt(m.group(1)), Integer.parseInt(m.group(2)), -1,
                    cleanup(normalizeSeparators(base.substring(0, m.start()))), episodeTitle(base.substring(m.end())));
        }
        if (folderSeason >= 0) {
            String norm = base.trim();
            m = LEADING_NUMBER.matcher(norm);
            if (m.find()) {
                return new EpisodeInfo(folderSeason, Integer.parseInt(m.group(1)), -1, "",
                        episodeTitle(norm.substring(m.end())));
            }
        }
        return new EpisodeInfo(folderSeason, -1, -1, "", episodeTitle(base));
    }

    private static String episodeTitle(String rest) {
        String s = BRACKETS.matcher(rest).replaceAll(" ");
        s = normalizeSeparators(s);
        Matcher tm = RELEASE_TAG.matcher(s);
        if (tm.find()) s = s.substring(0, tm.start());
        return cleanup(s);
    }

    /** Scene names use dots or underscores instead of spaces; turn them into spaces. */
    private static String normalizeSeparators(String s) {
        String r = s.replace('_', ' ');
        // Only treat dots as separators if the name has (almost) no real spaces, so "Mr. Robot" stays intact.
        long spaces = r.chars().filter(c -> c == ' ').count();
        long dots = r.chars().filter(c -> c == '.').count();
        if (dots >= 2 && dots > spaces) {
            r = r.replace('.', ' ').replaceAll("\\s+", " ");
        }
        return r;
    }

    private static String cleanup(String s) {
        String r = s.replaceAll("\\(\\s*\\)", " ").replaceAll("\\s+", " ").trim();
        // Trim separators and stray brackets left over at the edges.
        r = r.replaceAll("^[\\s\\-–._:(\\[]+", "").replaceAll("[\\s\\-–._:(\\[]+$", "");
        return r.trim();
    }

    /** Sort key: lower case without leading articles (English and German). */
    public static String sortTitle(String title) {
        String t = title.trim();
        String lower = t.toLowerCase(Locale.ROOT);
        for (String article : new String[]{"the ", "a ", "an ", "der ", "die ", "das "}) {
            if (lower.startsWith(article) && t.length() > article.length()) {
                return lower.substring(article.length()).trim();
            }
        }
        return lower;
    }
}

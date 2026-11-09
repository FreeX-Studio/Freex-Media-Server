<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# Organizing your media

Add one or more folders per library in *Settings → Libraries*. A library contains either movies or TV shows. The
server scans new libraries right away, rescans all of them on an interval and reuses the results for unchanged
files.

## Folder layout

The layout common to the well-known media servers works here:

```
Movies/
  The Matrix (1999)/
    The Matrix (1999).mkv
    The Matrix (1999).de.srt        subtitles: <name>.<language>[.forced].srt / .vtt
    poster.jpg                      or The Matrix (1999)-poster.jpg
    fanart.jpg                      background image
    movie.nfo                       optional Kodi metadata
  Inception.2010.1080p.BluRay.x264.mkv   scene release names work too

Shows/
  Dark (2017)/
    poster.jpg   fanart.jpg   tvshow.nfo
    season01-poster.jpg
    Season 1/                       also "Staffel 1", "S01", "Specials"
      Dark - S01E01 - Secrets.mkv   also S01E01-E02, 1x01, or "01 - Title.mkv" in a season folder
      Dark - S01E01 - Secrets-thumb.jpg
```

Folders named `Extras`, `Trailers`, `Featurettes`, `Samples` and similar, and folders that contain a `.ignore` file,
are skipped.

## Local metadata and artwork

Kodi `.nfo` files next to the video (or `tvshow.nfo` in the show folder) provide title, original title, plot,
tagline, year, genres, rating and the TMDB id. Local artwork (`poster.jpg`, `folder.jpg`, `fanart.jpg`,
`<name>-poster.jpg`, `<name>-thumb.jpg`, …) is used as well. Local data always takes precedence over online data, so
a library can be run completely offline.

## Online metadata from TMDB

With a free API key from [themoviedb.org](https://www.themoviedb.org/settings/api), the server adds localized titles
and descriptions, posters and backgrounds, episode titles and stills, cast with photos, directors and writers,
studio or network, age ratings and trailers:

```yaml
tmdb:
  apiKey: "your-key-or-read-access-token"
  language: "de-DE"
  region: "DE"
```

After every scan, new or changed movies and shows are matched by title and year. Metadata is refreshed after 30
days; items without a match are retried after a few days. Movie series appear as collections automatically as soon as
two of their movies are in the library.

To pin an item to a specific TMDB entry, put the id into the file or folder name, e.g.
`Inception (2010) {tmdb-27205}`, or into the `.nfo` file (`<tmdbid>` or `<uniqueid type="tmdb">`). Wrong matches can
be corrected with *Fix match* in the item's "…" menu.

This product uses the TMDB API but is not endorsed or certified by TMDB.

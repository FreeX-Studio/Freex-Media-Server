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
    The Matrix (1999)-trailer.mkv   an extra (see below)
    Featurettes/                    extras folder
      Making of.mkv
  Inception.2010.1080p.BluRay.x264.mkv   scene release names work too

Shows/
  Dark (2017)/
    poster.jpg   fanart.jpg   tvshow.nfo
    season01-poster.jpg
    Season 1/                       also "Staffel 1", "S01", "Specials"
      Dark - S01E01 - Secrets.mkv   also S01E01-E02, 1x01, or "01 - Title.mkv" in a season folder
      Dark - S01E01 - Secrets-thumb.jpg
```

Folders named `Samples`, system folders (`@eaDir`, `#recycle`, …) and folders that contain a `.ignore` file are
skipped, as are files with "sample" in the name under 300 MB.

### Several versions of a movie

Files of the same movie in one folder, with the same title and year, become one entry with a version picker on the
item page and in the player's settings:

```
Blade Runner (1982)/
  Blade Runner (1982) - 4K.mkv
  Blade Runner (1982) {edition-Final Cut}.mkv
```

The name of a version is the edition tag (`{edition-…}`) or what follows the year; without one, resolution, codec
and size tell the versions apart. The best version (most pixels, then bitrate) plays unless another is chosen. All
versions share one watch state and resume position, kept under the id of the file that was in the library first, so
adding a 4K file to a movie keeps its watch state.

### Trailers and extras

Bonus videos are shown on the page of their movie or show under *Trailers and extras* and play like any other title
(without resume position or watch history). They are found in two ways:

| Where | Type |
|---|---|
| A folder next to the movie, or in the show or season folder: `Trailers/`, `Featurettes/`, `Behind The Scenes/`, `Deleted Scenes/`, `Interviews/`, `Scenes/`, `Shorts/`, `Extras/`, `Other/` | by the folder; the title is the file name |
| A file next to the movie named with a suffix: `-trailer`, `-featurette`, `-behindthescenes`, `-deleted`, `-interview`, `-scene`, `-short`, `-other`, `-extra` | by the suffix; the title is the type ("Trailer") |

Only the files directly inside an extras folder count. In a folder with several movies, a suffixed file belongs to
the movie with the same name (`Dune (2021)-trailer.mkv` → `Dune (2021).mkv`); extras folders there and at the
library root are ignored, since it is not clear which title they belong to. Extras of shows always belong to the
show, also when they lie in a season folder.

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

## Filters and smart collections

The *Library* tab of a library has a filter panel: watch state (unwatched, in progress, watched), genre, years,
minimum rating, resolution (of any version or episode), age rating, recently added, an actor or director (from the
online metadata) and part of the title. The filters are part of the address, so a filtered view can be bookmarked.

Admins can save a filter as a **smart collection**. It keeps the filter instead of a list of titles: titles added
later that match appear in it by themselves, and every user sees it with their own watch state (an "Unwatched"
collection shows each user what they have not seen). *Edit filter* on the collection page opens the library with
the filter, *Update smart collection* saves the change.

## Editing metadata by hand

Admins can change title, original title, sort title, year, rating, age rating, genres, tagline and summary of a
movie, show, season or episode (*… → Edit metadata*), and upload their own poster and background. Edited fields win
over file names, `.nfo` files and the online metadata, also after scans and refreshes; *Reset* brings back what those
say. The edits are kept in the database, uploaded pictures in `<data>/artwork`.

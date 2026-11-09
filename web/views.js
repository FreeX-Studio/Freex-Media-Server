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

/* FreeX Web, part 2: home screen, libraries, detail pages and search. */
(() => {
    "use strict";
    const M = window.FreeX;
    const {S, t, tn, h, link, icon, api, card, hubRow, emptyState} = M;

    // ------------------------------------------------------------------ home

    M.route("/", async (main) => {
        const hubs = await api("GET", "/api/hubs");
        if (!hubs.length) {
            const text = S.libraries.length
                ? t("The libraries are being scanned or are still empty.")
                : t("Add a library with your movies or shows to see something here.");
            main.replaceChildren(emptyState(t("Nothing to see yet"), text,
                S.user.admin && !S.libraries.length ? link("/settings/libraries", {class: "btn primary"}, t("Add library")) : null));
            return;
        }
        main.replaceChildren(h("div", {class: "page"}, hubs.map((hub) =>
            hubRow(hub, hub.libraryId ? `/library/${hub.libraryId}/all?sort=added` : null))));
    });

    // ------------------------------------------------------------------ library

    function libraryHeader(lib, tab, tools) {
        return h("div", {class: "lib-head"},
            h("h1", null, lib.name),
            h("nav", {class: "lib-tabs", "aria-label": t("View")},
                link(`/library/${lib.id}`, {class: tab === "recommended" ? "active" : ""}, t("Recommended")),
                link(`/library/${lib.id}/all`, {class: tab === "all" ? "active" : ""}, t("Library")),
                lib.type === "movies" ? link(`/library/${lib.id}/collections`, {class: tab === "collections" ? "active" : ""}, t("Collections")) : null),
            h("div", {class: "lib-tools"}, tools));
    }

    M.route("/library/:id", async (main, p) => {
        const lib = S.libraries.find((l) => l.id === p.id) || (await api("GET", "/api/libraries")).find((l) => l.id === p.id);
        if (!lib) throw new M.ApiError(404, t("This library no longer exists."));
        const hubs = await api("GET", `/api/libraries/${p.id}/hubs`);
        const body = hubs.length ? hubs.map((hub) => hubRow(hub, hub.sort ? `/library/${p.id}/all?sort=${hub.sort}` : null))
            : h("div", {class: "empty"}, h("h2", null, lib.scanning ? t("Scanning") : t("No recommendations yet")),
                h("p", null, lib.scanning ? t("The library is being scanned.") : t("Rows appear here as soon as there is enough in this library.")),
                link(`/library/${p.id}/all`, {class: "btn"}, t("Show the whole library")));
        main.replaceChildren(h("div", {class: "page"}, libraryHeader(lib, "recommended"), body));
    });

    const POSTER_KEY = "freex.posterSize";

    function applyPosterSize(v) {
        document.documentElement.style.setProperty("--poster", v + "px");
    }

    applyPosterSize(Number(M.store.get(POSTER_KEY)) || 156);

    /** Filter keys of the library list (see api/filter.rs); everything else in the URL is not a filter. */
    const FILTER_KEYS = ["watched", "genre", "yearFrom", "yearTo", "minRating", "resolution", "contentRating", "person", "addedDays", "q"];

    M.route("/library/:id/all", async (main, p, query) => {
        const sort = query.get("sort") || "title";
        const smartId = query.get("smart") || "";
        const q = new URLSearchParams({sort});
        for (const k of FILTER_KEYS) if (query.get(k)) q.set(k, query.get(k));
        // Links of older versions
        if (query.get("unwatched") === "1" && !query.get("watched")) q.set("watched", "unwatched");
        const data = await api("GET", `/api/libraries/${p.id}/items?${q}`);
        const lib = data.library;
        const active = FILTER_KEYS.filter((k) => q.get(k));

        const update = (changes) => {
            const u = new URLSearchParams(location.search);
            u.delete("unwatched");
            for (const [k, v] of Object.entries(changes)) {
                if (v) u.set(k, v); else u.delete(k);
            }
            M.navigate(`/library/${p.id}/all?${u}`, true);
        };
        const resetAll = () => update(Object.fromEntries(FILTER_KEYS.map((k) => [k, ""])));
        const sorts = [["title", t("Title")], ["added", t("Added")], ["year", t("Release year")], ["rating", t("Rating")]];
        if (lib.type === "movies") sorts.push(["duration", t("Duration")]);
        const sortSel = h("select", {"aria-label": t("Sort order"), onchange: (e) => update({sort: e.target.value})},
            sorts.map(([v, l]) => h("option", {value: v, selected: v === sort}, l)));
        const size = h("input", {type: "range", min: 110, max: 240, step: 2, value: Number(M.store.get(POSTER_KEY)) || 156,
            "aria-label": t("Poster size"), class: "size-slider", oninput: (e) => {
                applyPosterSize(e.target.value);
                M.store.set(POSTER_KEY, e.target.value);
            }});

        // ---- the filter panel
        const select = (key, label, options) => h("label", {class: "filter-field"}, h("span", null, label),
            h("select", {onchange: (e) => update({[key]: e.target.value})},
                options.map(([v, l]) => h("option", {value: v, selected: String(q.get(key) || "") === String(v)}, l))));
        const text = (key, label, attrs) => {
            const input = h("input", Object.assign({value: q.get(key) || "", onchange: (e) => update({[key]: e.target.value.trim()})}, attrs));
            input.addEventListener("keydown", (e) => {
                if (e.key === "Enter") input.blur();
            });
            return h("label", {class: "filter-field"}, h("span", null, label), input);
        };
        const any = ["", t("Any")];
        const [yMin, yMax] = data.years || ["", ""];
        const fields = [
            select("watched", t("Watched"), [["", t("All")], ["unwatched", t("Unwatched")], ["inProgress", t("In progress")], ["watched", t("Watched")]]),
            data.genres.length ? select("genre", "Genre", [["", t("All genres")]].concat(data.genres.map((g) => [g, g]))) : null,
            text("yearFrom", t("Year from"), {type: "number", min: 1880, max: 2100, placeholder: yMin, class: "year"}),
            text("yearTo", t("Year to"), {type: "number", min: 1880, max: 2100, placeholder: yMax, class: "year"}),
            select("minRating", t("Rating"), [any].concat([9, 8, 7, 6, 5].map((r) => [r, t("{0} or more", r)]))),
            data.resolutions.length ? select("resolution", t("Resolution"), [any].concat(data.resolutions.map((r) => [r, r]))) : null,
            data.contentRatings.length ? select("contentRating", t("Age rating"), [any].concat(data.contentRatings.map((r) => [r, r]))) : null,
            select("addedDays", t("Added"), [any, [7, t("Last 7 days")], [30, t("Last 30 days")], [90, t("Last 3 months")], [365, t("Last year")]]),
            text("person", t("Actor or director"), {type: "search", placeholder: t("Name")}),
            text("q", t("Title contains"), {type: "search"}),
        ];
        const saveBtn = S.user.admin && active.length ? h("button", {class: "btn small", onclick: () => saveSmart()},
            smartId ? t("Update smart collection") : t("Save as smart collection")) : null;
        const panel = h("div", {class: "filter-panel", hidden: !active.length && M.store.get("freex.filters") !== "1"},
            h("div", {class: "filter-fields"}, fields),
            h("div", {class: "filter-actions"},
                active.length ? h("button", {class: "btn small", onclick: resetAll}, t("Reset filters")) : null, saveBtn));
        const filterBtn = h("button", {class: "btn small filter-btn" + (active.length ? " on" : ""), "aria-expanded": String(!panel.hidden),
            onclick: () => {
                panel.hidden = !panel.hidden;
                filterBtn.setAttribute("aria-expanded", String(!panel.hidden));
                M.store.set("freex.filters", panel.hidden ? "0" : "1");
            }}, icon("filter"), t("Filter"), active.length ? h("span", {class: "badge"}, active.length) : null);

        async function saveSmart() {
            const filter = data.filter;
            if (smartId) {
                await api("PUT", `/api/collections/${smartId}`, {filter});
                M.toast(t("Smart collection updated"));
                return M.navigate(`/collection/${smartId}`);
            }
            const name = h("input", {id: "sc-name", maxlength: 80, placeholder: t("e.g. Unwatched 90s movies")});
            const close = M.modal(t("Save as smart collection"),
                h("p", {class: "hint"}, t("The collection keeps this filter: titles that match later appear in it by themselves, and everyone sees it with their own watch state.")),
                h("div", {class: "field"}, h("label", {for: "sc-name"}, t("Name")), name),
                h("div", {class: "btn-row"}, h("button", {class: "btn primary", onclick: async () => {
                    const c = await api("POST", "/api/collections", {libraryId: p.id, name: name.value, filter});
                    close();
                    M.navigate(`/collection/${c.id}`);
                }}, t("Save"))));
            name.focus();
        }

        let body;
        if (data.items.length) {
            body = h("div", {class: "grid"}, data.items.map((it) => card(it, "poster")));
        } else if (lib.scanning) {
            body = h("div", {class: "empty"}, h("h2", null, t("Scanning")), h("p", null, t("The library is being scanned. The page updates as soon as the scan is finished.")));
        } else if (active.length) {
            body = h("div", {class: "empty"}, h("h2", null, t("No matches")), h("p", null, t("Nothing matches these filters.")),
                h("button", {class: "btn", onclick: resetAll}, t("Reset filters")));
        } else {
            body = h("div", {class: "empty"}, h("h2", null, t("This library is empty")),
                h("p", null, t("No videos were found in the folders. Check the folders in the settings and start a new scan.")),
                S.user.admin ? link("/settings/libraries", {class: "btn"}, t("Check folders")) : null);
        }
        main.replaceChildren(h("div", {class: "page"},
            libraryHeader(lib, "all", [h("span", {class: "count"}, data.items.length), filterBtn, sortSel, size]),
            panel,
            body));
    });

    // ------------------------------------------------------------------ details

    const trackKey = (id) => "freex.tracks." + id;

    function savedTracks(id) {
        try {
            return JSON.parse(M.store.get(trackKey(id)) || "{}");
        } catch (e) {
            return {};
        }
    }

    function playUrl(item, fromStart) {
        const tracks = savedTracks(item.id);
        const q = new URLSearchParams();
        // A movie in several versions plays the chosen one (its own id); the watch state is shared.
        const version = item.versions && item.versions.find((v) => v.id === tracks.version);
        const id = version && !version.primary ? version.id : item.id;
        const off = item.progress && !item.progress.watched ? item.progress.offsetMs : 0;
        if (off && !fromStart) q.set("t", off);
        if (tracks.audio != null) q.set("a", tracks.audio);
        // "none" = switched off on purpose; without "s" the player applies the preferences
        if ("sub" in tracks) q.set("s", tracks.sub || "none");
        const qs = q.toString();
        return `/play/${id}` + (qs ? "?" + qs : "");
    }

    M.route("/item/:id", async (main, p) => {
        const it = await api("GET", `/api/items/${p.id}`);
        const playable = it.type === "movie" || it.type === "episode";
        const landscape = it.type === "episode";
        const watched = M.isWatched(it);

        const parent = [];
        if (it.type === "season" && it.showId) parent.push(link(`/item/${it.showId}`, null, it.showTitle));
        if (it.type === "episode" && it.showId) parent.push(link(`/item/${it.showId}`, null, it.showTitle));

        const subtitle = [];
        if (it.type === "episode") subtitle.push(link(`/item/${it.seasonId}`, null, it.seasonIndex === 0 ? t("Specials") : t("Season {0}", it.seasonIndex)), " " + t("Episode {0}", it.index));
        else if (it.year) subtitle.push(String(it.year));

        const facts = [];
        if (it.type === "episode" && it.year) facts.push(String(it.year));
        if (it.media && it.media.durationMs) {
            facts.push(M.fmtDuration(it.media.durationMs));
            const end = new Date(Date.now() + it.media.durationMs - (it.progress && !it.progress.watched ? it.progress.offsetMs : 0));
            facts.push(t("Ends at {0}", end.toLocaleTimeString(M.locale(), {hour: "2-digit", minute: "2-digit"})));
        }
        if (it.type === "show") facts.push(tn(it.childCount, "1 season", "{0} seasons"));
        if (it.type === "show" || it.type === "season") facts.push(tn(it.leafCount, "1 episode", "{0} episodes"));
        const rating = it.rating ? h("span", {class: "rating"}, "★ " + M.num(it.rating, 1)) : null;

        // Primary action
        const actions = h("div", {class: "actions"});
        if (playable) {
            const off = it.progress && !it.progress.watched ? it.progress.offsetMs : 0;
            actions.append(link(playUrl(it), {class: "btn primary play"}, icon("play"), off ? t("Resume") : t("Play")));
            if (off) actions.append(link(playUrl(it, true), {class: "round-btn from-start", title: t("Play from the beginning"), "aria-label": t("Play from the beginning")}, icon("restart")));
        } else if (it.onDeck) {
            const d = it.onDeck;
            actions.append(link(M.playHref(d), {class: "btn primary play"}, icon("play"),
                `${d.progress && d.progress.offsetMs ? t("Resume") : t("Play")} ${M.episodeCode(d)}`));
        }
        actions.append(h("button", {class: "round-btn" + (watched ? " on" : ""), title: watched ? t("Mark as unwatched") : t("Mark as watched"),
            "aria-label": watched ? t("Mark as unwatched") : t("Mark as watched"), "aria-pressed": String(!!watched),
            onclick: () => M.setWatched(it, !watched)}, icon("check")));
        if (playable) {
            actions.append(h("a", {class: "round-btn", href: `/api/items/${it.id}/stream?download=1`, title: t("Download"), "aria-label": t("Download")}, icon("download")));
        }
        actions.append(h("button", {class: "round-btn", title: t("More"), "aria-label": t("More"), "data-menu": true,
            onclick: (e) => M.itemMenu(e.currentTarget, it)}, icon("more")));

        // Track selection on the detail page
        let tracks = null;
        if (playable && it.media) {
            const saved = savedTracks(it.id);
            const def = M.defaultTracks(it.media, S.user.preferences);
            const save = (k, v) => {
                const tracks = savedTracks(it.id);
                tracks[k] = v;
                M.store.set(trackKey(it.id), JSON.stringify(tracks));
                actions.querySelectorAll("a.play, a.from-start").forEach((a, i) =>
                    a.setAttribute("href", playUrl(it, i > 0)));
            };
            const m = it.media;
            const videoText = [m.resolution, M.codecName(m.videoCodec), m.bitrate ? M.fmtBitrate(m.bitrate) : ""].filter(Boolean).join(" ");
            const audioSel = m.audio.length > 1 ? h("select", {"aria-label": t("Audio track"), onchange: (e) => save("audio", Number(e.target.value))},
                m.audio.map((a) => h("option", {value: a.index, selected: saved.audio != null ? saved.audio === a.index : a.index === def.audio}, M.audioLabel(a))))
                : h("span", null, m.audio.length ? M.audioLabel(m.audio[0]) : t("No audio"));
            const subSel = h("select", {"aria-label": t("Subtitles"), onchange: (e) => save("sub", e.target.value || null)},
                h("option", {value: ""}, t("None")),
                m.subtitles.map((s) => h("option", {value: s.id, selected: "sub" in saved ? saved.sub === s.id : def.sub === s.id}, M.subtitleLabel(s))));
            // Several versions: the choice is kept per title; audio and subtitle tracks of another
            // file are chosen in the player.
            const versions = it.versions && it.versions.length > 1 ? it.versions : null;
            const versionSel = versions ? h("select", {"aria-label": t("Version"), onchange: (e) => {
                const tracks = savedTracks(it.id);
                tracks.version = e.target.value;
                delete tracks.audio;
                delete tracks.sub;
                M.store.set(trackKey(it.id), JSON.stringify(tracks));
                const other = !versions.find((v) => v.id === e.target.value).primary;
                trackRows.forEach((el) => el.hidden = other);
                actions.querySelectorAll("a.play, a.from-start").forEach((a, i) => a.setAttribute("href", playUrl(it, i > 0)));
            }}, versions.map((v) => h("option", {value: v.id, selected: saved.version ? saved.version === v.id : v.primary}, M.versionLabel(v)))) : null;
            const chosenOther = versions && saved.version && versions.some((v) => v.id === saved.version && !v.primary);
            const trackRows = [h("dt", null, "Video"), h("dd", null, videoText || t("Unknown")),
                h("dt", null, "Audio"), h("dd", null, audioSel),
                h("dt", null, t("Subtitles")), h("dd", null, m.subtitles.length ? subSel : h("span", {class: "muted"}, t("None available")))];
            trackRows.forEach((el) => el.hidden = !!chosenOther);
            tracks = h("dl", {class: "tracks"},
                versions ? [h("dt", null, t("Version")), h("dd", null, versionSel)] : null,
                trackRows);
        }

        const summary = it.summary ? h("p", {class: "summary clamp"}, it.summary) : null;
        if (summary) {
            summary.addEventListener("click", () => summary.classList.toggle("clamp"));
            summary.title = t("Click for the full text");
        }

        const posterKind = landscape ? "thumb" : "poster";
        const mirror = h("div", {class: "mirror" + (landscape ? " landscape" : "")},
            h("div", {class: "poster"}, M.art(it, posterKind, it.title), M.progressBar(it), M.unwatchedBadge(it)),
            h("div", {class: "reflection", "aria-hidden": "true"},
                h("img", {src: M.img(it, posterKind), alt: "", onerror: (e) => e.target.parentNode.remove()})));

        const ex = M.detailExtras ? M.detailExtras(it, actions) : {facts: []};

        const info = h("div", {class: "detail-info"},
            parent.length ? h("div", {class: "parent-title"}, parent) : null,
            h("h1", null, it.title),
            subtitle.length ? h("div", {class: "sub-title"}, subtitle) : null,
            h("div", {class: "facts"}, facts.map((f) => h("span", null, f)), rating, ex.facts),
            it.originalTitle ? h("div", {class: "original"}, it.originalTitle) : null,
            actions,
            it.tagline ? h("div", {class: "tagline"}, it.tagline) : null,
            summary,
            it.genres ? h("div", {class: "genres"}, it.genres) : null,
            ex.crew,
            ex.collections,
            tracks,
            ex.unmatched,
            it.media && S.user.admin ? h("div", {class: "file-line"}, `${it.media.fileName}, ${M.fmtSize(it.media.size)}`) : null);

        const backdrop = h("div", {class: "backdrop"}, h("img", {src: M.img(it, "backdrop"), alt: "", onerror: (e) => e.target.remove()}));
        main.replaceChildren(h("div", {class: "detail"}, backdrop,
            h("div", {class: "detail-body"}, mirror, info),
            h("div", {class: "detail-children"}, childrenSection(it),
                it.extras && it.extras.length ? hubRow({title: t("Trailers and extras"), style: "landscape", items: it.extras}) : null,
                ex.cast)));
    });

    function childrenSection(it) {
        if (!it.children || !it.children.length) return null;
        if (it.type === "show") {
            return hubRow({title: tn(it.children.length, "1 season", "{0} seasons"), style: "poster", items: it.children});
        }
        return [h("h2", {class: "section-title"}, tn(it.children.length, "1 episode", "{0} episodes")),
            h("div", {class: "episode-list"}, it.children.map((ep) => {
                const off = ep.progress && !ep.progress.watched ? ep.progress.offsetMs : 0;
                return h("div", {class: "episode"},
                    link(M.playHref(ep), {class: "card landscape", "aria-label": t("Play {0}", ep.title)},
                        h("div", {class: "art"}, M.art(ep, "thumb", ep.title), M.unwatchedBadge(ep), M.progressBar(ep),
                            h("div", {class: "card-actions"}, h("span", {class: "card-play", html: M.ICONS.play})))),
                    h("div", {class: "ep-text"},
                        link(`/item/${ep.id}`, null, h("h3", null, ep.title)),
                        h("div", {class: "ep-meta"}, [t("Episode {0}", ep.index), M.fmtDuration(ep.media && ep.media.durationMs),
                            off ? t("{0} left", M.fmtDuration(ep.media.durationMs - off)) : ""].filter(Boolean).join(", ")),
                        ep.summary ? h("p", null, ep.summary) : null),
                    h("button", {class: "round-btn small", title: t("More"), "aria-label": t("More"), "data-menu": true,
                        onclick: (e) => M.itemMenu(e.currentTarget, ep)}, icon("more")));
            }))];
    }

    // ------------------------------------------------------------------ search

    M.route("/search", async (main, p, query) => {
        const q = query.get("q") || "";
        const input = document.querySelector(".search input");
        if (input && document.activeElement !== input) input.value = q;
        const res = q.length >= 2 ? await api("GET", "/api/search?q=" + encodeURIComponent(q)) : {movies: [], shows: [], episodes: []};
        const sections = [[t("Movies"), res.movies, "poster"], [t("Shows"), res.shows, "poster"], [t("Episodes"), res.episodes, "landscape"]]
            .filter(([, items]) => items.length)
            .map(([title, items, style]) => h("section", {class: "hub"}, h("div", {class: "hub-head"}, h("h2", null, title)),
                h("div", {class: "grid" + (style === "landscape" ? " landscape" : "")}, items.map((i) => card(i, style)))));
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, t("Search for “{0}”", q))),
            sections.length ? sections : h("div", {class: "empty"}, h("h2", null, t("Nothing found")),
                h("p", null, t("No title contains this search term.")))));
    });

    M.playUrl = playUrl;
    M.libraryHeader = libraryHeader;
    M.savedTracks = savedTracks;
})();

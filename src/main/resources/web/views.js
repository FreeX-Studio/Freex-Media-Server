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

/* FreeX web interface, part 2: home, libraries, details and search. */
(() => {
    "use strict";
    const M = window.FreeX;
    const {S, h, link, icon, api, card, hubRow, emptyState} = M;

    // ------------------------------------------------------------------ home

    M.route("/", async (main) => {
        const hubs = await api("GET", "/api/hubs");
        if (!hubs.length) {
            const text = S.libraries.length
                ? "Die Bibliotheken werden gerade gescannt oder sind noch leer."
                : "Füge eine Bibliothek mit deinen Filmen oder Serien hinzu, damit hier etwas erscheint.";
            main.replaceChildren(emptyState("Noch nichts zu sehen", text,
                S.user.admin && !S.libraries.length ? link("/settings/libraries", {class: "btn primary"}, "Bibliothek hinzufügen") : null));
            return;
        }
        main.replaceChildren(h("div", {class: "page"}, hubs.map((hub) =>
            hubRow(hub, hub.libraryId ? `/library/${hub.libraryId}/all?sort=added` : null))));
    });

    // ------------------------------------------------------------------ library

    function libraryHeader(lib, tab, tools) {
        return h("div", {class: "lib-head"},
            h("h1", null, lib.name),
            h("nav", {class: "lib-tabs", "aria-label": "Ansicht"},
                link(`/library/${lib.id}`, {class: tab === "recommended" ? "active" : ""}, "Empfohlen"),
                link(`/library/${lib.id}/all`, {class: tab === "all" ? "active" : ""}, "Bibliothek"),
                lib.type === "movies" ? link(`/library/${lib.id}/collections`, {class: tab === "collections" ? "active" : ""}, "Sammlungen") : null),
            h("div", {class: "lib-tools"}, tools));
    }

    M.route("/library/:id", async (main, p) => {
        const lib = S.libraries.find((l) => l.id === p.id) || (await api("GET", "/api/libraries")).find((l) => l.id === p.id);
        if (!lib) throw new M.ApiError(404, "Diese Bibliothek gibt es nicht mehr.");
        const hubs = await api("GET", `/api/libraries/${p.id}/hubs`);
        const body = hubs.length ? hubs.map((hub) => hubRow(hub, hub.sort ? `/library/${p.id}/all?sort=${hub.sort}` : null))
            : h("div", {class: "empty"}, h("h2", null, lib.scanning ? "Wird gescannt" : "Noch keine Empfehlungen"),
                h("p", null, lib.scanning ? "Die Bibliothek wird gerade eingelesen." : "Sobald genug in dieser Bibliothek ist, erscheinen hier Reihen."),
                link(`/library/${p.id}/all`, {class: "btn"}, "Ganze Bibliothek anzeigen"));
        main.replaceChildren(h("div", {class: "page"}, libraryHeader(lib, "recommended"), body));
    });

    const POSTER_KEY = "freex.posterSize";

    function applyPosterSize(v) {
        document.documentElement.style.setProperty("--poster", v + "px");
    }

    applyPosterSize(Number(M.store.get(POSTER_KEY)) || 156);

    M.route("/library/:id/all", async (main, p, query) => {
        const sort = query.get("sort") || "title";
        const unwatched = query.get("unwatched") === "1";
        const genre = query.get("genre") || "";
        const q = new URLSearchParams({sort});
        if (unwatched) q.set("unwatched", "1");
        if (genre) q.set("genre", genre);
        const data = await api("GET", `/api/libraries/${p.id}/items?${q}`);
        const lib = data.library;

        const update = (changes) => {
            const u = new URLSearchParams(location.search);
            for (const [k, v] of Object.entries(changes)) {
                if (v) u.set(k, v); else u.delete(k);
            }
            M.navigate(`/library/${p.id}/all?${u}`, true);
        };
        const sorts = [["title", "Titel"], ["added", "Hinzugefügt"], ["year", "Erscheinungsjahr"], ["rating", "Bewertung"]];
        if (lib.type === "movies") sorts.push(["duration", "Laufzeit"]);
        const sortSel = h("select", {"aria-label": "Sortierung", onchange: (e) => update({sort: e.target.value})},
            sorts.map(([v, l]) => h("option", {value: v, selected: v === sort}, l)));
        const filterSel = h("select", {"aria-label": "Filter", onchange: (e) => update({unwatched: e.target.value})},
            h("option", {value: ""}, "Alle"), h("option", {value: "1", selected: unwatched}, "Ungesehen"));
        const genreSel = data.genres.length ? h("select", {"aria-label": "Genre", onchange: (e) => update({genre: e.target.value})},
            h("option", {value: ""}, "Alle Genres"),
            data.genres.map((g) => h("option", {value: g, selected: g === genre}, g))) : null;
        const size = h("input", {type: "range", min: 110, max: 240, step: 2, value: Number(M.store.get(POSTER_KEY)) || 156,
            "aria-label": "Postergröße", class: "size-slider", oninput: (e) => {
                applyPosterSize(e.target.value);
                M.store.set(POSTER_KEY, e.target.value);
            }});

        let body;
        if (data.items.length) {
            body = h("div", {class: "grid"}, data.items.map((it) => card(it, "poster")));
        } else if (lib.scanning) {
            body = h("div", {class: "empty"}, h("h2", null, "Wird gescannt"), h("p", null, "Die Bibliothek wird gerade eingelesen. Die Seite aktualisiert sich, sobald der Scan fertig ist."));
        } else if (unwatched || genre) {
            body = h("div", {class: "empty"}, h("h2", null, "Keine Treffer"), h("p", null, "Mit diesen Filtern gibt es hier nichts."),
                h("button", {class: "btn", onclick: () => update({unwatched: "", genre: ""})}, "Filter zurücksetzen"));
        } else {
            body = h("div", {class: "empty"}, h("h2", null, "Diese Bibliothek ist leer"),
                h("p", null, "In den Ordnern wurden keine Videos gefunden. Prüfe die Ordner in den Einstellungen und starte einen neuen Scan."),
                S.user.admin ? link("/settings/libraries", {class: "btn"}, "Ordner prüfen") : null);
        }
        main.replaceChildren(h("div", {class: "page"},
            libraryHeader(lib, "all", [h("span", {class: "count"}, data.items.length), filterSel, genreSel, sortSel, size]),
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
        const t = savedTracks(item.id);
        const q = new URLSearchParams();
        const off = item.progress && !item.progress.watched ? item.progress.offsetMs : 0;
        if (off && !fromStart) q.set("t", off);
        if (t.audio != null) q.set("a", t.audio);
        // "none" = switched off on purpose; without "s" the player applies the preferences
        if ("sub" in t) q.set("s", t.sub || "none");
        const qs = q.toString();
        return `/play/${item.id}` + (qs ? "?" + qs : "");
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
        if (it.type === "episode") subtitle.push(link(`/item/${it.seasonId}`, null, it.seasonIndex === 0 ? "Specials" : `Staffel ${it.seasonIndex}`), ` Episode ${it.index}`);
        else if (it.year) subtitle.push(String(it.year));

        const facts = [];
        if (it.type === "episode" && it.year) facts.push(String(it.year));
        if (it.media && it.media.durationMs) {
            facts.push(M.fmtDuration(it.media.durationMs));
            const end = new Date(Date.now() + it.media.durationMs - (it.progress && !it.progress.watched ? it.progress.offsetMs : 0));
            facts.push("Endet um " + end.toLocaleTimeString("de-DE", {hour: "2-digit", minute: "2-digit"}));
        }
        if (it.type === "show") facts.push(it.childCount === 1 ? "1 Staffel" : `${it.childCount} Staffeln`);
        if (it.type === "show" || it.type === "season") facts.push(`${it.leafCount} Episoden`);
        const rating = it.rating ? h("span", {class: "rating"}, "★ " + String(it.rating).replace(".", ",")) : null;

        // Primary action
        const actions = h("div", {class: "actions"});
        if (playable) {
            const off = it.progress && !it.progress.watched ? it.progress.offsetMs : 0;
            actions.append(link(playUrl(it), {class: "btn primary play"}, icon("play"), off ? `Fortsetzen` : "Abspielen"));
            if (off) actions.append(link(playUrl(it, true), {class: "round-btn", title: "Von vorne abspielen", "aria-label": "Von vorne abspielen"}, icon("restart")));
        } else if (it.onDeck) {
            const d = it.onDeck;
            actions.append(link(M.playHref(d), {class: "btn primary play"}, icon("play"),
                `${d.progress && d.progress.offsetMs ? "Fortsetzen" : "Abspielen"} ${M.episodeCode(d)}`));
        }
        actions.append(h("button", {class: "round-btn" + (watched ? " on" : ""), title: watched ? "Als ungesehen markieren" : "Als gesehen markieren",
            "aria-label": watched ? "Als ungesehen markieren" : "Als gesehen markieren", "aria-pressed": String(!!watched),
            onclick: () => M.setWatched(it, !watched)}, icon("check")));
        if (playable) {
            actions.append(h("a", {class: "round-btn", href: `/api/items/${it.id}/stream?download=1`, title: "Herunterladen", "aria-label": "Herunterladen"}, icon("download")));
        }
        actions.append(h("button", {class: "round-btn", title: "Mehr", "aria-label": "Mehr", "data-menu": true,
            onclick: (e) => M.itemMenu(e.currentTarget, it)}, icon("more")));

        // Track selection on the detail page
        let tracks = null;
        if (playable && it.media) {
            const saved = savedTracks(it.id);
            const def = M.defaultTracks(it.media, S.user.preferences);
            const save = (k, v) => {
                const t = savedTracks(it.id);
                t[k] = v;
                M.store.set(trackKey(it.id), JSON.stringify(t));
                actions.querySelectorAll("a.play, a.round-btn[title^='Von vorne']").forEach((a, i) =>
                    a.setAttribute("href", playUrl(it, i > 0)));
            };
            const m = it.media;
            const videoText = [m.resolution, M.codecName(m.videoCodec), m.bitrate ? M.fmtBitrate(m.bitrate) : ""].filter(Boolean).join(" ");
            const audioSel = m.audio.length > 1 ? h("select", {"aria-label": "Tonspur", onchange: (e) => save("audio", Number(e.target.value))},
                m.audio.map((a) => h("option", {value: a.index, selected: saved.audio != null ? saved.audio === a.index : a.index === def.audio}, M.audioLabel(a))))
                : h("span", null, m.audio.length ? M.audioLabel(m.audio[0]) : "Kein Ton");
            const subSel = h("select", {"aria-label": "Untertitel", onchange: (e) => save("sub", e.target.value || null)},
                h("option", {value: ""}, "Keine"),
                m.subtitles.map((s) => h("option", {value: s.id, selected: "sub" in saved ? saved.sub === s.id : def.sub === s.id}, M.subtitleLabel(s))));
            tracks = h("dl", {class: "tracks"},
                h("dt", null, "Video"), h("dd", null, videoText || "Unbekannt"),
                h("dt", null, "Audio"), h("dd", null, audioSel),
                h("dt", null, "Untertitel"), h("dd", null, m.subtitles.length ? subSel : h("span", {class: "muted"}, "Keine vorhanden")));
        }

        const summary = it.summary ? h("p", {class: "summary clamp"}, it.summary) : null;
        if (summary) {
            summary.addEventListener("click", () => summary.classList.toggle("clamp"));
            summary.title = "Klicken für den ganzen Text";
        }

        const posterKind = landscape ? "thumb" : "poster";
        const mirror = h("div", {class: "mirror" + (landscape ? " landscape" : "")},
            h("div", {class: "poster"}, M.art(it.id, posterKind, it.title), M.progressBar(it), M.unwatchedBadge(it)),
            h("div", {class: "reflection", "aria-hidden": "true"},
                h("img", {src: M.img(it.id, posterKind), alt: "", onerror: (e) => e.target.parentNode.remove()})));

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

        const backdrop = h("div", {class: "backdrop"}, h("img", {src: M.img(it.id, "backdrop"), alt: "", onerror: (e) => e.target.remove()}));
        main.replaceChildren(h("div", {class: "detail"}, backdrop,
            h("div", {class: "detail-body"}, mirror, info),
            h("div", {class: "detail-children"}, childrenSection(it), ex.cast)));
    });

    function childrenSection(it) {
        if (!it.children || !it.children.length) return null;
        if (it.type === "show") {
            return hubRow({title: it.children.length === 1 ? "1 Staffel" : `${it.children.length} Staffeln`, style: "poster", items: it.children});
        }
        return [h("h2", {class: "section-title"}, `${it.children.length} Episoden`),
            h("div", {class: "episode-list"}, it.children.map((ep) => {
                const off = ep.progress && !ep.progress.watched ? ep.progress.offsetMs : 0;
                return h("div", {class: "episode"},
                    link(M.playHref(ep), {class: "card landscape", "aria-label": `${ep.title} abspielen`},
                        h("div", {class: "art"}, M.art(ep.id, "thumb", ep.title), M.unwatchedBadge(ep), M.progressBar(ep),
                            h("div", {class: "card-actions"}, h("span", {class: "card-play", html: M.ICONS.play})))),
                    h("div", {class: "ep-text"},
                        link(`/item/${ep.id}`, null, h("h3", null, ep.title)),
                        h("div", {class: "ep-meta"}, [`Episode ${ep.index}`, M.fmtDuration(ep.media && ep.media.durationMs),
                            off ? `noch ${M.fmtDuration(ep.media.durationMs - off)}` : ""].filter(Boolean).join(", ")),
                        ep.summary ? h("p", null, ep.summary) : null),
                    h("button", {class: "round-btn small", title: "Mehr", "aria-label": "Mehr", "data-menu": true,
                        onclick: (e) => M.itemMenu(e.currentTarget, ep)}, icon("more")));
            }))];
    }

    // ------------------------------------------------------------------ search

    M.route("/search", async (main, p, query) => {
        const q = query.get("q") || "";
        const input = document.querySelector(".search input");
        if (input && document.activeElement !== input) input.value = q;
        const res = q.length >= 2 ? await api("GET", "/api/search?q=" + encodeURIComponent(q)) : {movies: [], shows: [], episodes: []};
        const sections = [["Filme", res.movies, "poster"], ["Serien", res.shows, "poster"], ["Episoden", res.episodes, "landscape"]]
            .filter(([, items]) => items.length)
            .map(([title, items, style]) => h("section", {class: "hub"}, h("div", {class: "hub-head"}, h("h2", null, title)),
                h("div", {class: "grid" + (style === "landscape" ? " landscape" : "")}, items.map((i) => card(i, style)))));
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, `Suche nach „${q}“`)),
            sections.length ? sections : h("div", {class: "empty"}, h("h2", null, "Nichts gefunden"),
                h("p", null, "Kein Titel enthält diesen Suchbegriff."))));
    });

    M.playUrl = playUrl;
    M.libraryHeader = libraryHeader;
    M.savedTracks = savedTracks;
})();

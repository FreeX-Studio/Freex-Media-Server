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

/*
 * FreeX Web, part 5: watchlist, playlists, watch history, collections, people, fix match,
 * trailers and Watch Together.
 */
(() => {
    "use strict";
    const M = window.FreeX;
    const {S, t, tn, h, link, icon, api, card} = M;

    // ------------------------------------------------------------------ dialogs

    function modal(title, ...content) {
        const back = h("div", {class: "modal-back", onclick: (e) => e.target === back && close()});
        const close = () => {
            back.remove();
            document.removeEventListener("keydown", onKey);
        };
        const onKey = (e) => e.key === "Escape" && close();
        document.addEventListener("keydown", onKey);
        back.append(h("div", {class: "modal", role: "dialog", "aria-modal": "true", "aria-label": title},
            h("div", {class: "modal-head"}, h("h2", null, title),
                h("button", {class: "icon-btn", "aria-label": t("Close"), onclick: () => close()}, icon("close"))),
            content));
        document.body.append(back);
        back.querySelector("input, select, button.btn")?.focus();
        return close;
    }

    async function addToPlaylist(item) {
        const lists = await api("GET", "/api/playlists");
        const name = h("input", {id: "pl-name", placeholder: t("Name of the new playlist"), maxlength: 80});
        const error = h("div", {class: "error-text", role: "alert"});
        const what = item.type === "show" ? t("Adds all episodes of the show.") : item.type === "season" ? t("Adds all episodes of the season.") : t("Adds this title.");
        const close = modal(t("Add to playlist"),
            h("p", {class: "muted"}, what),
            lists.length ? h("div", {class: "pick-list"}, lists.map((p) => h("button", {class: "pick", onclick: async () => {
                    const r = await api("POST", `/api/playlists/${p.id}/items`, {itemId: item.id});
                    close();
                    M.toast(t("{0} added to “{1}”", r.added, p.title));
                }}, icon("playlist"), h("span", null, p.title), h("span", {class: "muted small"}, `${p.childCount}`)))) : null,
            h("div", {class: "field"}, h("label", {for: "pl-name"}, t("New playlist")), h("div", {class: "inline"}, name,
                h("button", {class: "btn primary", onclick: async () => {
                        try {
                            const p = await api("POST", "/api/playlists", {name: name.value, itemId: item.id});
                            close();
                            M.toast(t("Playlist “{0}” created", p.title));
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }}, t("Create")))),
            error);
    }

    async function addToCollection(item) {
        const colls = (await api("GET", `/api/libraries/${item.libraryId}/collections`)).filter((c) => !c.auto);
        const name = h("input", {id: "co-name", placeholder: t("Name of the new collection"), maxlength: 80});
        const error = h("div", {class: "error-text", role: "alert"});
        const close = modal(t("Add to collection"),
            colls.length ? h("div", {class: "pick-list"}, colls.map((c) => h("button", {class: "pick", onclick: async () => {
                    await api("POST", `/api/collections/${c.id}/items`, {itemId: item.id});
                    close();
                    M.toast(t("Added to “{0}”", c.title));
                }}, icon("collection"), h("span", null, c.title), h("span", {class: "muted small"}, `${c.childCount}`)))) : null,
            h("div", {class: "field"}, h("label", {for: "co-name"}, t("New collection")), h("div", {class: "inline"}, name,
                h("button", {class: "btn primary", onclick: async () => {
                        try {
                            await api("POST", "/api/collections", {libraryId: item.libraryId, name: name.value, itemId: item.id});
                            close();
                            M.toast(t("Collection created"));
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }}, t("Create")))),
            h("p", {class: "hint"}, t("Movie series from TMDB appear as collections automatically.")),
            error);
    }

    function fixMatch(item) {
        const q = h("input", {id: "fm-q", value: item.title});
        const year = h("input", {id: "fm-y", value: item.year || "", inputmode: "numeric", maxlength: 4, class: "year-input"});
        const results = h("div", {class: "match-results"});
        const error = h("div", {class: "error-text", role: "alert"});
        let close;
        let seq = 0;
        const search = async () => {
            const mine = ++seq; // only the latest search may fill the list
            error.textContent = "";
            results.replaceChildren(h("div", {class: "loading small"}, h("span", {class: "spin"})));
            try {
                const res = await api("GET", `/api/items/${item.id}/metadata/search?q=${encodeURIComponent(q.value)}&year=${encodeURIComponent(year.value || 0)}`);
                if (mine !== seq) return;
                results.replaceChildren(...(res.length ? res.map((r) => h("button", {class: "match", onclick: async () => {
                        results.replaceChildren(h("div", {class: "loading small"}, h("span", {class: "spin"})));
                        try {
                            await api("POST", `/api/items/${item.id}/metadata/match`, {tmdbId: r.id});
                            close();
                            M.toast(t("Matched: {0}", r.title));
                            M.render();
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }},
                    r.poster ? h("img", {src: `/api/metadata/image?path=${encodeURIComponent(r.poster)}`, alt: "", loading: "lazy"}) : h("div", {class: "match-noimg"}),
                    h("div", null, h("div", {class: "name"}, r.title, r.year ? h("span", {class: "muted"}, ` (${r.year})`) : null),
                        r.originalTitle && r.originalTitle !== r.title ? h("div", {class: "muted small"}, r.originalTitle) : null,
                        h("div", {class: "muted small clamp2"}, r.summary || "")))) : [h("p", {class: "muted"}, t("No matches."))]));
            } catch (e) {
                results.replaceChildren();
                error.textContent = e.message;
            }
        };
        close = modal(t("Fix match"),
            h("div", {class: "inline"}, q, year, h("button", {class: "btn primary", onclick: search}, t("Search"))),
            error, results);
        q.addEventListener("keydown", (e) => e.key === "Enter" && search());
        year.addEventListener("keydown", (e) => e.key === "Enter" && search());
        search();
    }

    function trailer(key, title) {
        modal(t("Trailer: {0}", title), h("div", {class: "trailer"},
            h("iframe", {src: `https://www.youtube-nocookie.com/embed/${encodeURIComponent(key)}?autoplay=1&rel=0`,
                title: t("Trailer: {0}", title), allow: "autoplay; encrypted-media; fullscreen; picture-in-picture",
                allowfullscreen: true, referrerpolicy: "strict-origin-when-cross-origin"})));
    }

    // ------------------------------------------------------------------ Watch Together

    async function startTogether(item) {
        const off = item.progress && !item.progress.watched ? item.progress.offsetMs : 0;
        const room = await api("POST", "/api/together", {itemId: item.id, positionMs: off});
        M.navigate(`/play/${item.id}?room=${room.code}${off ? `&t=${off}` : ""}`);
    }

    function inviteDialog(code) {
        const url = `${location.origin}/together/${code}`;
        modal(t("Invite friends"),
            h("p", null, t("Send this link to everyone who should watch along. They need an account on this server.")),
            h("div", {class: "inline"}, h("input", {value: url, readonly: true, onfocus: (e) => e.target.select(), "aria-label": t("Invitation link")}),
                h("button", {class: "btn primary", onclick: async () => {
                        try {
                            await navigator.clipboard.writeText(url);
                            M.toast(t("Link copied"));
                        } catch (e) {
                            M.toast(t("Please select and copy the link"), true);
                        }
                    }}, t("Copy"))),
            h("p", {class: "hint"}, M.tParts("Room code: {0}. Everyone in the room can play, pause and seek.", h("strong", {class: "mono"}, code))));
    }

    M.route("/together/:code", async (main, p) => {
        try {
            const room = await api("GET", `/api/together/${p.code}`);
            M.navigate(`/play/${room.itemId}?room=${room.code}`, true);
        } catch (e) {
            main.replaceChildren(M.emptyState(t("Room not found"), e.message, link("/", {class: "btn"}, t("Go to the home screen"))));
        }
    });

    // ------------------------------------------------------------------ menu entries for items

    M.menuExtensions.push((item) => {
        const out = [];
        if (item.type === "movie" || item.type === "episode") out.push({label: t("Watch Together"), onclick: () => startTogether(item)});
        out.push({label: t("Add to playlist"), onclick: () => addToPlaylist(item)});
        if (item.type === "movie" || item.type === "show") {
            out.push({label: item.onWatchlist ? t("Remove from watchlist") : t("Add to watchlist"), onclick: () => toggleWatchlist(item)});
        }
        if (S.user.admin && (item.type === "movie" || item.type === "show")) {
            out.push({label: t("Add to collection"), onclick: () => addToCollection(item)});
            out.push({label: t("Fix match"), onclick: () => fixMatch(item)});
        }
        if (S.user.admin && item.type !== "collection") {
            out.push({label: t("Refresh metadata"), onclick: async () => {
                    M.toast(t("Loading metadata …"));
                    try {
                        await api("POST", `/api/items/${item.id}/metadata/refresh`);
                        M.toast(t("Metadata refreshed"));
                        M.render();
                    } catch (e) {
                        M.toast(e.message, true);
                    }
                }});
        }
        return out;
    });

    async function toggleWatchlist(item) {
        const on = !item.onWatchlist;
        await api(on ? "PUT" : "DELETE", `/api/watchlist/${item.id}`);
        item.onWatchlist = on;
        M.toast(on ? t("Added to the watchlist") : t("Removed from the watchlist"));
        history.replaceState({keepScroll: true}, "");
        M.render();
    }

    // ------------------------------------------------------------------ pages

    M.route("/watchlist", async (main) => {
        const items = await api("GET", "/api/watchlist");
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, t("Watchlist")), h("span", {class: "count-pill"}, items.length)),
            items.length ? h("div", {class: "grid"}, items.map((i) => card(i, "poster")))
                : h("div", {class: "empty"}, h("h2", null, t("Nothing on the watchlist yet")),
                    h("p", null, t("Use the bookmark on a movie or show page to keep titles for later.")))));
    });

    M.route("/playlists", async (main) => {
        const lists = await api("GET", "/api/playlists");
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, "Playlists")),
            lists.length ? h("div", {class: "grid"}, lists.map((p) => card(p)))
                : h("div", {class: "empty"}, h("h2", null, t("No playlists yet")),
                    h("p", null, t("Create playlists from the “…” menu of a movie, show or episode.")))));
    });

    // ------------------------------------------------------------------ watch history

    M.route("/history", async (main) => {
        const all = S.user.admin && new URLSearchParams(location.search).get("all") === "1";
        const list = h("div", {class: "list history"});
        const more = h("button", {class: "btn", hidden: true}, t("Show more"));
        let offset = 0;

        function day(ms) {
            const d = new Date(ms);
            const today = new Date();
            const yesterday = new Date(Date.now() - 86400000);
            if (d.toDateString() === today.toDateString()) return t("Today");
            if (d.toDateString() === yesterday.toDateString()) return t("Yesterday");
            return d.toLocaleDateString(M.locale(), {weekday: "long", day: "numeric", month: "long", year: "numeric"});
        }

        let lastDay = null;

        function row(e) {
            const it = e.item;
            const landscape = it ? it.type === "episode" : false;
            const title = it ? it.title : e.title || t("Removed title");
            const parent = it && it.type === "episode" ? `${it.showTitle || ""} · ${M.episodeCode(it)}` : (it && it.year ? String(it.year) : e.parentTitle);
            const thumb = h("div", {class: "history-art" + (landscape ? " landscape" : "")},
                it ? M.art(it.id, landscape ? "thumb" : "poster", title) : h("div", {class: "placeholder"}));
            const time = new Date(e.watchedAt).toLocaleTimeString(M.locale(), {hour: "2-digit", minute: "2-digit"});
            const r = h("div", {class: "list-row history-row"},
                it ? link(`/item/${it.id}`, {class: "history-link"}, thumb) : thumb,
                h("div", {class: "grow"},
                    h("div", {class: "name"}, it ? link(`/item/${it.id}`, null, title) : title),
                    h("div", {class: "detail-line"}, [parent, time].filter(Boolean).join(" · "))),
                e.user ? h("span", {class: "history-user", title: e.user.username}, M.avatar(e.user, "small"), e.user.username) : null,
                all ? null : h("button", {class: "icon-btn", title: t("Remove from history"), "aria-label": t("Remove from history"), onclick: async () => {
                        await api("DELETE", `/api/me/history/${e.id}`);
                        r.remove();
                    }}, icon("close")));
            const label = day(e.watchedAt);
            if (label === lastDay) return r;
            lastDay = label;
            return [h("div", {class: "history-day"}, label), r];
        }

        async function load() {
            more.hidden = true;
            const res = await api("GET", (all ? "/api/history" : "/api/me/history") + `?offset=${offset}&limit=50`);
            offset += res.entries.length;
            list.append(...res.entries.flatMap((e) => [row(e)].flat()));
            more.hidden = !res.more;
            if (!offset) {
                list.replaceWith(h("div", {class: "empty"}, h("h2", null, t("Nothing watched yet")),
                    h("p", null, t("Movies and episodes appear here as soon as you have finished watching them."))));
            }
        }

        more.addEventListener("click", load);
        const head = h("div", {class: "page-head"}, h("h1", null, t("History")),
            S.user.admin ? h("nav", {class: "lib-tabs history-tabs", "aria-label": t("View")},
                link("/history", {class: all ? "" : "active"}, t("My history")),
                link("/history?all=1", {class: all ? "active" : ""}, t("All users"))) : null,
            all ? null : h("button", {class: "btn small push-right", "data-menu": true, onclick: (ev) => M.popup(ev.currentTarget, [
                    {label: t("Delete the whole history"), danger: true, onclick: async () => {
                            const r = await api("DELETE", "/api/me/history");
                            M.toast(tn(r.removed, "1 entry deleted", "{0} entries deleted"));
                            M.render();
                        }}])}, t("Delete history")));
        main.replaceChildren(h("div", {class: "page"}, head, list, h("div", {class: "btn-row center"}, more)));
        await load();
    });

    M.route("/playlist/:id", async (main, p) => {
        const pl = await api("GET", `/api/playlists/${p.id}`);
        const first = pl.items[0];
        const actions = h("div", {class: "actions"},
            first ? link(`/play/${first.id}?pl=${pl.id}`, {class: "btn primary play"}, icon("play"), t("Play")) : null,
            h("button", {class: "round-btn", title: t("Rename"), "aria-label": t("Rename"), onclick: () => {
                    const name = h("input", {id: "pl-rename", value: pl.title, maxlength: 80});
                    const close = modal(t("Rename playlist"), h("div", {class: "inline"}, name, h("button", {class: "btn primary", onclick: async () => {
                            await api("PUT", `/api/playlists/${pl.id}`, {name: name.value});
                            close();
                            M.render();
                        }}, t("Save"))));
                }}, icon("settings")),
            h("button", {class: "round-btn", title: t("Delete"), "aria-label": t("Delete playlist"), "data-menu": true, onclick: (e) => M.popup(e.currentTarget, [
                    {label: t("Really delete the playlist"), danger: true, onclick: async () => {
                            await api("DELETE", `/api/playlists/${pl.id}`);
                            M.toast(t("Playlist deleted"));
                            M.navigate("/playlists", true);
                        }}])}, icon("close")));
        const rows = h("div", {class: "episode-list"}, pl.items.map((it, i) => h("div", {class: "episode"},
            link(`/play/${it.id}?pl=${pl.id}`, {class: "card landscape", "aria-label": t("Play {0}", it.title)},
                h("div", {class: "art"}, M.art(it.id, "thumb", it.title), M.unwatchedBadge(it), M.progressBar(it),
                    h("div", {class: "card-actions"}, h("span", {class: "card-play", html: M.ICONS.play})))),
            h("div", {class: "ep-text"},
                link(`/item/${it.id}`, null, h("h3", null, it.type === "episode" ? `${it.showTitle}: ${it.title}` : it.title)),
                h("div", {class: "ep-meta"}, [it.type === "episode" ? M.episodeCode(it) : it.year, M.fmtDuration(it.media && it.media.durationMs)].filter(Boolean).join(", "))),
            h("button", {class: "round-btn small", title: t("Remove from playlist"), "aria-label": t("Remove from playlist"), onclick: async () => {
                    await api("DELETE", `/api/playlists/${pl.id}/items/${i}`);
                    M.render();
                }}, icon("close")))));
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, pl.title),
                h("span", {class: "muted"}, tn(pl.childCount, "1 entry", "{0} entries") + (pl.durationMs ? ", " + M.fmtDuration(pl.durationMs) : ""))),
            actions, pl.items.length ? rows : h("p", {class: "muted"}, t("This playlist is empty."))));
    });

    M.route("/library/:id/collections", async (main, p) => {
        const lib = S.libraries.find((l) => l.id === p.id) || (await api("GET", "/api/libraries")).find((l) => l.id === p.id);
        if (!lib) throw new M.ApiError(404, t("This library no longer exists."));
        const colls = await api("GET", `/api/libraries/${p.id}/collections`);
        main.replaceChildren(h("div", {class: "page"}, M.libraryHeader(lib, "collections"),
            colls.length ? h("div", {class: "grid"}, colls.map((c) => card(c)))
                : h("div", {class: "empty"}, h("h2", null, t("No collections yet")),
                    h("p", null, t("Movie series from TMDB appear here automatically as soon as at least two parts are in the library. Create your own collections from the “…” menu of a movie.")))));
    });

    M.route("/collection/:id", async (main, p) => {
        const c = await api("GET", `/api/collections/${p.id}`);
        const unwatched = c.items.find((i) => !(i.progress && i.progress.watched));
        const actions = h("div", {class: "actions"},
            unwatched && unwatched.type === "movie" ? link(M.playHref(unwatched), {class: "btn primary play"}, icon("play"), t("Play")) : null,
            !c.auto && S.user.admin ? h("button", {class: "round-btn", title: t("Edit collection"), "aria-label": t("Edit collection"), onclick: () => {
                    const name = h("input", {id: "co-n", value: c.title, maxlength: 80});
                    const summary = h("textarea", {id: "co-s", rows: 4}, c.summary || "");
                    const close = modal(t("Edit collection"),
                        h("div", {class: "field"}, h("label", {for: "co-n"}, t("Name")), name),
                        h("div", {class: "field"}, h("label", {for: "co-s"}, t("Description")), summary),
                        h("div", {class: "btn-row"},
                            h("button", {class: "btn primary", onclick: async () => {
                                    await api("PUT", `/api/collections/${c.id}`, {name: name.value, summary: summary.value});
                                    close();
                                    M.render();
                                }}, t("Save")),
                            h("button", {class: "btn danger", onclick: async () => {
                                    await api("DELETE", `/api/collections/${c.id}`);
                                    close();
                                    M.toast(t("Collection deleted"));
                                    M.navigate(`/library/${c.libraryId}/collections`, true);
                                }}, t("Delete collection"))));
                }}, icon("settings")) : null);
        const grid = h("div", {class: "grid"}, c.items.map((it) => {
            const el = card(it, "poster");
            if (!c.auto && S.user.admin) {
                el.querySelector(".card-actions").append(h("button", {class: "card-btn card-remove", title: t("Remove from collection"),
                    "aria-label": t("Remove from collection"), html: M.ICONS.close, onclick: async (e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        await api("DELETE", `/api/collections/${c.id}/items/${it.id}`);
                        M.render();
                    }}));
            }
            return el;
        }));
        const poster = h("div", {class: "mirror"}, h("div", {class: "poster"},
            h("img", {src: `/api/collections/${encodeURIComponent(c.id)}/image/poster`, alt: "", onerror: (e) => e.target.replaceWith(h("div", {class: "placeholder"}, h("span", null, c.title)))})));
        main.replaceChildren(h("div", {class: "detail"},
            h("div", {class: "backdrop"}, h("img", {src: `/api/collections/${encodeURIComponent(c.id)}/image/backdrop`, alt: "", onerror: (e) => e.target.remove()})),
            h("div", {class: "detail-body"}, poster, h("div", {class: "detail-info"},
                h("div", {class: "parent-title"}, t("Collection")),
                h("h1", null, c.title),
                h("div", {class: "facts"}, h("span", null, tn(c.childCount, "1 title", "{0} titles")), h("span", null, t("{0} watched", c.watchedCount)),
                    c.auto ? h("span", null, t("from TMDB")) : null),
                actions,
                c.summary ? h("p", {class: "summary"}, c.summary) : null)),
            h("div", {class: "detail-children"}, grid)));
    });

    M.route("/person/:id", async (main, p) => {
        const person = await api("GET", `/api/people/${p.id}`);
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "person-head"},
                h("div", {class: "person-photo"}, person.hasImage ? h("img", {src: `/api/people/${p.id}/image`, alt: ""}) : icon("user")),
                h("div", null, h("h1", null, person.name), h("div", {class: "muted"}, tn(person.items.length, "1 title in your libraries", "{0} titles in your libraries")))),
            h("div", {class: "grid"}, person.items.map((i) => card(i, "poster")))));
    });

    // ------------------------------------------------------------------ detail page parts

    /** Extra parts of the detail page: buttons, facts, cast, crew, collections. */
    M.detailExtras = (it, actions) => {
        const md = it.metadata || {};
        if (it.type === "movie" || it.type === "show") {
            const on = !!it.onWatchlist;
            actions.insertBefore(h("button", {class: "round-btn" + (on ? " on" : ""), title: on ? t("Remove from watchlist") : t("Add to watchlist"),
                "aria-label": on ? t("Remove from watchlist") : t("Add to watchlist"), "aria-pressed": String(on), onclick: () => toggleWatchlist(it)},
                icon(on ? "bookmarked" : "bookmark")), actions.lastChild);
        }
        if (md.trailer) {
            actions.insertBefore(h("button", {class: "round-btn", title: "Trailer", "aria-label": t("Watch trailer"),
                onclick: () => trailer(md.trailer, it.title)}, icon("trailer")), actions.lastChild);
        }
        if (it.type === "movie" || it.type === "episode") {
            actions.insertBefore(h("button", {class: "round-btn", title: t("Watch Together"), "aria-label": t("Watch Together"),
                onclick: () => startTogether(it)}, icon("together")), actions.lastChild);
        }
        const facts = [];
        if (md.contentRating) facts.push(h("span", {class: "chip rating-chip"}, md.contentRating));
        if (md.studio) facts.push(h("span", null, md.studio));
        const crew = [];
        if (md.directors && md.directors.length) crew.push([it.type === "show" ? t("Created by") : t("Director"), md.directors.join(", ")]);
        if (md.writers && md.writers.length) crew.push([t("Writer"), md.writers.join(", ")]);
        const crewEl = crew.length ? h("dl", {class: "crew"}, crew.map(([k, v]) => [h("dt", null, k), h("dd", null, v)])) : null;
        const colls = (it.collections || []).length ? h("div", {class: "in-collections"}, M.tParts("Part of {0}",
            it.collections.map((c, i) => [i ? ", " : "", link(`/collection/${c.id}`, null, c.name)]))) : null;
        const cast = md.cast && md.cast.length ? M.hubRow({title: t("Cast"), style: "cast", items: []}) : null;
        if (cast) {
            const row = cast.querySelector(".row");
            row.classList.add("cast-row");
            row.replaceChildren(...md.cast.map((c) => link(`/person/${c.id}`, {class: "cast"},
                h("div", {class: "cast-photo"}, c.hasImage ? h("img", {src: `/api/people/${c.id}/image`, alt: "", loading: "lazy"}) : icon("user")),
                h("div", {class: "cast-name"}, c.name), h("div", {class: "cast-role"}, c.role))));
        }
        const unmatched = S.user.admin && (it.type === "movie" || it.type === "show") && it.metadata && !it.metadata.matched
            ? h("p", {class: "hint"}, t("No online metadata found."), " ", h("button", {class: "link-btn", onclick: () => fixMatch(it)}, t("Fix match"))) : null;
        return {facts, crew: crewEl, collections: colls, cast, unmatched};
    };

    M.startTogether = startTogether;
    M.inviteDialog = inviteDialog;
})();

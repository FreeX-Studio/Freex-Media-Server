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
 * FreeX web interface, part 5: watchlist, playlists, collections, people, fix match,
 * trailers and Watch Together.
 */
(() => {
    "use strict";
    const M = window.FreeX;
    const {S, h, link, icon, api, card} = M;

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
                h("button", {class: "icon-btn", "aria-label": "Schließen", onclick: () => close()}, icon("close"))),
            content));
        document.body.append(back);
        back.querySelector("input, select, button.btn")?.focus();
        return close;
    }

    async function addToPlaylist(item) {
        const lists = await api("GET", "/api/playlists");
        const name = h("input", {id: "pl-name", placeholder: "Name der neuen Playlist", maxlength: 80});
        const error = h("div", {class: "error-text", role: "alert"});
        const what = item.type === "show" ? "alle Episoden der Serie" : item.type === "season" ? "alle Episoden der Staffel" : "diesen Titel";
        const close = modal("Zu Playlist hinzufügen",
            h("p", {class: "muted"}, `Fügt ${what} hinzu.`),
            lists.length ? h("div", {class: "pick-list"}, lists.map((p) => h("button", {class: "pick", onclick: async () => {
                    const r = await api("POST", `/api/playlists/${p.id}/items`, {itemId: item.id});
                    close();
                    M.toast(`${r.added} zu „${p.title}“ hinzugefügt`);
                }}, icon("playlist"), h("span", null, p.title), h("span", {class: "muted small"}, `${p.childCount}`)))) : null,
            h("div", {class: "field"}, h("label", {for: "pl-name"}, "Neue Playlist"), h("div", {class: "inline"}, name,
                h("button", {class: "btn primary", onclick: async () => {
                        try {
                            const p = await api("POST", "/api/playlists", {name: name.value, itemId: item.id});
                            close();
                            M.toast(`Playlist „${p.title}“ angelegt`);
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }}, "Anlegen"))),
            error);
    }

    async function addToCollection(item) {
        const colls = (await api("GET", `/api/libraries/${item.libraryId}/collections`)).filter((c) => !c.auto);
        const name = h("input", {id: "co-name", placeholder: "Name der neuen Sammlung", maxlength: 80});
        const error = h("div", {class: "error-text", role: "alert"});
        const close = modal("Zu Sammlung hinzufügen",
            colls.length ? h("div", {class: "pick-list"}, colls.map((c) => h("button", {class: "pick", onclick: async () => {
                    await api("POST", `/api/collections/${c.id}/items`, {itemId: item.id});
                    close();
                    M.toast(`Zu „${c.title}“ hinzugefügt`);
                }}, icon("collection"), h("span", null, c.title), h("span", {class: "muted small"}, `${c.childCount}`)))) : null,
            h("div", {class: "field"}, h("label", {for: "co-name"}, "Neue Sammlung"), h("div", {class: "inline"}, name,
                h("button", {class: "btn primary", onclick: async () => {
                        try {
                            await api("POST", "/api/collections", {libraryId: item.libraryId, name: name.value, itemId: item.id});
                            close();
                            M.toast("Sammlung angelegt");
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }}, "Anlegen"))),
            h("p", {class: "hint"}, "Filmreihen aus TMDB erscheinen automatisch als Sammlung."),
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
                            M.toast(`Zugeordnet: ${r.title}`);
                            M.render();
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }},
                    r.poster ? h("img", {src: `/api/metadata/image?path=${encodeURIComponent(r.poster)}`, alt: "", loading: "lazy"}) : h("div", {class: "match-noimg"}),
                    h("div", null, h("div", {class: "name"}, r.title, r.year ? h("span", {class: "muted"}, ` (${r.year})`) : null),
                        r.originalTitle && r.originalTitle !== r.title ? h("div", {class: "muted small"}, r.originalTitle) : null,
                        h("div", {class: "muted small clamp2"}, r.summary || "")))) : [h("p", {class: "muted"}, "Keine Treffer.")]));
            } catch (e) {
                results.replaceChildren();
                error.textContent = e.message;
            }
        };
        close = modal("Zuordnung korrigieren",
            h("div", {class: "inline"}, q, year, h("button", {class: "btn primary", onclick: search}, "Suchen")),
            error, results);
        q.addEventListener("keydown", (e) => e.key === "Enter" && search());
        year.addEventListener("keydown", (e) => e.key === "Enter" && search());
        search();
    }

    function trailer(key, title) {
        modal(`Trailer: ${title}`, h("div", {class: "trailer"},
            h("iframe", {src: `https://www.youtube-nocookie.com/embed/${encodeURIComponent(key)}?autoplay=1&rel=0`,
                title: `Trailer ${title}`, allow: "autoplay; encrypted-media; fullscreen; picture-in-picture",
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
        modal("Freunde einladen",
            h("p", null, "Schickt diesen Link an alle, die mitschauen sollen. Sie brauchen ein Konto auf diesem Server."),
            h("div", {class: "inline"}, h("input", {value: url, readonly: true, onfocus: (e) => e.target.select(), "aria-label": "Einladungslink"}),
                h("button", {class: "btn primary", onclick: async () => {
                        try {
                            await navigator.clipboard.writeText(url);
                            M.toast("Link kopiert");
                        } catch (e) {
                            M.toast("Bitte den Link markieren und kopieren", true);
                        }
                    }}, "Kopieren")),
            h("p", {class: "hint"}, "Raumcode: ", h("strong", {class: "mono"}, code), ". Jeder im Raum kann abspielen, pausieren und springen."));
    }

    M.route("/together/:code", async (main, p) => {
        try {
            const room = await api("GET", `/api/together/${p.code}`);
            M.navigate(`/play/${room.itemId}?room=${room.code}`, true);
        } catch (e) {
            main.replaceChildren(M.emptyState("Raum nicht gefunden", e.message, link("/", {class: "btn"}, "Zur Startseite")));
        }
    });

    // ------------------------------------------------------------------ menu entries for items

    M.menuExtensions.push((item) => {
        const out = [];
        if (item.type === "movie" || item.type === "episode") out.push({label: "Gemeinsam ansehen", onclick: () => startTogether(item)});
        out.push({label: "Zu Playlist hinzufügen", onclick: () => addToPlaylist(item)});
        if (item.type === "movie" || item.type === "show") {
            out.push({label: item.onWatchlist ? "Von Merkliste entfernen" : "Zur Merkliste hinzufügen", onclick: () => toggleWatchlist(item)});
        }
        if (S.user.admin && (item.type === "movie" || item.type === "show")) {
            out.push({label: "Zu Sammlung hinzufügen", onclick: () => addToCollection(item)});
            out.push({label: "Zuordnung korrigieren", onclick: () => fixMatch(item)});
        }
        if (S.user.admin && item.type !== "collection") {
            out.push({label: "Metadaten aktualisieren", onclick: async () => {
                    M.toast("Metadaten werden geladen …");
                    try {
                        await api("POST", `/api/items/${item.id}/metadata/refresh`);
                        M.toast("Metadaten aktualisiert");
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
        M.toast(on ? "Zur Merkliste hinzugefügt" : "Von der Merkliste entfernt");
        history.replaceState({keepScroll: true}, "");
        M.render();
    }

    // ------------------------------------------------------------------ pages

    M.route("/watchlist", async (main) => {
        const items = await api("GET", "/api/watchlist");
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, "Merkliste"), h("span", {class: "count-pill"}, items.length)),
            items.length ? h("div", {class: "grid"}, items.map((i) => card(i, "poster")))
                : h("div", {class: "empty"}, h("h2", null, "Noch nichts vorgemerkt"),
                    h("p", null, "Über das Lesezeichen auf einer Film- oder Serienseite merkst du dir Titel für später."))));
    });

    M.route("/playlists", async (main) => {
        const lists = await api("GET", "/api/playlists");
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, "Playlists")),
            lists.length ? h("div", {class: "grid"}, lists.map((p) => card(p)))
                : h("div", {class: "empty"}, h("h2", null, "Noch keine Playlists"),
                    h("p", null, "Über das „…“-Menü eines Films, einer Serie oder Episode legst du Playlists an."))));
    });

    // ------------------------------------------------------------------ watch history

    M.route("/history", async (main) => {
        const all = S.user.admin && new URLSearchParams(location.search).get("all") === "1";
        const list = h("div", {class: "list history"});
        const more = h("button", {class: "btn", hidden: true}, "Mehr anzeigen");
        let offset = 0;

        function day(ms) {
            const d = new Date(ms);
            const today = new Date();
            const yesterday = new Date(Date.now() - 86400000);
            if (d.toDateString() === today.toDateString()) return "Heute";
            if (d.toDateString() === yesterday.toDateString()) return "Gestern";
            return d.toLocaleDateString("de-DE", {weekday: "long", day: "numeric", month: "long", year: "numeric"});
        }

        let lastDay = null;

        function row(e) {
            const it = e.item;
            const landscape = it ? it.type === "episode" : false;
            const title = it ? it.title : e.title || "Entfernter Titel";
            const parent = it && it.type === "episode" ? `${it.showTitle || ""} · ${M.episodeCode(it)}` : (it && it.year ? String(it.year) : e.parentTitle);
            const thumb = h("div", {class: "history-art" + (landscape ? " landscape" : "")},
                it ? M.art(it.id, landscape ? "thumb" : "poster", title) : h("div", {class: "placeholder"}));
            const time = new Date(e.watchedAt).toLocaleTimeString("de-DE", {hour: "2-digit", minute: "2-digit"});
            const r = h("div", {class: "list-row history-row"},
                it ? link(`/item/${it.id}`, {class: "history-link"}, thumb) : thumb,
                h("div", {class: "grow"},
                    h("div", {class: "name"}, it ? link(`/item/${it.id}`, null, title) : title),
                    h("div", {class: "detail-line"}, [parent, time].filter(Boolean).join(" · "))),
                e.user ? h("span", {class: "history-user", title: e.user.username}, M.avatar(e.user, "small"), e.user.username) : null,
                all ? null : h("button", {class: "icon-btn", title: "Aus dem Verlauf entfernen", "aria-label": "Aus dem Verlauf entfernen", onclick: async () => {
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
                list.replaceWith(h("div", {class: "empty"}, h("h2", null, "Noch nichts angesehen"),
                    h("p", null, "Filme und Episoden erscheinen hier, sobald du sie zu Ende geschaut hast.")));
            }
        }

        more.addEventListener("click", load);
        const head = h("div", {class: "page-head"}, h("h1", null, "Verlauf"),
            S.user.admin ? h("nav", {class: "lib-tabs history-tabs", "aria-label": "Ansicht"},
                link("/history", {class: all ? "" : "active"}, "Mein Verlauf"),
                link("/history?all=1", {class: all ? "active" : ""}, "Alle Benutzer")) : null,
            all ? null : h("button", {class: "btn small push-right", "data-menu": true, onclick: (ev) => M.popup(ev.currentTarget, [
                    {label: "Ganzen Verlauf löschen", danger: true, onclick: async () => {
                            const r = await api("DELETE", "/api/me/history");
                            M.toast(`${r.removed} Einträge gelöscht`);
                            M.render();
                        }}])}, "Verlauf löschen"));
        main.replaceChildren(h("div", {class: "page"}, head, list, h("div", {class: "btn-row center"}, more)));
        await load();
    });

    M.route("/playlist/:id", async (main, p) => {
        const pl = await api("GET", `/api/playlists/${p.id}`);
        const first = pl.items[0];
        const actions = h("div", {class: "actions"},
            first ? link(`/play/${first.id}?pl=${pl.id}`, {class: "btn primary play"}, icon("play"), "Abspielen") : null,
            h("button", {class: "round-btn", title: "Umbenennen", "aria-label": "Umbenennen", onclick: () => {
                    const name = h("input", {id: "pl-rename", value: pl.title, maxlength: 80});
                    const close = modal("Playlist umbenennen", h("div", {class: "inline"}, name, h("button", {class: "btn primary", onclick: async () => {
                            await api("PUT", `/api/playlists/${pl.id}`, {name: name.value});
                            close();
                            M.render();
                        }}, "Speichern")));
                }}, icon("settings")),
            h("button", {class: "round-btn", title: "Löschen", "aria-label": "Playlist löschen", "data-menu": true, onclick: (e) => M.popup(e.currentTarget, [
                    {label: "Playlist wirklich löschen", danger: true, onclick: async () => {
                            await api("DELETE", `/api/playlists/${pl.id}`);
                            M.toast("Playlist gelöscht");
                            M.navigate("/playlists", true);
                        }}])}, icon("close")));
        const rows = h("div", {class: "episode-list"}, pl.items.map((it, i) => h("div", {class: "episode"},
            link(`/play/${it.id}?pl=${pl.id}`, {class: "card landscape", "aria-label": `${it.title} abspielen`},
                h("div", {class: "art"}, M.art(it.id, "thumb", it.title), M.unwatchedBadge(it), M.progressBar(it),
                    h("div", {class: "card-actions"}, h("span", {class: "card-play", html: M.ICONS.play})))),
            h("div", {class: "ep-text"},
                link(`/item/${it.id}`, null, h("h3", null, it.type === "episode" ? `${it.showTitle}: ${it.title}` : it.title)),
                h("div", {class: "ep-meta"}, [it.type === "episode" ? M.episodeCode(it) : it.year, M.fmtDuration(it.media && it.media.durationMs)].filter(Boolean).join(", "))),
            h("button", {class: "round-btn small", title: "Aus Playlist entfernen", "aria-label": "Aus Playlist entfernen", onclick: async () => {
                    await api("DELETE", `/api/playlists/${pl.id}/items/${i}`);
                    M.render();
                }}, icon("close")))));
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "page-head"}, h("h1", null, pl.title),
                h("span", {class: "muted"}, `${pl.childCount} Einträge${pl.durationMs ? ", " + M.fmtDuration(pl.durationMs) : ""}`)),
            actions, pl.items.length ? rows : h("p", {class: "muted"}, "Diese Playlist ist leer.")));
    });

    M.route("/library/:id/collections", async (main, p) => {
        const lib = S.libraries.find((l) => l.id === p.id) || (await api("GET", "/api/libraries")).find((l) => l.id === p.id);
        if (!lib) throw new M.ApiError(404, "Diese Bibliothek gibt es nicht mehr.");
        const colls = await api("GET", `/api/libraries/${p.id}/collections`);
        main.replaceChildren(h("div", {class: "page"}, M.libraryHeader(lib, "collections"),
            colls.length ? h("div", {class: "grid"}, colls.map((c) => card(c)))
                : h("div", {class: "empty"}, h("h2", null, "Noch keine Sammlungen"),
                    h("p", null, "Filmreihen aus TMDB erscheinen hier automatisch, sobald mindestens zwei Teile in der Bibliothek sind. Eigene Sammlungen legst du über das „…“-Menü eines Films an."))));
    });

    M.route("/collection/:id", async (main, p) => {
        const c = await api("GET", `/api/collections/${p.id}`);
        const unwatched = c.items.find((i) => !(i.progress && i.progress.watched));
        const actions = h("div", {class: "actions"},
            unwatched && unwatched.type === "movie" ? link(M.playHref(unwatched), {class: "btn primary play"}, icon("play"), "Abspielen") : null,
            !c.auto && S.user.admin ? h("button", {class: "round-btn", title: "Sammlung bearbeiten", "aria-label": "Sammlung bearbeiten", onclick: () => {
                    const name = h("input", {id: "co-n", value: c.title, maxlength: 80});
                    const summary = h("textarea", {id: "co-s", rows: 4}, c.summary || "");
                    const close = modal("Sammlung bearbeiten",
                        h("div", {class: "field"}, h("label", {for: "co-n"}, "Name"), name),
                        h("div", {class: "field"}, h("label", {for: "co-s"}, "Beschreibung"), summary),
                        h("div", {class: "btn-row"},
                            h("button", {class: "btn primary", onclick: async () => {
                                    await api("PUT", `/api/collections/${c.id}`, {name: name.value, summary: summary.value});
                                    close();
                                    M.render();
                                }}, "Speichern"),
                            h("button", {class: "btn danger", onclick: async () => {
                                    await api("DELETE", `/api/collections/${c.id}`);
                                    close();
                                    M.toast("Sammlung gelöscht");
                                    M.navigate(`/library/${c.libraryId}/collections`, true);
                                }}, "Sammlung löschen")));
                }}, icon("settings")) : null);
        const grid = h("div", {class: "grid"}, c.items.map((it) => {
            const el = card(it, "poster");
            if (!c.auto && S.user.admin) {
                el.querySelector(".card-actions").append(h("button", {class: "card-btn card-remove", title: "Aus Sammlung entfernen",
                    "aria-label": "Aus Sammlung entfernen", html: M.ICONS.close, onclick: async (e) => {
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
                h("div", {class: "parent-title"}, "Sammlung"),
                h("h1", null, c.title),
                h("div", {class: "facts"}, h("span", null, `${c.childCount} Titel`), h("span", null, `${c.watchedCount} gesehen`),
                    c.auto ? h("span", null, "aus TMDB") : null),
                actions,
                c.summary ? h("p", {class: "summary"}, c.summary) : null)),
            h("div", {class: "detail-children"}, grid)));
    });

    M.route("/person/:id", async (main, p) => {
        const person = await api("GET", `/api/people/${p.id}`);
        main.replaceChildren(h("div", {class: "page"},
            h("div", {class: "person-head"},
                h("div", {class: "person-photo"}, person.hasImage ? h("img", {src: `/api/people/${p.id}/image`, alt: ""}) : icon("user")),
                h("div", null, h("h1", null, person.name), h("div", {class: "muted"}, `${person.items.length} Titel in deinen Bibliotheken`))),
            h("div", {class: "grid"}, person.items.map((i) => card(i, "poster")))));
    });

    // ------------------------------------------------------------------ detail page parts

    /** Extra parts of the detail page: buttons, facts, cast, crew, collections. */
    M.detailExtras = (it, actions) => {
        const md = it.metadata || {};
        if (it.type === "movie" || it.type === "show") {
            const on = !!it.onWatchlist;
            actions.insertBefore(h("button", {class: "round-btn" + (on ? " on" : ""), title: on ? "Von Merkliste entfernen" : "Zur Merkliste",
                "aria-label": on ? "Von Merkliste entfernen" : "Zur Merkliste", "aria-pressed": String(on), onclick: () => toggleWatchlist(it)},
                icon(on ? "bookmarked" : "bookmark")), actions.lastChild);
        }
        if (md.trailer) {
            actions.insertBefore(h("button", {class: "round-btn", title: "Trailer", "aria-label": "Trailer ansehen",
                onclick: () => trailer(md.trailer, it.title)}, icon("trailer")), actions.lastChild);
        }
        if (it.type === "movie" || it.type === "episode") {
            actions.insertBefore(h("button", {class: "round-btn", title: "Gemeinsam ansehen", "aria-label": "Gemeinsam ansehen",
                onclick: () => startTogether(it)}, icon("together")), actions.lastChild);
        }
        const facts = [];
        if (md.contentRating) facts.push(h("span", {class: "chip rating-chip"}, md.contentRating));
        if (md.studio) facts.push(h("span", null, md.studio));
        const crew = [];
        if (md.directors && md.directors.length) crew.push([it.type === "show" ? "Erfunden von" : "Regie", md.directors.join(", ")]);
        if (md.writers && md.writers.length) crew.push(["Drehbuch", md.writers.join(", ")]);
        const crewEl = crew.length ? h("dl", {class: "crew"}, crew.map(([k, v]) => [h("dt", null, k), h("dd", null, v)])) : null;
        const colls = (it.collections || []).length ? h("div", {class: "in-collections"}, "Teil von ",
            it.collections.map((c, i) => [i ? ", " : "", link(`/collection/${c.id}`, null, c.name)])) : null;
        const cast = md.cast && md.cast.length ? M.hubRow({title: "Besetzung", style: "cast", items: []}) : null;
        if (cast) {
            const row = cast.querySelector(".row");
            row.classList.add("cast-row");
            row.replaceChildren(...md.cast.map((c) => link(`/person/${c.id}`, {class: "cast"},
                h("div", {class: "cast-photo"}, c.hasImage ? h("img", {src: `/api/people/${c.id}/image`, alt: "", loading: "lazy"}) : icon("user")),
                h("div", {class: "cast-name"}, c.name), h("div", {class: "cast-role"}, c.role))));
        }
        const unmatched = S.user.admin && (it.type === "movie" || it.type === "show") && it.metadata && !it.metadata.matched
            ? h("p", {class: "hint"}, "Keine Online-Metadaten gefunden. ", h("button", {class: "link-btn", onclick: () => fixMatch(it)}, "Zuordnung korrigieren")) : null;
        return {facts, crew: crewEl, collections: colls, cast, unmatched};
    };

    M.startTogether = startTogether;
    M.inviteDialog = inviteDialog;
})();

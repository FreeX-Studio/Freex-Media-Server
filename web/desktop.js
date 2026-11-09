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
 * FreeX Web, FreeX Desktop part: only active when FreeX Web runs inside the Windows app
 * (github.com/FreeX-Studio/freex-desktop), which loads it straight from the server and gives its pages
 * a few app commands. In a browser this file does nothing and M.desktop stays undefined.
 *
 * The app adds: switching to another server, real full screen for the player, and playback in
 * mpv. With mpv every file plays directly (no conversion on the server); the position comes back
 * from the app as events and is saved here, just as the built-in player does.
 */
(() => {
    "use strict";
    const M = window.FreeX;
    const tauri = window.__TAURI__;
    if (!tauri || !tauri.core || typeof tauri.core.invoke !== "function") return;
    const {S, t, h, icon, api} = M;
    const invoke = tauri.core.invoke;

    let infoPromise = null;

    /** App name and version, player choice and whether mpv is installed. */
    function info(fresh) {
        if (fresh || !infoPromise) infoPromise = invoke("desktop_info").catch(() => null);
        return infoPromise;
    }

    async function usesMpv() {
        const i = await info();
        return !!(i && i.player === "mpv" && i.mpvAvailable);
    }

    function switchServer() {
        invoke("show_servers").catch((e) => M.toast(String(e), true));
    }

    // The page's full screen (player) also makes the window full screen.
    document.addEventListener("fullscreenchange", () => {
        const win = tauri.window && tauri.window.getCurrentWindow && tauri.window.getCurrentWindow();
        if (win) win.setFullscreen(!!document.fullscreenElement).catch(() => null);
    });

    // ------------------------------------------------------------------ settings page

    async function settingsPage(c) {
        const i = await info(true);
        if (!i) return c.append(h("h1", null, "FreeX Desktop"));
        const player = h("select", {id: "d-player"},
            h("option", {value: "builtin"}, t("FreeX player")),
            h("option", {value: "mpv", disabled: !i.mpvAvailable && i.player !== "mpv"}, t("mpv (plays every file directly)")));
        player.value = i.player;
        const fs = h("input", {type: "checkbox", id: "d-mpv-fs", checked: i.mpvFullscreen});
        const fsRow = h("label", {class: "check"}, fs, h("span", null, t("Start mpv in full screen")));
        const hint = h("div", {class: "hint"});
        const sync = () => {
            fsRow.hidden = player.value !== "mpv";
            hint.textContent = player.value === "mpv"
                ? (i.mpvAvailable ? t("mpv plays every file directly, without conversion on the server. Skip intro and Watch Together are only available in the FreeX player.")
                    : t("mpv was not found. Reinstall FreeX Desktop or install mpv and add it to the PATH."))
                : t("The FreeX player plays in the app and converts files your PC cannot play on the server.");
        };
        const save = async () => {
            try {
                await invoke("set_player", {player: player.value, mpvFullscreen: fs.checked});
                await info(true);
                sync();
                M.toast(t("Playback settings saved"));
            } catch (e) {
                M.toast(String(e), true);
            }
        };
        player.addEventListener("change", save);
        fs.addEventListener("change", save);
        sync();
        c.append(h("h1", null, "FreeX Desktop"),
            h("section", {class: "panel"}, h("h2", null, t("Player")),
                h("div", {class: "field"}, h("label", {for: "d-player"}, t("Play videos with")), player, hint),
                fsRow),
            h("section", {class: "panel"}, h("h2", null, t("Server")),
                h("p", {class: "muted"}, i.server ? t("Connected to {0} ({1}).", i.server.name, i.server.url) : ""),
                h("button", {class: "btn", onclick: switchServer}, t("Switch server"))),
            h("section", {class: "panel"}, h("h2", null, t("Versions")),
                h("dl", {class: "kv"}, h("dt", null, i.app), h("dd", null, i.version))));
    }

    // ------------------------------------------------------------------ playback in mpv

    /**
     * Plays an item in mpv instead of the built-in player. {audio, subtitle} are the tracks as
     * chosen for the built-in player (audio index, subtitle id), startMs the start position.
     */
    async function playInMpv(item, o) {
        const media = item.media || {};
        const durationMs = media.durationMs || 0;
        // mpv numbers tracks from 1 in file order; external files come after the embedded ones.
        const subs = media.subtitles || [];
        const embedded = subs.filter((s) => !s.external);
        const external = subs.filter((s) => s.external);
        let sid = null;
        if (o.subtitle) {
            const e = embedded.findIndex((s) => s.id === o.subtitle);
            const x = external.findIndex((s) => s.id === o.subtitle);
            sid = e >= 0 ? e + 1 : x >= 0 ? embedded.length + x + 1 : null;
        }
        const audios = media.audio || [];
        const a = audios.findIndex((x) => x.index === o.audio);
        const clientId = (crypto.randomUUID ? crypto.randomUUID() : String(Math.random()).slice(2)).replace(/-/g, "").slice(0, 32);

        const heading = item.type === "episode" ? item.showTitle : item.title;
        const sub = item.type === "episode" ? `${M.episodeCode(item)}, ${item.title}` : (item.year || "");
        const status = h("div", {class: "mpv-status"}, t("Starting mpv …"));
        const time = h("div", {class: "mpv-time"});
        const bar = h("div", {class: "mpv-bar"}, h("span"));
        const actions = h("div", {class: "btn-row"});
        const back = () => history.length > 1 ? history.back() : M.navigate(`/item/${item.id}`, true);
        const stopBtn = h("button", {class: "btn primary", onclick: () => invoke("mpv_stop")}, icon("close"), t("Stop playback"));
        actions.append(stopBtn, h("button", {class: "btn", onclick: back}, t("Back")));
        const view = h("div", {class: "mpv-play"},
            h("div", {class: "mpv-backdrop"}, M.art(item, "backdrop", item.title)),
            h("div", {class: "mpv-box"},
                h("div", {class: "mpv-art" + (item.type === "episode" ? " wide" : "")}, M.art(item, item.type === "episode" ? "thumb" : "poster", item.title)),
                h("div", {class: "mpv-text"},
                    h("div", {class: "mpv-label"}, icon("play"), t("Playing in mpv")),
                    h("h1", null, heading), sub ? h("div", {class: "muted"}, sub) : null,
                    bar, time, status, actions)));
        document.getElementById("app").replaceChildren(view);
        document.title = `${heading} – ${S.status.name}`;

        let ended = false;
        let lastSent = -1;
        let lastSentAt = 0;
        let position = o.startMs || 0;
        let total = durationMs;
        let nextTimer = null;

        function show(pos, dur, paused) {
            position = pos;
            if (dur) total = dur;
            time.textContent = total ? `${M.fmtClock(pos)} / ${M.fmtClock(total)}` : M.fmtClock(pos);
            bar.firstChild.style.width = total ? `${Math.min(100, pos / total * 100)}%` : "0";
            status.textContent = paused ? t("Paused") : "";
        }

        function report(pos, paused, stopped) {
            if (!stopped && Math.abs(pos - lastSent) < 5000 && Date.now() - lastSentAt < 10000) return;
            lastSent = pos;
            lastSentAt = Date.now();
            api("PUT", `/api/items/${item.id}/progress`, {offsetMs: Math.round(pos), durationMs: total,
                client: clientId, state: paused ? "paused" : "playing", stopped: !!stopped}).catch(() => null);
        }

        function finish(e) {
            ended = true;
            stopBtn.remove();
            show(e.positionMs, e.durationMs, false);
            report(e.positionMs, false, true);
            if (e.error) {
                status.textContent = e.error;
                status.classList.add("error-text");
                return;
            }
            const next = o.nextItem;
            if (e.eof && next) {
                let left = 10;
                const go = () => M.navigate(o.nextHref(next), true);
                const label = h("span");
                const tick = () => label.textContent = t("Next in {0} s: {1}", left, next.type === "episode"
                    ? `${M.episodeCode(next)} ${next.title}` : next.title);
                tick();
                nextTimer = setInterval(() => --left <= 0 ? go() : tick(), 1000);
                status.replaceChildren(label);
                actions.replaceChildren(h("button", {class: "btn primary", onclick: go}, icon("play"), t("Play now")),
                    h("button", {class: "btn", onclick: () => {
                            clearInterval(nextTimer);
                            status.textContent = "";
                            actions.replaceChildren(h("button", {class: "btn", onclick: back}, t("Back")));
                        }}, t("Cancel")));
                return;
            }
            back();
        }

        const unlisten = await tauri.event.listen("freex-mpv", (ev) => {
            const e = ev.payload || {};
            if (e.itemId !== item.id || ended) return;
            if (e.kind === "started") status.textContent = "";
            if (e.kind === "progress") {
                show(e.positionMs, e.durationMs, e.paused);
                report(e.positionMs, e.paused, false);
            }
            if (e.kind === "ended") finish(e);
        });

        S.cleanup = () => {
            unlisten();
            clearInterval(nextTimer);
            if (!ended) {
                ended = true;
                invoke("mpv_stop").catch(() => null);
                report(position, false, true);
            }
            view.remove();
            document.title = S.status.name;
        };

        try {
            // Announces the playback in the activity list and gives the stream URL.
            const plan = await api("POST", `/api/items/${item.id}/playback`, {player: "external", audio: o.audio,
                subtitle: o.subtitle, startMs: Math.max(0, Math.round(o.startMs || 0)), client: clientId, device: "FreeX Desktop (mpv)"});
            show(plan.startMs || 0, total, false);
            await invoke("mpv_play", {request: {
                itemId: item.id,
                url: new URL(plan.url, location.origin).href,
                title: sub ? `${heading} – ${sub}` : heading,
                startMs: plan.startMs || 0,
                token: S.token || null,
                audio: a >= 0 ? a + 1 : null,
                subtitle: sid,
                subtitleFiles: external.map((s) => new URL(`/api/items/${item.id}/subtitles/${s.id}.vtt`, location.origin).href),
            }});
        } catch (e) {
            ended = true;
            stopBtn.remove();
            status.textContent = e && e.message ? e.message : String(e);
            status.classList.add("error-text");
        }
    }

    M.desktop = {info, usesMpv, switchServer, settingsPage, playInMpv};
})();

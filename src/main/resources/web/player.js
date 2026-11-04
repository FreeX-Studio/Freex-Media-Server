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
 * FreeX web interface, part 3: the player.
 *
 * The server decides per file whether it is played directly or converted to HLS (see
 * PlaybackPlanner). A converted stream starts at the requested position, so the player keeps an
 * offset and shows absolute times. Seeking inside what was already converted stays in the
 * stream; seeking further ahead asks the server for a new stream from there. Subtitles are
 * rendered here (not with <track>), so they stay in sync with that offset.
 */
(() => {
    "use strict";
    const M = window.FreeX;
    const {S, h, icon, api} = M;

    // ------------------------------------------------------------------ browser capabilities

    function capabilities(directPlay) {
        const v = document.createElement("video");
        const can = (t) => v.canPlayType(t) !== "";
        const hasMse = !!(window.MediaSource && window.Hls && window.Hls.isSupported());
        const nativeHls = can("application/vnd.apple.mpegurl");
        const mse = (t) => hasMse ? MediaSource.isTypeSupported(t) : nativeHls && can(t);
        const pick = (table, test) => Object.entries(table).filter(([, types]) => types.some(test)).map(([k]) => k);
        const videoTypes = {
            h264: ['video/mp4; codecs="avc1.640028"'],
            hevc: ['video/mp4; codecs="hvc1.1.6.L120.90"', 'video/mp4; codecs="hev1.1.6.L120.90"'],
            vp9: ['video/webm; codecs="vp9"', 'video/mp4; codecs="vp09.00.10.08"'],
            av1: ['video/mp4; codecs="av01.0.08M.08"'],
        };
        const audioTypes = {
            aac: ['audio/mp4; codecs="mp4a.40.2"'],
            mp3: ["audio/mpeg", 'audio/mp4; codecs="mp3"'],
            opus: ['audio/webm; codecs="opus"', 'audio/mp4; codecs="opus"'],
            vorbis: ['audio/webm; codecs="vorbis"'],
            flac: ["audio/flac", 'audio/mp4; codecs="flac"'],
            ac3: ['audio/mp4; codecs="ac-3"'],
            eac3: ['audio/mp4; codecs="ec-3"'],
        };
        const containers = [];
        if (can("video/mp4")) containers.push("mp4");
        if (can("video/webm")) containers.push("webm");
        // Chrome and Edge play Matroska even though some versions report "" here; a failed attempt
        // falls back to conversion automatically.
        if (can("video/x-matroska") || can('video/x-matroska; codecs="avc1.640028"') || /Chrome\//.test(navigator.userAgent)) containers.push("mkv");
        return {
            containers,
            video: pick(videoTypes, can),
            audio: pick(audioTypes, can),
            mseVideo: pick({h264: videoTypes.h264, hevc: videoTypes.hevc, av1: videoTypes.av1}, mse),
            mseAudio: pick({aac: audioTypes.aac, mp3: ['audio/mp4; codecs="mp3"', "audio/mpeg"], ac3: audioTypes.ac3,
                eac3: audioTypes.eac3, flac: ['audio/mp4; codecs="flac"'], opus: ['audio/mp4; codecs="opus"']}, mse),
            directPlay,
            hls: hasMse || nativeHls,
        };
    }

    // ------------------------------------------------------------------ WebVTT

    function parseTime(t) {
        const p = t.trim().split(":").map(Number);
        return p.length === 3 ? (p[0] * 3600 + p[1] * 60 + p[2]) * 1000 : (p[0] * 60 + p[1]) * 1000;
    }

    function parseVtt(text) {
        const cues = [];
        for (const block of text.replace(/\r/g, "").split(/\n\n+/)) {
            const lines = block.split("\n");
            const i = lines.findIndex((l) => l.includes("-->"));
            if (i < 0) continue;
            const [a, b] = lines[i].split("-->");
            const end = b.trim().split(/\s+/)[0];
            const body = lines.slice(i + 1).join("\n").replace(/<[^>]+>/g, "").replace(/\{\\[^}]*}/g, "").trim();
            if (body) cues.push({start: parseTime(a), end: parseTime(end), text: body});
        }
        cues.sort((x, y) => x.start - y.start);
        return cues;
    }

    // ------------------------------------------------------------------ the view

    M.route("/play/:id", async (main, params, query) => {
        let item;
        try {
            item = await api("GET", `/api/items/${params.id}`);
        } catch (e) {
            M.toast(e.message, true);
            return M.navigate("/", true);
        }
        const media = item.media;
        const durationMs = media.durationMs || 0;
        // Tracks from the link (picked on the detail page), otherwise from the user's preferences.
        const preferred = M.defaultTracks(media, S.user.preferences);
        const state = {
            quality: M.store.get("freex.quality") || "original",
            audio: query.get("a") != null ? Number(query.get("a")) : preferred.audio,
            subtitle: query.get("s") != null ? (query.get("s") === "none" ? null : query.get("s")) : preferred.sub,
            directPlay: true,
            plan: null,
            offsetMs: 0,
            hls: null,
            cues: [],
            pendingSeek: 0,
            starting: false,
        };
        const roomCode = query.get("room");
        // Identifies this playback in the server's activity list (one per opened player).
        const clientId = (crypto.randomUUID ? crypto.randomUUID() : String(Math.random()).slice(2)).replace(/-/g, "").slice(0, 32);
        const playlistId = query.get("pl");
        let remoteApplying = false;

        // What plays after this one: the next playlist entry, otherwise the next episode.
        let nextItem = item.next || null;
        if (playlistId) {
            try {
                const pl = await api("GET", `/api/playlists/${playlistId}`);
                const i = pl.items.findIndex((x) => x.id === item.id);
                nextItem = i >= 0 && i + 1 < pl.items.length ? pl.items[i + 1] : null;
            } catch (e) {
                nextItem = item.next || null;
            }
        }
        const nextHref = (n) => {
            const q = new URLSearchParams();
            const off = n.progress && !n.progress.watched ? n.progress.offsetMs : 0;
            if (off && !roomCode) q.set("t", off);
            if (playlistId) q.set("pl", playlistId);
            if (roomCode) q.set("room", roomCode);
            const qs = q.toString();
            return `/play/${n.id}` + (qs ? "?" + qs : "");
        };
        const markers = item.markers || {};

        const video = h("video", {playsinline: true, preload: "auto"});
        video.volume = Math.min(1, Math.max(0, Number(M.store.get("freex.volume") ?? 1)));
        video.muted = M.store.get("freex.muted") === "1";
        const subs = h("div", {class: "subs", "aria-live": "off"});
        const spinner = h("div", {class: "player-spinner"}, h("span", {class: "spin big"}));
        const notice = h("div", {class: "notice", hidden: true});

        const back = () => history.length > 1 ? history.back() : M.navigate(`/item/${item.id}`, true);
        const heading = item.type === "episode" ? item.showTitle : item.title;
        const sub = item.type === "episode" ? `${M.episodeCode(item)}, ${item.title}` : (item.year || "");

        // ---- bottom bar
        const seek = h("input", {type: "range", class: "seek", min: 0, max: Math.max(1, Math.round(durationMs / 1000)), step: 1, value: 0,
            "aria-label": "Position"});
        const produced = h("div", {class: "seek-produced"});
        const seekWrap = h("div", {class: "seek-wrap"}, produced, seek);
        const timeNow = h("span", {class: "time"}, "0:00");
        const timeTotal = h("span", {class: "time muted"}, M.fmtClock(durationMs));
        const playBtn = h("button", {class: "ctl big", "aria-label": "Abspielen"}, icon("play"));
        const muteBtn = h("button", {class: "ctl", "aria-label": "Ton aus"}, icon(video.muted ? "mute" : "volume"));
        const vol = h("input", {type: "range", class: "volume", min: 0, max: 1, step: 0.05, value: video.muted ? 0 : video.volume, "aria-label": "Lautstärke"});
        const settingsBtn = h("button", {class: "ctl", "aria-label": "Wiedergabe-Einstellungen", "data-menu": true}, icon("gear"));
        const castBtn = h("button", {class: "ctl cast-btn", "aria-label": "Auf Chromecast abspielen", title: "Auf Chromecast abspielen", hidden: true}, icon("cast"));
        const castInfo = h("div", {class: "cast-info", hidden: true});
        const fsBtn = h("button", {class: "ctl", "aria-label": "Vollbild"}, icon("fullscreen"));
        const controls = h("div", {class: "controls"},
            seekWrap,
            h("div", {class: "ctl-row"},
                h("div", {class: "ctl-left"},
                    h("div", {class: "now-art"}, M.art(item.id, item.type === "episode" ? "thumb" : "poster", item.title)),
                    h("div", {class: "now-text"}, h("div", {class: "now-title"}, heading), h("div", {class: "now-sub"}, sub))),
                h("div", {class: "ctl-center"},
                    h("button", {class: "ctl", "aria-label": "10 Sekunden zurück", onclick: () => seekTo(position() - 10000, true)}, icon("replay10")),
                    playBtn,
                    h("button", {class: "ctl", "aria-label": "30 Sekunden vor", onclick: () => seekTo(position() + 30000, true)}, icon("forward30"))),
                h("div", {class: "ctl-right"}, timeNow, h("span", {class: "time muted"}, "/"), timeTotal, muteBtn, vol, castBtn, settingsBtn, fsBtn)));
        const members = h("div", {class: "room-members"});
        const top = h("div", {class: "player-top"},
            h("button", {class: "ctl", onclick: back, "aria-label": "Zurück", title: "Zurück"}, icon("back")),
            roomCode ? h("div", {class: "room"}, icon("together"), h("span", null, "Gemeinsam ansehen"), members,
                h("button", {class: "btn small", onclick: () => M.inviteDialog(roomCode)}, "Einladen")) : null);
        const skipBtn = h("button", {class: "skip-btn", hidden: true});
        const player = h("div", {class: "player"}, video, castInfo, subs, spinner, notice, top, skipBtn, controls);
        document.getElementById("app").replaceChildren(player);
        document.title = `${heading} – ${S.status.name}`;

        // ---- time handling
        // While casting, the Chromecast's position counts (see the Chromecast section below).
        let casting = null;
        const position = () => casting && casting.plan ? casting.plan.offsetMs + casting.remote.currentTime * 1000
            : state.plan && state.plan.mode !== "direct" ? state.offsetMs + video.currentTime * 1000 : video.currentTime * 1000;
        const isDirect = () => state.plan && state.plan.mode === "direct";

        function producedEndMs() {
            if (isDirect() || casting) return durationMs;
            const r = video.seekable;
            return r && r.length ? state.offsetMs + r.end(r.length - 1) * 1000 : state.offsetMs;
        }

        let dragging = false;

        function updateUi() {
            const pos = position();
            if (!dragging) seek.value = Math.round(pos / 1000);
            timeNow.textContent = M.fmtClock(dragging ? seek.value * 1000 : pos);
            if (durationMs) {
                const a = isDirect() ? 0 : state.offsetMs / durationMs * 100;
                const b = Math.min(100, producedEndMs() / durationMs * 100);
                Object.assign(produced.style, {left: a + "%", width: Math.max(0, b - a) + "%"});
                seek.style.setProperty("--pct", (pos / durationMs * 100) + "%");
            }
            renderSubtitle(pos);
            updateSkip(pos);
        }

        let skipKind = "";
        let upNextShown = false;

        function updateSkip(pos) {
            let kind = "";
            if (markers.intro && pos >= markers.intro.startMs && pos < markers.intro.endMs - 1500) kind = "intro";
            else if (markers.credits && pos >= markers.credits.startMs && pos < markers.credits.endMs - 1500) kind = nextItem ? "next" : "credits";
            if (kind === "next" && !upNextShown && !video.paused) {
                upNextShown = true;
                showUpNext(nextItem);
            }
            if (kind !== skipKind) {
                skipKind = kind;
                skipBtn.hidden = !kind;
                skipBtn.replaceChildren(icon("skip"), {intro: "Intro überspringen", credits: "Abspann überspringen", next: "Nächste Episode"}[kind] || "");
            }
        }

        skipBtn.addEventListener("click", () => {
            if (skipKind === "intro") seekTo(markers.intro.endMs, true);
            else if (skipKind === "credits") seekTo(markers.credits.endMs - 500, true);
            else if (skipKind === "next") goNext();
        });

        function goNext() {
            if (!nextItem) return;
            if (roomCode) sendRoom("item", 0, nextItem.id);
            M.navigate(nextHref(nextItem), true);
        }

        function renderSubtitle(pos) {
            if (!state.cues.length) {
                if (subs.childNodes.length) subs.replaceChildren();
                return;
            }
            const active = state.cues.filter((c) => c.start <= pos && c.end > pos);
            const text = active.map((c) => c.text).join("\n");
            if (subs.dataset.text !== text) {
                subs.dataset.text = text;
                subs.replaceChildren(...(text ? text.split("\n").map((l) => h("span", null, l)) : []));
            }
        }

        let raf = 0;
        const loop = () => {
            updateUi();
            raf = requestAnimationFrame(loop);
        };

        // ---- starting a stream
        async function start(atMs) {
            if (state.starting) return;
            state.starting = true;
            spinner.hidden = false;
            notice.hidden = true;
            const oldSession = state.plan && state.plan.session;
            if (state.hls) {
                state.hls.destroy();
                state.hls = null;
            }
            let plan;
            try {
                plan = await api("POST", `/api/items/${item.id}/playback`, {
                    caps: capabilities(state.directPlay), quality: state.quality, audio: state.audio,
                    subtitle: state.subtitle, startMs: Math.max(0, Math.round(atMs)),
                    client: clientId, device: M.deviceName ? M.deviceName(navigator.userAgent) : "",
                });
            } catch (e) {
                state.starting = false;
                return fail(e.message);
            }
            if (oldSession && oldSession !== plan.session) stopSession(oldSession);
            state.plan = plan;
            state.audio = plan.audio;
            state.offsetMs = plan.mode === "direct" ? 0 : plan.offsetMs;
            state.pendingSeek = plan.mode === "direct" ? plan.startMs : 0;
            if (plan.mode === "direct") {
                video.src = plan.url;
            } else if (window.Hls && window.Hls.isSupported()) {
                const hls = new window.Hls({
                    startPosition: 0,
                    maxBufferLength: 30,
                    backBufferLength: 60,
                    liveDurationInfinity: false,
                    fragLoadingTimeOut: 70000,
                    manifestLoadingTimeOut: 35000,
                    xhrSetup: (xhr) => {
                        if (S.token) xhr.setRequestHeader("X-FreeX-Token", S.token);
                    },
                });
                state.hls = hls;
                let recovered = false;
                hls.on(window.Hls.Events.ERROR, (ev, data) => {
                    if (!data.fatal) return;
                    if (data.type === window.Hls.ErrorTypes.MEDIA_ERROR && !recovered) {
                        recovered = true;
                        hls.recoverMediaError();
                        return;
                    }
                    const status = data.response && data.response.code;
                    fail(status === 503 ? "Der Server ist ausgelastet." : "Die Umwandlung ist fehlgeschlagen. " +
                        (S.user.admin ? "Details stehen im Server-Log." : ""));
                });
                hls.loadSource(plan.url);
                hls.attachMedia(video);
            } else {
                video.src = plan.url; // Safari plays HLS natively
            }
            loadSubtitle();
            video.play().catch(() => null);
            state.starting = false;
        }

        async function loadSubtitle() {
            state.cues = [];
            const id = state.subtitle;
            if (!id || (state.plan && state.plan.burnedSubtitle === id)) return;
            try {
                const res = await fetch(`/api/items/${item.id}/subtitles/${id}.vtt`, {credentials: "same-origin",
                    headers: S.token ? {"X-FreeX-Token": S.token} : {}});
                if (!res.ok) throw new Error();
                if (state.subtitle === id) state.cues = parseVtt(await res.text());
            } catch (e) {
                M.toast("Untertitel konnten nicht geladen werden", true);
            }
        }

        function stopSession(id) {
            fetch(`/api/transcode/${id}`, {method: "DELETE", keepalive: true, credentials: "same-origin",
                headers: S.token ? {"X-FreeX-Token": S.token} : {}}).catch(() => null);
        }

        function seekTo(ms, byUser) {
            const target = Math.max(0, Math.min(durationMs ? durationMs - 1000 : Infinity, ms));
            if (!state.plan) return;
            if (byUser && roomCode) sendRoom("seek", target);
            if (casting) return castSeek(target);
            if (isDirect()) {
                video.currentTime = target / 1000;
                return;
            }
            const rel = (target - state.offsetMs) / 1000;
            const r = video.seekable;
            const end = r && r.length ? r.end(r.length - 1) : 0;
            if (rel >= 0 && rel <= end - 2) video.currentTime = rel;
            else start(target);
        }

        function fail(message) {
            spinner.hidden = true;
            const fmt = [media.container, media.videoCodec, media.audioCodec].filter(Boolean).map((s) => s.toUpperCase()).join(", ");
            notice.hidden = false;
            notice.replaceChildren(h("div", null,
                h("h2", null, "Wiedergabe nicht möglich"),
                h("p", {class: "muted"}, message),
                h("p", {class: "muted small"}, `Datei: ${fmt || "unbekanntes Format"}`),
                h("div", {class: "actions center"},
                    h("button", {class: "btn primary", onclick: () => start(position())}, "Erneut versuchen"),
                    h("a", {class: "btn", href: `/api/items/${item.id}/stream?download=1`}, icon("download"), "Herunterladen"),
                    h("button", {class: "btn", onclick: back}, "Zurück"))));
        }

        // ---- video events
        video.addEventListener("loadedmetadata", () => {
            if (state.pendingSeek > 0) {
                video.currentTime = state.pendingSeek / 1000;
                state.pendingSeek = 0;
            }
        });
        video.addEventListener("waiting", () => spinner.hidden = false);
        video.addEventListener("seeking", () => spinner.hidden = false);
        video.addEventListener("playing", () => spinner.hidden = true);
        video.addEventListener("canplay", () => spinner.hidden = video.paused ? true : spinner.hidden);
        video.addEventListener("seeked", () => {
            spinner.hidden = !video.paused ? spinner.hidden : true;
            report(false);
        });
        video.addEventListener("play", () => {
            playBtn.replaceChildren(icon("pause"));
            report(true);
        });
        video.addEventListener("pause", () => {
            playBtn.replaceChildren(icon("play"));
            spinner.hidden = true;
            report(true);
            wake();
        });
        video.addEventListener("error", () => {
            if (!isDirect()) return; // HLS errors are handled by hls.js
            if (state.directPlay) {
                // The browser could not decode the file after all: let the server convert it.
                state.directPlay = false;
                M.toast("Datei wird für deinen Browser umgewandelt");
                start(position());
            } else {
                fail("Dein Browser kann diese Datei nicht abspielen.");
            }
        });
        video.addEventListener("ended", () => {
            report(true);
            if (nextItem && !upNextShown) showUpNext(nextItem);
        });
        video.addEventListener("volumechange", () => {
            M.store.set("freex.volume", String(video.volume));
            M.store.set("freex.muted", video.muted ? "1" : "0");
            muteBtn.replaceChildren(icon(video.muted || video.volume === 0 ? "mute" : "volume"));
            vol.value = video.muted ? 0 : video.volume;
        });

        const togglePlay = () => {
            if (casting) return casting.controller.playOrPause();
            const play = video.paused;
            if (play) video.play().catch(() => null); else video.pause();
            if (roomCode) sendRoom(play ? "play" : "pause", position());
        };
        playBtn.addEventListener("click", togglePlay);
        video.addEventListener("click", togglePlay);
        video.addEventListener("dblclick", () => toggleFullscreen());
        muteBtn.addEventListener("click", () => {
            if (casting) return casting.controller.muteOrUnmute();
            video.muted = !video.muted;
        });
        vol.addEventListener("input", () => {
            if (casting) {
                casting.remote.volumeLevel = Number(vol.value);
                return casting.controller.setVolumeLevel();
            }
            video.volume = Number(vol.value);
            video.muted = video.volume === 0;
        });
        seek.addEventListener("input", () => {
            dragging = true;
            timeNow.textContent = M.fmtClock(seek.value * 1000);
        });
        seek.addEventListener("change", () => {
            dragging = false;
            seekTo(seek.value * 1000, true);
        });

        function toggleFullscreen() {
            if (document.fullscreenElement) document.exitFullscreen();
            else player.requestFullscreen?.().catch(() => null);
        }

        fsBtn.addEventListener("click", toggleFullscreen);
        const onFs = () => fsBtn.replaceChildren(icon(document.fullscreenElement ? "exitFullscreen" : "fullscreen"));
        document.addEventListener("fullscreenchange", onFs);

        // ---- settings menu
        settingsBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            if (player.querySelector(".player-menu")) return player.querySelector(".player-menu").remove();
            const plan = state.plan || {};
            const section = (title, options, current, onPick) => h("div", {class: "pm-section"},
                h("div", {class: "pm-title"}, title),
                options.map(([value, label]) => h("button", {class: "pm-option" + (String(value) === String(current) ? " on" : ""),
                    onclick: () => {
                        menu.remove();
                        onPick(value);
                    }}, h("span", {class: "pm-check", html: String(value) === String(current) ? M.ICONS.check : ""}), label)));
            const qualities = (plan.qualities || [{id: "original", label: "Original"}]).map((q) => [q.id, q.label]);
            const audio = media.audio.map((a) => [a.index, M.audioLabel(a)]);
            const subsList = [["", "Keine"]].concat(media.subtitles.map((s) => [s.id, M.subtitleLabel(s)]));
            const modeText = {direct: "Direkte Wiedergabe", remux: "Direct Stream", audio: "Direct Stream", transcode: "Umwandlung",
                cast: casting && casting.plan ? `Chromecast · ${{direct: "Direkte Wiedergabe", remux: "Direct Stream", audio: "Direct Stream", transcode: "Umwandlung"}[casting.plan.mode] || ""}` : "Chromecast"}[plan.mode] || "";
            const menu = h("div", {class: "player-menu popup-like", role: "dialog", "aria-label": "Wiedergabe-Einstellungen"},
                h("div", {class: "pm-info"}, h("strong", null, modeText), plan.reason && plan.mode !== "direct" ? h("span", null, plan.reason) : null),
                section("Qualität", qualities, state.quality, (v) => {
                    state.quality = v;
                    M.store.set("freex.quality", v);
                    if (v === "original") state.directPlay = true;
                    restart();
                }),
                audio.length > 1 ? section("Audio", audio, state.audio, (v) => {
                    state.audio = Number(v);
                    restart();
                }) : null,
                section("Untertitel", subsList, state.subtitle || "", (v) => {
                    const old = state.subtitle;
                    state.subtitle = v || null;
                    const s = media.subtitles.find((x) => x.id === v);
                    const needsRestart = (s && !s.textBased) || (state.plan && state.plan.burnedSubtitle && state.plan.burnedSubtitle === old);
                    if (needsRestart || casting) restart(); else loadSubtitle();
                }));
            player.append(menu);
        });
        const closePlayerMenu = (e) => {
            const m = player.querySelector(".player-menu");
            if (m && !m.contains(e.target) && !settingsBtn.contains(e.target)) m.remove();
        };
        document.addEventListener("click", closePlayerMenu);

        // ---- auto-hiding controls
        let idleTimer = null;

        function wake() {
            player.classList.remove("idle");
            clearTimeout(idleTimer);
            idleTimer = setTimeout(() => {
                if (!video.paused && !player.querySelector(".player-menu") && !controls.matches(":hover")) player.classList.add("idle");
            }, 3000);
        }

        player.addEventListener("mousemove", wake);
        player.addEventListener("touchstart", wake, {passive: true});
        wake();

        // ---- keyboard
        const onKey = (e) => {
            if (e.target.closest && e.target.closest("input, select, textarea")) {
                if (e.key !== "Escape") return;
            }
            const k = e.key;
            if (k === " " || k === "k") {
                e.preventDefault();
                togglePlay();
            } else if (k === "ArrowLeft" || k === "j") {
                seekTo(position() - 10000, true);
            } else if (k === "ArrowRight" || k === "l") {
                seekTo(position() + 30000, true);
            } else if (k === "f") {
                toggleFullscreen();
            } else if (k === "m") {
                video.muted = !video.muted;
            } else if (k === "Escape" && !document.fullscreenElement) {
                const m = player.querySelector(".player-menu");
                if (m) m.remove(); else back();
            } else {
                return;
            }
            wake();
        };
        document.addEventListener("keydown", onKey);

        // ---- Chromecast
        function restart() {
            if (casting) castStart(position());
            else start(position());
        }

        function updateCastButton() {
            castBtn.hidden = !(M.cast && (M.cast.devicesAvailable || M.cast.session)) || !!roomCode;
            castBtn.classList.toggle("on", !!casting);
            castBtn.title = casting ? `Verbunden mit ${M.cast.deviceName}, klicken zum Trennen` : "Auf Chromecast abspielen";
        }

        castBtn.addEventListener("click", () => {
            if (M.cast.session) M.cast.endSession();
            else M.cast.requestSession().catch(() => null); // closed the picker
        });

        const offCast = M.cast ? M.cast.onChange(() => {
            updateCastButton();
            if (M.cast.session && !casting && state.plan) castStart(position());
            else if (!M.cast.session && casting) castStop();
        }) : () => null;

        async function castStart(atMs) {
            if (castStart.busy) return;
            castStart.busy = true;
            try {
                let plan;
                try {
                    plan = await api("POST", `/api/items/${item.id}/cast`, {
                        quality: state.quality, audio: state.audio, subtitle: state.subtitle, startMs: Math.max(0, Math.round(atMs)),
                        client: clientId, device: `Chromecast ${M.cast.deviceName}`.trim(),
                    });
                } catch (e) {
                    M.toast(e.message, true);
                    return;
                }
                // The local player stops; its conversion slot is freed right away.
                if (state.hls) {
                    state.hls.destroy();
                    state.hls = null;
                }
                if (state.plan && state.plan.session) stopSession(state.plan.session);
                video.pause();
                video.removeAttribute("src");
                video.load();
                state.plan = {mode: "cast", reason: plan.reason, burnedSubtitle: plan.burnedSubtitle, qualities: state.plan && state.plan.qualities};
                state.cues = [];
                subs.replaceChildren();
                if (!casting) {
                    const remote = new cast.framework.RemotePlayer();
                    const controller = new cast.framework.RemotePlayerController(remote);
                    controller.addEventListener(cast.framework.RemotePlayerEventType.ANY_CHANGE, onRemoteChange);
                    casting = {remote, controller, plan: null};
                }
                casting.plan = plan;
                casting.loading = true;
                spinner.hidden = false;
                castInfo.hidden = false;
                castInfo.replaceChildren(icon("cast"), h("span", null, `Wird auf ${M.cast.deviceName || "dem Chromecast"} abgespielt`));
                updateCastButton();
                try {
                    await M.cast.load(item, plan, state.subtitle);
                } catch (e) {
                    M.toast("Chromecast: " + (e && (e.description || e.code) || "Laden fehlgeschlagen"), true);
                } finally {
                    casting.loading = false;
                    spinner.hidden = true;
                }
            } finally {
                castStart.busy = false;
            }
        }

        function castSeek(target) {
            const {remote, controller, plan} = casting;
            const rel = (target - plan.offsetMs) / 1000;
            // Within what the Chromecast has (direct stream, or the converted part): seek there;
            // further ahead the server converts from the new position.
            if (!plan.hls || (rel >= 0 && rel <= (remote.duration || 0) - 2)) {
                remote.currentTime = Math.max(0, rel);
                controller.seek();
            } else {
                castStart(target);
            }
        }

        function onRemoteChange(e) {
            if (!casting) return;
            const r = casting.remote;
            // Kept, because after a disconnect the remote player no longer knows the position.
            if (e.field === "currentTime" && !casting.loading) casting.lastPos = casting.plan.offsetMs + r.currentTime * 1000;
            if (e.field === "isPaused" || e.field === "playerState") {
                playBtn.replaceChildren(icon(r.isPaused ? "play" : "pause"));
                report(true);
            }
            if (e.field === "volumeLevel" || e.field === "isMuted") {
                vol.value = r.isMuted ? 0 : r.volumeLevel;
                muteBtn.replaceChildren(icon(r.isMuted || r.volumeLevel === 0 ? "mute" : "volume"));
            }
            // Played to the end on the Chromecast: count it and go on with the next title.
            if (e.field === "playerState" && r.playerState === "IDLE" && !casting.loading && durationMs
                && position() >= durationMs * 0.9) {
                report(true);
                if (nextItem) goNext();
            }
        }

        function castStop() {
            const at = casting.lastPos != null ? casting.lastPos : position();
            casting.controller.removeEventListener(cast.framework.RemotePlayerEventType.ANY_CHANGE, onRemoteChange);
            casting = null;
            castInfo.hidden = true;
            updateCastButton();
            M.toast("Chromecast getrennt, Wiedergabe läuft hier weiter");
            start(at);
        }

        // ---- progress
        let lastSent = -1;
        let lastSentAt = 0;

        /**
         * Saves the position and keeps this playback in the activity list. Sent on changes and
         * at least every 20 s, also while paused; {@code stopped} ends the playback there.
         */
        function report(final, stopped) {
            if (!state.plan) return;
            const offsetMs = Math.round(position());
            if (!final && Math.abs(offsetMs - lastSent) < 5000 && Date.now() - lastSentAt < 20000) return;
            lastSent = offsetMs;
            lastSentAt = Date.now();
            fetch(`/api/items/${item.id}/progress`, {
                method: "PUT", keepalive: true, credentials: "same-origin",
                headers: {"Content-Type": "application/json", ...(S.token ? {"X-FreeX-Token": S.token} : {})},
                body: JSON.stringify({offsetMs, durationMs: durationMs || Math.round((video.duration || 0) * 1000),
                    client: clientId, state: (casting ? casting.remote.isPaused : video.paused) ? "paused" : "playing", stopped: !!stopped}),
            }).catch(() => null);
        }

        const interval = setInterval(() => report(false), 10000);
        const onHide = () => {
            report(true, true);
            // Tab closed or reloaded: free the transcoding slot right away instead of after the timeout.
            if (state.plan && state.plan.session) stopSession(state.plan.session);
        };
        window.addEventListener("pagehide", onHide);

        // ---- up next
        let nextTimer = null;

        function showUpNext(next) {
            let left = 10;
            const counter = h("span", null, left);
            const go = () => goNext();
            const box = h("div", {class: "up-next"},
                h("div", {class: "art"}, M.art(next.id, "thumb", next.title)),
                h("div", null,
                    h("div", {class: "muted small"}, "Als Nächstes in ", counter, " s"),
                    h("div", {class: "title"}, next.type === "episode" ? `${M.episodeCode(next)}, ${next.title}` : next.title),
                    h("div", {class: "row-btns"},
                        h("button", {class: "btn primary small", onclick: go}, icon("play"), "Jetzt abspielen"),
                        h("button", {class: "btn small", onclick: () => {
                                clearInterval(nextTimer);
                                box.remove();
                            }}, "Abbrechen"))));
            player.querySelector(".up-next")?.remove();
            player.append(box);
            nextTimer = setInterval(() => {
                left -= 1;
                counter.textContent = left;
                if (left <= 0) {
                    clearInterval(nextTimer);
                    go();
                }
            }, 1000);
        }

        // ---- Watch Together
        let events = null;
        let syncTimer = null;
        let lastRoom = null;

        // Only called from the player's own controls, never while applying a change from the room,
        // so every call is a real user action and must reach the others.
        let roomVersion = -1;   // newest room state we know
        let pendingSends = 0;   // own changes not yet confirmed by the server

        function sendRoom(action, positionMs, itemId) {
            if (!roomCode) return;
            pendingSends++;
            // Our own change is the room's state from now on; the echo from the server confirms it.
            if (lastRoom) {
                const st = {...lastRoom.st, positionMs: Math.round(positionMs)};
                if (action === "play") st.playing = true;
                if (action === "pause") st.playing = false;
                lastRoom = {st, at: Date.now()};
            }
            fetch(`/api/together/${roomCode}/state`, {method: "POST", credentials: "same-origin",
                headers: {"Content-Type": "application/json", ...(S.token ? {"X-FreeX-Token": S.token} : {})},
                body: JSON.stringify({action, positionMs: Math.round(positionMs), itemId})})
                .then((r) => r.ok ? r.json() : null)
                .then((d) => {
                    if (d && d.version > roomVersion) roomVersion = d.version;
                })
                .catch(() => null)
                .finally(() => pendingSends--);
        }

        /** Older than what we know, or overtaken by our own change still on its way: ignore. */
        function stale(st) {
            return pendingSends > 0 || (typeof st.version === "number" && st.version < roomVersion);
        }

        /** Where the room is right now, from the last state and the time since. */
        function roomPosition() {
            if (!lastRoom) return null;
            return lastRoom.st.positionMs + (lastRoom.st.playing ? Date.now() - lastRoom.at : 0);
        }

        function applyRoom(st) {
            if (stale(st)) return;
            if (typeof st.version === "number") roomVersion = Math.max(roomVersion, st.version);
            if (st.itemId && st.itemId !== item.id) {
                M.navigate(nextHref({id: st.itemId}), true);
                return;
            }
            lastRoom = {st, at: Date.now()};
            remoteApplying = true;
            const target = st.positionMs;
            if (!state.plan) {
                start(target).then(() => {
                    if (!st.playing) video.pause();
                });
            } else if (Math.abs(position() - target) > 2500) {
                seekTo(target);
            }
            if (st.playing && video.paused) video.play().catch(() => null);
            if (!st.playing && !video.paused) video.pause();
            setTimeout(() => remoteApplying = false, 600);
        }

        function renderMembers(st) {
            members.replaceChildren(...st.members.map((m) => {
                const a = M.avatar(m, "small" + (m.online ? "" : " offline"));
                a.title = m.name + (m.online ? "" : " (nicht verbunden)");
                return a;
            }));
        }

        if (roomCode) {
            video.autoplay = false;
            events = new EventSource(`/api/together/${roomCode}/events`);
            events.addEventListener("state", (e) => {
                const st = JSON.parse(e.data);
                renderMembers(st);
                applyRoom(st);
            });
            events.addEventListener("members", (e) => renderMembers(JSON.parse(e.data)));
            // A change that arrived while the video was still loading is applied once it can play.
            video.addEventListener("canplay", () => {
                const target = roomPosition();
                if (target == null || !state.plan) return;
                if (Math.abs(position() - target) > 2500) {
                    remoteApplying = true;
                    seekTo(target);
                    setTimeout(() => remoteApplying = false, 600);
                }
                if (lastRoom.st.playing && video.paused) video.play().catch(() => null);
            });
            events.onerror = () => {
                if (events.readyState === EventSource.CLOSED) M.toast("Verbindung zum Raum verloren", true);
            };
            // Correct drift from buffering every now and then.
            syncTimer = setInterval(async () => {
                try {
                    const st = await api("GET", `/api/together/${roomCode}`);
                    if (stale(st)) return;
                    lastRoom = {st, at: Date.now()};
                    if (st.playing !== !video.paused) {
                        remoteApplying = true;
                        if (st.playing) video.play().catch(() => null); else video.pause();
                        setTimeout(() => remoteApplying = false, 600);
                    }
                    if (st.playing && !video.paused && Math.abs(position() - st.positionMs) > 3000) {
                        remoteApplying = true;
                        seekTo(st.positionMs);
                        setTimeout(() => remoteApplying = false, 600);
                    }
                } catch (e) {
                    // room gone; the event stream reports it
                }
            }, 8000);
        }

        S.cleanup = () => {
            offCast();
            // Leaving the player stops the video on the Chromecast; the connection stays for the next title.
            if (casting) {
                casting.controller.removeEventListener(cast.framework.RemotePlayerEventType.ANY_CHANGE, onRemoteChange);
                if (casting.remote.isMediaLoaded) casting.controller.stop();
            }
            if (events) events.close();
            clearInterval(syncTimer);
            report(true, true);
            cancelAnimationFrame(raf);
            clearInterval(interval);
            clearInterval(nextTimer);
            clearTimeout(idleTimer);
            document.removeEventListener("keydown", onKey);
            document.removeEventListener("click", closePlayerMenu);
            document.removeEventListener("fullscreenchange", onFs);
            window.removeEventListener("pagehide", onHide);
            if (document.fullscreenElement) document.exitFullscreen().catch(() => null);
            if (state.hls) state.hls.destroy();
            if (state.plan && state.plan.session) stopSession(state.plan.session);
            video.pause();
            video.removeAttribute("src");
            video.load();
            player.remove();
            document.title = S.status.name;
        };

        loop();
        updateCastButton();
        // In a room the first state event decides where to start; with a running cast session the
        // title goes straight to the Chromecast (e.g. the next episode).
        if (!roomCode) {
            if (M.cast && M.cast.session) castStart(Number(query.get("t") || 0));
            else start(Number(query.get("t") || 0));
        }
    }, {shell: false});

    M.capabilities = capabilities;
})();

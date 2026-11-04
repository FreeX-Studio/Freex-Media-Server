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
 * FreeX web interface, part 1: state, helpers, API access, routing, the app shell and the
 * poster cards. All markup is built with h(), so data from the server is never parsed as HTML.
 * The parts share the global object `FreeX`.
 */
window.FreeX = (() => {
    "use strict";

    const S = {
        token: null,
        user: null,
        status: null,
        libraries: [],
        pollTimer: null,
        cleanup: null, // called when leaving the current view
    };

    const store = {
        get(k) {
            try {
                return localStorage.getItem(k);
            } catch (e) {
                return null;
            }
        },
        set(k, v) {
            try {
                if (v == null) localStorage.removeItem(k); else localStorage.setItem(k, v);
            } catch (e) {
                // storage unavailable (private mode): the cookie still keeps the login
            }
        },
    };

    // ------------------------------------------------------------------ DOM helpers

    function h(tag, attrs, ...children) {
        const el = document.createElement(tag);
        if (attrs) {
            for (const [k, v] of Object.entries(attrs)) {
                if (v == null || v === false) continue;
                if (k === "class") el.className = v;
                else if (k === "html") el.innerHTML = v; // only for the static icons below
                else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
                else if (k === "style") Object.assign(el.style, v); // CSSOM, allowed by the CSP
                else if (k === "value") el.value = v;
                else el.setAttribute(k, v === true ? "" : v);
            }
        }
        for (const c of children.flat(Infinity)) {
            if (c == null || c === false) continue;
            el.append(c instanceof Node ? c : document.createTextNode(String(c)));
        }
        return el;
    }

    const svg = (body, extra) => `<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"${extra || ""}>${body}</svg>`;
    const ICONS = {
        home: svg('<path d="M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6h-6v6H4a1 1 0 0 1-1-1z"/>'),
        movies: svg('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 4v16M17 4v16M3 9h4M3 15h4M17 9h4M17 15h4"/>'),
        shows: svg('<rect x="2" y="6" width="20" height="13" rx="2"/><path d="m8 2 4 4 4-4"/>'),
        settings: svg('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>'),
        search: svg('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>'),
        play: '<svg viewBox="0 0 24 24"><path d="M7 4.5v15a1 1 0 0 0 1.5.86l12.5-7.5a1 1 0 0 0 0-1.72L8.5 3.64A1 1 0 0 0 7 4.5z"/></svg>',
        pause: '<svg viewBox="0 0 24 24"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>',
        back: svg('<path d="M15 18l-6-6 6-6"/>'),
        next: svg('<path d="M9 18l6-6-6-6"/>'),
        check: svg('<path d="M5 12.5l4.5 4.5L19 7.5"/>', ' stroke-width="2.4"'),
        more: '<svg viewBox="0 0 24 24"><circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/></svg>',
        menu: svg('<path d="M4 6h16M4 12h16M4 18h16"/>'),
        restart: svg('<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>'),
        download: svg('<path d="M12 3v12M7 10l5 5 5-5M5 21h14"/>'),
        replay10: svg('<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/><text x="12" y="15.5" font-size="7" text-anchor="middle" fill="currentColor" stroke="none" font-weight="700">10</text>'),
        forward30: svg('<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/><text x="12" y="15.5" font-size="7" text-anchor="middle" fill="currentColor" stroke="none" font-weight="700">30</text>'),
        volume: svg('<path d="M11 5 6 9H3v6h3l5 4z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13"/>'),
        mute: svg('<path d="M11 5 6 9H3v6h3l5 4z"/><path d="m16 9 5 6M21 9l-5 6"/>'),
        fullscreen: svg('<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>'),
        exitFullscreen: svg('<path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5"/>'),
        gear: svg('<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1"/>'),
        subtitles: svg('<rect x="2" y="5" width="20" height="14" rx="2"/><path d="M7 15h4M14 15h3M7 11h2M12 11h5"/>'),
        shield: svg('<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/><path d="m9 12 2 2 4-4"/>'),
        close: svg('<path d="M6 6l12 12M18 6 6 18"/>'),
        bookmark: svg('<path d="M6 3h12v18l-6-4-6 4z"/>'),
        wrench: svg('<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-6.9 6.9a2.1 2.1 0 0 1-3-3l6.9-6.9a6 6 0 0 1 7.9-7.9z"/>'),
        cast: svg('<path d="M2 16a5 5 0 0 1 5 5M2 12a9 9 0 0 1 9 9M2 8V6a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-6"/><path d="M2 20h.01"/>'),
        activity: svg('<path d="M3 12h4l3-8 4 16 3-8h4"/>'),
        history: svg('<path d="M3 12a9 9 0 1 0 2.6-6.4L3 8"/><path d="M3 3v5h5M12 7v5l3 2"/>'),
        bookmarked: '<svg viewBox="0 0 24 24"><path d="M6 3h12v18l-6-4-6 4z"/></svg>',
        playlist: svg('<path d="M4 6h12M4 12h12M4 18h7"/><path d="M17 15v6l4-3z"/>'),
        collection: svg('<rect x="3" y="7" width="14" height="14" rx="2"/><path d="M7 3h14v14"/>'),
        trailer: svg('<rect x="2" y="5" width="20" height="14" rx="2"/><path d="m10 9 5 3-5 3z"/>'),
        together: svg('<circle cx="9" cy="8" r="3.5"/><circle cx="17" cy="9" r="2.5"/><path d="M2.5 20c.8-3.6 3.4-5.5 6.5-5.5s5.7 1.9 6.5 5.5M15.5 14.6c2.6-.2 4.9 1.4 5.6 4.4"/>'),
        skip: svg('<path d="M5 5v14l9-7z"/><path d="M17 5v14"/>'),
        user: svg('<circle cx="12" cy="8" r="4"/><path d="M4 21c1-4.4 4.2-6.5 8-6.5s7 2.1 8 6.5"/>'),
    };
    const icon = (name, cls) => h("span", {class: "ico" + (cls ? " " + cls : ""), html: ICONS[name], "aria-hidden": "true"});

    // ------------------------------------------------------------------ formatting

    function fmtDuration(ms) {
        if (!ms) return "";
        const min = Math.round(ms / 60000);
        if (min < 60) return `${min} Min.`;
        const hrs = Math.floor(min / 60);
        const rest = min % 60;
        return rest ? `${hrs} Std. ${rest} Min.` : `${hrs} Std.`;
    }

    function fmtClock(ms) {
        const total = Math.max(0, Math.floor(ms / 1000));
        const hh = Math.floor(total / 3600);
        const mm = Math.floor((total % 3600) / 60);
        const ss = total % 60;
        const p = (n) => String(n).padStart(2, "0");
        return hh ? `${hh}:${p(mm)}:${p(ss)}` : `${mm}:${p(ss)}`;
    }

    function fmtDate(ms) {
        if (!ms) return "nie";
        return new Date(ms).toLocaleString("de-DE", {dateStyle: "medium", timeStyle: "short"});
    }

    function fmtSize(bytes) {
        if (!bytes) return "";
        const gb = bytes / 1024 ** 3;
        return gb >= 1 ? `${gb.toFixed(1).replace(".", ",")} GB` : `${Math.round(bytes / 1024 ** 2)} MB`;
    }

    function fmtBitrate(bps) {
        if (!bps) return "";
        return bps >= 1e6 ? `${(bps / 1e6).toFixed(1).replace(".", ",")} Mbit/s` : `${Math.round(bps / 1e3)} kbit/s`;
    }

    const LANGS = {de: "Deutsch", ger: "Deutsch", deu: "Deutsch", en: "Englisch", eng: "Englisch", fr: "Französisch",
        fre: "Französisch", fra: "Französisch", es: "Spanisch", spa: "Spanisch", it: "Italienisch", ita: "Italienisch",
        nl: "Niederländisch", nld: "Niederländisch", dut: "Niederländisch", ja: "Japanisch", jpn: "Japanisch",
        tr: "Türkisch", tur: "Türkisch", pl: "Polnisch", pol: "Polnisch", ru: "Russisch", rus: "Russisch",
        sv: "Schwedisch", swe: "Schwedisch", da: "Dänisch", dan: "Dänisch", no: "Norwegisch", nor: "Norwegisch",
        ko: "Koreanisch", kor: "Koreanisch", zh: "Chinesisch", chi: "Chinesisch", zho: "Chinesisch", pt: "Portugiesisch",
        por: "Portugiesisch"};
    const language = (code) => LANGS[code] || (code ? code.toUpperCase() : "");

    const CODECS = {ac3: "Dolby Digital", eac3: "Dolby Digital+", dts: "DTS", truehd: "Dolby TrueHD", aac: "AAC",
        mp3: "MP3", flac: "FLAC", opus: "Opus", vorbis: "Vorbis", pcm_s16le: "PCM", pcm_s24le: "PCM",
        subrip: "SRT", srt: "SRT", ass: "ASS", ssa: "SSA", webvtt: "VTT", vtt: "VTT", mov_text: "Text",
        hdmv_pgs_subtitle: "PGS", dvd_subtitle: "VobSub", dvb_subtitle: "DVB"};
    const codecName = (c) => CODECS[c] || (c || "").toUpperCase();

    function channels(n) {
        return {1: "Mono", 2: "Stereo", 6: "5.1", 8: "7.1"}[n] || (n ? `${n} Kanäle` : "");
    }

    // ------------------------------------------------------------------ language preferences

    /** Languages offered in the preferences (ISO 639-1), as the server accepts them. */
    const PREF_LANGS = [["de", "Deutsch"], ["en", "Englisch"], ["fr", "Französisch"], ["es", "Spanisch"],
        ["it", "Italienisch"], ["nl", "Niederländisch"], ["pt", "Portugiesisch"], ["pl", "Polnisch"], ["ru", "Russisch"],
        ["tr", "Türkisch"], ["sv", "Schwedisch"], ["da", "Dänisch"], ["no", "Norwegisch"], ["fi", "Finnisch"],
        ["cs", "Tschechisch"], ["hu", "Ungarisch"], ["el", "Griechisch"], ["ja", "Japanisch"], ["ko", "Koreanisch"],
        ["zh", "Chinesisch"], ["ar", "Arabisch"], ["hi", "Hindi"]];

    /** ISO 639-2 (B and T) codes as found in media files, mapped to ISO 639-1. */
    const ISO3 = {ger: "de", deu: "de", eng: "en", fre: "fr", fra: "fr", spa: "es", ita: "it", dut: "nl", nld: "nl",
        por: "pt", pol: "pl", rus: "ru", tur: "tr", swe: "sv", dan: "da", nor: "no", nob: "no", nno: "no", fin: "fi",
        cze: "cs", ces: "cs", hun: "hu", gre: "el", ell: "el", jpn: "ja", kor: "ko", chi: "zh", zho: "zh", ara: "ar", hin: "hi"};

    const normLang = (code) => {
        const c = String(code || "").toLowerCase().split(/[-_]/)[0];
        return ISO3[c] || c;
    };

    /**
     * The tracks to start with when the user picked none: audio in the preferred language
     * (otherwise the file's default), subtitles as set in the preferences. Returns
     * {audio: index or -1, sub: subtitle id or null}.
     */
    function defaultTracks(media, prefs) {
        prefs = prefs || {};
        const audioPref = prefs.audioLanguage || "";
        const subLang = prefs.subtitleLanguage || audioPref || "de";
        const mode = prefs.subtitleMode || "forced";
        const audios = media.audio || [];
        let audio = audios.find((a) => a.default) || audios[0];
        if (audioPref) {
            const matching = audios.filter((a) => normLang(a.language) === audioPref);
            if (matching.length) audio = matching.find((a) => a.default) || matching[0];
        }
        const subs = (media.subtitles || []).filter((s) => normLang(s.language) === subLang);
        const full = subs.find((s) => !s.forced && s.textBased) || subs.find((s) => !s.forced);
        const forced = subs.find((s) => s.forced);
        const audioLang = audio ? normLang(audio.language) : "";
        let sub = null;
        if (mode === "always") sub = full || forced;
        else if (mode === "foreign" && audioLang && audioLang !== subLang) sub = full || forced;
        else if (mode !== "off") sub = forced; // "forced", or "foreign" with audio in the own language
        return {audio: audio ? audio.index : -1, sub: sub ? sub.id : null};
    }

    /** Round profile picture, or the initial when there is none. */
    function avatar(user, cls) {
        const el = h("span", {class: "avatar" + (cls ? " " + cls : "")});
        const initial = (user.username || user.name || "?").slice(0, 1).toUpperCase();
        if (user.avatar) {
            const im = h("img", {src: `/api/users/${encodeURIComponent(user.id)}/avatar?v=${user.avatar}`, alt: "", loading: "lazy"});
            im.addEventListener("error", () => el.replaceChildren(initial));
            el.append(im);
        } else {
            el.append(initial);
        }
        return el;
    }

    function audioLabel(a) {
        return [language(a.language) || "Unbekannt", a.title && !/^(stereo|surround|5\.1)$/i.test(a.title) ? `(${a.title})` : "",
            `${codecName(a.codec)} ${channels(a.channels)}`.trim()].filter(Boolean).join(" ");
    }

    function subtitleLabel(s) {
        const parts = [language(s.language) || "Unbekannt"];
        if (s.forced) parts.push("erzwungen");
        if (s.title && !s.forced) parts.push(s.title);
        return `${parts.join(", ")} (${codecName(s.codec)}${s.external ? ", Datei" : ""})`;
    }

    function episodeCode(it) {
        if (it.type !== "episode") return "";
        const s = it.seasonIndex === 0 ? "Special" : `S${it.seasonIndex ?? "?"}`;
        return `${s} E${it.index}`;
    }

    /** Short device name from a user agent, for the session list. */
    function deviceName(ua) {
        if (!ua) return "Unbekanntes Gerät";
        const browser = /Edg\//.test(ua) ? "Edge" : /OPR\//.test(ua) ? "Opera" : /Firefox\//.test(ua) ? "Firefox"
            : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : /curl|wget|python|java/i.test(ua) ? "Skript" : "Browser";
        const os = /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android"
            : /Windows/.test(ua) ? "Windows" : /Mac OS X|Macintosh/.test(ua) ? "macOS" : /CrOS/.test(ua) ? "ChromeOS"
                : /Linux/.test(ua) ? "Linux" : "";
        return os ? `${browser} auf ${os}` : browser;
    }

    function toast(msg, isError) {
        document.querySelectorAll(".toast").forEach((t) => t.remove());
        const t = h("div", {class: "toast" + (isError ? " error" : ""), role: "status"}, msg);
        document.body.append(t);
        setTimeout(() => t.remove(), 3800);
    }

    const img = (id, kind) => `/api/items/${encodeURIComponent(id)}/image/${kind}`;

    /** Image that turns into a titled placeholder when the server has no artwork. */
    function art(id, kind, title) {
        const i = h("img", {src: img(id, kind), alt: "", loading: "lazy", decoding: "async"});
        i.addEventListener("error", () => i.replaceWith(h("div", {class: "placeholder"}, h("span", null, title || ""))));
        return i;
    }

    // ------------------------------------------------------------------ API

    class ApiError extends Error {
        constructor(status, message) {
            super(message);
            this.status = status;
        }
    }

    async function api(method, path, body) {
        const headers = {};
        if (S.token) headers["X-FreeX-Token"] = S.token;
        if (body !== undefined) headers["Content-Type"] = "application/json";
        let res;
        try {
            res = await fetch(path, {method, headers, credentials: "same-origin",
                body: body === undefined ? undefined : JSON.stringify(body)});
        } catch (e) {
            throw new ApiError(0, "Der Server ist nicht erreichbar");
        }
        if (res.status === 204) return null;
        let data = null;
        try {
            data = await res.json();
        } catch (e) {
            data = null;
        }
        if (!res.ok) {
            if (res.status === 401 && S.user && !path.startsWith("/api/login") && !path.startsWith("/api/me/")) {
                logoutLocal();
                navigate("/login?next=" + encodeURIComponent(location.pathname + location.search), true);
            }
            throw new ApiError(res.status, (data && data.error) || `Fehler ${res.status}`);
        }
        return data;
    }

    function logoutLocal() {
        S.token = null;
        S.user = null;
        store.set("freex.token", null);
        document.querySelector(".shell")?.remove();
    }

    async function logout() {
        try {
            await api("POST", "/api/logout");
        } catch (e) {
            // already logged out
        }
        logoutLocal();
        location.href = "/login";
    }

    // ------------------------------------------------------------------ routing

    const routes = [];

    /** Registers a view: pattern with :params, handler(main, params, query). shell=false for full-screen views. */
    function route(pattern, handler, opts) {
        const names = [];
        const re = new RegExp("^" + pattern.replace(/:(\w+)/g, (_, n) => {
            names.push(n);
            return "([\\w-]+)";
        }) + "$");
        routes.push({re, names, handler, shell: !(opts && opts.shell === false), auth: !(opts && opts.auth === false)});
    }

    function navigate(path, replace) {
        if (replace) history.replaceState(null, "", path); else history.pushState(null, "", path);
        render();
    }

    document.addEventListener("click", (e) => {
        const a = e.target.closest("a[data-link]");
        if (!a || e.ctrlKey || e.metaKey || e.shiftKey || e.button !== 0) return;
        e.preventDefault();
        navigate(a.getAttribute("href"));
    });
    window.addEventListener("popstate", () => render());

    const link = (href, attrs, ...children) => h("a", {href, "data-link": true, ...attrs}, ...children);

    async function render() {
        if (S.cleanup) {
            const c = S.cleanup;
            S.cleanup = null;
            try {
                c();
            } catch (e) {
                // ignore
            }
        }
        closeMenus();
        document.querySelectorAll(".modal-back").forEach((m) => m.remove());
        const path = location.pathname;
        const query = new URLSearchParams(location.search);

        if (!S.status) {
            try {
                S.status = await api("GET", "/api/status");
            } catch (e) {
                document.getElementById("app").replaceChildren(h("div", {class: "auth"},
                    h("div", {class: "auth-box"}, h("h1", null, "Server nicht erreichbar"), h("p", null, e.message),
                        h("button", {class: "btn", onclick: () => location.reload()}, "Erneut versuchen"))));
                return;
            }
            document.title = S.status.name;
        }
        if (S.status.setupRequired && path !== "/setup") return navigate("/setup", true);

        let match = null;
        for (const r of routes) {
            const m = path.match(r.re);
            if (m) {
                const params = {};
                r.names.forEach((n, i) => params[n] = m[i + 1]);
                match = {r, params};
                break;
            }
        }
        if (match && !match.r.auth) return match.r.handler(null, match.params, query);

        if (!S.user) {
            try {
                S.user = await api("GET", "/api/me");
                S.status = await api("GET", "/api/status"); // admins see more details
            } catch (e) {
                logoutLocal();
                return navigate("/login?next=" + encodeURIComponent(path + location.search), true);
            }
        }
        if (match && !match.r.shell) {
            document.querySelector(".shell")?.remove();
            return match.r.handler(null, match.params, query);
        }

        const main = shell(path);
        main.replaceChildren(h("div", {class: "loading"}, h("span", {class: "spin"})));
        try {
            if (match) await match.r.handler(main, match.params, query);
            else main.replaceChildren(emptyState("Seite nicht gefunden", "Diese Adresse gibt es nicht.",
                link("/", {class: "btn"}, "Zur Startseite")));
        } catch (e) {
            if (e instanceof ApiError && e.status === 401) return;
            main.replaceChildren(emptyState("Das hat nicht geklappt", e.message, link("/", {class: "btn"}, "Zur Startseite")));
        }
        if (!(history.state && history.state.keepScroll)) main.scrollTop = 0;
    }

    function emptyState(title, text, ...actions) {
        return h("div", {class: "page"}, h("div", {class: "empty"}, h("h2", null, title), h("p", null, text), actions));
    }

    // ------------------------------------------------------------------ popup menus

    function closeMenus() {
        document.querySelectorAll(".popup").forEach((m) => m.remove());
    }

    document.addEventListener("click", (e) => {
        if (!e.target.closest(".popup") && !e.target.closest("[data-menu]")) closeMenus();
    });
    document.addEventListener("keydown", (e) => {
        if (e.key === "Escape") closeMenus();
    });

    /** Opens a menu below the anchor element. entries: [{label, onclick, danger}] or null for a divider. */
    function popup(anchor, entries) {
        closeMenus();
        const menu = h("div", {class: "popup", role: "menu"}, entries.filter((e) => e !== undefined).map((e) => e === null
            ? h("div", {class: "popup-sep"})
            : h("button", {role: "menuitem", class: e.danger ? "danger" : "", onclick: (ev) => {
                    ev.stopPropagation();
                    closeMenus();
                    e.onclick();
                }}, e.label)));
        document.body.append(menu);
        const r = anchor.getBoundingClientRect();
        const w = menu.offsetWidth;
        const left = Math.min(window.innerWidth - w - 8, Math.max(8, r.right - w));
        const top = r.bottom + 6 + menu.offsetHeight > window.innerHeight ? r.top - menu.offsetHeight - 6 : r.bottom + 6;
        Object.assign(menu.style, {left: left + "px", top: Math.max(8, top) + "px"});
        menu.querySelector("button")?.focus();
    }

    // ------------------------------------------------------------------ activity (admins)

    const MODE_LABELS = {direct: "Direkte Wiedergabe", remux: "Direct Stream", audio: "Direct Stream (Ton umgewandelt)", transcode: "Umwandlung"};

    /** Header button with the number of running playbacks; opens the activity panel. */
    function activityButton() {
        const badge = h("span", {class: "badge", hidden: true});
        const btn = h("button", {class: "icon-btn activity-btn", title: "Aktivität", "aria-label": "Aktivität", "data-menu": true,
            onclick: () => document.querySelector(".activity-panel") ? closeMenus() : openActivity(btn, badge)}, icon("activity"), badge);
        const update = async () => {
            if (!btn.isConnected) return clearInterval(timer);
            if (document.hidden) return;
            try {
                const st = await api("GET", "/api/status");
                setBadge(badge, st.playing || 0);
            } catch (e) {
                // offline or logged out; the next tick tries again
            }
        };
        const timer = setInterval(update, 15000);
        setTimeout(update, 500);
        return btn;
    }

    function setBadge(badge, n) {
        badge.hidden = !n;
        badge.textContent = n > 9 ? "9+" : String(n);
    }

    function openActivity(anchor, badge) {
        closeMenus();
        const body = h("div", {class: "activity-body"}, h("div", {class: "activity-empty"}, "Wird geladen …"));
        const panel = h("div", {class: "popup activity-panel", role: "dialog", "aria-label": "Aktivität"},
            h("div", {class: "activity-head"}, "Aktivität"), body);
        document.body.append(panel);
        const r = anchor.getBoundingClientRect();
        Object.assign(panel.style, {top: r.bottom + 8 + "px", left: Math.max(8, Math.min(window.innerWidth - panel.offsetWidth - 8, r.right - panel.offsetWidth)) + "px"});

        const render = (a) => {
            setBadge(badge, a.sessions.length);
            const rows = a.sessions.map((s) => {
                const it = s.item;
                const landscape = it.type === "episode";
                const pct = s.durationMs ? Math.min(100, s.offsetMs / s.durationMs * 100) : 0;
                const title = landscape ? `${it.showTitle || ""} · ${episodeCode(it)}` : it.title + (it.year ? ` (${it.year})` : "");
                return h("a", {class: "activity-session", href: `/item/${it.id}`, onclick: (e) => {
                        e.preventDefault();
                        closeMenus();
                        navigate(`/item/${it.id}`);
                    }},
                    h("div", {class: "activity-art" + (landscape ? " landscape" : "")}, art(it.id, landscape ? "thumb" : "poster", it.title)),
                    h("div", {class: "activity-info"},
                        h("div", {class: "activity-title"}, title),
                        landscape ? h("div", {class: "activity-sub"}, it.title) : null,
                        h("div", {class: "activity-user"}, s.user ? avatar(s.user, "small") : null,
                            h("span", null, [s.user ? s.user.username : "", s.device || deviceName(s.userAgent)].filter(Boolean).join(" · "))),
                        h("div", {class: "activity-mode" + (s.mode === "transcode" ? " transcode" : "")},
                            s.state === "paused" ? "Pausiert · " : "", MODE_LABELS[s.mode] || s.mode || "",
                            s.quality && s.quality !== "Original" ? ` · ${s.quality}` : ""),
                        h("div", {class: "activity-progress"}, h("span", {style: {width: pct + "%"}})),
                        h("div", {class: "activity-time"}, `${fmtClock(s.offsetMs)} / ${fmtClock(s.durationMs)}`)));
            });
            const tasks = a.tasks.map((t) => h("div", {class: "activity-task"}, h("span", {class: "spin"}), t.title));
            body.replaceChildren(...rows, ...tasks);
            if (!rows.length && !tasks.length) body.append(h("div", {class: "activity-empty"}, "Gerade läuft nichts."));
        };
        const load = async () => {
            if (!panel.isConnected) return clearInterval(timer);
            try {
                render(await api("GET", "/api/admin/activity"));
            } catch (e) {
                body.replaceChildren(h("div", {class: "activity-empty"}, e.message));
            }
        };
        const timer = setInterval(load, 3000);
        load();
    }

    // ------------------------------------------------------------------ shell

    function toggleNav() {
        const root = document.querySelector(".shell");
        if (!root) return;
        if (window.matchMedia("(max-width: 860px)").matches) {
            root.classList.toggle("nav-open");
            return;
        }
        const collapsed = root.classList.toggle("nav-collapsed");
        store.set("freex.navCollapsed", collapsed ? "1" : null);
    }

    function shell(path) {
        let root = document.querySelector(".shell");
        if (!root) {
            const search = h("input", {type: "search", placeholder: "Suchen", "aria-label": "Filme und Serien durchsuchen"});
            let timer = null;
            search.addEventListener("input", () => {
                clearTimeout(timer);
                timer = setTimeout(() => {
                    const q = search.value.trim();
                    if (q.length >= 2) navigate("/search?q=" + encodeURIComponent(q), location.pathname === "/search");
                }, 280);
            });
            search.addEventListener("keydown", (e) => {
                if (e.key === "Enter" && search.value.trim()) navigate("/search?q=" + encodeURIComponent(search.value.trim()));
            });

            const userBtn = h("button", {class: "user-button", "data-menu": true, "aria-haspopup": "menu", "aria-label": "Konto",
                onclick: (e) => popup(e.currentTarget, [
                    {label: S.user.username, onclick: () => navigate("/settings/account")},
                    null,
                    {label: "Konto und Sicherheit", onclick: () => navigate("/settings/security")},
                    S.user.admin ? {label: "Server-Einstellungen", onclick: () => navigate("/settings/libraries")} : undefined,
                    null,
                    {label: "Abmelden", onclick: logout},
                ])}, avatar(S.user));

            root = h("div", {class: "shell"},
                h("header", {class: "topbar"},
                    h("button", {class: "icon-btn menu-toggle", "aria-label": "Menü ein- oder ausklappen", title: "Menü", onclick: toggleNav}, icon("menu")),
                    link("/", {class: "brand", "aria-label": "Startseite"}, h("img", {src: "/icon.svg", alt: ""}), h("span", null, "Free", h("b", null, "X"))),
                    h("div", {class: "search"}, icon("search"), search),
                    h("div", {class: "spacer"}),
                    S.user.admin ? activityButton() : null,
                    link("/settings", {class: "icon-btn", title: "Einstellungen", "aria-label": "Einstellungen"}, icon("wrench")),
                    userBtn),
                h("nav", {class: "sidebar", "aria-label": "Navigation"}),
                h("main", {class: "main"}));
            // Collapsed sidebar (icons only) is remembered per browser; on phones the menu slides in instead.
            if (store.get("freex.navCollapsed") === "1") root.classList.add("nav-collapsed");
            root.querySelector(".sidebar").addEventListener("click", (e) => {
                if (e.target.closest("a")) root.classList.remove("nav-open");
            });
            document.getElementById("app").replaceChildren(root);
            refreshLibraries();
        }
        const search = root.querySelector(".search input");
        if (path !== "/search" && document.activeElement !== search) search.value = "";
        renderSidebar(path);
        return root.querySelector(".main");
    }

    function renderSidebar(path) {
        const nav = document.querySelector(".sidebar");
        if (!nav) return;
        const current = path || location.pathname;
        const item = (href, iconName, label, extra, active) => link(href,
            {class: "nav-link" + (active ? " active" : ""), title: label}, icon(iconName), h("span", {class: "label"}, label), extra);
        nav.replaceChildren(...[
            item("/", "home", "Startseite", null, current === "/"),
            item("/watchlist", "bookmark", "Merkliste", null, current === "/watchlist"),
            item("/playlists", "playlist", "Playlists", null, current.startsWith("/playlist")),
            item("/history", "history", "Verlauf", null, current === "/history"),
            S.libraries.length ? h("div", {class: "nav-group"}, S.status.name) : null,
            S.libraries.map((l) => item(`/library/${l.id}`, l.type === "shows" ? "shows" : "movies", l.name,
                l.scanning ? h("span", {class: "spin", title: "Wird gescannt"}) : null,
                current.startsWith(`/library/${l.id}`))),
        ].flat(Infinity).filter(Boolean));
    }

    /** Reloads the library list; while a scan is running it polls and refreshes the view when done. */
    async function refreshLibraries() {
        try {
            const before = S.libraries.some((l) => l.scanning);
            S.libraries = await api("GET", "/api/libraries");
            renderSidebar();
            const scanning = S.libraries.some((l) => l.scanning);
            clearTimeout(S.pollTimer);
            if (scanning) S.pollTimer = setTimeout(refreshLibraries, 2500);
            if (before && !scanning && !location.pathname.startsWith("/play/")) {
                toast("Scan abgeschlossen");
                render();
            }
        } catch (e) {
            // offline or logged out; the next navigation shows it
        }
    }

    // ------------------------------------------------------------------ cards

    function progressBar(item) {
        const p = item.progress;
        const dur = item.media && item.media.durationMs;
        if (!p || !p.offsetMs || !dur) return null;
        return h("div", {class: "progress"}, h("span", {style: {width: Math.min(100, p.offsetMs / dur * 100) + "%"}}));
    }

    function unwatchedBadge(item) {
        if (item.type === "show" || item.type === "season") {
            const left = (item.leafCount || 0) - (item.viewedLeafCount || 0);
            if (!left) return null;
            return left === item.leafCount ? h("span", {class: "badge-new", title: "Ungesehen"})
                : h("span", {class: "badge-count", title: `${left} ungesehen`}, left);
        }
        if (item.progress && !item.progress.watched && !item.progress.offsetMs) return h("span", {class: "badge-new", title: "Ungesehen"});
        return null;
    }

    function cardMeta(item) {
        switch (item.type) {
            case "movie":
                return item.year || "";
            case "show":
                return item.childCount === 1 ? "1 Staffel" : `${item.childCount || 0} Staffeln`;
            case "season":
                return `${item.leafCount || 0} Episoden`;
            case "episode":
                return `${episodeCode(item)}, ${item.title}`;
            default:
                return "";
        }
    }

    function isWatched(item) {
        if (item.type === "movie" || item.type === "episode") return item.progress && item.progress.watched;
        return item.leafCount > 0 && item.leafCount === item.viewedLeafCount;
    }

    async function setWatched(item, watched) {
        await api(watched ? "POST" : "DELETE", `/api/items/${item.id}/watched`);
        toast(watched ? "Als gesehen markiert" : "Als ungesehen markiert");
        history.replaceState({keepScroll: true}, "");
        render();
    }

    function playHref(item) {
        const off = item.progress && !item.progress.watched ? item.progress.offsetMs : 0;
        return `/play/${item.id}` + (off ? `?t=${off}` : "");
    }

    /** Other parts add entries to the "…" menu of items here (playlists, collections, …). */
    const menuExtensions = [];

    function itemMenu(anchor, item) {
        const watched = isWatched(item);
        const extra = menuExtensions.flatMap((f) => f(item) || []);
        popup(anchor, [
            item.type === "movie" || item.type === "episode" ? {label: "Abspielen", onclick: () => navigate(playHref(item))} : undefined,
            item.type === "movie" || item.type === "episode" ? {label: "Von vorne abspielen", onclick: () => navigate(`/play/${item.id}`)} : undefined,
            {label: watched ? "Als ungesehen markieren" : "Als gesehen markieren", onclick: () => setWatched(item, !watched)},
            null,
            {label: "Details anzeigen", onclick: () => navigate(`/item/${item.id}`)},
            item.showId ? {label: "Zur Serie", onclick: () => navigate(`/item/${item.showId}`)} : undefined,
            item.seasonId ? {label: "Zur Staffel", onclick: () => navigate(`/item/${item.seasonId}`)} : undefined,
            extra.length ? null : undefined,
            ...extra,
        ]);
    }

    /** Card of a collection or playlist (no watch state, own artwork route). */
    function listCard(item) {
        const isColl = item.type === "collection";
        const href = isColl ? `/collection/${item.id}` : `/playlist/${item.id}`;
        const src = isColl ? `/api/collections/${encodeURIComponent(item.id)}/image/poster`
            : item.firstItemId ? img(item.firstItemId, "poster") : null;
        const pic = src ? h("img", {src, alt: "", loading: "lazy"}) : h("div", {class: "placeholder"}, h("span", null, item.title));
        if (src) pic.addEventListener("error", () => pic.replaceWith(h("div", {class: "placeholder"}, h("span", null, item.title))));
        const count = isColl ? `${item.childCount} Titel` : `${item.childCount} Einträge${item.durationMs ? ", " + fmtDuration(item.durationMs) : ""}`;
        return link(href, {class: "card " + (isColl ? "collection" : "playlist")},
            h("div", {class: "art"}, pic, h("span", {class: "list-badge"}, icon(isColl ? "collection" : "playlist"))),
            h("div", {class: "title", title: item.title}, item.title),
            h("div", {class: "meta"}, count));
    }

    /** Poster (portrait) or thumbnail (landscape) card with hover actions. */
    function card(item, style) {
        if (item.type === "collection" || item.type === "playlist") return listCard(item);
        const landscape = style === "landscape";
        const kind = landscape ? "thumb" : "poster";
        const title = item.type === "episode" && landscape ? item.showTitle || item.title : item.title;
        const playable = item.type === "movie" || item.type === "episode";
        const actions = h("div", {class: "card-actions"},
            playable ? h("button", {class: "card-play", title: "Abspielen", "aria-label": `${title} abspielen`, html: ICONS.play,
                onclick: (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    navigate(playHref(item));
                }}) : null,
            h("button", {class: "card-btn card-check", title: isWatched(item) ? "Als ungesehen markieren" : "Als gesehen markieren",
                "aria-label": "Gesehen umschalten", html: ICONS.check, onclick: (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setWatched(item, !isWatched(item));
                }}),
            h("button", {class: "card-btn card-more", title: "Mehr", "aria-label": "Mehr", "data-menu": true, html: ICONS.more,
                onclick: (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    itemMenu(e.currentTarget, item);
                }}));
        return link(`/item/${item.id}`, {class: "card" + (landscape ? " landscape" : "")},
            h("div", {class: "art"}, art(item.id, kind, title), unwatchedBadge(item), progressBar(item), actions),
            h("div", {class: "title", title}, title),
            h("div", {class: "meta"}, cardMeta(item)));
    }

    /** A horizontally scrolling row with arrow buttons (a hub). */
    function hubRow(hub, moreHref) {
        const row = h("div", {class: "row" + (hub.style === "landscape" ? " landscape" : "")}, hub.items.map((it) => card(it, hub.style)));
        if (hub.href) moreHref = hub.href;
        const scrollBy = (dir) => row.scrollBy({left: dir * row.clientWidth * 0.85, behavior: "smooth"});
        const prev = h("button", {class: "row-arrow", "aria-label": "Zurück", onclick: () => scrollBy(-1)}, icon("back"));
        const next = h("button", {class: "row-arrow", "aria-label": "Weiter", onclick: () => scrollBy(1)}, icon("next"));
        const update = () => {
            prev.disabled = row.scrollLeft <= 4;
            next.disabled = row.scrollLeft + row.clientWidth >= row.scrollWidth - 4;
        };
        row.addEventListener("scroll", update, {passive: true});
        requestAnimationFrame(update);
        return h("section", {class: "hub"},
            h("div", {class: "hub-head"},
                moreHref ? link(moreHref, {class: "hub-title"}, h("h2", null, hub.title), icon("next")) : h("h2", null, hub.title),
                h("div", {class: "hub-arrows"}, prev, next)),
            row);
    }

    return {S, store, h, ICONS, icon, fmtDuration, fmtClock, fmtDate, fmtSize, fmtBitrate, language, codecName, channels,
        audioLabel, subtitleLabel, episodeCode, deviceName, toast, img, art, api, ApiError, logoutLocal, logout, route,
        navigate, link, render, emptyState, popup, closeMenus, refreshLibraries, renderSidebar, card, hubRow, isWatched,
        setWatched, playHref, itemMenu, progressBar, unwatchedBadge, menuExtensions, PREF_LANGS, defaultTracks, avatar,
        SLOGAN: "Your Media. Your Server. Your Freedom."};
})();

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
 * FreeX Web, part 1: state, helpers, API access, routing, the app shell and the
 * poster cards. All markup is built with h(), so data from the server is never parsed as HTML.
 * The parts share the global object `FreeX`.
 */
// The DOM methods that take child nodes print null, undefined and false as text ("null") and
// arrays as "a,b". Views pass optional parts as `cond ? part : null`, as they do to h(), so these
// methods get the same rules as h(): such values are left out and arrays are flattened.
(() => {
    const clean = (nodes) => nodes.flat(Infinity).filter((n) => n != null && n !== false);
    const wrap = (proto, names) => {
        for (const name of names) {
            const native = proto[name];
            if (typeof native === "function") proto[name] = function (...nodes) { return native.apply(this, clean(nodes)); };
        }
    };
    wrap(Element.prototype, ["append", "prepend", "replaceChildren", "before", "after", "replaceWith"]);
    wrap(DocumentFragment.prototype, ["append", "prepend", "replaceChildren"]);
    wrap(CharacterData.prototype, ["before", "after", "replaceWith"]);
})();

window.FreeX = (() => {
    "use strict";
    const {t, tn, tParts, num, locale} = window.FreeXI18n;

    const S = {
        token: null,
        user: null,
        status: null,
        libraries: [],
        pollTimer: null,
        cleanup: null, // called when leaving the current view
        navMore: false, // sidebar shows all entries with pin buttons instead of the pinned ones
        outdated: false, // the server has a newer FreeX Web; loaded with the next page change
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
        chapters: svg('<path d="M9 6h11M9 12h11M9 18h11"/><path d="M4 5v2M4 11v2M4 17v2"/>'),
        filter: svg('<path d="M3 5h18l-7 8v6l-4 2v-8z"/>'),
        playlist: svg('<path d="M4 6h12M4 12h12M4 18h7"/><path d="M17 15v6l4-3z"/>'),
        collection: svg('<rect x="3" y="7" width="14" height="14" rx="2"/><path d="M7 3h14v14"/>'),
        trailer: svg('<rect x="2" y="5" width="20" height="14" rx="2"/><path d="m10 9 5 3-5 3z"/>'),
        together: svg('<circle cx="9" cy="8" r="3.5"/><circle cx="17" cy="9" r="2.5"/><path d="M2.5 20c.8-3.6 3.4-5.5 6.5-5.5s5.7 1.9 6.5 5.5M15.5 14.6c2.6-.2 4.9 1.4 5.6 4.4"/>'),
        skip: svg('<path d="M5 5v14l9-7z"/><path d="M17 5v14"/>'),
        user: svg('<circle cx="12" cy="8" r="4"/><path d="M4 21c1-4.4 4.2-6.5 8-6.5s7 2.1 8 6.5"/>'),
        pin: svg('<path d="M9 4h6l-1 6 4 4H6l4-4z"/><path d="M12 14v7"/>'),
        pinned: '<svg viewBox="0 0 24 24" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 4h6l-1 6 4 4H6l4-4z" fill="currentColor" stroke="currentColor"/><path d="M12 14v7" fill="none" stroke="currentColor"/></svg>',
        moreVertical: '<svg viewBox="0 0 24 24"><circle cx="12" cy="5" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="12" cy="19" r="2"/></svg>',
    };
    const icon = (name, cls) => h("span", {class: "ico" + (cls ? " " + cls : ""), html: ICONS[name], "aria-hidden": "true"});

    // ------------------------------------------------------------------ formatting

    function fmtDuration(ms) {
        if (!ms) return "";
        const min = Math.round(ms / 60000);
        if (min < 60) return t("{0} min", min);
        const hrs = Math.floor(min / 60);
        const rest = min % 60;
        return rest ? t("{0} hr {1} min", hrs, rest) : t("{0} hr", hrs);
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
        if (!ms) return t("never");
        return new Date(ms).toLocaleString(locale(), {dateStyle: "medium", timeStyle: "short"});
    }

    function fmtSize(bytes) {
        if (!bytes) return "";
        const gb = bytes / 1024 ** 3;
        return gb >= 1 ? `${num(gb, 1)} GB` : `${num(bytes / 1024 ** 2, 0)} MB`;
    }

    function fmtBitrate(bps) {
        if (!bps) return "";
        return bps >= 1e6 ? `${num(bps / 1e6, 1)} Mbit/s` : `${num(bps / 1e3, 0)} kbit/s`;
    }


    const CODECS = {ac3: "Dolby Digital", eac3: "Dolby Digital+", dts: "DTS", truehd: "Dolby TrueHD", aac: "AAC",
        mp3: "MP3", flac: "FLAC", opus: "Opus", vorbis: "Vorbis", pcm_s16le: "PCM", pcm_s24le: "PCM",
        subrip: "SRT", srt: "SRT", ass: "ASS", ssa: "SSA", webvtt: "VTT", vtt: "VTT", mov_text: "Text",
        hdmv_pgs_subtitle: "PGS", dvd_subtitle: "VobSub", dvb_subtitle: "DVB"};
    const codecName = (c) => CODECS[c] || (c || "").toUpperCase();

    function channels(n) {
        return {1: "Mono", 2: "Stereo", 6: "5.1", 8: "7.1"}[n] || (n ? t("{0} channels", n) : "");
    }

    // ------------------------------------------------------------------ language preferences

    /** Audio and subtitle languages offered in the preferences (ISO 639-1), as the server accepts them. */
    const PREF_LANGS = ["de", "en", "fr", "es", "it", "nl", "pt", "pl", "ru", "tr", "sv", "da", "no", "fi", "cs", "hu",
        "el", "ja", "ko", "zh", "ar", "hi"];

    /** ISO 639-2 (B and T) codes as found in media files, mapped to ISO 639-1. */
    const ISO3 = {ger: "de", deu: "de", eng: "en", fre: "fr", fra: "fr", spa: "es", ita: "it", dut: "nl", nld: "nl",
        por: "pt", pol: "pl", rus: "ru", tur: "tr", swe: "sv", dan: "da", nor: "no", nob: "no", nno: "no", fin: "fi",
        cze: "cs", ces: "cs", hun: "hu", gre: "el", ell: "el", jpn: "ja", kor: "ko", chi: "zh", zho: "zh", ara: "ar", hin: "hi"};

    const normLang = (code) => {
        const c = String(code || "").toLowerCase().split(/[-_]/)[0];
        return ISO3[c] || c;
    };

    /** Name of a language (ISO 639-1 or -2 code) in the display language. */
    function language(code) {
        if (!code) return "";
        const c = normLang(code);
        try {
            const name = new Intl.DisplayNames([locale()], {type: "language"}).of(c);
            if (name && name.toLowerCase() !== c) return name.charAt(0).toUpperCase() + name.slice(1);
        } catch (e) {
            // not a valid language code
        }
        return String(code).toUpperCase();
    }

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
        return [language(a.language) || t("Unknown"), a.title && !/^(stereo|surround|5\.1)$/i.test(a.title) ? `(${a.title})` : "",
            `${codecName(a.codec)} ${channels(a.channels)}`.trim()].filter(Boolean).join(" ");
    }

    function subtitleLabel(s) {
        const parts = [language(s.language) || t("Unknown")];
        if (s.forced) parts.push(t("forced"));
        if (s.title && !s.forced) parts.push(s.title);
        return `${parts.join(", ")} (${codecName(s.codec)}${s.external ? ", " + t("file") : ""})`;
    }

    function episodeCode(it) {
        if (it.type !== "episode") return "";
        const s = it.seasonIndex === 0 ? "Special" : `S${it.seasonIndex ?? "?"}`;
        return `${s} E${it.index}`;
    }

    /** Short device name from a user agent, for the session list. */
    function deviceName(ua) {
        if (!ua) return t("Unknown device");
        // Jellyfin apps: the server records "Jellyfin app; <app> <version> (<device>)".
        const app = /^Jellyfin app; (.+)$/.exec(ua);
        if (app) return app[1];
        const browser = /Edg\//.test(ua) ? "Edge" : /OPR\//.test(ua) ? "Opera" : /Firefox\//.test(ua) ? "Firefox"
            : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : /curl|wget|python|java/i.test(ua) ? t("Script") : "Browser";
        const os = /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android"
            : /Windows/.test(ua) ? "Windows" : /Mac OS X|Macintosh/.test(ua) ? "macOS" : /CrOS/.test(ua) ? "ChromeOS"
                : /Linux/.test(ua) ? "Linux" : "";
        return os ? t("{0} on {1}", browser, os) : browser;
    }

    function toast(msg, isError) {
        document.querySelectorAll(".toast").forEach((el) => el.remove());
        const el = h("div", {class: "toast" + (isError ? " error" : ""), role: "status"}, msg);
        document.body.append(el);
        setTimeout(() => el.remove(), 3800);
    }

    /**
     * URL of an item's artwork. Given the item itself rather than its id, the URL carries the
     * item's artwork version, so a changed picture is loaded instead of the cached one.
     */
    const img = (item, kind) => {
        const id = typeof item === "string" ? item : item.id;
        const v = typeof item === "object" && item.artVersion ? `?v=${encodeURIComponent(item.artVersion)}` : "";
        return `/api/items/${encodeURIComponent(id)}/image/${kind}${v}`;
    };

    /** Image that turns into a titled placeholder when the server has no artwork. */
    function art(item, kind, title) {
        const i = h("img", {src: img(item, kind), alt: "", loading: "lazy", decoding: "async"});
        i.addEventListener("error", () => i.replaceWith(h("div", {class: "placeholder"}, h("span", null, title || ""))));
        return i;
    }

    // ------------------------------------------------------------------ API

    /** An error answer of the server: HTTP status, translated message and the message key as code. */
    class ApiError extends Error {
        constructor(status, message, code) {
            super(message);
            this.status = status;
            this.code = code || "";
        }
    }

    async function api(method, path, body) {
        // Signed-in users get messages in their display language; before that, in the one shown here.
        const headers = {"Accept-Language": FreeXI18n.lang()};
        if (S.token) headers["X-FreeX-Token"] = S.token;
        if (body !== undefined) headers["Content-Type"] = "application/json";
        let res;
        try {
            res = await fetch(path, {method, headers, credentials: "same-origin",
                body: body === undefined ? undefined : JSON.stringify(body)});
        } catch (e) {
            throw new ApiError(0, t("The server cannot be reached"));
        }
        noteBuild(res);
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
            throw new ApiError(res.status, (data && data.error) || t("Error {0}", res.status), data && data.code);
        }
        return data;
    }

    // The build of this page; the server sends its current one with every answer.
    const BUILD = document.querySelector('meta[name="freex-build"]')?.content || "";

    function noteBuild(res) {
        const b = res.headers.get("X-FreeX-Build");
        if (BUILD && b && b !== BUILD) S.outdated = true;
    }

    /** Switches to the user's display language; the shell is rebuilt if it was drawn in another one. */
    function applyLanguage() {
        const before = FreeXI18n.lang();
        if (FreeXI18n.setLanguage((S.user.preferences || {}).uiLanguage || "") !== before) {
            document.querySelector(".shell")?.remove();
        }
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
        // The server was updated while this page was open: load the new version with the next
        // page change, but never in the middle of a playback.
        if (S.outdated && !location.pathname.startsWith("/play/")) {
            location.reload();
            return;
        }
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
                    h("div", {class: "auth-box"}, h("h1", null, t("Server not reachable")), h("p", null, e.message),
                        h("div", {class: "btn-row center"},
                            h("button", {class: "btn", onclick: () => location.reload()}, t("Try again")),
                            window.FreeX.desktop ? h("button", {class: "btn", onclick: () => window.FreeX.desktop.switchServer()}, t("Switch server")) : null))));
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
        applyLanguage();
        if (match && !match.r.shell) {
            document.querySelector(".shell")?.remove();
            return match.r.handler(null, match.params, query);
        }

        const main = shell(path);
        main.replaceChildren(h("div", {class: "loading"}, h("span", {class: "spin"})));
        try {
            if (match) await match.r.handler(main, match.params, query);
            else main.replaceChildren(emptyState(t("Page not found"), t("This address does not exist."),
                link("/", {class: "btn"}, t("Go to the home screen"))));
        } catch (e) {
            if (e instanceof ApiError && e.status === 401) return;
            main.replaceChildren(emptyState(t("That did not work"), e.message, link("/", {class: "btn"}, t("Go to the home screen"))));
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

    const MODE_LABELS = {direct: "Direct play", remux: "Direct stream", audio: "Direct stream (audio converted)", transcode: "Transcoding"};

    /** Header button with the number of running playbacks; opens the activity panel. */
    function activityButton() {
        const badge = h("span", {class: "badge", hidden: true});
        const btn = h("button", {class: "icon-btn activity-btn", title: t("Activity"), "aria-label": t("Activity"), "data-menu": true,
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
        const body = h("div", {class: "activity-body"}, h("div", {class: "activity-empty"}, t("Loading …")));
        const panel = h("div", {class: "popup activity-panel", role: "dialog", "aria-label": t("Activity")},
            h("div", {class: "activity-head"}, t("Activity")), body);
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
                    h("div", {class: "activity-art" + (landscape ? " landscape" : "")}, art(it, landscape ? "thumb" : "poster", it.title)),
                    h("div", {class: "activity-info"},
                        h("div", {class: "activity-title"}, title),
                        landscape ? h("div", {class: "activity-sub"}, it.title) : null,
                        h("div", {class: "activity-user"}, s.user ? avatar(s.user, "small") : null,
                            h("span", null, [s.user ? s.user.username : "", s.device || deviceName(s.userAgent)].filter(Boolean).join(" · "))),
                        h("div", {class: "activity-mode" + (s.mode === "transcode" ? " transcode" : "")},
                            s.state === "paused" ? t("Paused") + " · " : s.state === "starting" ? t("Starting") + " · " : "", MODE_LABELS[s.mode] ? t(MODE_LABELS[s.mode]) : s.mode || "",
                            s.quality && s.quality !== "Original" ? ` · ${s.quality}` : "",
                            s.bitrate ? ` · ${fmtBitrate(s.bitrate)}` : "", s.cpu != null ? ` · CPU ${num(s.cpu, 0)} %` : ""),
                        h("div", {class: "activity-progress"}, h("span", {style: {width: pct + "%"}})),
                        h("div", {class: "activity-time"}, `${fmtClock(s.offsetMs)} / ${fmtClock(s.durationMs)}`)));
            });
            const tasks = a.tasks.map((task) => h("div", {class: "activity-task"}, h("span", {class: "spin"}), task.title));
            body.replaceChildren(...rows, ...tasks, link("/settings/dashboard", {class: "activity-more", onclick: () => closeMenus()}, t("Server dashboard")));
            if (!rows.length && !tasks.length) body.append(h("div", {class: "activity-empty"}, t("Nothing is playing right now.")));
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

    // ------------------------------------------------------------------ search field

    const PREVIEW_LIMIT = 5;

    /** Text with the first occurrence of the search term marked. */
    function markMatch(text, q) {
        const lower = text.toLowerCase();
        const i = lower.indexOf(q.toLowerCase());
        // Lower-casing can change the length of some characters; then the positions do not fit.
        if (i < 0 || lower.length !== text.length) return text;
        return [text.slice(0, i), h("mark", null, text.slice(i, i + q.length)), text.slice(i + q.length)];
    }

    /**
     * The search field of the top bar. While typing, a preview below it shows the first hits of
     * each kind; arrow keys pick one, Enter opens it, or the search page when none is picked. On
     * the search page itself the page follows the input instead.
     */
    function searchBox() {
        const input = h("input", {type: "search", placeholder: t("Search"), "aria-label": t("Search movies and shows"),
            role: "combobox", autocomplete: "off", "aria-autocomplete": "list", "aria-controls": "search-preview", "aria-expanded": "false"});
        let timer = null;
        let seq = 0; // only the answer to the latest input may fill the preview
        let pop = null;
        let hits = []; // [{el, item}] in display order
        let active = -1;

        const isOpen = () => !!pop && pop.isConnected; // closeMenus() and Escape remove it from outside
        const close = () => {
            if (isOpen()) pop.remove();
            pop = null;
            hits = [];
            active = -1;
            input.setAttribute("aria-expanded", "false");
            input.removeAttribute("aria-activedescendant");
        };
        const openPage = (q) => {
            close();
            navigate("/search?q=" + encodeURIComponent(q));
        };
        const openItem = (item) => {
            close();
            input.value = "";
            input.blur();
            navigate(`/item/${item.id}`);
        };
        const setActive = (i) => {
            active = i;
            hits.forEach((x, j) => x.el.classList.toggle("active", j === i));
            if (i < 0) return input.removeAttribute("aria-activedescendant");
            input.setAttribute("aria-activedescendant", hits[i].el.id);
            hits[i].el.scrollIntoView({block: "nearest"});
        };
        const place = () => {
            const r = input.getBoundingClientRect();
            const width = Math.min(window.innerWidth - 16, Math.max(r.width, 420));
            const left = Math.max(8, Math.min(r.left, window.innerWidth - width - 8));
            const top = r.bottom + 8;
            Object.assign(pop.style, {left: left + "px", top: top + "px", width: width + "px", maxHeight: window.innerHeight - top - 16 + "px"});
        };

        const hitRow = (item, q) => {
            const episode = item.type === "episode";
            const el = h("a", {class: "search-hit", href: `/item/${item.id}`, id: "search-hit-" + hits.length, role: "option", tabindex: "-1",
                onclick: (e) => {
                    if (e.ctrlKey || e.metaKey || e.shiftKey || e.button !== 0) return; // new tab or window
                    e.preventDefault();
                    openItem(item);
                }},
                h("div", {class: "search-art" + (episode ? " landscape" : "")}, art(item, episode ? "thumb" : "poster", item.title)),
                h("div", {class: "search-info"},
                    h("div", {class: "search-title"}, markMatch(item.title, q)),
                    h("div", {class: "search-meta"}, episode ? `${item.showTitle || ""} · ${episodeCode(item)}` : cardMeta(item))));
            hits.push({el, item});
            return el;
        };

        const show = (res, q) => {
            hits = [];
            active = -1;
            const groups = [[t("Movies"), res.movies], [t("Shows"), res.shows], [t("Episodes"), res.episodes]]
                .filter(([, items]) => items.length)
                .map(([title, items]) => h("div", {class: "search-group", role: "group", "aria-label": title},
                    h("div", {class: "search-group-head"}, title), items.map((it) => hitRow(it, q))));
            if (!isOpen()) {
                pop = h("div", {class: "popup search-preview", id: "search-preview", role: "listbox", "aria-label": t("Search results")});
                document.body.append(pop);
            }
            pop.replaceChildren(
                h("div", {class: "search-scroll"}, groups.length ? groups : h("div", {class: "search-empty"}, t("Nothing found"))),
                groups.length ? h("button", {class: "search-all", onclick: () => openPage(q)}, t("Show all results for “{0}”", q)) : null);
            input.setAttribute("aria-expanded", "true");
            input.removeAttribute("aria-activedescendant");
            place();
        };

        const update = (q) => {
            const mine = ++seq;
            api("GET", `/api/search?limit=${PREVIEW_LIMIT}&q=${encodeURIComponent(q)}`).then((res) => {
                // Stale, or the user has left the field in the meantime.
                if (mine === seq && document.activeElement === input && location.pathname !== "/search") show(res, q);
            }).catch(() => {
                // offline or signed out: no preview; Enter still opens the search page
            });
        };

        input.addEventListener("input", () => {
            clearTimeout(timer);
            const q = input.value.trim();
            if (q.length < 2) {
                seq++;
                close();
                return;
            }
            if (location.pathname === "/search") {
                timer = setTimeout(() => navigate("/search?q=" + encodeURIComponent(q), true), 280);
            } else {
                timer = setTimeout(() => update(q), 150);
            }
        });
        input.addEventListener("focus", () => {
            const q = input.value.trim();
            if (q.length >= 2 && !isOpen() && location.pathname !== "/search") update(q);
        });
        input.addEventListener("keydown", (e) => {
            const q = input.value.trim();
            if ((e.key === "ArrowDown" || e.key === "ArrowUp") && isOpen() && hits.length) {
                e.preventDefault();
                // The selection cycles through the hits and the input (-1): up from the input picks the last hit.
                const n = hits.length + 1;
                setActive((active + 1 + (e.key === "ArrowDown" ? 1 : -1) + n) % n - 1);
            } else if (e.key === "Enter") {
                clearTimeout(timer);
                if (isOpen() && active >= 0) openItem(hits[active].item);
                else if (q) openPage(q);
            } else if (e.key === "Tab") {
                close();
            }
        });
        window.addEventListener("resize", () => isOpen() && place());
        return input;
    }

    function shell(path) {
        let root = document.querySelector(".shell");
        if (!root) {
            const search = searchBox();

            const userBtn = h("button", {class: "user-button", "data-menu": true, "aria-haspopup": "menu", "aria-label": t("Account"),
                onclick: (e) => popup(e.currentTarget, [
                    {label: S.user.username, onclick: () => navigate("/settings/account")},
                    null,
                    {label: t("Account and security"), onclick: () => navigate("/settings/security")},
                    S.user.admin ? {label: t("Server settings"), onclick: () => navigate("/settings/libraries")} : undefined,
                    {label: t("Switch user"), onclick: () => window.FreeX.switchUser()},
                    window.FreeX.desktop ? {label: t("Switch server"), onclick: () => window.FreeX.desktop.switchServer()} : undefined,
                    null,
                    {label: t("Sign out"), onclick: logout},
                ])}, avatar(S.user));

            root = h("div", {class: "shell"},
                h("header", {class: "topbar"},
                    h("button", {class: "icon-btn menu-toggle", "aria-label": t("Expand or collapse the menu"), title: t("Menu"), onclick: toggleNav}, icon("menu")),
                    link("/", {class: "brand", "aria-label": t("Home")}, h("img", {src: "/icon.svg", alt: ""}), h("span", null, "Free", h("b", null, "X"))),
                    // data-menu: a click into the field does not close the search preview
                    h("div", {class: "search", "data-menu": true}, icon("search"), search),
                    h("div", {class: "spacer"}),
                    S.user.admin ? activityButton() : null,
                    link("/settings", {class: "icon-btn", title: t("Settings"), "aria-label": t("Settings")}, icon("wrench")),
                    userBtn),
                h("nav", {class: "sidebar", "aria-label": "Navigation"}),
                h("main", {class: "main"}));
            // Collapsed sidebar (icons only) is remembered per browser; on phones the menu slides in instead.
            if (store.get("freex.navCollapsed") === "1") root.classList.add("nav-collapsed");
            // Open overlay menu (small screens): a tap next to it only closes it.
            root.querySelector(".main").addEventListener("click", (e) => {
                if (!root.classList.contains("nav-open")) return;
                e.preventDefault();
                e.stopPropagation();
                root.classList.remove("nav-open");
            }, true);
            root.querySelector(".sidebar").addEventListener("click", (e) => {
                if (e.target.closest("a")) root.classList.remove("nav-open");
            });
            document.getElementById("app").replaceChildren(root);
            refreshLibraries();
        }
        const search = root.querySelector(".search input");
        if (document.activeElement !== search) {
            if (path !== "/search") search.value = "";
            document.getElementById("search-preview")?.remove();
        }
        renderSidebar(path);
        return root.querySelector(".main");
    }

    /**
     * All sidebar entries. The id is what the user's preferences store when the entry is unpinned;
     * the home screen cannot be unpinned.
     */
    function navEntries(current) {
        const entries = [
            {id: "home", href: "/", icon: "home", label: t("Home"), active: current === "/", fixed: true},
            {id: "watchlist", href: "/watchlist", icon: "bookmark", label: t("Watchlist"), active: current === "/watchlist"},
            {id: "playlists", href: "/playlists", icon: "playlist", label: t("Playlists"), active: current.startsWith("/playlist")},
            {id: "history", href: "/history", icon: "history", label: t("History"), active: current === "/history"},
        ];
        for (const l of S.libraries) {
            entries.push({id: "library:" + l.id, href: `/library/${l.id}`, icon: l.type === "shows" ? "shows" : "movies",
                label: l.name, active: current.startsWith(`/library/${l.id}`), library: true,
                extra: l.scanning ? h("span", {class: "spin", title: t("Scanning")}) : null});
        }
        return entries;
    }

    const unpinned = () => new Set((S.user.preferences || {}).unpinned || []);

    async function setPinned(id, pin) {
        const set = unpinned();
        if (pin) set.delete(id); else set.add(id);
        const before = S.user.preferences.unpinned;
        S.user.preferences.unpinned = [...set];
        renderSidebar();
        try {
            S.user = await api("PUT", "/api/me/preferences", {unpinned: [...set]});
        } catch (e) {
            S.user.preferences.unpinned = before;
            renderSidebar();
            toast(e.message, true);
        }
    }

    function showAllNav(on) {
        S.navMore = on;
        const root = document.querySelector(".shell");
        // The list with pin buttons needs room: leave the icon-only modes.
        if (on && root) {
            if (window.matchMedia("(max-width: 860px)").matches) root.classList.add("nav-open");
            else if (root.classList.contains("nav-collapsed")) toggleNav();
        }
        renderSidebar();
        document.querySelector(".sidebar .nav-link, .sidebar button")?.focus();
    }

    /** Replaces the children of an element with a nested list of nodes, skipping empty entries. */
    const fill = (el, nodes) => el.replaceChildren(...nodes.flat(Infinity).filter(Boolean));

    function renderSidebar(path) {
        const nav = document.querySelector(".sidebar");
        if (!nav) return;
        const entries = navEntries(path || location.pathname);
        const hidden = unpinned();
        const navLink = (e) => link(e.href, {class: "nav-link" + (e.active ? " active" : ""), title: e.label},
            icon(e.icon), h("span", {class: "label"}, e.label), e.extra);
        const groupTitle = h("div", {class: "nav-group"}, S.status.name);

        if (S.navMore) {
            const row = (e) => {
                const pinned = !hidden.has(e.id);
                return h("div", {class: "nav-item"}, navLink(e), e.fixed ? null : h("button", {
                    class: "nav-action pin" + (pinned ? " on" : ""), "aria-pressed": String(pinned),
                    title: pinned ? t("Unpin") : t("Pin"), "aria-label": pinned ? t("Unpin {0}", e.label) : t("Pin {0}", e.label),
                    html: ICONS[pinned ? "pinned" : "pin"], onclick: () => setPinned(e.id, !pinned)}));
            };
            fill(nav, [
                h("button", {class: "nav-link nav-back", onclick: () => showAllNav(false)}, icon("back"),
                    h("span", {class: "label"}, t("Pinned"))),
                entries.filter((e) => !e.library).map(row),
                entries.some((e) => e.library) ? groupTitle : null,
                entries.filter((e) => e.library).map(row)]);
            return;
        }
        const shown = entries.filter((e) => !hidden.has(e.id));
        const pinnedRow = (e) => h("div", {class: "nav-item"}, navLink(e), e.fixed ? null : h("button", {
            class: "nav-action", "data-menu": true, title: t("More"), "aria-label": t("More options for {0}", e.label),
            "aria-haspopup": "menu", html: ICONS.moreVertical, onclick: (ev) => popup(ev.currentTarget, [
                {label: t("Unpin"), onclick: () => setPinned(e.id, false)},
                {label: t("Edit sidebar"), onclick: () => showAllNav(true)},
            ])}));
        fill(nav, [
            shown.filter((e) => !e.library).map(pinnedRow),
            shown.some((e) => e.library) ? groupTitle : null,
            shown.filter((e) => e.library).map(pinnedRow),
            h("button", {class: "nav-link nav-more", title: t("More"), onclick: () => showAllNav(true)},
                icon("next"), h("span", {class: "label"}, t("More")))]);
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
                toast(t("Scan finished"));
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
            return left === item.leafCount ? h("span", {class: "badge-new", title: t("Unwatched")})
                : h("span", {class: "badge-count", title: t("{0} unwatched", left)}, left);
        }
        if (item.progress && !item.progress.watched && !item.progress.offsetMs) return h("span", {class: "badge-new", title: t("Unwatched")});
        return null;
    }

    function cardMeta(item) {
        switch (item.type) {
            case "movie":
                return item.year || "";
            case "show":
                return tn(item.childCount || 0, "1 season", "{0} seasons");
            case "season":
                return tn(item.leafCount || 0, "1 episode", "{0} episodes");
            case "episode":
                return `${episodeCode(item)}, ${item.title}`;
            case "extra":
                return [EXTRA_LABELS[item.extraType] ? t(EXTRA_LABELS[item.extraType]) : t("Extra"),
                    item.media && item.media.durationMs ? fmtClock(item.media.durationMs) : ""].filter(Boolean).join(" · ");
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
        toast(watched ? t("Marked as watched") : t("Marked as unwatched"));
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
            item.type === "movie" || item.type === "episode" ? {label: t("Play"), onclick: () => navigate(playHref(item))} : undefined,
            item.type === "movie" || item.type === "episode" ? {label: t("Play from the beginning"), onclick: () => navigate(`/play/${item.id}`)} : undefined,
            {label: watched ? t("Mark as unwatched") : t("Mark as watched"), onclick: () => setWatched(item, !watched)},
            null,
            {label: t("Show details"), onclick: () => navigate(`/item/${item.id}`)},
            item.showId ? {label: t("Go to show"), onclick: () => navigate(`/item/${item.showId}`)} : undefined,
            item.seasonId ? {label: t("Go to season"), onclick: () => navigate(`/item/${item.seasonId}`)} : undefined,
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
        const count = isColl ? tn(item.childCount, "1 title", "{0} titles")
            : tn(item.childCount, "1 entry", "{0} entries") + (item.durationMs ? ", " + fmtDuration(item.durationMs) : "");
        return link(href, {class: "card " + (isColl ? "collection" : "playlist")},
            h("div", {class: "art"}, pic, h("span", {class: "list-badge"}, icon(isColl ? "collection" : "playlist"))),
            h("div", {class: "title", title: item.title}, item.title),
            h("div", {class: "meta"}, count));
    }

    /** Poster (portrait) or thumbnail (landscape) card with hover actions. */
    /** English names of the kinds of extras (translated with t). */
    const EXTRA_LABELS = {trailer: "Trailer", featurette: "Featurette", behindTheScenes: "Behind the scenes",
        deletedScene: "Deleted scene", interview: "Interview", scene: "Scene", short: "Short film", other: "Extra"};

    /** The title of an extra; extras without a name of their own are called after their kind. */
    function extraTitle(item) {
        return Object.values(EXTRA_LABELS).includes(item.title) ? t(item.title) : item.title;
    }

    /** Puts the user's subtitle look (size, background, position, colour) on a subtitle box. */
    function applySubtitleStyle(el) {
        const st = (S.user && S.user.preferences && S.user.preferences.subtitleStyle) || {};
        el.dataset.size = st.size || "m";
        el.dataset.background = st.background || "box";
        el.dataset.position = st.position || "bottom";
        el.dataset.color = st.color || "white";
    }

    /** "4K · 2160p HEVC · 18.2 GB": what tells the versions of a movie apart. */
    function versionLabel(v) {
        const parts = [v.label, v.resolution, codecName(v.videoCodec), v.size ? fmtSize(v.size) : ""].filter(Boolean);
        return parts.filter((p, i) => parts.indexOf(p) === i).join(" · ") || v.fileName;
    }

    /** A trailer or other extra: plays at once, it has no page of its own. */
    function extraCard(item) {
        const title = extraTitle(item);
        return link(`/play/${item.id}`, {class: "card landscape extra", "aria-label": t("Play {0}", title)},
            h("div", {class: "art"}, art(item, "thumb", title),
                h("div", {class: "card-actions"}, h("span", {class: "card-play", html: ICONS.play}))),
            h("div", {class: "title", title}, title),
            h("div", {class: "meta"}, cardMeta(item)));
    }

    function card(item, style) {
        if (item.type === "collection" || item.type === "playlist") return listCard(item);
        if (item.type === "extra") return extraCard(item);
        const landscape = style === "landscape";
        const kind = landscape ? "thumb" : "poster";
        const title = item.type === "episode" && landscape ? item.showTitle || item.title : item.title;
        const playable = item.type === "movie" || item.type === "episode";
        const actions = h("div", {class: "card-actions"},
            playable ? h("button", {class: "card-play", title: t("Play"), "aria-label": t("Play {0}", title), html: ICONS.play,
                onclick: (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    navigate(playHref(item));
                }}) : null,
            h("button", {class: "card-btn card-check", title: isWatched(item) ? t("Mark as unwatched") : t("Mark as watched"),
                "aria-label": t("Toggle watched"), html: ICONS.check, onclick: (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setWatched(item, !isWatched(item));
                }}),
            h("button", {class: "card-btn card-more", title: t("More"), "aria-label": t("More"), "data-menu": true, html: ICONS.more,
                onclick: (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    itemMenu(e.currentTarget, item);
                }}));
        return link(`/item/${item.id}`, {class: "card" + (landscape ? " landscape" : "")},
            h("div", {class: "art"}, art(item, kind, title), unwatchedBadge(item), progressBar(item), actions),
            h("div", {class: "title", title}, title),
            h("div", {class: "meta"}, cardMeta(item)));
    }

    /** A horizontally scrolling row with arrow buttons (a hub). */
    function hubRow(hub, moreHref) {
        const row = h("div", {class: "row" + (hub.style === "landscape" ? " landscape" : "")}, hub.items.map((it) => card(it, hub.style)));
        if (hub.href) moreHref = hub.href;
        const scrollBy = (dir) => row.scrollBy({left: dir * row.clientWidth * 0.85, behavior: "smooth"});
        const prev = h("button", {class: "row-arrow", "aria-label": t("Back"), onclick: () => scrollBy(-1)}, icon("back"));
        const next = h("button", {class: "row-arrow", "aria-label": t("Next"), onclick: () => scrollBy(1)}, icon("next"));
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

    return {S, store, noteBuild, extraTitle, versionLabel, applySubtitleStyle, t, tn, tParts, num, locale, h, ICONS, icon, fmtDuration, fmtClock, fmtDate, fmtSize, fmtBitrate, language, codecName, channels,
        audioLabel, subtitleLabel, episodeCode, deviceName, toast, img, art, api, ApiError, logoutLocal, logout, route,
        navigate, link, render, emptyState, popup, closeMenus, refreshLibraries, renderSidebar, card, hubRow, isWatched,
        setWatched, playHref, itemMenu, progressBar, unwatchedBadge, menuExtensions, PREF_LANGS, defaultTracks, avatar,
        SLOGAN: "Your Media. Your Server. Your Freedom."};
})();

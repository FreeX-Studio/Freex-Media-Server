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
 * FreeX Web: translations. Texts are written in English in the code and passed through t(),
 * which looks them up in the catalog of the display language (i18n/<language>.js) and fills in
 * the placeholders {0}, {1}, … English is used when a text has no translation. Loaded first.
 */
window.FreeXI18n = (() => {
    "use strict";

    /** Display languages, each with its name in its own language. */
    const LANGUAGES = {en: "English", de: "Deutsch"};
    const STORE_KEY = "freex.lang";
    const catalogs = {};
    let current = "en";

    function stored() {
        try {
            return localStorage.getItem(STORE_KEY) || "";
        } catch (e) {
            return "";
        }
    }

    /** The first browser language FreeX has, otherwise English. */
    function browserLanguage() {
        for (const l of navigator.languages || [navigator.language || ""]) {
            const code = String(l).toLowerCase().split("-")[0];
            if (LANGUAGES[code]) return code;
        }
        return "en";
    }

    /**
     * Applies the user's choice: a language code, or "" for the browser's language. Remembered in
     * this browser, so the sign-in page already uses it.
     */
    function setLanguage(preference) {
        const pref = LANGUAGES[preference] ? preference : "";
        try {
            if (pref) localStorage.setItem(STORE_KEY, pref); else localStorage.removeItem(STORE_KEY);
        } catch (e) {
            // storage unavailable: the choice still applies to this page
        }
        current = pref || browserLanguage();
        document.documentElement.lang = current;
        return current;
    }

    function fill(text, args) {
        return text.replace(/\{(\d)\}/g, (m, i) => (args[i] == null ? "" : String(args[i])));
    }

    /** Translates an English text and fills in its placeholders. */
    function t(text, ...args) {
        const c = catalogs[current];
        return fill((c && c[text]) || text, args);
    }

    /** Text for a count: `one` for exactly 1, otherwise `other` with the count as {0}. */
    function tn(count, one, other, ...args) {
        return t(count === 1 ? one : other, count, ...args);
    }

    /** Like t(), but placeholders may be DOM nodes: returns the parts for h(). */
    function tParts(text, ...nodes) {
        const c = catalogs[current];
        return ((c && c[text]) || text).split(/(\{\d\})/).filter(Boolean)
            .map((part) => /^\{\d\}$/.test(part) ? nodes[Number(part.slice(1, -1))] : part);
    }

    /** Locale for dates and numbers: the browser's own variant if it speaks the display language. */
    function locale() {
        const nav = navigator.language || "";
        return nav.toLowerCase().split("-")[0] === current ? nav : current;
    }

    function num(n, digits) {
        return Number(n).toLocaleString(locale(), {minimumFractionDigits: digits, maximumFractionDigits: digits});
    }

    setLanguage(stored());

    return {
        LANGUAGES, t, tn, tParts, num, locale, setLanguage,
        lang: () => current,
        add: (lang, catalog) => catalogs[lang] = Object.assign(catalogs[lang] || {}, catalog),
    };
})();

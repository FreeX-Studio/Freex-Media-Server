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

/* FreeX Web, part 4: sign-in and setup, settings, account security, users, server settings. */
(() => {
    "use strict";
    const M = window.FreeX;
    const {S, h, link, icon, api} = M;

    // ------------------------------------------------------------------ sign-in pages

    function authPage(title, text, fields, buttonLabel, onSubmit, footer) {
        const error = h("div", {class: "error-text", role: "alert"});
        const btn = h("button", {class: "btn primary wide", type: "submit"}, buttonLabel);
        const form = h("form", {onsubmit: async (e) => {
                e.preventDefault();
                error.textContent = "";
                btn.disabled = true;
                try {
                    await onSubmit(error);
                } catch (err) {
                    error.textContent = err.message;
                } finally {
                    btn.disabled = false;
                }
            }}, fields, error, btn);
        document.getElementById("app").replaceChildren(h("div", {class: "auth"}, h("div", {class: "auth-box"},
            h("div", {class: "auth-brand"}, h("img", {src: "/logo.svg", alt: ""}), h("span", null, "Free", h("b", null, "X"))),
            h("p", {class: "slogan"}, M.SLOGAN),
            h("div", {class: "auth-card"}, h("h1", null, title), text ? h("p", null, text) : null, form, footer))));
        form.querySelector("input")?.focus();
    }

    function afterLogin(res, target) {
        S.token = res.token;
        S.user = res.user;
        M.store.set("freex.token", res.token);
        S.status.setupRequired = false;
        S.status = null; // reload with admin details
        const next = target || new URLSearchParams(location.search).get("next");
        M.navigate(next && next.startsWith("/") && !next.startsWith("//") && !next.startsWith("/login") ? next : "/", true);
    }

    M.route("/login", () => {
        if (S.user) return M.navigate("/", true);
        const user = h("input", {id: "l-user", autocomplete: "username", required: true, autocapitalize: "none", spellcheck: "false"});
        const pass = h("input", {id: "l-pass", type: "password", autocomplete: "current-password", required: true});
        authPage("Anmelden", null, [
            h("div", {class: "field"}, h("label", {for: "l-user"}, "Benutzername"), user),
            h("div", {class: "field"}, h("label", {for: "l-pass"}, "Passwort"), pass),
        ], "Anmelden", async () => {
            const res = await api("POST", "/api/login", {username: user.value, password: pass.value});
            if (res.twoFactorRequired) return secondFactor(res.challenge);
            afterLogin(res);
        }, h("p", {class: "auth-foot"}, `Bei ${S.status.name}`));
    }, {auth: false, shell: false});

    function secondFactor(challenge) {
        let recovery = false;
        const code = h("input", {id: "l-code", inputmode: "numeric", autocomplete: "one-time-code", required: true,
            pattern: "[0-9 ]{6,7}", maxlength: 7, class: "code-input", "aria-describedby": "code-help"});
        const help = h("p", {id: "code-help", class: "muted small"}, "Gib den 6-stelligen Code aus deiner Authenticator-App ein.");
        const toggle = h("button", {type: "button", class: "link-btn", onclick: () => {
                recovery = !recovery;
                code.value = "";
                code.removeAttribute("pattern");
                if (recovery) {
                    code.setAttribute("inputmode", "text");
                    code.setAttribute("maxlength", 11);
                    code.classList.remove("code-input");
                    help.textContent = "Gib einen deiner Wiederherstellungscodes ein (z. B. abcde-12345). Jeder Code funktioniert nur einmal.";
                    toggle.textContent = "Stattdessen Authenticator-Code verwenden";
                } else {
                    code.setAttribute("inputmode", "numeric");
                    code.setAttribute("maxlength", 7);
                    code.setAttribute("pattern", "[0-9 ]{6,7}");
                    code.classList.add("code-input");
                    help.textContent = "Gib den 6-stelligen Code aus deiner Authenticator-App ein.";
                    toggle.textContent = "Wiederherstellungscode verwenden";
                }
                code.focus();
            }}, "Wiederherstellungscode verwenden");
        authPage("Bestätigung", null, [h("div", {class: "field"}, h("label", {for: "l-code"}, "Bestätigungscode"), code), help],
            "Bestätigen", async (error) => {
                try {
                    afterLogin(await api("POST", "/api/login/verify", {challenge, code: code.value}));
                } catch (e) {
                    if (e.status === 401 && /abgelaufen|Zu viele/.test(e.message)) {
                        M.toast(e.message, true);
                        return M.navigate("/login", true);
                    }
                    code.select();
                    throw e;
                }
            }, h("div", {class: "auth-foot"}, toggle, h("br"), h("a", {href: "/login", "data-link": true}, "Abbrechen")));
    }

    M.route("/setup", () => {
        if (!S.status.setupRequired) return M.navigate("/", true);
        const user = h("input", {id: "s-user", autocomplete: "username", required: true});
        const pass = h("input", {id: "s-pass", type: "password", autocomplete: "new-password", required: true, minlength: 10});
        const pass2 = h("input", {id: "s-pass2", type: "password", autocomplete: "new-password", required: true});
        authPage("Server einrichten", "Lege das Administratorkonto an. Danach fügst du deine Film- und Serienordner hinzu.", [
            h("div", {class: "field"}, h("label", {for: "s-user"}, "Benutzername"), user),
            h("div", {class: "field"}, h("label", {for: "s-pass"}, "Passwort (mindestens 10 Zeichen)"), pass),
            h("div", {class: "field"}, h("label", {for: "s-pass2"}, "Passwort wiederholen"), pass2),
        ], "Konto anlegen", async () => {
            if (pass.value !== pass2.value) throw new Error("Die Passwörter stimmen nicht überein");
            afterLogin(await api("POST", "/api/setup", {username: user.value, password: pass.value}), "/settings/libraries");
        });
    }, {auth: false, shell: false});

    // ------------------------------------------------------------------ settings frame

    const TABS = [
        ["account", "Konto", false],
        ["security", "Sicherheit", false],
        ["libraries", "Bibliotheken", true],
        ["users", "Benutzer", true],
        ["server", "Server", true],
        ["about", "Über FreeX", false],
    ];

    M.route("/settings", (main) => M.navigate("/settings/" + (S.user.admin ? "libraries" : "account"), true));
    M.route("/settings/:tab", async (main, p) => {
        let tab = p.tab;
        if (!TABS.some(([id, , admin]) => id === tab && (!admin || S.user.admin))) tab = "account";
        const nav = h("nav", {class: "settings-nav", "aria-label": "Einstellungen"},
            h("div", {class: "nav-group"}, "Mein Konto"),
            TABS.filter(([, , admin]) => !admin).map(([id, label]) => link(`/settings/${id}`, {class: "nav-link" + (id === tab ? " active" : "")}, label)),
            S.user.admin ? [h("div", {class: "nav-group"}, "Server"),
                TABS.filter(([, , admin]) => admin).map(([id, label]) => link(`/settings/${id}`, {class: "nav-link" + (id === tab ? " active" : "")}, label))] : null);
        const content = h("div", {class: "settings-content"});
        main.replaceChildren(h("div", {class: "page settings"}, nav, content));
        await {account, security, libraries, users, server, about}[tab](content);
    });

    const panel = (title, ...children) => h("section", {class: "panel"}, h("h2", null, title), children);
    const field = (id, label, input, hint) => h("div", {class: "field"}, h("label", {for: id}, label), input, hint ? h("div", {class: "hint"}, hint) : null);

    function confirmButton(label, confirmLabel, onConfirm, cls) {
        const b = h("button", {class: "btn small " + (cls || "danger"), onclick: async () => {
                if (b.dataset.confirm !== "1") {
                    b.dataset.confirm = "1";
                    b.textContent = confirmLabel;
                    setTimeout(() => {
                        b.dataset.confirm = "";
                        b.textContent = label;
                    }, 4000);
                    return;
                }
                await onConfirm();
            }}, label);
        return b;
    }

    // ------------------------------------------------------------------ account

    async function account(c) {
        const cur = h("input", {id: "p-cur", type: "password", autocomplete: "current-password"});
        const next = h("input", {id: "p-new", type: "password", autocomplete: "new-password"});
        const next2 = h("input", {id: "p-new2", type: "password", autocomplete: "new-password"});
        const error = h("div", {class: "error-text", role: "alert"});
        c.append(h("h1", null, "Konto"),
            panel("Profil",
                h("dl", {class: "kv"}, h("dt", null, "Benutzername"), h("dd", null, S.user.username),
                    h("dt", null, "Rolle"), h("dd", null, S.user.admin ? "Administrator" : "Benutzer"),
                    h("dt", null, "Passwort geändert"), h("dd", null, M.fmtDate(S.user.passwordChangedAt)))),
            avatarPanel(),
            playbackPanel(),
            panel("Passwort ändern",
                field("p-cur", "Aktuelles Passwort", cur),
                field("p-new", "Neues Passwort", next, "Mindestens 10 Zeichen. Nach dem Ändern werden alle anderen Geräte abgemeldet."),
                field("p-new2", "Neues Passwort wiederholen", next2),
                error,
                h("button", {class: "btn primary", onclick: async () => {
                        error.textContent = "";
                        if (next.value !== next2.value) return error.textContent = "Die neuen Passwörter stimmen nicht überein";
                        try {
                            const r = await api("PUT", "/api/me/password", {currentPassword: cur.value, password: next.value});
                            cur.value = next.value = next2.value = "";
                            S.user = await api("GET", "/api/me");
                            M.toast(r.endedSessions ? `Passwort geändert, ${r.endedSessions} andere Geräte abgemeldet` : "Passwort geändert");
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }}, "Passwort ändern")),
            panel("Abmelden", h("p", {class: "muted"}, "Meldet dieses Gerät ab."), h("button", {class: "btn", onclick: M.logout}, "Abmelden")));
    }

    function avatarPanel() {
        const box = panel("Profilbild");
        const error = h("div", {class: "error-text", role: "alert"});
        const file = h("input", {type: "file", accept: "image/jpeg,image/png,image/gif", hidden: true});

        async function changed(user) {
            S.user = user;
            M.render(); // header and this page show the new picture
        }

        file.addEventListener("change", async () => {
            const f = file.files[0];
            if (!f) return;
            error.textContent = "";
            if (f.size > 10 * 1024 * 1024) return error.textContent = "Das Bild ist größer als 10 MB";
            try {
                // Sent as is; the server crops and scales it and keeps only its own copy.
                const res = await fetch("/api/me/avatar", {method: "PUT", credentials: "same-origin", body: f,
                    headers: {"X-FreeX-Token": S.token, "Content-Type": f.type || "application/octet-stream"}});
                const data = await res.json().catch(() => null);
                if (!res.ok) throw new Error((data && data.error) || `Fehler ${res.status}`);
                M.toast("Profilbild gespeichert");
                await changed(data);
            } catch (e) {
                error.textContent = e.message;
            }
        });
        box.append(h("div", {class: "avatar-edit"},
                M.avatar(S.user, "large"),
                h("div", null,
                    h("p", {class: "muted"}, "JPEG, PNG oder GIF bis 10 MB. Das Bild wird quadratisch zugeschnitten."),
                    h("div", {class: "btn-row"},
                        h("button", {class: "btn", onclick: () => file.click()}, S.user.avatar ? "Anderes Bild wählen" : "Bild hochladen"),
                        S.user.avatar ? h("button", {class: "btn", onclick: async () => {
                                await changed(await api("DELETE", "/api/me/avatar"));
                                M.toast("Profilbild entfernt");
                            }}, "Entfernen") : null))),
            file, error);
        return box;
    }

    function playbackPanel() {
        const p = S.user.preferences || {};
        const langOptions = (empty) => [h("option", {value: ""}, empty)].concat(M.PREF_LANGS.map(([code, name]) => h("option", {value: code}, name)));
        const audio = h("select", {id: "pref-audio"}, langOptions("Standardspur der Datei"));
        audio.value = p.audioLanguage || "";
        const mode = h("select", {id: "pref-submode"},
            h("option", {value: "off"}, "Aus"),
            h("option", {value: "forced"}, "Nur erzwungene Untertitel (fremdsprachige Stellen)"),
            h("option", {value: "foreign"}, "Wenn der Ton eine andere Sprache hat"),
            h("option", {value: "always"}, "Immer"));
        mode.value = p.subtitleMode || "forced";
        const subLang = h("select", {id: "pref-sublang"}, langOptions("Wie die Audiosprache"));
        subLang.value = p.subtitleLanguage || "";
        const subLangField = field("pref-sublang", "Untertitelsprache", subLang);
        const sync = () => subLangField.hidden = mode.value === "off";
        mode.addEventListener("change", sync);
        sync();
        return panel("Wiedergabe",
            h("p", {class: "muted"}, "Gilt für alle Titel, solange du auf der Detailseite oder im Player nichts anderes auswählst."),
            field("pref-audio", "Bevorzugte Audiosprache", audio, "Gibt es die Sprache in einer Datei nicht, wird deren Standardspur abgespielt."),
            field("pref-submode", "Untertitel einschalten", mode),
            subLangField,
            h("button", {class: "btn primary", onclick: async () => {
                    S.user = await api("PUT", "/api/me/preferences",
                        {audioLanguage: audio.value, subtitleMode: mode.value, subtitleLanguage: subLang.value});
                    M.toast("Wiedergabe-Einstellungen gespeichert");
                }}, "Speichern"));
    }

    // ------------------------------------------------------------------ security

    async function security(c) {
        const [me, sessions, logins] = await Promise.all([api("GET", "/api/me"), api("GET", "/api/me/sessions"), api("GET", "/api/me/logins")]);
        S.user = me;
        c.append(h("h1", null, "Sicherheit"), twoFactorPanel(me), sessionsPanel(sessions), loginsPanel(logins));
    }

    function passwordPrompt(id) {
        return h("input", {id, type: "password", autocomplete: "current-password", placeholder: "Aktuelles Passwort"});
    }

    function twoFactorPanel(me) {
        const box = panel("Zwei-Faktor-Anmeldung");
        if (me.twoFactor) {
            const pw = passwordPrompt("tf-pw");
            const error = h("div", {class: "error-text", role: "alert"});
            box.append(
                h("p", {class: "status-line ok"}, icon("shield"), "Aktiv. Beim Anmelden wird zusätzlich ein Code aus deiner Authenticator-App abgefragt."),
                h("p", {class: "muted"}, `Noch ${me.recoveryCodesLeft} Wiederherstellungscodes übrig.`),
                field("tf-pw", "Zur Bestätigung: aktuelles Passwort", pw),
                error,
                h("div", {class: "btn-row"},
                    h("button", {class: "btn", onclick: async () => {
                            error.textContent = "";
                            try {
                                const r = await api("POST", "/api/me/2fa/recovery", {currentPassword: pw.value});
                                pw.value = "";
                                showRecoveryCodes(box, r.recoveryCodes);
                            } catch (e) {
                                error.textContent = e.message;
                            }
                        }}, "Neue Wiederherstellungscodes"),
                    h("button", {class: "btn danger", onclick: async () => {
                            error.textContent = "";
                            try {
                                await api("POST", "/api/me/2fa/disable", {currentPassword: pw.value});
                                M.toast("Zwei-Faktor-Anmeldung ausgeschaltet");
                                M.render();
                            } catch (e) {
                                error.textContent = e.message;
                            }
                        }}, "Ausschalten")));
            return box;
        }
        const pw = passwordPrompt("tf-pw");
        const error = h("div", {class: "error-text", role: "alert"});
        box.append(
            h("p", {class: "status-line"}, "Aus. Schütze dein Konto zusätzlich mit einem Code aus einer Authenticator-App wie Aegis, 2FAS, Google oder Microsoft Authenticator."),
            field("tf-pw", "Zur Bestätigung: aktuelles Passwort", pw),
            error,
            h("button", {class: "btn primary", onclick: async () => {
                    error.textContent = "";
                    try {
                        const r = await api("POST", "/api/me/2fa/setup", {currentPassword: pw.value});
                        twoFactorSetup(box, r);
                    } catch (e) {
                        error.textContent = e.message;
                    }
                }}, "Einrichten"));
        return box;
    }

    function twoFactorSetup(box, setup) {
        let qrImg = null;
        try {
            const qr = window.qrcode(0, "M");
            qr.addData(setup.uri);
            qr.make();
            qrImg = h("img", {class: "qr", src: qr.createDataURL(5, 12), alt: "QR-Code für die Authenticator-App", width: 220, height: 220});
        } catch (e) {
            qrImg = null;
        }
        const code = h("input", {id: "tf-code", inputmode: "numeric", autocomplete: "one-time-code", maxlength: 7, class: "code-input"});
        const error = h("div", {class: "error-text", role: "alert"});
        const secret = setup.secret.replace(/(.{4})/g, "$1 ").trim();
        box.replaceChildren(h("h2", null, "Zwei-Faktor-Anmeldung einrichten"),
            h("ol", {class: "steps"},
                h("li", null, "Scanne den QR-Code mit deiner Authenticator-App."),
                h("li", null, "Oder gib den Schlüssel von Hand ein: ", h("code", {class: "secret"}, secret)),
                h("li", null, "Gib zur Bestätigung den 6-stelligen Code ein, den die App anzeigt.")),
            qrImg,
            field("tf-code", "Code aus der App", code),
            error,
            h("div", {class: "btn-row"},
                h("button", {class: "btn primary", onclick: async () => {
                        error.textContent = "";
                        try {
                            const r = await api("POST", "/api/me/2fa/enable", {code: code.value});
                            S.user = await api("GET", "/api/me");
                            showRecoveryCodes(box, r.recoveryCodes, true);
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }}, "Aktivieren"),
                h("button", {class: "btn", onclick: () => M.render()}, "Abbrechen")));
        code.focus();
    }

    function showRecoveryCodes(box, codes, justEnabled) {
        const text = codes.join("\n");
        box.replaceChildren(h("h2", null, justEnabled ? "Zwei-Faktor-Anmeldung ist aktiv" : "Neue Wiederherstellungscodes"),
            h("p", null, "Bewahre diese Codes sicher auf, zum Beispiel ausgedruckt oder im Passwort-Manager. Mit jedem Code kannst du dich einmal anmelden, falls du dein Handy nicht hast. Sie werden nur jetzt angezeigt."),
            justEnabled ? h("p", {class: "muted"}, "Alle anderen Geräte wurden abgemeldet.") : null,
            h("pre", {class: "codes"}, text),
            h("div", {class: "btn-row"},
                h("button", {class: "btn", onclick: async () => {
                        try {
                            await navigator.clipboard.writeText(text);
                            M.toast("Codes kopiert");
                        } catch (e) {
                            M.toast("Kopieren nicht möglich, bitte markieren und kopieren", true);
                        }
                    }}, "Kopieren"),
                h("button", {class: "btn primary", onclick: () => M.render()}, "Ich habe die Codes gespeichert")));
    }

    function sessionsPanel(sessions) {
        const box = panel("Angemeldete Geräte",
            h("div", {class: "list"}, sessions.map((s) => h("div", {class: "list-row"},
                h("div", {class: "grow"},
                    h("div", {class: "name"}, M.deviceName(s.userAgent), s.current ? h("span", {class: "pill"}, "Dieses Gerät") : null),
                    h("div", {class: "detail-line"}, `${s.ip || "?"}, zuletzt aktiv ${M.fmtDate(s.lastSeen)}, angemeldet ${M.fmtDate(s.createdAt)}`)),
                s.current ? null : confirmButton("Abmelden", "Wirklich abmelden?", async () => {
                    await api("DELETE", `/api/me/sessions/${s.id}`);
                    M.toast("Gerät abgemeldet");
                    M.render();
                })))));
        if (sessions.length > 1) {
            box.append(h("div", {class: "btn-row"}, confirmButton("Alle anderen Geräte abmelden", "Wirklich alle anderen abmelden?", async () => {
                const r = await api("POST", "/api/me/sessions/logout-others");
                M.toast(`${r.ended} Geräte abgemeldet`);
                M.render();
            })));
        }
        return box;
    }

    function loginsPanel(logins) {
        return panel("Letzte Anmeldungen",
            logins.length ? h("div", {class: "list"}, logins.map((e) => h("div", {class: "list-row"},
                h("span", {class: "dot " + (e.success ? "ok" : "bad"), title: e.success ? "Erfolgreich" : "Fehlgeschlagen"}),
                h("div", {class: "grow"},
                    h("div", {class: "name"}, e.success ? "Angemeldet" : "Fehlversuch", h("span", {class: "muted"}, `  ${e.detail}`)),
                    h("div", {class: "detail-line"}, `${M.fmtDate(e.time)}, ${e.ip}, ${M.deviceName(e.userAgent)}`)))))
                : h("p", {class: "muted"}, "Noch keine Einträge."),
            h("p", {class: "hint"}, "Fehlversuche mit deinem Benutzernamen erscheinen hier. Wenn du sie nicht kennst, ändere dein Passwort und schalte die Zwei-Faktor-Anmeldung ein."));
    }

    // ------------------------------------------------------------------ libraries

    async function libraries(c) {
        S.libraries = await api("GET", "/api/libraries");
        M.renderSidebar();
        const list = S.libraries.length ? h("div", {class: "list"}, S.libraries.map((l) => h("div", {class: "list-row"},
            icon(l.type === "shows" ? "shows" : "movies", "muted-ico"),
            h("div", {class: "grow"},
                h("div", {class: "name"}, l.name),
                h("div", {class: "detail-line"}, `${l.type === "shows" ? "Serien" : "Filme"}, ${l.itemCount} Einträge, zuletzt gescannt ${M.fmtDate(l.lastScan)}`),
                l.paths.map((p) => h("div", {class: "detail-line mono"}, p))),
            l.scanning ? h("span", {class: "spin", title: "Wird gescannt"}) : h("button", {class: "btn small", onclick: async () => {
                    await api("POST", `/api/libraries/${l.id}/scan`);
                    M.toast(`„${l.name}“ wird gescannt`);
                    await M.refreshLibraries();
                    M.render();
                }}, "Jetzt scannen"),
            confirmButton("Entfernen", "Wirklich entfernen?", async () => {
                await api("DELETE", `/api/libraries/${l.id}`);
                M.toast(`„${l.name}“ entfernt`);
                await M.refreshLibraries();
                M.render();
            })))) : h("p", {class: "muted"}, "Es gibt noch keine Bibliotheken.");
        c.append(h("h1", null, "Bibliotheken"), list, libraryForm());
        if (S.libraries.some((l) => l.scanning)) M.refreshLibraries();
    }

    function libraryForm() {
        const paths = [];
        const name = h("input", {id: "lib-name", placeholder: "z. B. Filme", maxlength: 64});
        const type = h("select", {id: "lib-type"}, h("option", {value: "movies"}, "Filme"), h("option", {value: "shows"}, "Serien"));
        const pathList = h("div", {class: "path-list"});
        const error = h("div", {class: "error-text", role: "alert"});
        const manual = h("input", {placeholder: "/pfad/zum/ordner", "aria-label": "Ordnerpfad"});

        const drawPaths = () => pathList.replaceChildren(...paths.map((p, i) => h("div", {class: "path-item"}, h("span", {class: "mono"}, p),
            h("button", {class: "btn small", onclick: () => {
                    paths.splice(i, 1);
                    drawPaths();
                }}, "Entfernen"))));
        const addPath = (p) => {
            p = (p || "").trim();
            if (p && !paths.includes(p)) paths.push(p);
            drawPaths();
        };

        const picker = h("div", {class: "picker"});
        const browse = async (path) => {
            try {
                const res = await api("GET", "/api/fs?path=" + encodeURIComponent(path || ""));
                picker.replaceChildren(
                    h("div", {class: "current"},
                        res.parent != null ? h("button", {class: "btn small icon", "aria-label": "Eine Ebene höher", onclick: () => browse(res.parent)}, icon("back")) : null,
                        h("span", {class: "mono"}, res.path || "Speicherorte"),
                        res.path ? h("button", {class: "btn small primary", onclick: () => addPath(res.path)}, "Hinzufügen") : null),
                    h("div", {class: "folders"}, res.folders.length ? res.folders.map((f) => h("button", {onclick: () => browse(f.path)}, f.name))
                        : h("div", {class: "muted pad"}, "Keine Unterordner")));
            } catch (e) {
                error.textContent = e.message;
            }
        };
        browse("");

        const submit = h("button", {class: "btn primary", onclick: async () => {
                error.textContent = "";
                if (manual.value.trim()) {
                    addPath(manual.value);
                    manual.value = "";
                }
                submit.disabled = true;
                try {
                    await api("POST", "/api/libraries", {name: name.value, type: type.value, paths});
                    M.toast("Bibliothek angelegt, Scan läuft");
                    await M.refreshLibraries();
                    M.render();
                } catch (e) {
                    error.textContent = e.message;
                } finally {
                    submit.disabled = false;
                }
            }}, "Bibliothek anlegen");

        return panel("Bibliothek hinzufügen",
            field("lib-name", "Name", name),
            field("lib-type", "Inhalt", type),
            h("div", {class: "field"}, h("label", null, "Ordner"), pathList, picker,
                h("div", {class: "inline"}, manual, h("button", {class: "btn", onclick: () => {
                        addPath(manual.value);
                        manual.value = "";
                    }}, "Hinzufügen"))),
            error, submit);
    }

    // ------------------------------------------------------------------ users

    /**
     * Choice of the libraries a user may see: all (also future ones) or selected ones.
     * Returns the element and a function that gives "all" or the list of ids.
     */
    function libraryPicker(libraries, current, idPrefix) {
        const all = h("input", {type: "radio", name: idPrefix + "-mode", value: "all"});
        const some = h("input", {type: "radio", name: idPrefix + "-mode", value: "some"});
        const selected = new Set(Array.isArray(current) ? current : []);
        const boxes = libraries.map((l) => {
            const cb = h("input", {type: "checkbox", value: l.id});
            cb.checked = selected.has(l.id);
            return {cb, el: h("label", {class: "check"}, cb, icon(l.type === "shows" ? "shows" : "movies"), l.name)};
        });
        const list = h("div", {class: "lib-picker"}, boxes.map((b) => b.el),
            libraries.length ? null : h("p", {class: "muted"}, "Noch keine Bibliotheken angelegt."));
        const sync = () => {
            list.classList.toggle("disabled", all.checked);
            boxes.forEach((b) => b.cb.disabled = all.checked);
        };
        (current === "all" || current == null ? all : some).checked = true;
        all.addEventListener("change", sync);
        some.addEventListener("change", sync);
        sync();
        const el = h("div", null,
            h("label", {class: "check"}, all, "Alle Bibliotheken, auch neu hinzukommende"),
            h("label", {class: "check"}, some, "Nur ausgewählte Bibliotheken:"),
            list);
        return {el, value: () => all.checked ? "all" : boxes.filter((b) => b.cb.checked).map((b) => b.cb.value)};
    }

    function librarySummary(u, libraries) {
        if (u.admin) return "alle Bibliotheken (Administrator)";
        if (u.libraries === "all") return "alle Bibliotheken";
        const names = libraries.filter((l) => u.libraries.includes(l.id)).map((l) => l.name);
        return names.length ? names.join(", ") : "keine Bibliothek freigegeben";
    }

    function editLibraries(u, libraries) {
        const picker = libraryPicker(libraries, u.libraries, "el-" + u.id);
        const error = h("div", {class: "error-text", role: "alert"});
        const dialog = h("div", {class: "modal-back", onclick: (e) => e.target === dialog && dialog.remove()},
            h("div", {class: "modal", role: "dialog", "aria-modal": "true", "aria-label": "Bibliotheken freigeben"},
                h("h2", null, `Bibliotheken für ${u.username}`),
                h("p", {class: "muted"}, "Nur freigegebene Bibliotheken erscheinen für diesen Benutzer, auch in Suche, Startseite und Watch Together."),
                picker.el,
                error,
                h("div", {class: "btn-row"},
                    h("button", {class: "btn primary", onclick: async () => {
                            try {
                                await api("PUT", `/api/users/${u.id}`, {libraries: picker.value()});
                                dialog.remove();
                                M.toast(`Freigaben für ${u.username} gespeichert`);
                                M.render();
                            } catch (e) {
                                error.textContent = e.message;
                            }
                        }}, "Speichern"),
                    h("button", {class: "btn", onclick: () => dialog.remove()}, "Abbrechen"))));
        document.body.append(dialog);
    }

    async function users(c) {
        const [list, libraries] = await Promise.all([api("GET", "/api/users"), api("GET", "/api/libraries")]);
        const rows = h("div", {class: "list"}, list.map((u) => {
            const row = h("div", {class: "list-row"},
                M.avatar(u),
                h("div", {class: "grow"},
                    h("div", {class: "name"}, u.username, u.id === S.user.id ? h("span", {class: "pill"}, "Du") : null),
                    h("div", {class: "detail-line"}, [u.admin ? "Administrator" : "Benutzer", u.twoFactor ? "Zwei-Faktor aktiv" : "ohne Zwei-Faktor"].join(", ")),
                    h("div", {class: "detail-line"}, "Sieht: " + librarySummary(u, libraries))),
                h("button", {class: "btn small", "data-menu": true, onclick: (e) => M.popup(e.currentTarget, [
                        u.admin ? undefined : {label: "Bibliotheken freigeben", onclick: () => editLibraries(u, libraries)},
                        {label: "Passwort zurücksetzen", onclick: () => resetPassword(u)},
                        u.twoFactor ? {label: "Zwei-Faktor zurücksetzen", onclick: async () => {
                                await api("PUT", `/api/users/${u.id}`, {resetTwoFactor: true});
                                M.toast(`Zwei-Faktor für ${u.username} zurückgesetzt, Geräte abgemeldet`);
                                M.render();
                            }} : undefined,
                        u.id !== S.user.id ? {label: u.admin ? "Zum Benutzer machen" : "Zum Administrator machen", onclick: async () => {
                                try {
                                    await api("PUT", `/api/users/${u.id}`, {admin: !u.admin});
                                    M.render();
                                } catch (err) {
                                    M.toast(err.message, true);
                                }
                            }} : undefined,
                    ])}, "Bearbeiten"),
                u.id === S.user.id ? null : confirmButton("Löschen", "Wirklich löschen?", async () => {
                    await api("DELETE", `/api/users/${u.id}`);
                    M.toast(`${u.username} gelöscht`);
                    M.render();
                }));
            return row;
        }));

        const name = h("input", {id: "u-name", autocomplete: "off"});
        const pass = h("input", {id: "u-pass", type: "password", autocomplete: "new-password"});
        const admin = h("input", {type: "checkbox"});
        const picker = libraryPicker(libraries, "all", "new-user");
        const pickerField = h("div", {class: "field"}, h("label", null, "Freigegebene Bibliotheken"), picker.el);
        admin.addEventListener("change", () => pickerField.hidden = admin.checked);
        const error = h("div", {class: "error-text", role: "alert"});
        c.append(h("h1", null, "Benutzer"), rows, panel("Benutzer hinzufügen",
            field("u-name", "Benutzername", name),
            field("u-pass", "Passwort", pass, "Mindestens 10 Zeichen. Der Benutzer kann es danach selbst ändern."),
            h("label", {class: "check"}, admin, "Administrator: darf Bibliotheken und Benutzer verwalten und sieht alles"),
            pickerField,
            error,
            h("button", {class: "btn primary", onclick: async () => {
                    error.textContent = "";
                    try {
                        await api("POST", "/api/users", {username: name.value, password: pass.value, admin: admin.checked,
                            libraries: admin.checked ? "all" : picker.value()});
                        M.toast(`${name.value.trim()} angelegt`);
                        M.render();
                    } catch (e) {
                        error.textContent = e.message;
                    }
                }}, "Benutzer anlegen")));
    }

    function resetPassword(u) {
        const pass = h("input", {id: "rp-pass", type: "password", autocomplete: "new-password"});
        const error = h("div", {class: "error-text", role: "alert"});
        const dialog = h("div", {class: "modal-back", onclick: (e) => e.target === dialog && dialog.remove()},
            h("div", {class: "modal", role: "dialog", "aria-modal": "true", "aria-label": "Passwort zurücksetzen"},
                h("h2", null, `Neues Passwort für ${u.username}`),
                field("rp-pass", "Neues Passwort", pass, "Alle Geräte dieses Benutzers werden abgemeldet."),
                error,
                h("div", {class: "btn-row"},
                    h("button", {class: "btn primary", onclick: async () => {
                            try {
                                await api("PUT", `/api/users/${u.id}`, {password: pass.value});
                                dialog.remove();
                                M.toast("Passwort gesetzt");
                            } catch (e) {
                                error.textContent = e.message;
                            }
                        }}, "Passwort setzen"),
                    h("button", {class: "btn", onclick: () => dialog.remove()}, "Abbrechen"))));
        document.body.append(dialog);
        pass.focus();
    }

    // ------------------------------------------------------------------ about

    /** Product, slogan and the versions of server and web client, for every user. */
    async function about(c) {
        const st = await api("GET", "/api/status");
        const kv = (a, b) => [h("dt", null, a), h("dd", null, b)];
        c.append(h("h1", null, "Über FreeX"),
            h("section", {class: "panel about"},
                h("div", {class: "about-brand"}, h("img", {src: "/logo.svg", alt: ""}),
                    h("div", null, h("div", {class: "about-name"}, "Free", h("b", null, "X")), h("div", {class: "slogan"}, M.SLOGAN)))),
            panel("Versionen",
                h("dl", {class: "kv"},
                    kv(st.server ? st.server.name : "FreeX Media Server", st.server ? st.server.version : st.version),
                    kv(`${st.web ? st.web.name : "FreeX Web"} (Weboberfläche)`, st.web ? st.web.version : ""),
                    kv("Servername", st.name))),
            panel("Quellcode", h("p", null, h("a", {href: "https://github.com/LinzN/freex-media-server", target: "_blank", rel: "noopener"},
                "github.com/LinzN/freex-media-server"))));
    }

    // ------------------------------------------------------------------ server settings

    /** Sections of the settings form: [key, label, hint]. Validation happens on the server. */
    const SETTINGS = [
        ["Allgemein", [
            ["server.name", "Servername", "Erscheint in der Weboberfläche und in der Authenticator-App."]]],
        ["Netzwerk", [
            ["server.bindAddress", "Adresse", "0.0.0.0 = alle Netzwerkschnittstellen, 127.0.0.1 = nur dieser Rechner (z. B. hinter einem Reverse-Proxy)."],
            ["server.port", "Port"],
            ["server.trustProxy", "Läuft hinter meinem eigenen Reverse-Proxy",
                "Dann gelten X-Forwarded-For/-Proto/-Host für Client-IP und HTTPS. Nur einschalten, wenn der Server ausschließlich über den Proxy erreichbar ist, sonst lassen sich IP-Adressen fälschen."]]],
        ["HTTPS", [
            ["server.tlsKeystore", "Keystore (PKCS#12)", "Pfad zur .p12/.pfx-Datei mit Zertifikat und Schlüssel, relativ zur config.yml. Leer = unverschlüsseltes HTTP."],
            ["server.tlsKeystorePassword", "Keystore-Passwort"]]],
        ["Anmeldung", [
            ["security.sessionDays", "Anmeldung bleibt gültig (Tage ohne Nutzung)"],
            ["security.minPasswordLength", "Mindestlänge für neue Passwörter"]]],
        ["Metadaten", [
            ["tmdb.apiKey", "TMDB-Schlüssel", "API-Schlüssel (v3) oder Lesezugriffs-Token (v4) von themoviedb.org. Ohne Schlüssel werden nur lokale .nfo-Dateien und Bilder genutzt."],
            ["tmdb.language", "Sprache", "Für Titel und Beschreibungen, z. B. de-DE"],
            ["tmdb.region", "Land für Altersfreigaben", "z. B. DE"],
            ["markers.enabled", "Intro und Abspann von Serienfolgen erkennen"]]],
        ["Umwandlung", [
            ["transcoder.enabled", "Dateien umwandeln, die der Browser nicht direkt abspielen kann"],
            ["transcoder.maxSessions", "Gleichzeitige Umwandlungen"],
            ["transcoder.hardwareAcceleration", "Hardware-Beschleunigung"],
            ["transcoder.vaapiDevice", "VAAPI-Gerät", "Nur für VAAPI und Quick Sync unter Linux."],
            ["transcoder.preset", "x264-Voreinstellung", "Schneller braucht weniger CPU, die Datenmenge wird etwas größer."],
            ["transcoder.directory", "Umwandlungsordner",
                "Hier liegen die Videostücke laufender Umwandlungen, z. B. auf einer schnellen SSD. Relativ zum Cache-Ordner oder ein absoluter Pfad. Der Server löscht dort nur seine eigenen Sitzungsordner."]]],
        ["Bibliotheks-Scan", [
            ["scanner.intervalMinutes", "Automatisch scannen alle … Minuten", "0 = nur von Hand"],
            ["scanner.threads", "Dateien gleichzeitig untersuchen"]]],
    ];
    const READONLY = [
        ["data.directory", "Datenordner"],
        ["data.cacheDirectory", "Cache-Ordner"],
        ["ffmpeg.ffprobe", "ffprobe"],
        ["ffmpeg.ffmpeg", "ffmpeg"],
    ];
    const CHOICE_LABELS = {
        none: "Keine (Software)", nvenc: "NVIDIA (NVENC)", qsv: "Intel Quick Sync", vaapi: "VAAPI (Intel/AMD unter Linux)",
        ultrafast: "ultrafast", superfast: "superfast", veryfast: "veryfast (empfohlen)", faster: "faster", fast: "fast", medium: "medium",
    };

    async function server(c) {
        const [st, cfg] = await Promise.all([api("GET", "/api/status"), api("GET", "/api/admin/settings")]);
        S.status = st;
        const row = (label, value, ok) => h("div", {class: "list-row"}, h("div", {class: "grow"}, h("div", {class: "name"}, label)),
            h("span", {class: ok === false ? "warn" : "muted"}, value));
        const restartBtn = confirmButton("Server neu starten", "Wirklich neu starten?", () => restartServer(cfg, c));
        restartBtn.className = "btn";
        // Node.append() would print null as text, so the optional parts go through h()
        c.append(h("div", null, h("h1", null, "Server"),
            cfg.pendingRestart ? restartPanel(cfg) : null,
            h("div", {class: "list"},
                row("Name", st.name),
                row("FreeX Media Server", st.server ? st.server.version : st.version),
                row("FreeX Web", st.web ? st.web.version : ""),
                row("Verbindung", st.tls ? "HTTPS" : "HTTP (unverschlüsselt)", st.tls ? true : false),
                row("ffprobe (Medieninfos)", st.ffprobe ? "gefunden" : "nicht gefunden", st.ffprobe),
                row("ffmpeg (Vorschaubilder, Umwandlung)", st.ffmpeg ? "gefunden" : "nicht gefunden", st.ffmpeg),
                row("Umwandlung", st.transcoder ? `aktiv, ${st.activeTranscodes} laufend` : "aus", st.transcoder)),
            st.tls ? null : h("p", {class: "hint"}, "Für Zugriff aus dem Internet: HTTPS unten einschalten oder einen Reverse-Proxy mit TLS davor setzen."),
            h("div", {class: "btn-row"},
                h("button", {class: "btn", onclick: async () => {
                        await api("POST", "/api/scan");
                        M.toast("Alle Bibliotheken werden gescannt");
                        M.refreshLibraries();
                    }}, "Alle Bibliotheken scannen"),
                restartBtn),
            settingsForm(cfg, c)));
    }

    function restartPanel(cfg) {
        return h("section", {class: "panel attention"},
            h("h2", null, "Neustart nötig"),
            h("p", null, "Gespeicherte Änderungen gelten erst nach einem Neustart des Servers. Laufende Wiedergaben werden dabei kurz unterbrochen."),
            h("button", {class: "btn primary", onclick: (e) => restartServer(cfg, e.target.closest(".settings-content"))}, "Jetzt neu starten"));
    }

    /** Where the web interface is reachable after the restart: a new port/protocol only matters without a proxy. */
    function originAfterRestart(cfg) {
        const here = Number(location.port || (location.protocol === "https:" ? 443 : 80));
        if (here !== cfg.running.port) return location.origin; // behind a proxy
        const tls = !!cfg.values["server.tlsKeystore"];
        const port = cfg.values["server.port"];
        if (port === cfg.running.port && tls === cfg.running.tls) return location.origin;
        return `${tls ? "https" : "http"}://${location.hostname}:${port}`;
    }

    async function restartServer(cfg, c) {
        const target = originAfterRestart(cfg);
        const same = target === location.origin;
        const status = h("p", {class: "status-line"}, "Der Server startet neu …");
        c.replaceChildren(h("h1", null, "Server"), h("section", {class: "panel attention"}, h("h2", null, "Neustart"), status));
        try {
            await api("POST", "/api/admin/restart");
        } catch (e) {
            status.textContent = e.message;
            return;
        }
        const probe = () => fetch(target + "/api/status", same ? {cache: "no-store"} : {cache: "no-store", mode: "no-cors"})
            .then((r) => same ? r.ok : true, () => false);
        const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
        // first wait for the old instance to go away, then for the new one
        for (let i = 0; i < 10 && await probe(); i++) await sleep(500);
        for (let i = 0; i < 60; i++) {
            await sleep(1000);
            if (await probe()) {
                if (same) {
                    M.toast("Server neu gestartet");
                    M.navigate("/settings/server", true);
                    M.render();
                } else {
                    location.href = target + "/settings/server";
                }
                return;
            }
        }
        status.replaceChildren("Der Server antwortet noch nicht. Startet er nicht, steht der Grund im Server-Log. ",
            "Wenn die neue Konfiguration nicht startet, stellt der Server die vorherige automatisch wieder her. ",
            same ? null : h("a", {href: target}, "Unter " + target + " öffnen"));
    }

    function settingsForm(cfg, c) {
        const types = Object.fromEntries(cfg.fields.map((f) => [f.key, f]));
        const inputs = {};
        const disabled = !cfg.writable;

        function control(key, label) {
            const f = types[key];
            const id = "set-" + key.replace(/\./g, "-");
            const v = cfg.values[key];
            if (f.type === "bool") {
                const input = h("input", {type: "checkbox", id, disabled});
                input.checked = !!v;
                inputs[key] = () => input.checked;
                return [input, label];
            }
            if (f.type === "choice") {
                const sel = h("select", {id, disabled}, f.choices.map((ch) => h("option", {value: ch}, CHOICE_LABELS[ch] || ch)));
                sel.value = v;
                inputs[key] = () => sel.value;
                return sel;
            }
            if (f.type === "secret") {
                const set = cfg.secrets[key];
                const input = h("input", {id, type: "password", autocomplete: "off", disabled,
                    placeholder: set ? "gesetzt, leer lassen zum Beibehalten" : "nicht gesetzt"});
                const clear = h("input", {type: "checkbox", disabled});
                inputs[key] = () => clear.checked ? "" : (input.value ? input.value : null);
                return set ? [input, h("label", {class: "check"}, clear, "Entfernen")] : input;
            }
            const input = h("input", {id, disabled, type: f.type === "number" ? "number" : "text",
                min: f.min, max: f.max, value: v == null ? "" : String(v), spellcheck: "false"});
            inputs[key] = () => f.type === "number" ? (input.value === "" ? input.value : Number(input.value)) : input.value;
            return input;
        }

        const sections = SETTINGS.map(([title, rows]) => panel(title, rows.map(([key, label, hint]) => {
            const id = "set-" + key.replace(/\./g, "-");
            if (types[key].type === "bool") {
                return [h("label", {class: "check"}, control(key, label)), hint ? h("div", {class: "hint field-hint"}, hint) : null];
            }
            const resolved = cfg.paths[key] ? ` Aktuell: ${cfg.paths[key]}` : "";
            return field(id, label, control(key, label), hint ? hint + resolved : null);
        })));

        const paths = panel("Speicherorte und Programme",
            h("p", {class: "muted"}, "Diese Pfade lassen sich nur direkt in der config.yml ändern: Über ffmpeg/ffprobe startet der Server Programme, in den Ordnern liegt sein Zustand. Eine gestohlene Admin-Anmeldung soll daran nichts ändern können."),
            h("dl", {class: "kv"}, READONLY.map(([key, label]) => [h("dt", null, label),
                h("dd", {class: "mono"}, cfg.paths[key] || cfg.values[key] || "")]),
                h("dt", null, "Konfigurationsdatei"), h("dd", {class: "mono"}, cfg.file)));

        const pw = passwordPrompt("set-pw");
        const error = h("div", {class: "error-text", role: "alert"});
        const save = h("button", {class: "btn primary", disabled, onclick: async () => {
                error.textContent = "";
                const values = {};
                for (const [key, get] of Object.entries(inputs)) {
                    const v = get();
                    if (types[key].type === "secret") {
                        if (v !== null) values[key] = v;
                    } else if (v !== cfg.values[key]) {
                        values[key] = v;
                    }
                }
                if (!Object.keys(values).length) return M.toast("Keine Änderungen");
                save.disabled = true;
                try {
                    const next = await api("PUT", "/api/admin/settings", {currentPassword: pw.value, values});
                    M.toast("Einstellungen gespeichert");
                    c.replaceChildren();
                    await server(c);
                    if (next.pendingRestart) c.querySelector(".panel.attention")?.scrollIntoView({behavior: "smooth"});
                } catch (e) {
                    error.textContent = e.message;
                    save.disabled = false;
                }
            }}, "Speichern");

        return h("div", {class: "settings-form"},
            h("h1", null, "Einstellungen"),
            disabled ? h("section", {class: "panel attention"}, h("h2", null, "Nur lesen"),
                h("p", null, `Der Server darf ${cfg.file} nicht ändern. Passe die Datei direkt an oder gib dem Server-Benutzer Schreibrechte auf die Datei und ihren Ordner.`)) : null,
            sections, paths,
            panel("Speichern",
                h("p", {class: "muted"}, "Die Änderungen werden in die config.yml geschrieben und gelten nach einem Neustart."),
                field("set-pw", "Zur Bestätigung: dein aktuelles Passwort", pw),
                error,
                save));
    }
})();

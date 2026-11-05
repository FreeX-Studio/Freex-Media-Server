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
    const {S, t, tn, h, link, icon, api} = M;

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
        authPage(t("Sign in"), null, [
            h("div", {class: "field"}, h("label", {for: "l-user"}, t("Username")), user),
            h("div", {class: "field"}, h("label", {for: "l-pass"}, t("Password")), pass),
        ], t("Sign in"), async () => {
            const res = await api("POST", "/api/login", {username: user.value, password: pass.value});
            if (res.twoFactorRequired) return secondFactor(res.challenge);
            afterLogin(res);
        }, h("p", {class: "auth-foot"}, t("On {0}", S.status.name),
            M.desktop ? [" · ", h("button", {type: "button", class: "link-btn", onclick: M.desktop.switchServer}, t("Switch server"))] : null));
    }, {auth: false, shell: false});

    function secondFactor(challenge) {
        let recovery = false;
        const code = h("input", {id: "l-code", inputmode: "numeric", autocomplete: "one-time-code", required: true,
            pattern: "[0-9 ]{6,7}", maxlength: 7, class: "code-input", "aria-describedby": "code-help"});
        const help = h("p", {id: "code-help", class: "muted small"}, t("Enter the 6-digit code from your authenticator app."));
        const toggle = h("button", {type: "button", class: "link-btn", onclick: () => {
                recovery = !recovery;
                code.value = "";
                code.removeAttribute("pattern");
                if (recovery) {
                    code.setAttribute("inputmode", "text");
                    code.setAttribute("maxlength", 11);
                    code.classList.remove("code-input");
                    help.textContent = t("Enter one of your recovery codes (e.g. abcde-12345). Each code works only once.");
                    toggle.textContent = t("Use an authenticator code instead");
                } else {
                    code.setAttribute("inputmode", "numeric");
                    code.setAttribute("maxlength", 7);
                    code.setAttribute("pattern", "[0-9 ]{6,7}");
                    code.classList.add("code-input");
                    help.textContent = t("Enter the 6-digit code from your authenticator app.");
                    toggle.textContent = t("Use a recovery code");
                }
                code.focus();
            }}, t("Use a recovery code"));
        authPage(t("Verification"), null, [h("div", {class: "field"}, h("label", {for: "l-code"}, t("Verification code")), code), help],
            t("Verify"), async (error) => {
                try {
                    afterLogin(await api("POST", "/api/login/verify", {challenge, code: code.value}));
                } catch (e) {
                    if (e.code === "error.challengeExpired" || e.code === "error.tooManyCodes") {
                        M.toast(e.message, true);
                        return M.navigate("/login", true);
                    }
                    code.select();
                    throw e;
                }
            }, h("div", {class: "auth-foot"}, toggle, h("br"), h("a", {href: "/login", "data-link": true}, t("Cancel"))));
    }

    M.route("/setup", () => {
        if (!S.status.setupRequired) return M.navigate("/", true);
        const user = h("input", {id: "s-user", autocomplete: "username", required: true});
        const pass = h("input", {id: "s-pass", type: "password", autocomplete: "new-password", required: true, minlength: 10});
        const pass2 = h("input", {id: "s-pass2", type: "password", autocomplete: "new-password", required: true});
        authPage(t("Set up the server"), t("Create the administrator account. Then add your movie and TV show folders."), [
            h("div", {class: "field"}, h("label", {for: "s-user"}, t("Username")), user),
            h("div", {class: "field"}, h("label", {for: "s-pass"}, t("Password (at least 10 characters)")), pass),
            h("div", {class: "field"}, h("label", {for: "s-pass2"}, t("Repeat password")), pass2),
        ], t("Create account"), async () => {
            if (pass.value !== pass2.value) throw new Error(t("The passwords do not match"));
            afterLogin(await api("POST", "/api/setup", {username: user.value, password: pass.value}), "/settings/libraries");
        });
    }, {auth: false, shell: false});

    // ------------------------------------------------------------------ settings frame

    const TABS = [
        ["account", "Account", false],
        ["security", "Security", false],
        ["libraries", "Libraries", true],
        ["users", "Users", true],
        ["server", "Server", true],
        // Only inside the Windows app (see desktop.js).
        ...(M.desktop ? [["desktop", "FreeX Desktop", false]] : []),
        ["about", "About FreeX", false],
    ];

    M.route("/settings", (main) => M.navigate("/settings/" + (S.user.admin ? "libraries" : "account"), true));
    M.route("/settings/:tab", async (main, p) => {
        let tab = p.tab;
        if (!TABS.some(([id, , admin]) => id === tab && (!admin || S.user.admin))) tab = "account";
        const nav = h("nav", {class: "settings-nav", "aria-label": t("Settings")},
            h("div", {class: "nav-group"}, t("My account")),
            TABS.filter(([, , admin]) => !admin).map(([id, label]) => link(`/settings/${id}`, {class: "nav-link" + (id === tab ? " active" : "")}, t(label))),
            S.user.admin ? [h("div", {class: "nav-group"}, "Server"),
                TABS.filter(([, , admin]) => admin).map(([id, label]) => link(`/settings/${id}`, {class: "nav-link" + (id === tab ? " active" : "")}, t(label)))] : null);
        const content = h("div", {class: "settings-content"});
        main.replaceChildren(h("div", {class: "page settings"}, nav, content));
        await {account, security, libraries, users, server, about, desktop: M.desktop && M.desktop.settingsPage}[tab](content);
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
        c.append(h("h1", null, t("Account")),
            panel(t("Profile"),
                h("dl", {class: "kv"}, h("dt", null, t("Username")), h("dd", null, S.user.username),
                    h("dt", null, t("Role")), h("dd", null, S.user.admin ? t("Administrator") : t("User")),
                    h("dt", null, t("Password changed")), h("dd", null, M.fmtDate(S.user.passwordChangedAt)))),
            avatarPanel(),
            languagePanel(),
            playbackPanel(),
            panel(t("Change password"),
                field("p-cur", t("Current password"), cur),
                field("p-new", t("New password"), next, t("At least 10 characters. All other devices are signed out after the change.")),
                field("p-new2", t("Repeat new password"), next2),
                error,
                h("button", {class: "btn primary", onclick: async () => {
                        error.textContent = "";
                        if (next.value !== next2.value) return error.textContent = t("The new passwords do not match");
                        try {
                            const r = await api("PUT", "/api/me/password", {currentPassword: cur.value, password: next.value});
                            cur.value = next.value = next2.value = "";
                            S.user = await api("GET", "/api/me");
                            M.toast(r.endedSessions ? tn(r.endedSessions, "Password changed, 1 other device signed out", "Password changed, {0} other devices signed out") : t("Password changed"));
                        } catch (e) {
                            error.textContent = e.message;
                        }
                    }}, t("Change password"))),
            panel(t("Sign out"), h("p", {class: "muted"}, t("Signs out this device.")), h("button", {class: "btn", onclick: M.logout}, t("Sign out"))));
    }

    function avatarPanel() {
        const box = panel(t("Profile picture"));
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
            if (f.size > 10 * 1024 * 1024) return error.textContent = t("The image is larger than 10 MB");
            try {
                // Sent as is; the server crops and scales it and keeps only its own copy.
                const res = await fetch("/api/me/avatar", {method: "PUT", credentials: "same-origin", body: f,
                    headers: {"X-FreeX-Token": S.token, "Content-Type": f.type || "application/octet-stream"}});
                const data = await res.json().catch(() => null);
                if (!res.ok) throw new Error((data && data.error) || t("Error {0}", res.status));
                M.toast(t("Profile picture saved"));
                await changed(data);
            } catch (e) {
                error.textContent = e.message;
            }
        });
        box.append(h("div", {class: "avatar-edit"},
                M.avatar(S.user, "large"),
                h("div", null,
                    h("p", {class: "muted"}, t("JPEG, PNG or GIF up to 10 MB. The image is cropped to a square.")),
                    h("div", {class: "btn-row"},
                        h("button", {class: "btn", onclick: () => file.click()}, S.user.avatar ? t("Choose another image") : t("Upload image")),
                        S.user.avatar ? h("button", {class: "btn", onclick: async () => {
                                await changed(await api("DELETE", "/api/me/avatar"));
                                M.toast(t("Profile picture removed"));
                            }}, t("Remove")) : null))),
            file, error);
        return box;
    }

    /** Display language of the interface and of messages from the server. */
    function languagePanel() {
        const current = (S.user.preferences || {}).uiLanguage || "";
        const sel = h("select", {id: "pref-ui"}, h("option", {value: ""}, t("Automatic (browser language)")),
            Object.entries(FreeXI18n.LANGUAGES).map(([code, name]) => h("option", {value: code}, name)));
        sel.value = current;
        return panel(t("Language"),
            field("pref-ui", t("Display language"), sel, t("Language of the interface and of messages from the server.")),
            h("button", {class: "btn primary", onclick: async () => {
                    S.user = await api("PUT", "/api/me/preferences", {uiLanguage: sel.value});
                    FreeXI18n.setLanguage(sel.value);
                    location.reload();
                }}, t("Save")));
    }

    function playbackPanel() {
        const p = S.user.preferences || {};
        const langOptions = (empty) => [h("option", {value: ""}, empty)].concat(M.PREF_LANGS
            .map((code) => [code, M.language(code)])
            .sort((a, b) => a[1].localeCompare(b[1], M.locale()))
            .map(([code, name]) => h("option", {value: code}, name)));
        const audio = h("select", {id: "pref-audio"}, langOptions(t("Default track of the file")));
        audio.value = p.audioLanguage || "";
        const mode = h("select", {id: "pref-submode"},
            h("option", {value: "off"}, t("Off")),
            h("option", {value: "forced"}, t("Only forced subtitles (foreign-language parts)")),
            h("option", {value: "foreign"}, t("When the audio is in another language")),
            h("option", {value: "always"}, t("Always")));
        mode.value = p.subtitleMode || "forced";
        const subLang = h("select", {id: "pref-sublang"}, langOptions(t("Same as the audio language")));
        subLang.value = p.subtitleLanguage || "";
        const subLangField = field("pref-sublang", t("Subtitle language"), subLang);
        const sync = () => subLangField.hidden = mode.value === "off";
        mode.addEventListener("change", sync);
        sync();
        return panel(t("Playback"),
            h("p", {class: "muted"}, t("Applies to all titles unless you choose something else on the detail page or in the player.")),
            field("pref-audio", t("Preferred audio language"), audio, t("If a file does not have this language, its default track is played.")),
            field("pref-submode", t("Switch on subtitles"), mode),
            subLangField,
            h("button", {class: "btn primary", onclick: async () => {
                    S.user = await api("PUT", "/api/me/preferences",
                        {audioLanguage: audio.value, subtitleMode: mode.value, subtitleLanguage: subLang.value});
                    M.toast(t("Playback settings saved"));
                }}, t("Save")));
    }

    // ------------------------------------------------------------------ security

    async function security(c) {
        const [me, sessions, logins] = await Promise.all([api("GET", "/api/me"), api("GET", "/api/me/sessions"), api("GET", "/api/me/logins")]);
        S.user = me;
        c.append(h("h1", null, t("Security")), twoFactorPanel(me), sessionsPanel(sessions), loginsPanel(logins));
    }

    function passwordPrompt(id) {
        return h("input", {id, type: "password", autocomplete: "current-password", placeholder: t("Current password")});
    }

    function twoFactorPanel(me) {
        const box = panel(t("Two-factor sign-in"));
        if (me.twoFactor) {
            const pw = passwordPrompt("tf-pw");
            const error = h("div", {class: "error-text", role: "alert"});
            box.append(
                h("p", {class: "status-line ok"}, icon("shield"), t("On. Signing in also asks for a code from your authenticator app.")),
                h("p", {class: "muted"}, tn(me.recoveryCodesLeft, "1 recovery code left.", "{0} recovery codes left.")),
                field("tf-pw", t("To confirm: current password"), pw),
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
                        }}, t("New recovery codes")),
                    h("button", {class: "btn danger", onclick: async () => {
                            error.textContent = "";
                            try {
                                await api("POST", "/api/me/2fa/disable", {currentPassword: pw.value});
                                M.toast(t("Two-factor sign-in switched off"));
                                M.render();
                            } catch (e) {
                                error.textContent = e.message;
                            }
                        }}, t("Switch off"))));
            return box;
        }
        const pw = passwordPrompt("tf-pw");
        const error = h("div", {class: "error-text", role: "alert"});
        box.append(
            h("p", {class: "status-line"}, t("Off. Protect your account with an additional code from an authenticator app such as Aegis, 2FAS, Google or Microsoft Authenticator.")),
            field("tf-pw", t("To confirm: current password"), pw),
            error,
            h("button", {class: "btn primary", onclick: async () => {
                    error.textContent = "";
                    try {
                        const r = await api("POST", "/api/me/2fa/setup", {currentPassword: pw.value});
                        twoFactorSetup(box, r);
                    } catch (e) {
                        error.textContent = e.message;
                    }
                }}, t("Set up")));
        return box;
    }

    function twoFactorSetup(box, setup) {
        let qrImg = null;
        try {
            const qr = window.qrcode(0, "M");
            qr.addData(setup.uri);
            qr.make();
            qrImg = h("img", {class: "qr", src: qr.createDataURL(5, 12), alt: t("QR code for the authenticator app"), width: 220, height: 220});
        } catch (e) {
            qrImg = null;
        }
        const code = h("input", {id: "tf-code", inputmode: "numeric", autocomplete: "one-time-code", maxlength: 7, class: "code-input"});
        const error = h("div", {class: "error-text", role: "alert"});
        const secret = setup.secret.replace(/(.{4})/g, "$1 ").trim();
        box.replaceChildren(h("h2", null, t("Set up two-factor sign-in")),
            h("ol", {class: "steps"},
                h("li", null, t("Scan the QR code with your authenticator app.")),
                h("li", null, M.tParts("Or enter the key by hand: {0}", h("code", {class: "secret"}, secret))),
                h("li", null, t("To confirm, enter the 6-digit code the app shows."))),
            qrImg,
            field("tf-code", t("Code from the app"), code),
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
                    }}, t("Switch on")),
                h("button", {class: "btn", onclick: () => M.render()}, t("Cancel"))));
        code.focus();
    }

    function showRecoveryCodes(box, codes, justEnabled) {
        const text = codes.join("\n");
        box.replaceChildren(h("h2", null, justEnabled ? t("Two-factor sign-in is on") : t("New recovery codes")),
            h("p", null, t("Keep these codes safe, for example printed or in a password manager. Each code lets you sign in once if you do not have your phone. They are only shown now.")),
            justEnabled ? h("p", {class: "muted"}, t("All other devices were signed out.")) : null,
            h("pre", {class: "codes"}, text),
            h("div", {class: "btn-row"},
                h("button", {class: "btn", onclick: async () => {
                        try {
                            await navigator.clipboard.writeText(text);
                            M.toast(t("Codes copied"));
                        } catch (e) {
                            M.toast(t("Copying is not possible, please select and copy"), true);
                        }
                    }}, t("Copy")),
                h("button", {class: "btn primary", onclick: () => M.render()}, t("I have saved the codes"))));
    }

    function sessionsPanel(sessions) {
        const box = panel(t("Signed-in devices"),
            h("div", {class: "list"}, sessions.map((s) => h("div", {class: "list-row"},
                h("div", {class: "grow"},
                    h("div", {class: "name"}, M.deviceName(s.userAgent), s.current ? h("span", {class: "pill"}, t("This device")) : null),
                    h("div", {class: "detail-line"}, t("{0}, last active {1}, signed in {2}", s.ip || "?", M.fmtDate(s.lastSeen), M.fmtDate(s.createdAt)))),
                s.current ? null : confirmButton(t("Sign out"), t("Really sign out?"), async () => {
                    await api("DELETE", `/api/me/sessions/${s.id}`);
                    M.toast(t("Device signed out"));
                    M.render();
                })))));
        if (sessions.length > 1) {
            box.append(h("div", {class: "btn-row"}, confirmButton(t("Sign out all other devices"), t("Really sign out all others?"), async () => {
                const r = await api("POST", "/api/me/sessions/logout-others");
                M.toast(tn(r.ended, "1 device signed out", "{0} devices signed out"));
                M.render();
            })));
        }
        return box;
    }

    function loginsPanel(logins) {
        return panel(t("Recent sign-ins"),
            logins.length ? h("div", {class: "list"}, logins.map((e) => h("div", {class: "list-row"},
                h("span", {class: "dot " + (e.success ? "ok" : "bad"), title: e.success ? t("Successful") : t("Failed")}),
                h("div", {class: "grow"},
                    h("div", {class: "name"}, e.success ? t("Signed in") : t("Failed attempt"), h("span", {class: "muted"}, `  ${e.detail}`)),
                    h("div", {class: "detail-line"}, `${M.fmtDate(e.time)}, ${e.ip}, ${M.deviceName(e.userAgent)}`)))))
                : h("p", {class: "muted"}, t("No entries yet.")),
            h("p", {class: "hint"}, t("Failed attempts with your username appear here. If you do not recognise them, change your password and switch on two-factor sign-in.")));
    }

    // ------------------------------------------------------------------ libraries

    async function libraries(c) {
        S.libraries = await api("GET", "/api/libraries");
        M.renderSidebar();
        const list = S.libraries.length ? h("div", {class: "list"}, S.libraries.map((l) => h("div", {class: "list-row"},
            icon(l.type === "shows" ? "shows" : "movies", "muted-ico"),
            h("div", {class: "grow"},
                h("div", {class: "name"}, l.name),
                h("div", {class: "detail-line"}, [l.type === "shows" ? t("Shows") : t("Movies"), tn(l.itemCount, "1 entry", "{0} entries"),
                    t("last scanned {0}", M.fmtDate(l.lastScan))].join(", ")),
                l.paths.map((p) => h("div", {class: "detail-line mono"}, p))),
            l.scanning ? h("span", {class: "spin", title: t("Scanning")}) : h("button", {class: "btn small", onclick: async () => {
                    await api("POST", `/api/libraries/${l.id}/scan`);
                    M.toast(t("Scanning “{0}”", l.name));
                    await M.refreshLibraries();
                    M.render();
                }}, t("Scan now")),
            confirmButton(t("Remove"), t("Really remove?"), async () => {
                await api("DELETE", `/api/libraries/${l.id}`);
                M.toast(t("“{0}” removed", l.name));
                await M.refreshLibraries();
                M.render();
            })))) : h("p", {class: "muted"}, t("There are no libraries yet."));
        c.append(h("h1", null, t("Libraries")), list, libraryForm());
        if (S.libraries.some((l) => l.scanning)) M.refreshLibraries();
    }

    function libraryForm() {
        const paths = [];
        const name = h("input", {id: "lib-name", placeholder: t("e.g. Movies"), maxlength: 64});
        const type = h("select", {id: "lib-type"}, h("option", {value: "movies"}, t("Movies")), h("option", {value: "shows"}, t("Shows")));
        const pathList = h("div", {class: "path-list"});
        const error = h("div", {class: "error-text", role: "alert"});
        const manual = h("input", {placeholder: t("/path/to/folder"), "aria-label": t("Folder path")});

        const drawPaths = () => pathList.replaceChildren(...paths.map((p, i) => h("div", {class: "path-item"}, h("span", {class: "mono"}, p),
            h("button", {class: "btn small", onclick: () => {
                    paths.splice(i, 1);
                    drawPaths();
                }}, t("Remove")))));
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
                        res.parent != null ? h("button", {class: "btn small icon", "aria-label": t("One level up"), onclick: () => browse(res.parent)}, icon("back")) : null,
                        h("span", {class: "mono"}, res.path || t("Locations")),
                        res.path ? h("button", {class: "btn small primary", onclick: () => addPath(res.path)}, t("Add")) : null),
                    h("div", {class: "folders"}, res.folders.length ? res.folders.map((f) => h("button", {onclick: () => browse(f.path)}, f.name))
                        : h("div", {class: "muted pad"}, t("No subfolders"))));
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
                    M.toast(t("Library created, scan running"));
                    await M.refreshLibraries();
                    M.render();
                } catch (e) {
                    error.textContent = e.message;
                } finally {
                    submit.disabled = false;
                }
            }}, t("Create library"));

        return panel(t("Add library"),
            field("lib-name", t("Name"), name),
            field("lib-type", t("Content"), type),
            h("div", {class: "field"}, h("label", null, t("Folders")), pathList, picker,
                h("div", {class: "inline"}, manual, h("button", {class: "btn", onclick: () => {
                        addPath(manual.value);
                        manual.value = "";
                    }}, t("Add")))),
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
            libraries.length ? null : h("p", {class: "muted"}, t("No libraries created yet.")));
        const sync = () => {
            list.classList.toggle("disabled", all.checked);
            boxes.forEach((b) => b.cb.disabled = all.checked);
        };
        (current === "all" || current == null ? all : some).checked = true;
        all.addEventListener("change", sync);
        some.addEventListener("change", sync);
        sync();
        const el = h("div", null,
            h("label", {class: "check"}, all, t("All libraries, including ones added later")),
            h("label", {class: "check"}, some, t("Only selected libraries:")),
            list);
        return {el, value: () => all.checked ? "all" : boxes.filter((b) => b.cb.checked).map((b) => b.cb.value)};
    }

    function librarySummary(u, libraries) {
        if (u.admin) return t("all libraries (administrator)");
        if (u.libraries === "all") return t("all libraries");
        const names = libraries.filter((l) => u.libraries.includes(l.id)).map((l) => l.name);
        return names.length ? names.join(", ") : t("no library shared");
    }

    function editLibraries(u, libraries) {
        const picker = libraryPicker(libraries, u.libraries, "el-" + u.id);
        const error = h("div", {class: "error-text", role: "alert"});
        const dialog = h("div", {class: "modal-back", onclick: (e) => e.target === dialog && dialog.remove()},
            h("div", {class: "modal", role: "dialog", "aria-modal": "true", "aria-label": t("Share libraries")},
                h("h2", null, t("Libraries for {0}", u.username)),
                h("p", {class: "muted"}, t("Only shared libraries appear for this user, also in search, on the home screen and in Watch Together.")),
                picker.el,
                error,
                h("div", {class: "btn-row"},
                    h("button", {class: "btn primary", onclick: async () => {
                            try {
                                await api("PUT", `/api/users/${u.id}`, {libraries: picker.value()});
                                dialog.remove();
                                M.toast(t("Sharing for {0} saved", u.username));
                                M.render();
                            } catch (e) {
                                error.textContent = e.message;
                            }
                        }}, t("Save")),
                    h("button", {class: "btn", onclick: () => dialog.remove()}, t("Cancel")))));
        document.body.append(dialog);
    }

    async function users(c) {
        const [list, libraries] = await Promise.all([api("GET", "/api/users"), api("GET", "/api/libraries")]);
        const rows = h("div", {class: "list"}, list.map((u) => {
            const row = h("div", {class: "list-row"},
                M.avatar(u),
                h("div", {class: "grow"},
                    h("div", {class: "name"}, u.username, u.id === S.user.id ? h("span", {class: "pill"}, t("You")) : null),
                    h("div", {class: "detail-line"}, [u.admin ? t("Administrator") : t("User"), u.twoFactor ? t("two-factor on") : t("without two-factor")].join(", ")),
                    h("div", {class: "detail-line"}, t("Sees: {0}", librarySummary(u, libraries)))),
                h("button", {class: "btn small", "data-menu": true, onclick: (e) => M.popup(e.currentTarget, [
                        u.admin ? undefined : {label: t("Share libraries"), onclick: () => editLibraries(u, libraries)},
                        {label: t("Reset password"), onclick: () => resetPassword(u)},
                        u.twoFactor ? {label: t("Reset two-factor"), onclick: async () => {
                                await api("PUT", `/api/users/${u.id}`, {resetTwoFactor: true});
                                M.toast(t("Two-factor for {0} reset, devices signed out", u.username));
                                M.render();
                            }} : undefined,
                        u.id !== S.user.id ? {label: u.admin ? t("Make user") : t("Make administrator"), onclick: async () => {
                                try {
                                    await api("PUT", `/api/users/${u.id}`, {admin: !u.admin});
                                    M.render();
                                } catch (err) {
                                    M.toast(err.message, true);
                                }
                            }} : undefined,
                    ])}, t("Edit")),
                u.id === S.user.id ? null : confirmButton(t("Delete"), t("Really delete?"), async () => {
                    await api("DELETE", `/api/users/${u.id}`);
                    M.toast(t("{0} deleted", u.username));
                    M.render();
                }));
            return row;
        }));

        const name = h("input", {id: "u-name", autocomplete: "off"});
        const pass = h("input", {id: "u-pass", type: "password", autocomplete: "new-password"});
        const admin = h("input", {type: "checkbox"});
        const picker = libraryPicker(libraries, "all", "new-user");
        const pickerField = h("div", {class: "field"}, h("label", null, t("Shared libraries")), picker.el);
        admin.addEventListener("change", () => pickerField.hidden = admin.checked);
        const error = h("div", {class: "error-text", role: "alert"});
        c.append(h("h1", null, t("User")), rows, panel(t("Add user"),
            field("u-name", t("Username"), name),
            field("u-pass", t("Password"), pass, t("At least 10 characters. The user can change it afterwards.")),
            h("label", {class: "check"}, admin, t("Administrator: may manage libraries and users and sees everything")),
            pickerField,
            error,
            h("button", {class: "btn primary", onclick: async () => {
                    error.textContent = "";
                    try {
                        await api("POST", "/api/users", {username: name.value, password: pass.value, admin: admin.checked,
                            libraries: admin.checked ? "all" : picker.value()});
                        M.toast(t("{0} created", name.value.trim()));
                        M.render();
                    } catch (e) {
                        error.textContent = e.message;
                    }
                }}, t("Create user"))));
    }

    function resetPassword(u) {
        const pass = h("input", {id: "rp-pass", type: "password", autocomplete: "new-password"});
        const error = h("div", {class: "error-text", role: "alert"});
        const dialog = h("div", {class: "modal-back", onclick: (e) => e.target === dialog && dialog.remove()},
            h("div", {class: "modal", role: "dialog", "aria-modal": "true", "aria-label": t("Reset password")},
                h("h2", null, t("New password for {0}", u.username)),
                field("rp-pass", t("New password"), pass, t("All devices of this user are signed out.")),
                error,
                h("div", {class: "btn-row"},
                    h("button", {class: "btn primary", onclick: async () => {
                            try {
                                await api("PUT", `/api/users/${u.id}`, {password: pass.value});
                                dialog.remove();
                                M.toast(t("Password set"));
                            } catch (e) {
                                error.textContent = e.message;
                            }
                        }}, t("Set password")),
                    h("button", {class: "btn", onclick: () => dialog.remove()}, t("Cancel")))));
        document.body.append(dialog);
        pass.focus();
    }

    // ------------------------------------------------------------------ about

    /** Product, slogan and the versions of server and web client, for every user. */
    async function about(c) {
        const st = await api("GET", "/api/status");
        const app = M.desktop ? await M.desktop.info() : null;
        const kv = (a, b) => [h("dt", null, a), h("dd", null, b)];
        c.append(h("h1", null, t("About FreeX")),
            h("section", {class: "panel about"},
                h("div", {class: "about-brand"}, h("img", {src: "/logo.svg", alt: ""}),
                    h("div", null, h("div", {class: "about-name"}, "Free", h("b", null, "X")), h("div", {class: "slogan"}, M.SLOGAN)))),
            panel(t("Versions"),
                h("dl", {class: "kv"},
                    kv(st.server ? st.server.name : "FreeX Media Server", st.server ? st.server.version : st.version),
                    kv(`${st.web ? st.web.name : "FreeX Web"} (${t("web interface")})`, st.web ? st.web.version : ""),
                    app ? kv(`${app.app} (${t("Windows app")})`, app.version) : null,
                    kv(t("Server name"), st.name))),
            panel(t("Source code"), h("p", null, h("a", {href: "https://github.com/LinzN/freex-media-server", target: "_blank", rel: "noopener"},
                "github.com/LinzN/freex-media-server"))));
    }

    // ------------------------------------------------------------------ server settings

    /** Sections of the settings form: [key, label, hint]. Validation happens on the server. */
    const SETTINGS = [
        ["General", [
            ["server.name", "Server name", "Shown in the web interface and in the authenticator app."]]],
        ["Network", [
            ["server.bindAddress", "Address", "0.0.0.0 = all network interfaces, 127.0.0.1 = only this machine (e.g. behind a reverse proxy)."],
            ["server.port", "Port"],
            ["server.trustProxy", "Runs behind my own reverse proxy",
                "Then X-Forwarded-For/-Proto/-Host are used for the client IP and HTTPS. Only switch this on if the server can only be reached through the proxy, otherwise IP addresses can be faked."]]],
        ["HTTPS", [
            ["server.tlsKeystore", "Keystore (PKCS#12)", "Path to the .p12/.pfx file with certificate and key, relative to config.yml. Empty = unencrypted HTTP."],
            ["server.tlsKeystorePassword", "Keystore password"]]],
        ["Sign-in", [
            ["security.sessionDays", "Sign-in stays valid (days without use)"],
            ["security.minPasswordLength", "Minimum length of new passwords"]]],
        ["Metadata", [
            ["tmdb.apiKey", "TMDB key", "API key (v3) or read access token (v4) from themoviedb.org. Without a key only local .nfo files and images are used."],
            ["tmdb.language", "Language", "For titles and descriptions, e.g. en-US or de-DE"],
            ["tmdb.region", "Country for age ratings", "e.g. US or DE"],
            ["markers.enabled", "Detect intros and credits of episodes"]]],
        ["Transcoding", [
            ["transcoder.enabled", "Convert files the browser cannot play directly"],
            ["transcoder.maxSessions", "Simultaneous conversions"],
            ["transcoder.hardwareAcceleration", "Hardware acceleration"],
            ["transcoder.vaapiDevice", "VAAPI device", "Only for VAAPI and Quick Sync on Linux."],
            ["transcoder.preset", "x264 preset", "Faster needs less CPU, the amount of data grows a little."],
            ["transcoder.directory", "Transcoding folder",
                "Holds the video segments of running conversions, e.g. on a fast SSD. Relative to the cache folder or an absolute path. The server only deletes its own session folders there."]]],
        ["Library scan", [
            ["scanner.intervalMinutes", "Scan automatically every … minutes", "0 = only by hand"],
            ["scanner.threads", "Files examined at the same time"]]],
    ];
    const READONLY = [
        ["data.directory", "Data folder"],
        ["data.cacheDirectory", "Cache folder"],
        ["ffmpeg.ffprobe", "ffprobe"],
        ["ffmpeg.ffmpeg", "ffmpeg"],
    ];
    const CHOICE_LABELS = {
        none: "None (software)", nvenc: "NVIDIA (NVENC)", qsv: "Intel Quick Sync", vaapi: "VAAPI (Intel/AMD on Linux)",
        ultrafast: "ultrafast", superfast: "superfast", veryfast: "veryfast (recommended)", faster: "faster", fast: "fast", medium: "medium",
    };

    async function server(c) {
        const [st, cfg] = await Promise.all([api("GET", "/api/status"), api("GET", "/api/admin/settings")]);
        S.status = st;
        const row = (label, value, ok) => h("div", {class: "list-row"}, h("div", {class: "grow"}, h("div", {class: "name"}, label)),
            h("span", {class: ok === false ? "warn" : "muted"}, value));
        const restartBtn = confirmButton(t("Restart server"), t("Really restart?"), () => restartServer(cfg, c));
        restartBtn.className = "btn";
        // Node.append() would print null as text, so the optional parts go through h()
        c.append(h("div", null, h("h1", null, "Server"),
            cfg.pendingRestart ? restartPanel(cfg) : null,
            h("div", {class: "list"},
                row(t("Name"), st.name),
                row("FreeX Media Server", st.server ? st.server.version : st.version),
                row("FreeX Web", st.web ? st.web.version : ""),
                row(t("Connection"), st.tls ? "HTTPS" : t("HTTP (unencrypted)"), st.tls ? true : false),
                row(t("ffprobe (media information)"), st.ffprobe ? t("found") : t("not found"), st.ffprobe),
                row(t("ffmpeg (thumbnails, transcoding)"), st.ffmpeg ? t("found") : t("not found"), st.ffmpeg),
                row(t("Transcoding"), st.transcoder ? t("on, {0} running", st.activeTranscodes) : t("off"), st.transcoder)),
            st.tls ? null : h("p", {class: "hint"}, t("For access from the internet: switch on HTTPS below or put a reverse proxy with TLS in front.")),
            h("div", {class: "btn-row"},
                h("button", {class: "btn", onclick: async () => {
                        await api("POST", "/api/scan");
                        M.toast(t("All libraries are being scanned"));
                        M.refreshLibraries();
                    }}, t("Scan all libraries")),
                restartBtn),
            settingsForm(cfg, c)));
    }

    function restartPanel(cfg) {
        return h("section", {class: "panel attention"},
            h("h2", null, t("Restart needed")),
            h("p", null, t("Saved changes take effect after a restart of the server. Running playbacks are interrupted briefly.")),
            h("button", {class: "btn primary", onclick: (e) => restartServer(cfg, e.target.closest(".settings-content"))}, t("Restart now")));
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
        const status = h("p", {class: "status-line"}, t("The server is restarting …"));
        c.replaceChildren(h("h1", null, "Server"), h("section", {class: "panel attention"}, h("h2", null, t("Restart")), status));
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
                    M.toast(t("Server restarted"));
                    M.navigate("/settings/server", true);
                    M.render();
                } else {
                    location.href = target + "/settings/server";
                }
                return;
            }
        }
        status.replaceChildren(t("The server does not answer yet. If it does not start, the reason is in the server log. "),
            t("If the new configuration does not start, the server restores the previous one automatically. "),
            same ? null : h("a", {href: target}, t("Open at {0}", target)));
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
                const sel = h("select", {id, disabled}, f.choices.map((ch) => h("option", {value: ch}, CHOICE_LABELS[ch] ? t(CHOICE_LABELS[ch]) : ch)));
                sel.value = v;
                inputs[key] = () => sel.value;
                return sel;
            }
            if (f.type === "secret") {
                const set = cfg.secrets[key];
                const input = h("input", {id, type: "password", autocomplete: "off", disabled,
                    placeholder: set ? t("set, leave empty to keep") : t("not set")});
                const clear = h("input", {type: "checkbox", disabled});
                inputs[key] = () => clear.checked ? "" : (input.value ? input.value : null);
                return set ? [input, h("label", {class: "check"}, clear, t("Remove"))] : input;
            }
            const input = h("input", {id, disabled, type: f.type === "number" ? "number" : "text",
                min: f.min, max: f.max, value: v == null ? "" : String(v), spellcheck: "false"});
            inputs[key] = () => f.type === "number" ? (input.value === "" ? input.value : Number(input.value)) : input.value;
            return input;
        }

        const sections = SETTINGS.map(([title, rows]) => panel(t(title), rows.map(([key, label, hint]) => {
            const id = "set-" + key.replace(/\./g, "-");
            label = t(label);
            hint = hint ? t(hint) : null;
            if (types[key].type === "bool") {
                return [h("label", {class: "check"}, control(key, label)), hint ? h("div", {class: "hint field-hint"}, hint) : null];
            }
            const resolved = cfg.paths[key] ? " " + t("Currently: {0}", cfg.paths[key]) : "";
            return field(id, label, control(key, label), hint ? hint + resolved : null);
        })));

        const paths = panel(t("Locations and programs"),
            h("p", {class: "muted"}, t("These paths can only be changed directly in config.yml: through ffmpeg/ffprobe the server starts programs, the folders hold its state. A stolen admin sign-in must not be able to change them.")),
            h("dl", {class: "kv"}, READONLY.map(([key, label]) => [h("dt", null, t(label)),
                h("dd", {class: "mono"}, cfg.paths[key] || cfg.values[key] || "")]),
                h("dt", null, t("Configuration file")), h("dd", {class: "mono"}, cfg.file)));

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
                if (!Object.keys(values).length) return M.toast(t("No changes"));
                save.disabled = true;
                try {
                    const next = await api("PUT", "/api/admin/settings", {currentPassword: pw.value, values});
                    M.toast(t("Settings saved"));
                    c.replaceChildren();
                    await server(c);
                    if (next.pendingRestart) c.querySelector(".panel.attention")?.scrollIntoView({behavior: "smooth"});
                } catch (e) {
                    error.textContent = e.message;
                    save.disabled = false;
                }
            }}, t("Save"));

        return h("div", {class: "settings-form"},
            h("h1", null, t("Settings")),
            disabled ? h("section", {class: "panel attention"}, h("h2", null, t("Read only")),
                h("p", null, t("The server may not change {0}. Edit the file directly or give the server user write access to the file and its folder.", cfg.file))) : null,
            sections, paths,
            panel(t("Save"),
                h("p", {class: "muted"}, t("The changes are written to config.yml and take effect after a restart.")),
                field("set-pw", t("To confirm: your current password"), pw),
                error,
                save));
    }
})();

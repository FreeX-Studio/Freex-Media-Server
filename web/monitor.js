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
 * Server monitoring for admins: the CPU and memory graphs and the database panel of the
 * dashboard, and the log page of the settings. Used by account.js as M.monitor.
 */
(() => {
    "use strict";
    const M = window.FreeX;
    const {t, h, api, num} = M;
    const NS = "http://www.w3.org/2000/svg";

    /** An SVG element. */
    function s(tag, attrs, ...children) {
        const e = document.createElementNS(NS, tag);
        for (const [k, v] of Object.entries(attrs || {})) e.setAttribute(k, v);
        for (const c of children.flat()) if (c != null) e.append(c);
        return e;
    }

    /** Small sizes in KB, where M.fmtSize would show 0 MB. */
    const fmtSmall = (b) => b >= 1024 ** 2 ? M.fmtSize(b) : `${num(b / 1024, 0)} KB`;

    function fmtBytes(b) {
        if (!b) return "0";
        const gb = b / 1024 ** 3;
        return gb >= 1 ? `${num(gb, gb >= 10 ? 0 : 1)} GB` : `${num(b / 1024 ** 2, 0)} MB`;
    }

    const clock = (ms, seconds) => new Date(ms).toLocaleTimeString(M.locale(), seconds
        ? {hour: "2-digit", minute: "2-digit", second: "2-digit"} : {hour: "2-digit", minute: "2-digit"});

    // ------------------------------------------------------------------ line chart

    /**
     * A line chart over time with one y axis: a crosshair and tooltip that list every series at
     * the pointer (also with the arrow keys), a legend, labels at the line ends and a table view.
     * `series`: [{key, label, color}]; `max(graph)` the top of the y axis; `ticks(max)` its
     * grid values; `fmt(value)` the values as text. Lines break where samples are missing.
     */
    function lineChart({title, series, max, ticks, fmt}) {
        const M_ = {top: 10, right: 132, bottom: 24, left: 52};
        // Shortest time shown: a server that just started shows its few minutes, not an empty hour.
        const MIN_SPAN = 5 * 60e3;
        const HEIGHT = 190;
        const plot = h("div", {class: "chart-plot", tabindex: "0", role: "img", "aria-label": title});
        const tip = h("div", {class: "chart-tip", hidden: true});
        const legend = h("div", {class: "chart-legend"}, series.map((x) =>
            h("span", {class: "chart-key"}, h("i", {style: {background: x.color}}), x.label)));
        const table = h("details", {class: "chart-table"}, h("summary", null, t("Show as table")));
        const el = h("div", {class: "chart"}, h("div", {class: "chart-head"}, h("h3", null, title), legend), plot, tip, table);
        let graph = null;
        let span = 0;
        let maxSpan = 0;
        let active = -1;
        let geom = null;

        const draw = () => {
            const width = plot.clientWidth;
            if (!graph || !width) return;
            const pts = graph.points;
            const top = max(graph) || 1;
            const end = pts.length ? pts[pts.length - 1].time : Date.now();
            span = Math.min(maxSpan, Math.max(MIN_SPAN, pts.length ? end - pts[0].time : 0));
            const start = end - span;
            const w = width - M_.left - M_.right;
            const hgt = HEIGHT - M_.top - M_.bottom;
            const x = (time) => M_.left + (time - start) / span * w;
            const y = (v) => M_.top + hgt - Math.min(1, Math.max(0, v / top)) * hgt;
            geom = {x, y, start, end, w, hgt};
            const svg = s("svg", {width, height: HEIGHT, viewBox: `0 0 ${width} ${HEIGHT}`, "aria-hidden": "true"});
            for (const v of ticks(top)) {
                svg.append(s("line", {x1: M_.left, x2: M_.left + w, y1: y(v), y2: y(v), class: v === 0 ? "chart-base" : "chart-grid"}),
                    s("text", {x: M_.left - 8, y: y(v) + 4, "text-anchor": "end", class: "chart-axis"}, fmt(v)));
            }
            const marks = span > 2 * 3600e3 ? 4 : 6;
            for (let i = 0; i <= marks; i++) {
                const time = start + span * i / marks;
                svg.append(s("text", {x: x(time), y: HEIGHT - 6, "text-anchor": i === 0 ? "start" : i === marks ? "end" : "middle", class: "chart-axis"},
                    clock(time)));
            }
            const gap = graph.intervalMs * 3;
            for (const x_ of series) {
                let d = "";
                pts.forEach((p, i) => {
                    if (p.time < start) return;
                    const jump = i === 0 || p.time - pts[i - 1].time > gap || pts[i - 1].time < start;
                    d += `${jump ? "M" : "L"}${x(p.time).toFixed(1)},${y(p[x_.key]).toFixed(1)}`;
                });
                if (d) svg.append(s("path", {d, class: "chart-line", stroke: x_.color}));
            }
            // Labels at the line ends, pushed apart where they would overlap.
            const last = pts[pts.length - 1];
            if (last) {
                const labels = series.map((x_) => ({x_, y: y(last[x_.key])})).sort((a, b) => a.y - b.y);
                for (let i = 1; i < labels.length; i++) labels[i].y = Math.max(labels[i].y, labels[i - 1].y + 15);
                const overflow = labels.length ? labels[labels.length - 1].y - (M_.top + hgt) : 0;
                if (overflow > 0) labels.forEach((l) => l.y -= overflow);
                for (const l of labels) {
                    svg.append(s("circle", {cx: x(last.time) + 10, cy: l.y, r: 3.5, fill: l.x_.color}),
                        s("text", {x: x(last.time) + 18, y: l.y + 4, class: "chart-end"}, `${fmt(last[l.x_.key])} ${l.x_.label}`));
                }
            }
            const cross = s("g", {class: "chart-cross"});
            svg.append(cross);
            plot.replaceChildren(svg);
            if (active >= 0) show(active);
        };

        const show = (i) => {
            const pts = graph.points;
            const cross = plot.querySelector(".chart-cross");
            if (!cross || !geom || i < 0 || i >= pts.length || pts[i].time < geom.start) {
                tip.hidden = true;
                if (cross) cross.replaceChildren();
                return;
            }
            active = i;
            const p = pts[i];
            const px = geom.x(p.time);
            cross.replaceChildren(s("line", {x1: px, x2: px, y1: M_.top, y2: M_.top + geom.hgt, class: "chart-hair"}),
                ...series.map((x_) => s("circle", {cx: px, cy: geom.y(p[x_.key]), r: 4, fill: x_.color, class: "chart-dot"})));
            tip.replaceChildren(h("div", {class: "chart-tip-time"}, clock(p.time, graph.intervalMs < 60000)),
                ...series.map((x_) => h("div", {class: "chart-tip-row"}, h("i", {style: {background: x_.color}}),
                    h("strong", null, fmt(p[x_.key])), h("span", null, x_.label))));
            tip.hidden = false;
            const left = px + 14 + tip.offsetWidth > plot.clientWidth ? px - 14 - tip.offsetWidth : px + 14;
            tip.style.left = `${Math.max(0, left)}px`;
            tip.style.top = `${plot.offsetTop + M_.top}px`;
        };

        /** The sample closest to an x position. */
        const nearest = (clientX) => {
            const pts = graph?.points || [];
            if (!pts.length || !geom) return -1;
            const r = plot.getBoundingClientRect();
            const time = geom.start + (clientX - r.left - M_.left) / geom.w * span;
            let lo = 0;
            let hi = pts.length - 1;
            while (lo < hi) {
                const mid = (lo + hi) >> 1;
                if (pts[mid].time < time) lo = mid + 1; else hi = mid;
            }
            const i = lo > 0 && time - pts[lo - 1].time < pts[lo].time - time ? lo - 1 : lo;
            // Over a stretch without samples there is nothing to show.
            return Math.abs(pts[i].time - time) > Math.max(graph.intervalMs * 3, span / 100) ? -1 : i;
        };
        plot.addEventListener("pointermove", (e) => graph && show(nearest(e.clientX)));
        plot.addEventListener("pointerleave", () => {
            active = -1;
            show(-1);
        });
        plot.addEventListener("keydown", (e) => {
            if (!graph?.points.length || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) return;
            e.preventDefault();
            const n = graph.points.length;
            const i = active < 0 ? n - 1 : active;
            show({ArrowLeft: Math.max(0, i - 1), ArrowRight: Math.min(n - 1, i + 1), Home: 0, End: n - 1}[e.key]);
        });
        plot.addEventListener("blur", () => {
            active = -1;
            show(-1);
        });
        new ResizeObserver(() => draw()).observe(plot);

        const fillTable = () => {
            const pts = graph.points;
            // At most 60 rows: every nth sample.
            const step = Math.max(1, Math.ceil(pts.length / 60));
            const rows = [];
            for (let i = pts.length - 1; i >= 0; i -= step) rows.push(pts[i]);
            table.replaceChildren(table.firstChild, h("table", {class: "data-table"},
                h("thead", null, h("tr", null, h("th", null, t("Time")), series.map((x_) => h("th", null, x_.label)))),
                h("tbody", null, rows.map((p) => h("tr", null, h("td", null, clock(p.time, graph.intervalMs < 60000)),
                    series.map((x_) => h("td", null, fmt(p[x_.key]))))))));
        };
        table.addEventListener("toggle", () => table.open && graph && fillTable());

        return {
            el,
            update(g, windowMs) {
                graph = g;
                maxSpan = windowMs;
                draw();
                if (table.open) fillTable();
            },
        };
    }

    // ------------------------------------------------------------------ dashboard: performance

    /** CPU and memory of the last hour or day, refreshed while visible. */
    function performancePanel() {
        let range = M.store.get("freex.perfRange") === "day" ? "day" : "hour";
        const SERIES = {system: "var(--series-1)", freex: "var(--series-2)", ffmpeg: "var(--series-3)"};
        const cpu = lineChart({
            title: t("CPU"),
            series: [{key: "cpu", label: t("System"), color: SERIES.system}, {key: "serverCpu", label: "FreeX", color: SERIES.freex},
                {key: "ffmpegCpu", label: t("Conversions"), color: SERIES.ffmpeg}],
            max: () => 100,
            ticks: () => [0, 25, 50, 75, 100],
            fmt: (v) => `${num(v, 0)} %`,
        });
        const mem = lineChart({
            title: t("Memory"),
            series: [{key: "memory", label: t("System"), color: SERIES.system}, {key: "serverMemory", label: "FreeX", color: SERIES.freex}],
            max: (g) => g.memoryTotal,
            ticks: (top) => [0, .25, .5, .75, 1].map((f) => top * f),
            fmt: fmtBytes,
        });
        const note = h("p", {class: "muted chart-note"});
        const buttons = [["hour", t("Last hour")], ["day", t("Last 24 hours")]].map(([id, label]) =>
            h("button", {class: "seg-btn", "aria-pressed": String(id === range), onclick: () => {
                range = id;
                M.store.set("freex.perfRange", id);
                buttons.forEach((b, i) => b.setAttribute("aria-pressed", String(["hour", "day"][i] === id)));
                load();
            }}, label));
        const body = h("div", {class: "charts"}, cpu.el, mem.el);
        const root = h("section", {class: "panel monitor"},
            h("div", {class: "panel-head"}, h("h2", null, t("Performance")), h("div", {class: "seg", role: "group", "aria-label": t("Period")}, buttons)),
            note, body);
        let busy = false;
        const load = async () => {
            if (busy) return;
            busy = true;
            try {
                const g = await api("GET", `/api/admin/performance?range=${range}`);
                if (!g.supported) {
                    note.textContent = t("CPU and memory are measured on Linux only.");
                    body.hidden = true;
                    return;
                }
                note.textContent = g.points.length < 2 ? t("Collecting values …") : t("{0} cores, {1} memory. CPU in percent of all cores.", g.cores, fmtBytes(g.memoryTotal));
                const windowMs = range === "day" ? 86400e3 : 3600e3;
                cpu.update(g, windowMs);
                mem.update(g, windowMs);
            } catch (e) {
                // next time
            } finally {
                busy = false;
            }
        };
        load();
        const timer = setInterval(() => {
            if (!root.isConnected) return clearInterval(timer);
            if (!document.hidden) load();
        }, 5000);
        return root;
    }

    // ------------------------------------------------------------------ dashboard: database

    function databasePanel() {
        const box = h("div");
        const vacuum = h("input", {type: "checkbox", id: "db-vacuum"});
        const root = h("section", {class: "panel"}, h("h2", null, t("Database")), box);
        const render = (d) => {
            const st = d.stats;
            const size = st.fileBytes + st.walBytes;
            const free = st.freePages * st.pageSize;
            const last = d.last;
            const tables = st.tables.filter((x) => x.rows > 0).slice(0, 6).map((x) => `${x.name} ${num(x.rows, 0)}`).join(" · ");
            box.replaceChildren(
                h("dl", {class: "kv"},
                    h("dt", null, t("Size")), h("dd", null, fmtSmall(size),
                        st.walBytes ? h("span", {class: "muted"}, " ", t("({0} of it in the write-ahead log)", fmtSmall(st.walBytes))) : null),
                    h("dt", null, t("Free space")), h("dd", null, free ? `${fmtSmall(free)} (${num(st.freePages / Math.max(1, st.pages) * 100, 0)} %)` : t("none")),
                    h("dt", null, t("Largest tables")), h("dd", null, tables || "–"),
                    h("dt", null, t("Last optimization")), h("dd", null, last ? optimizeText(last) : t("not since the server started"))),
                h("p", {class: "muted"}, t("Optimizing checks the file, removes expired sign-ins, updates the statistics the database uses to plan its queries and gives free space back. It also runs in the nightly maintenance, right after the backup.")),
                h("label", {class: "check"}, vacuum, h("span", null, t("Also rewrite the file now (otherwise only when much space is free)"))),
                h("button", {class: "btn", disabled: d.running, onclick: async (e) => {
                    e.target.disabled = true;
                    e.target.textContent = t("Optimizing …");
                    try {
                        const r = await api("POST", "/api/admin/database/optimize", {vacuum: vacuum.checked});
                        M.toast(r.integrity === "ok" ? t("Database optimized") : t("The database check found problems"), r.integrity !== "ok");
                    } catch (err) {
                        M.toast(err.message, true);
                    }
                    refresh();
                }}, d.running ? t("Optimizing …") : t("Optimize now")));
        };
        const refresh = async () => {
            try {
                render(await api("GET", "/api/admin/database"));
            } catch (e) {
                box.replaceChildren(h("p", {class: "error-text"}, e.message));
            }
        };
        refresh();
        return root;
    }

    /** "10 Nov., 03:00 · 12 MB → 11 MB · file rewritten · check ok". */
    function optimizeText(r) {
        const parts = [M.fmtDate(r.finishedAt), `${fmtSmall(r.bytesBefore)} → ${fmtSmall(r.bytesAfter)}`];
        if (r.vacuumed) parts.push(t("file rewritten"));
        parts.push(r.integrity === "ok" ? t("check ok") : t("check found problems: {0}", r.integrity));
        return parts.join(" · ");
    }

    // ------------------------------------------------------------------ log

    const LEVELS = [["trace", "All"], ["debug", "Debug and above"], ["info", "Info and above"], ["warn", "Warnings and errors"], ["error", "Errors"]];
    const SERVER_LEVELS = [["warn", "Warnings and errors"], ["info", "Normal (info)"], ["debug", "Detailed (debug)"],
        ["info,freex_media_server::api::jellyfin=debug", "Normal, Jellyfin apps detailed"]];
    const MAX_ROWS = 3000;

    async function logPage(c) {
        let level = M.store.get("freex.logLevel") || "info";
        let query = "";
        let last = 0;
        let live = true;
        const list = h("div", {class: "log-lines", role: "log", "aria-live": "off", tabindex: "0"});
        const empty = h("p", {class: "muted"}, t("No lines."));
        const levelSel = h("select", {"aria-label": t("Show"), onchange: (e) => {
            level = e.target.value;
            M.store.set("freex.logLevel", level);
            reload();
        }}, LEVELS.map(([v, l]) => h("option", {value: v, selected: v === level}, t(l))));
        let searchTimer = null;
        const search = h("input", {type: "search", placeholder: t("Search in the log"), "aria-label": t("Search in the log"), oninput: () => {
            clearTimeout(searchTimer);
            searchTimer = setTimeout(() => {
                query = search.value.trim();
                reload();
            }, 300);
        }});
        const liveBox = h("input", {type: "checkbox", checked: true, onchange: (e) => live = e.target.checked});
        const download = h("a", {class: "btn small", href: "#", download: ""}, t("Download"));
        const serverSel = h("select", {id: "log-server-level", onchange: async (e) => {
            try {
                await api("PUT", "/api/admin/log/level", {level: e.target.value});
                M.toast(t("Log level changed until the server restarts"));
            } catch (err) {
                M.toast(err.message, true);
            }
        }});

        const row = (l) => h("div", {class: `log-line lvl-${l.level.toLowerCase()}`, title: l.target},
            h("span", {class: "log-time"}, new Date(l.time).toLocaleString(M.locale(), {day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit"})),
            h("span", {class: "log-level"}, l.level),
            h("span", {class: "log-msg"}, l.message));

        const add = (lines) => {
            const atEnd = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
            list.append(...lines.map(row));
            while (list.childElementCount > MAX_ROWS) list.firstChild.remove();
            empty.hidden = list.childElementCount > 0;
            if (atEnd) list.scrollTop = list.scrollHeight;
        };
        const params = () => `level=${encodeURIComponent(level)}&q=${encodeURIComponent(query)}`;
        const reload = async () => {
            const r = await api("GET", `/api/admin/log?${params()}&limit=1000`);
            list.replaceChildren();
            last = r.last;
            add(r.lines);
            list.scrollTop = list.scrollHeight;
            download.href = `/api/admin/log/download?${params()}`;
            const options = SERVER_LEVELS.some(([v]) => v === r.level) ? SERVER_LEVELS : [...SERVER_LEVELS, [r.level, r.level]];
            serverSel.replaceChildren(...options.map(([v, l]) => h("option", {value: v, selected: v === r.level}, t(l))));
        };

        c.append(h("h1", null, t("Log")),
            h("section", {class: "panel"},
                h("div", {class: "log-bar"}, levelSel, search, h("label", {class: "check"}, liveBox, h("span", null, t("Live"))), download),
                list, empty,
                h("p", {class: "muted log-hint"}, t("The last 5000 lines since the server started. The system keeps the full log as well (journalctl -u freex-media-server on Linux)."))),
            h("section", {class: "panel"}, h("h2", null, t("Level of detail")),
                h("div", {class: "field"}, h("label", {for: "log-server-level"}, t("The server writes")), serverSel,
                    h("div", {class: "hint"}, t("Applies right away, until the server restarts; then FREEX_LOG or Normal applies again. Detailed logging writes much more.")))));
        await reload();
        const timer = setInterval(async () => {
            if (!list.isConnected) return clearInterval(timer);
            if (!live || document.hidden) return;
            try {
                const r = await api("GET", `/api/admin/log?${params()}&after=${last}&limit=1000`);
                last = r.last;
                if (r.lines.length) add(r.lines);
            } catch (e) {
                // next time
            }
        }, 2000);
    }

    M.monitor = {performancePanel, databasePanel, logPage, optimizeText};
})();

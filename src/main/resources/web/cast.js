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
 * FreeX Web: Chromecast support (Google Cast sender), used by the player.
 *
 * The SDK is loaded from Google only in Chrome on a secure page (HTTPS or localhost); Chrome
 * offers the Cast API nowhere else. The receiver is Google's Default Media Receiver, which
 * loads the media from this server with a per-cast token (see CastApi on the server).
 */
(() => {
    "use strict";
    const M = window.FreeX;
    const listeners = new Set();
    let context = null;

    const supported = () => window.isSecureContext && !!window.chrome && /Chrome\//.test(navigator.userAgent)
        && !/Edg\/|OPR\//.test(navigator.userAgent);

    function emit() {
        for (const fn of listeners) {
            try {
                fn();
            } catch (e) {
                console.error(e);
            }
        }
    }

    if (supported()) {
        window.__onGCastApiAvailable = (available) => {
            if (!available || !window.cast || !window.cast.framework) return;
            context = cast.framework.CastContext.getInstance();
            context.setOptions({
                receiverApplicationId: chrome.cast.media.DEFAULT_MEDIA_RECEIVER_APP_ID,
                autoJoinPolicy: chrome.cast.AutoJoinPolicy.ORIGIN_SCOPED,
            });
            context.addEventListener(cast.framework.CastContextEventType.CAST_STATE_CHANGED, emit);
            context.addEventListener(cast.framework.CastContextEventType.SESSION_STATE_CHANGED, emit);
            emit();
        };
        const s = document.createElement("script");
        s.src = "https://www.gstatic.com/cv/js/sender/v1/cast_sender.js?loadCastFramework=1";
        s.async = true;
        document.head.append(s);
    }

    M.cast = {
        /** True once Chromecasts can be found from this page. */
        get ready() {
            return !!context;
        },
        /** At least one Chromecast is in the network. */
        get devicesAvailable() {
            return !!context && context.getCastState() !== cast.framework.CastState.NO_DEVICES_AVAILABLE;
        },
        /** The running cast session or null. */
        get session() {
            return context ? context.getCurrentSession() : null;
        },
        get deviceName() {
            const s = this.session;
            return s ? s.getCastDevice().friendlyName : "";
        },
        /** Opens Chrome's device picker. */
        requestSession() {
            return context ? context.requestSession() : Promise.reject(new Error(M.t("Chromecast is not available")));
        },
        endSession() {
            if (context) context.endCurrentSession(true);
        },
        /** Called on device and session changes; returns a function that removes the listener. */
        onChange(fn) {
            listeners.add(fn);
            return () => listeners.delete(fn);
        },

        /**
         * Loads an item on the Chromecast. {@code plan} is the answer of POST /api/items/{id}/cast.
         * Subtitle labels come from the item's media info.
         */
        load(item, plan, subtitleId) {
            const session = this.session;
            if (!session) return Promise.reject(new Error("Keine Verbindung zum Chromecast"));
            const origin = location.origin;
            const info = new chrome.cast.media.MediaInfo(origin + plan.url, plan.contentType);
            info.streamType = chrome.cast.media.StreamType.BUFFERED;
            if (plan.hls) {
                info.hlsSegmentFormat = chrome.cast.media.HlsSegmentFormat.FMP4;
                info.hlsVideoSegmentFormat = chrome.cast.media.HlsVideoSegmentFormat.FMP4;
            }
            const md = new chrome.cast.media.GenericMediaMetadata();
            md.title = item.type === "episode" ? `${item.showTitle || ""} – ${M.episodeCode(item)}` : item.title;
            md.subtitle = item.type === "episode" ? item.title : (item.year ? String(item.year) : "");
            md.images = [new chrome.cast.Image(origin + plan.image)];
            info.metadata = md;
            const byId = Object.fromEntries((item.media.subtitles || []).map((s) => [s.id, s]));
            info.tracks = plan.subtitles.map((s, i) => {
                const t = new chrome.cast.media.Track(i + 1, chrome.cast.media.TrackType.TEXT);
                t.trackContentId = origin + s.url;
                t.trackContentType = "text/vtt";
                t.subtype = chrome.cast.media.TextTrackType.SUBTITLES;
                t.name = byId[s.id] ? M.subtitleLabel(byId[s.id]) : s.id;
                t.language = byId[s.id] && byId[s.id].language || "";
                return t;
            });
            const req = new chrome.cast.media.LoadRequest(info);
            req.autoplay = true;
            req.currentTime = plan.startMs / 1000;
            const track = plan.subtitles.findIndex((s) => s.id === subtitleId);
            req.activeTrackIds = track >= 0 ? [track + 1] : [];
            return session.loadMedia(req);
        },
    };
})();

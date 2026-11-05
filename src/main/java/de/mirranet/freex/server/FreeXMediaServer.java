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

package de.mirranet.freex.server;

import de.mirranet.freex.server.config.Configuration;
import de.mirranet.freex.server.http.ActivityApi;
import de.mirranet.freex.server.http.AuthApi;
import de.mirranet.freex.server.http.CastApi;
import de.mirranet.freex.server.http.ExtrasApi;
import de.mirranet.freex.server.markers.MarkerService;
import de.mirranet.freex.server.metadata.MetadataService;
import de.mirranet.freex.server.metadata.TmdbClient;
import de.mirranet.freex.server.store.Database;
import de.mirranet.freex.server.store.ListStore;
import de.mirranet.freex.server.store.UserStore;
import de.mirranet.freex.server.together.TogetherService;
import de.mirranet.freex.server.http.MediaApi;
import de.mirranet.freex.server.http.ProfileApi;
import de.mirranet.freex.server.http.SettingsApi;
import de.mirranet.freex.server.http.WebServer;
import de.mirranet.freex.server.scan.LibraryScanner;
import de.mirranet.freex.server.scan.MediaProbe;
import de.mirranet.freex.server.scan.ThumbnailService;
import de.mirranet.freex.server.store.DataStore;
import de.mirranet.freex.server.transcode.SubtitleService;
import de.mirranet.freex.server.transcode.TranscodeManager;

import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.Locale;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.logging.ConsoleHandler;
import java.util.logging.Formatter;
import java.util.logging.Level;
import java.util.logging.LogRecord;
import java.util.logging.Logger;

/**
 * Entry point. Usage: {@code java -jar freex-media-server.jar [config.yml]}.
 * <p>
 * The server runs as an {@link Instance}. Changing the settings in the web interface restarts
 * it inside the same process: the instance is closed and a new one is started from the file.
 * If the new configuration does not start, the previous file is restored and started again,
 * so a bad setting never leaves the server unreachable.
 */
public final class FreeXMediaServer {

    private static final Logger LOG = Logger.getLogger(FreeXMediaServer.class.getName());

    private FreeXMediaServer() {
    }

    public static void main(String[] args) throws Exception {
        // Profile pictures are scaled with java.awt; no display is needed for that.
        System.setProperty("java.awt.headless", "true");
        setupLogging();
        String version = FreeXMediaServer.class.getPackage().getImplementationVersion();
        if (version == null) version = "dev";
        LOG.info("FreeX Media Server " + version + " starting, web client FreeX Web " + MediaApi.webClient().get("version"));
        String fsEncoding = System.getProperty("sun.jnu.encoding", "");
        // Windows hands file names to Java as Unicode whatever the code page, so only other systems need it.
        boolean windows = System.getProperty("os.name", "").toLowerCase(Locale.ROOT).contains("win");
        if (!windows && !fsEncoding.equalsIgnoreCase("UTF-8")) {
            LOG.warning("File names are decoded as '" + fsEncoding + "', so umlauts in folder and file names will be"
                    + " garbled. Start the server with a UTF-8 locale, e.g. LANG=C.UTF-8 (see README).");
        }
        Path configFile = Path.of(args.length > 0 ? args[0] : "config.yml");

        BlockingQueue<Boolean> restarts = new LinkedBlockingQueue<>();
        Runnable requestRestart = () -> restarts.offer(Boolean.TRUE);
        Object lock = new Object();
        AtomicReference<Instance> current = new AtomicReference<>();
        AtomicBoolean stopping = new AtomicBoolean();
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            synchronized (lock) {
                stopping.set(true);
                Instance i = current.getAndSet(null);
                if (i != null) i.close();
            }
        }, "shutdown"));

        String goodText = null; // the configuration the last successful start used
        while (true) {
            Instance instance;
            try {
                instance = Instance.start(configFile, version, requestRestart);
            } catch (Exception e) {
                if (goodText == null) throw e;
                Path failed = configFile.resolveSibling(configFile.getFileName() + ".failed");
                LOG.log(Level.SEVERE, "The server did not start with the changed configuration: " + e.getMessage()
                        + ". Restoring the previous configuration; the failed one is kept as " + failed, e);
                Files.copy(configFile, failed, StandardCopyOption.REPLACE_EXISTING);
                Files.writeString(configFile, goodText, StandardCharsets.UTF_8);
                try {
                    instance = Instance.start(configFile, version, requestRestart);
                } catch (Exception again) {
                    LOG.log(Level.SEVERE, "The previous configuration does not start either", again);
                    System.exit(1);
                    return;
                }
            }
            goodText = Files.readString(configFile, StandardCharsets.UTF_8);
            synchronized (lock) {
                if (stopping.get()) {
                    instance.close();
                    return;
                }
                current.set(instance);
            }
            restarts.take();
            restarts.clear();
            LOG.info("Restarting");
            synchronized (lock) {
                if (stopping.get()) return;
                current.set(null);
                instance.close();
            }
        }
    }

    /** One running server: all services wired together, closed in reverse order. */
    private static final class Instance implements AutoCloseable {

        private final Deque<AutoCloseable> closers = new ArrayDeque<>();

        private <T extends AutoCloseable> T add(T c) {
            closers.push(c);
            return c;
        }

        static Instance start(Path configFile, String version, Runnable requestRestart) throws Exception {
            Instance in = new Instance();
            try {
                in.wire(configFile, version, requestRestart);
                return in;
            } catch (Exception | Error e) {
                in.close();
                throw e;
            }
        }

        private void wire(Path configFile, String version, Runnable requestRestart) throws Exception {
            Configuration config = Configuration.load(configFile);
            LOG.info("Data directory: " + config.dataDirectory.toAbsolutePath()
                    + (config.cacheDirectory.equals(config.dataDirectory) ? "" : ", cache directory: " + config.cacheDirectory.toAbsolutePath()));
            Database db = add(new Database(config.dataDirectory.resolve("freex.db")));
            UserStore users = new UserStore(db);
            DataStore store = new DataStore(config.dataDirectory, users);
            add(store::close);
            store.setSessionTtlDays(config.sessionDays);
            MediaProbe probe = new MediaProbe(config.ffprobe);
            ThumbnailService thumbs = new ThumbnailService(config.ffmpeg, config.cacheDirectory);
            SubtitleService subtitles = new SubtitleService(config.ffmpeg, config.cacheDirectory);
            LibraryScanner scanner = new LibraryScanner(store, probe, config.scanThreads, config.scanIntervalMinutes);
            add(scanner::close);
            if (thumbs.available()) LOG.info("Using " + config.ffmpeg + " and " + config.ffprobe);
            TranscodeManager transcoder = null;
            if (config.transcoderEnabled && thumbs.available()) {
                transcoder = new TranscodeManager(config.ffmpeg, config.transcodeDirectory, config.maxTranscodeSessions,
                        config.hardwareAcceleration, config.vaapiDevice, config.x264Preset, probe);
                add(transcoder::close);
            } else if (config.transcoderEnabled) {
                LOG.warning("Transcoding disabled: ffmpeg not found. Only files the browser can play directly will work.");
            }

            TmdbClient tmdb = config.tmdbApiKey.isEmpty() ? null
                    : new TmdbClient(config.tmdbApiUrl, config.tmdbImageUrl, config.tmdbApiKey, config.tmdbLanguage);
            if (tmdb == null) LOG.info("No TMDB key configured (tmdb.apiKey): only local metadata is used");
            MetadataService metadata = new MetadataService(store, tmdb, config.tmdbRegion, config.dataDirectory);
            add(metadata::close);
            ListStore lists = new ListStore(db);
            add(lists::close);
            MarkerService markers = config.markersEnabled && thumbs.available()
                    ? new MarkerService(store, config.ffmpeg, config.dataDirectory) : null;
            if (markers != null) add(markers::close);
            TogetherService together = new TogetherService(id -> store.user(id).map(u -> u.avatarVersion).orElse(0L));
            add(together::close);
            scanner.onScanned(lib -> {
                metadata.applyAll(lib);
                metadata.refreshLibrary(lib, false);
                if (markers != null) markers.analyzeLibrary(lib, false);
            });

            WebServer web = new WebServer(config.bindAddress, config.port, store, config.tlsKeystore,
                    config.tlsKeystorePassword, config.trustProxy);
            add(web::close);
            MediaApi media = new MediaApi(config.serverName, version, store, scanner, thumbs, probe, thumbs.available(),
                    transcoder, subtitles);
            ProfileApi profile = new ProfileApi(store, media, config.dataDirectory);
            new AuthApi(config.serverName, store, config.minPasswordLength, config.sessionDays, id -> {
                lists.removeUser(id);
                profile.removeUser(id);
            }).register(web);
            ActivityApi activity = new ActivityApi(store, transcoder);
            media.setActivity(activity);
            media.register(web);
            profile.register(web);
            activity.register(web, media);
            new CastApi(store, media).register(web);
            new ExtrasApi(store, media, metadata, lists, markers, together).register(web);
            new SettingsApi(config, requestRestart).register(web);
            web.start();

            LOG.info("Web interface: " + (web.tls() ? "https" : "http") + "://"
                    + (config.bindAddress.equals("0.0.0.0") ? "localhost" : config.bindAddress) + ":" + config.port + "/");
            if (!web.tls() && !config.trustProxy) {
                LOG.info("Plain HTTP: fine in your LAN. For access from the internet use HTTPS (server.tlsKeystore) or a reverse proxy.");
            }
            if (!store.hasUsers()) LOG.info("First start: open the web interface to create the admin account");

            // Pick up changes made while the server was offline.
            scanner.scanAll();
        }

        @Override
        public void close() {
            LOG.info("Shutting down");
            while (!closers.isEmpty()) {
                try {
                    closers.pop().close();
                } catch (Exception e) {
                    LOG.log(Level.WARNING, "Error while shutting down", e);
                }
            }
        }
    }

    private static void setupLogging() {
        Logger root = Logger.getLogger("");
        for (var h : root.getHandlers()) root.removeHandler(h);
        ConsoleHandler handler = new ConsoleHandler();
        handler.setLevel(Level.ALL);
        handler.setFormatter(new Formatter() {
            @Override
            public String format(LogRecord r) {
                String cls = r.getLoggerName();
                cls = cls == null ? "" : cls.substring(cls.lastIndexOf('.') + 1);
                String msg = String.format("%1$tF %1$tT [%2$s] %3$s: %4$s%n", r.getMillis(), r.getLevel(), cls,
                        formatMessage(r));
                if (r.getThrown() != null) {
                    StringWriter sw = new StringWriter();
                    r.getThrown().printStackTrace(new PrintWriter(sw));
                    msg += sw;
                }
                return msg;
            }
        });
        root.addHandler(handler);
        root.setLevel(Level.INFO);
    }
}

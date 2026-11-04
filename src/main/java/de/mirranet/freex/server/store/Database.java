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

package de.mirranet.freex.server.store;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.PosixFilePermissions;
import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.PreparedStatement;
import java.sql.ResultSet;
import java.sql.SQLException;
import java.sql.Statement;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.locks.ReentrantLock;
import java.util.logging.Logger;

/**
 * The SQLite database {@code <data>/freex.db} with users and everything that belongs to them:
 * sessions, login history, watch state and history, preferences, playlists, watchlist, and
 * hand-made collections.
 * <p>
 * One connection is shared and guarded by a lock (SQLite serializes writes anyway, and the
 * statements here take microseconds). The lock is a {@link ReentrantLock} rather than
 * {@code synchronized}, so virtual threads waiting for it do not pin their carrier thread.
 * The schema is versioned with {@code PRAGMA user_version}; {@link #MIGRATIONS} are applied in order.
 */
public final class Database implements AutoCloseable {

    private static final Logger LOG = Logger.getLogger(Database.class.getName());

    /** Schema changes; index + 1 is the schema version they lead to. Never edit, only append. */
    private static final String[][] MIGRATIONS = {
            {
                    """
                    CREATE TABLE users (
                        id TEXT PRIMARY KEY,
                        username TEXT NOT NULL UNIQUE COLLATE NOCASE,
                        password_hash TEXT NOT NULL,
                        admin INTEGER NOT NULL DEFAULT 0,
                        created_at INTEGER NOT NULL,
                        password_changed_at INTEGER NOT NULL,
                        totp_secret TEXT,
                        totp_enabled INTEGER NOT NULL DEFAULT 0,
                        totp_last_step INTEGER NOT NULL DEFAULT 0
                    )""",
                    """
                    CREATE TABLE recovery_codes (
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        code_hash TEXT NOT NULL,
                        PRIMARY KEY (user_id, code_hash)
                    )""",
                    """
                    CREATE TABLE sessions (
                        token_hash TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        created_at INTEGER NOT NULL,
                        last_seen INTEGER NOT NULL,
                        ip TEXT NOT NULL DEFAULT '',
                        user_agent TEXT NOT NULL DEFAULT ''
                    )""",
                    "CREATE INDEX sessions_user ON sessions(user_id)",
                    """
                    CREATE TABLE login_events (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        time INTEGER NOT NULL,
                        ip TEXT NOT NULL DEFAULT '',
                        user_agent TEXT NOT NULL DEFAULT '',
                        success INTEGER NOT NULL,
                        detail TEXT NOT NULL DEFAULT ''
                    )""",
                    "CREATE INDEX login_events_user ON login_events(user_id, time)",
                    """
                    CREATE TABLE progress (
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        item_id TEXT NOT NULL,
                        offset_ms INTEGER NOT NULL DEFAULT 0,
                        watched INTEGER NOT NULL DEFAULT 0,
                        view_count INTEGER NOT NULL DEFAULT 0,
                        updated_at INTEGER NOT NULL DEFAULT 0,
                        PRIMARY KEY (user_id, item_id)
                    )""",
                    """
                    CREATE TABLE playlists (
                        id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        name TEXT NOT NULL,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    )""",
                    """
                    CREATE TABLE playlist_items (
                        playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                        position INTEGER NOT NULL,
                        item_id TEXT NOT NULL,
                        PRIMARY KEY (playlist_id, position)
                    )""",
                    """
                    CREATE TABLE watchlist (
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        item_id TEXT NOT NULL,
                        added_at INTEGER NOT NULL,
                        PRIMARY KEY (user_id, item_id)
                    )""",
                    """
                    CREATE TABLE collections (
                        id TEXT PRIMARY KEY,
                        library_id TEXT NOT NULL,
                        name TEXT NOT NULL,
                        summary TEXT NOT NULL DEFAULT '',
                        created_at INTEGER NOT NULL
                    )""",
                    """
                    CREATE TABLE collection_items (
                        collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
                        position INTEGER NOT NULL,
                        item_id TEXT NOT NULL,
                        PRIMARY KEY (collection_id, position)
                    )""",
            },
            // 2: profile picture, language preferences, watch history
            {
                    "ALTER TABLE users ADD COLUMN avatar_version INTEGER NOT NULL DEFAULT 0",
                    "ALTER TABLE users ADD COLUMN audio_language TEXT NOT NULL DEFAULT ''",
                    "ALTER TABLE users ADD COLUMN subtitle_mode TEXT NOT NULL DEFAULT 'forced'",
                    "ALTER TABLE users ADD COLUMN subtitle_language TEXT NOT NULL DEFAULT ''",
                    """
                    CREATE TABLE watch_history (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        item_id TEXT NOT NULL,
                        watched_at INTEGER NOT NULL,
                        title TEXT NOT NULL DEFAULT '',
                        parent_title TEXT NOT NULL DEFAULT ''
                    )""",
                    "CREATE INDEX watch_history_user ON watch_history(user_id, watched_at)",
                    // What was watched before the history existed, as far as it is known
                    "INSERT INTO watch_history (user_id, item_id, watched_at) "
                            + "SELECT user_id, item_id, updated_at FROM progress WHERE watched = 1 AND updated_at > 0",
            },
            // 3: libraries per user (existing users keep access to everything)
            {
                    "ALTER TABLE users ADD COLUMN all_libraries INTEGER NOT NULL DEFAULT 1",
                    """
                    CREATE TABLE user_libraries (
                        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                        library_id TEXT NOT NULL,
                        PRIMARY KEY (user_id, library_id)
                    )""",
            },
            // 4: display language, sidebar entries the user unpinned, login events as codes instead of German text
            {
                    "ALTER TABLE users ADD COLUMN ui_language TEXT NOT NULL DEFAULT ''",
                    "ALTER TABLE users ADD COLUMN unpinned TEXT NOT NULL DEFAULT ''",
                    "UPDATE login_events SET detail = 'password' WHERE detail = 'Passwort'",
                    "UPDATE login_events SET detail = 'password+totp' WHERE detail = 'Passwort + Authenticator'",
                    "UPDATE login_events SET detail = 'password+recovery' WHERE detail = 'Passwort + Wiederherstellungscode'",
                    "UPDATE login_events SET detail = 'wrong-password' WHERE detail = 'Falsches Passwort'",
                    "UPDATE login_events SET detail = 'wrong-code' WHERE detail = 'Falscher Bestätigungscode'",
            },
    };

    /** Maps one result row. */
    @FunctionalInterface
    public interface RowMapper<T> {
        T map(ResultSet rs) throws SQLException;
    }

    /** Work inside a transaction. */
    @FunctionalInterface
    public interface Work {
        void run(Database db) throws SQLException;
    }

    private final Connection conn;
    private final ReentrantLock lock = new ReentrantLock();
    private final Path file;

    public Database(Path file) throws IOException {
        this.file = file;
        Files.createDirectories(file.toAbsolutePath().getParent());
        boolean created = !Files.exists(file);
        try {
            Class.forName("org.sqlite.JDBC");
        } catch (ClassNotFoundException e) {
            throw new IOException("SQLite driver (org.xerial:sqlite-jdbc) not found on the class path. "
                    + "Build the server with 'mvn clean package' and start target/freex-media-server.jar, "
                    + "which contains the driver.", e);
        }
        try {
            conn = DriverManager.getConnection("jdbc:sqlite:" + file.toAbsolutePath());
            try (Statement s = conn.createStatement()) {
                s.execute("PRAGMA journal_mode=WAL");
                s.execute("PRAGMA synchronous=NORMAL");
                s.execute("PRAGMA foreign_keys=ON");
                s.execute("PRAGMA busy_timeout=5000");
            }
            migrate();
        } catch (SQLException e) {
            throw new IOException("Cannot open database " + file + ": " + e.getMessage(), e);
        }
        if (created) restrictPermissions();
    }

    public Path file() {
        return file;
    }

    private void restrictPermissions() {
        for (String suffix : new String[]{"", "-wal", "-shm"}) {
            Path p = file.resolveSibling(file.getFileName() + suffix);
            try {
                if (Files.exists(p)) Files.setPosixFilePermissions(p, PosixFilePermissions.fromString("rw-------"));
            } catch (UnsupportedOperationException | IOException ignored) {
                // Windows or a file system without POSIX permissions
            }
        }
    }

    private void migrate() throws SQLException {
        int version;
        try (Statement s = conn.createStatement(); ResultSet rs = s.executeQuery("PRAGMA user_version")) {
            version = rs.next() ? rs.getInt(1) : 0;
        }
        if (version > MIGRATIONS.length) {
            throw new SQLException("Database schema " + version + " is newer than this server (" + MIGRATIONS.length
                    + "). Please update the server.");
        }
        for (int v = version; v < MIGRATIONS.length; v++) {
            conn.setAutoCommit(false);
            try (Statement s = conn.createStatement()) {
                for (String sql : MIGRATIONS[v]) s.execute(sql);
                s.execute("PRAGMA user_version=" + (v + 1));
                conn.commit();
            } catch (SQLException e) {
                conn.rollback();
                throw e;
            } finally {
                conn.setAutoCommit(true);
            }
            LOG.info("Database schema updated to version " + (v + 1));
        }
    }

    /** INSERT/UPDATE/DELETE; returns the number of changed rows. */
    public int update(String sql, Object... params) {
        lock.lock();
        try (PreparedStatement ps = prepare(sql, params)) {
            return ps.executeUpdate();
        } catch (SQLException e) {
            throw new IllegalStateException("Database error: " + e.getMessage() + " in " + sql, e);
        } finally {
            lock.unlock();
        }
    }

    public <T> List<T> query(String sql, RowMapper<T> mapper, Object... params) {
        lock.lock();
        try (PreparedStatement ps = prepare(sql, params); ResultSet rs = ps.executeQuery()) {
            List<T> out = new ArrayList<>();
            while (rs.next()) out.add(mapper.map(rs));
            return out;
        } catch (SQLException e) {
            throw new IllegalStateException("Database error: " + e.getMessage() + " in " + sql, e);
        } finally {
            lock.unlock();
        }
    }

    /** True while a transaction is open; nested calls then simply join it. */
    private boolean inTransaction;

    /** Runs several statements atomically. A transaction inside a transaction joins the outer one. */
    public void transaction(Work work) {
        lock.lock();
        try {
            if (inTransaction) {
                work.run(this);
                return;
            }
            inTransaction = true;
            conn.setAutoCommit(false);
            try {
                work.run(this);
                conn.commit();
            } catch (SQLException | RuntimeException e) {
                conn.rollback();
                throw e instanceof RuntimeException r ? r : new IllegalStateException("Database error: " + e.getMessage(), e);
            } finally {
                inTransaction = false;
                conn.setAutoCommit(true);
            }
        } catch (SQLException e) {
            throw new IllegalStateException("Database error: " + e.getMessage(), e);
        } finally {
            lock.unlock();
        }
    }

    private PreparedStatement prepare(String sql, Object... params) throws SQLException {
        PreparedStatement ps = conn.prepareStatement(sql);
        for (int i = 0; i < params.length; i++) {
            Object p = params[i];
            if (p instanceof Boolean b) ps.setInt(i + 1, b ? 1 : 0);
            else ps.setObject(i + 1, p);
        }
        return ps;
    }

    @Override
    public void close() {
        lock.lock();
        try {
            try (Statement s = conn.createStatement()) {
                s.execute("PRAGMA wal_checkpoint(TRUNCATE)");
            }
            conn.close();
        } catch (SQLException e) {
            LOG.warning("Closing database: " + e.getMessage());
        } finally {
            lock.unlock();
        }
    }
}

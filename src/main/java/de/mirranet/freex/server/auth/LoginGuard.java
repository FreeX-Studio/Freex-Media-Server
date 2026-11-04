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

package de.mirranet.freex.server.auth;

import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;

/**
 * Slows down password guessing.
 * <ul>
 *   <li>Per address: after {@value #IP_LIMIT} failures within {@value #IP_WINDOW_MINUTES} minutes,
 *       further attempts from it are refused until the window has passed.</li>
 *   <li>Per account name: after {@value #FREE_ATTEMPTS} consecutive failures, each further attempt
 *       must wait twice as long as the one before (2 s, 4 s, 8 s … up to 15 minutes). This
 *       protects an account against attacks from many addresses, without locking it permanently.</li>
 * </ul>
 * Account names are tracked whether or not they exist, so the answer never reveals that.
 */
public final class LoginGuard {

    static final int IP_LIMIT = 20;
    static final int IP_WINDOW_MINUTES = 10;
    static final int FREE_ATTEMPTS = 5;
    private static final long MAX_DELAY_MS = TimeUnit.MINUTES.toMillis(15);

    private record Counter(int count, long first, long last) {
    }

    private final Map<String, Counter> byIp = new ConcurrentHashMap<>();
    private final Map<String, Counter> byAccount = new ConcurrentHashMap<>();

    /** Seconds the caller has to wait before trying again, 0 if an attempt is allowed now. */
    public long retryAfterSeconds(String ip, String account) {
        long now = System.currentTimeMillis();
        long wait = 0;
        Counter c = byIp.get(ip);
        if (c != null) {
            long windowEnd = c.first + TimeUnit.MINUTES.toMillis(IP_WINDOW_MINUTES);
            if (now > windowEnd) byIp.remove(ip);
            else if (c.count >= IP_LIMIT) wait = windowEnd - now;
        }
        Counter a = byAccount.get(key(account));
        if (a != null && a.count >= FREE_ATTEMPTS) {
            long delay = Math.min(MAX_DELAY_MS, 1000L << Math.min(20, a.count - FREE_ATTEMPTS + 1));
            long until = a.last + delay;
            if (until > now) wait = Math.max(wait, until - now);
        }
        return wait == 0 ? 0 : Math.max(1, (wait + 999) / 1000);
    }

    public void failure(String ip, String account) {
        long now = System.currentTimeMillis();
        byIp.merge(ip, new Counter(1, now, now), (o, n) -> new Counter(o.count + 1, o.first, now));
        byAccount.merge(key(account), new Counter(1, now, now), (o, n) -> new Counter(o.count + 1, o.first, now));
        if (byIp.size() > 10_000) cleanup();
    }

    public void success(String ip, String account) {
        byAccount.remove(key(account));
        Counter c = byIp.get(ip);
        if (c != null && c.count > 0) byIp.put(ip, new Counter(Math.max(0, c.count - 1), c.first, c.last));
    }

    private static String key(String account) {
        return account == null ? "" : account.trim().toLowerCase(Locale.ROOT);
    }

    private void cleanup() {
        long cutoff = System.currentTimeMillis() - TimeUnit.HOURS.toMillis(1);
        byIp.values().removeIf(c -> c.last < cutoff);
        byAccount.values().removeIf(c -> c.last < cutoff);
    }
}

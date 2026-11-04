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

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import java.net.URLEncoder;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.SecureRandom;
import java.util.Locale;

/**
 * Time-based one-time passwords (RFC 6238, SHA-1, 6 digits, 30 s), compatible with authenticator
 * apps such as Aegis, Google Authenticator, Microsoft Authenticator or 1Password.
 */
public final class Totp {

    private static final String BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    private static final int STEP_SECONDS = 30;
    private static final SecureRandom RANDOM = new SecureRandom();

    private Totp() {
    }

    /** New random 160-bit secret, Base32 encoded. */
    public static String newSecret() {
        byte[] b = new byte[20];
        RANDOM.nextBytes(b);
        return base32(b);
    }

    /** {@code otpauth://} URI for the QR code. */
    public static String uri(String issuer, String account, String secret) {
        String label = enc(issuer) + ":" + enc(account);
        return "otpauth://totp/" + label + "?secret=" + secret + "&issuer=" + enc(issuer)
                + "&algorithm=SHA1&digits=6&period=" + STEP_SECONDS;
    }

    private static String enc(String s) {
        return URLEncoder.encode(s, StandardCharsets.UTF_8).replace("+", "%20");
    }

    /**
     * Checks a code against the current time step and one step before/after (clock drift).
     *
     * @param lastUsedStep the step of the last accepted code; codes from that step or earlier are
     *                     rejected so an observed code cannot be replayed
     * @return the matching time step, or -1 if the code is wrong or already used
     */
    public static long verify(String secret, String code, long lastUsedStep) {
        if (secret == null || code == null) return -1;
        String c = code.replaceAll("\\s", "");
        if (!c.matches("\\d{6}")) return -1;
        byte[] key = decodeBase32(secret);
        long now = System.currentTimeMillis() / 1000 / STEP_SECONDS;
        long match = -1;
        for (long step = now - 1; step <= now + 1; step++) {
            // Compare every candidate, so timing does not reveal which window matched.
            if (constantTimeEquals(generate(key, step), c) && step > lastUsedStep) match = step;
        }
        return match;
    }

    static String generate(byte[] key, long step) {
        try {
            Mac mac = Mac.getInstance("HmacSHA1");
            mac.init(new SecretKeySpec(key, "HmacSHA1"));
            byte[] h = mac.doFinal(ByteBuffer.allocate(8).putLong(step).array());
            int offset = h[h.length - 1] & 0x0f;
            int bin = ((h[offset] & 0x7f) << 24) | ((h[offset + 1] & 0xff) << 16)
                    | ((h[offset + 2] & 0xff) << 8) | (h[offset + 3] & 0xff);
            return String.format(Locale.ROOT, "%06d", bin % 1_000_000);
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    private static boolean constantTimeEquals(String a, String b) {
        return java.security.MessageDigest.isEqual(a.getBytes(StandardCharsets.US_ASCII), b.getBytes(StandardCharsets.US_ASCII));
    }

    static String base32(byte[] data) {
        StringBuilder sb = new StringBuilder();
        int buffer = 0;
        int bits = 0;
        for (byte b : data) {
            buffer = (buffer << 8) | (b & 0xff);
            bits += 8;
            while (bits >= 5) {
                sb.append(BASE32.charAt((buffer >> (bits - 5)) & 31));
                bits -= 5;
            }
        }
        if (bits > 0) sb.append(BASE32.charAt((buffer << (5 - bits)) & 31));
        return sb.toString();
    }

    static byte[] decodeBase32(String s) {
        String in = s.toUpperCase(Locale.ROOT).replaceAll("[\\s=-]", "");
        ByteBuffer out = ByteBuffer.allocate(in.length() * 5 / 8 + 1);
        int buffer = 0;
        int bits = 0;
        for (char ch : in.toCharArray()) {
            int v = BASE32.indexOf(ch);
            if (v < 0) throw new IllegalArgumentException("Invalid Base32");
            buffer = (buffer << 5) | v;
            bits += 5;
            if (bits >= 8) {
                out.put((byte) (buffer >> (bits - 8)));
                bits -= 8;
            }
        }
        byte[] r = new byte[out.position()];
        out.flip();
        out.get(r);
        return r;
    }
}

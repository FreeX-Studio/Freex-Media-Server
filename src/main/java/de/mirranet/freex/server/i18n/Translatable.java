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

package de.mirranet.freex.server.i18n;

/** An exception whose message is meant for the user and can be translated. */
public interface Translatable {

    Text text();

    /** The text of an exception: its own if it has one, otherwise its message as it is. */
    static Text of(Throwable e) {
        return e instanceof Translatable t ? t.text() : Text.raw(e.getMessage());
    }
}

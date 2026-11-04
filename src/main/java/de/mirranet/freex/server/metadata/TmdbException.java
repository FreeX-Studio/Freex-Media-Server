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

package de.mirranet.freex.server.metadata;

import de.mirranet.freex.server.i18n.Text;
import de.mirranet.freex.server.i18n.Translatable;

import java.io.IOException;

/** TMDB could not be used (no or wrong key, overloaded, unexpected answer). */
public final class TmdbException extends IOException implements Translatable {

    private static final long serialVersionUID = 1L;

    private final transient Text text;

    TmdbException(String key, Object... args) {
        super(new Text(key, args).toString());
        this.text = new Text(key, args);
    }

    @Override
    public Text text() {
        return text;
    }
}

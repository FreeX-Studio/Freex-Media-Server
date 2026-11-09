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

//! Media libraries: the entries and their files, the scanner that builds them from folders,
//! the in-memory index and generated artwork.

pub mod catalog;
pub mod delete;
mod model;
pub mod names;
pub mod nfo;
pub mod playback;
pub mod previews;
pub mod probe;
pub mod ratings;
pub mod scanner;
pub mod subtitles;
pub mod thumbs;

pub use model::*;

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

// FreeX Media Server (Rust): checks, tests, release builds for Linux and Windows, the Debian
// package and the Windows installer.
//
// The build agent is a Linux machine with rustup, the MinGW-w64 cross compiler, dpkg, NSIS,
// curl, unzip and python3:
//
//   curl https://sh.rustup.rs -sSf | sh -s -- -y
//   sudo apt install build-essential gcc-mingw-w64-x86-64 dpkg nsis curl unzip python3
//
// The Windows program is cross-compiled (target x86_64-pc-windows-gnu); the toolchain stage adds
// the target. The installer stage downloads ffmpeg and WinSW (github.com, gyan.dev) once into
// ~/.cache/freex-build. Artifacts in dist/: the .deb, the setup .exe, the plain archives and
// SHA256SUMS.
//
// APT repository: when the environment variable APT_REPO names the directory of a reprepro
// repository on the build machine (for example /var/www/apt, set under Manage Jenkins → System →
// Global properties), the Publish stage adds the .deb to it, in the suite APT_SUITE (default
// "stable"). Builds of other branches than main are not published. See docs/apt-repository.md.
//
// TMDB key: when the environment variable FREEX_TMDB_KEY is set (Manage Jenkins → System →
// Global properties, like APT_REPO), it is built into the programs and used by servers whose
// tmdb.api_key is empty. Builds without it need a key in the configuration as before.

pipeline {
    agent any

    options {
        buildDiscarder(logRotator(numToKeepStr: '20', artifactNumToKeepStr: '10'))
        disableConcurrentBuilds()
        timeout(time: 60, unit: 'MINUTES')
    }

    environment {
        PATH = "${env.HOME}/.cargo/bin:${env.PATH}"
        CARGO_TERM_COLOR = 'never'
        // Keeps dependency downloads and compiled dependencies between builds of the job.
        CARGO_HOME = "${env.HOME}/.cargo"
        WINDOWS_TARGET = 'x86_64-pc-windows-gnu'
    }

    stages {
        stage('Toolchain') {
            steps {
                sh '''
                    # rust-toolchain.toml names the toolchain, its components and the Windows target;
                    # rustup installs what is missing on the first run.
                    rustup show active-toolchain || rustup toolchain install
                    rustc --version
                    cargo --version
                    command -v x86_64-w64-mingw32-gcc
                    command -v dpkg-deb
                    command -v makensis
                '''
            }
        }

        stage('Check') {
            steps {
                sh 'cargo fmt --check'
                sh 'cargo clippy --locked --all-targets -- -D warnings'
            }
        }

        stage('Test') {
            steps {
                sh 'cargo test --locked'
            }
        }

        stage('Build') {
            steps {
                sh 'cargo build --locked --release'
                sh 'cargo build --locked --release --target "$WINDOWS_TARGET"'
            }
        }

        stage('Package') {
            steps {
                sh '''
                    # The workspace is kept between builds (target/ caches the compiled
                    # dependencies), so packages of earlier versions must not be left in dist/.
                    rm -rf dist
                    VERSION="$(sed -n 's/^version = "\\(.*\\)"$/\\1/p' Cargo.toml | head -n 1)"
                    # Every build gets its own package version, newer than the one before, so apt
                    # sees it as an update even while Cargo.toml keeps the same version:
                    # 0.21.0+202611110746.git7ccb57a
                    COMMIT="$(git rev-parse --short=7 HEAD 2>/dev/null || echo local)"
                    DEB_VERSION="$VERSION+$(date -u +%Y%m%d%H%M).git$COMMIT"
                    packaging/deb/build-deb.sh "$DEB_VERSION" target/release/freex-media-server dist
                    packaging/windows/build-installer.sh "$VERSION" "target/$WINDOWS_TARGET/release/freex-media-server.exe" dist
                    packaging/dist.sh
                '''
            }
        }

        stage('Publish') {
            when {
                expression { env.APT_REPO?.trim() && (env.BRANCH_NAME == null || env.BRANCH_NAME == 'main') }
            }
            steps {
                sh 'packaging/apt/publish.sh "$APT_REPO" "${APT_SUITE:-stable}" dist/*.deb'
            }
        }
    }

    post {
        success {
            archiveArtifacts artifacts: 'dist/*', fingerprint: true, allowEmptyArchive: false
        }
    }
}

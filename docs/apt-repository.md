<p align="center"><img src="brand/freex-logo.svg" alt="FreeX" width="72"></p>

# APT repository

FreeX Media Server is published in the MirraNET APT repository at `https://apt.mirranet.de`, together with the other
MirraNET packages. Servers with Debian 12+ or Ubuntu 22.04+ install it from there and get every release with
`apt upgrade`.

## Installing from the repository

```sh
sudo install -d /etc/apt/keyrings
sudo curl -fsSL https://apt.mirranet.de/mirranet.gpg -o /etc/apt/keyrings/mirranet.gpg
echo "deb [signed-by=/etc/apt/keyrings/mirranet.gpg] https://apt.mirranet.de stable main" \
    | sudo tee /etc/apt/sources.list.d/mirranet.list
sudo apt update && sudo apt install freex-media-server
```

The first two commands import the repository's signing key, with which APT checks every package. All MirraNET
packages share the suite `stable`; each is installed by its own package name.

## How releases get there

Only releases go into the repository, not every build. A release is made in GitHub with *Actions → Release → Run
workflow* (see [Development](development.md#releases)). The *Build* workflow then builds the package for the tag,
and its job `apt` copies the `.deb` and `packaging/apt/publish.sh` to the repository server by SSH and runs the
script there; `reprepro` adds the package and signs the index with the key that never leaves that server.

The job reads five secrets, set once as organization secrets of FreeX-Studio:

| Secret | Value |
|---|---|
| `APT_HOST` | the deploy user and host of the repository server |
| `APT_PORT` | its SSH port |
| `APT_REPO` | the repository directory on the server |
| `APT_SSH_KEY` | the private deploy key, with the `BEGIN`/`END` lines |
| `APT_KNOWN_HOSTS` | the server's host key (`ssh-keyscan -p <port> <host>`) |

Without them the job is skipped with a notice and the release is still published on GitHub.

The repository keeps one version of each package: a newer version replaces the older one, and an older or equal
version than the one published is not taken.

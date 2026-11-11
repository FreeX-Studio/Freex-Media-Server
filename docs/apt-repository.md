<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

# APT repository

The Debian package can be offered from an APT repository of your own. Servers that add it to their package sources
install FreeX with `apt install freex-media-server` and get every new build with `apt upgrade`. The repository is
kept with [reprepro](https://salsa.debian.org/debian/reprepro), served by any web server and filled by the Jenkins
build ([Jenkinsfile](../Jenkinsfile), stage *Publish*). One suite serves Debian 12 and later and Ubuntu 22.04 and
later alike.

This page describes a repository on the machine that runs Jenkins, at `/var/www/apt`, published as
`https://apt.mirranet.de`. Replace the address with your own.

## Setting up the repository (once)

### 1. Directory and reprepro

```sh
sudo apt install reprepro gnupg
sudo install -d -o jenkins -g jenkins /var/www/apt /var/www/apt/conf
```

The directory belongs to the user Jenkins runs as (here `jenkins`), because the build writes to it.

### 2. Signing key

APT only accepts signed repositories. Create a key of its own for the repository, in the keyring of the Jenkins
user and without a passphrase, so builds can sign without anyone typing one:

```sh
sudo -u jenkins gpg --batch --passphrase '' --quick-gen-key "MirraNET APT <apt@mirranet.de>" rsa4096 sign never
sudo -u jenkins gpg --list-secret-keys --keyid-format long apt@mirranet.de    # the id after "rsa4096/"
```

Keep a copy of the secret key in a safe place (`sudo -u jenkins gpg --export-secret-keys --armor <KEY-ID>`). If it
is lost, every client has to import a new key.

### 3. Configuration

Copy the templates from `packaging/apt/` and put the key id in `SignWith`:

```sh
sudo -u jenkins cp packaging/apt/distributions packaging/apt/options /var/www/apt/conf/
sudo -u jenkins sed -i 's/REPLACE-WITH-KEY-ID/<KEY-ID>/' /var/www/apt/conf/distributions
```

Then publish the public key next to the repository, where clients download it:

```sh
sudo -u jenkins sh -c 'gpg --export <KEY-ID> > /var/www/apt/mirranet.gpg'
```

### 4. Web server

A DNS record for `apt.mirranet.de` pointing to the machine, then for nginx:

```nginx
server {
    server_name apt.mirranet.de;
    root /var/www/apt;
    autoindex on;
    # reprepro's configuration and database are not for the public
    location ~ ^/(conf|db)/ { deny all; }
}
```

Add HTTPS as for your other sites (for example `sudo certbot --nginx -d apt.mirranet.de`). The packages are signed
either way, HTTPS additionally hides which packages a client loads.

### 5. Jenkins

Under *Manage Jenkins → System → Global properties → Environment variables* add:

| Name | Value |
|---|---|
| `APT_REPO` | `/var/www/apt` |
| `APT_SUITE` | `stable` (optional, the default) |

From the next build on, the stage *Publish* adds the new package to the repository. Without `APT_REPO` the stage is
skipped, and in a multibranch job only builds of `main` are published.

To add a package by hand:

```sh
sudo -u jenkins packaging/apt/publish.sh /var/www/apt stable dist/freex-media-server_*_amd64.deb
```

## Package versions

Every Jenkins build gets a version of its own that is higher than the one before, even while `Cargo.toml` keeps
the same version: `0.21.0+202611110746.git7ccb57a` (version, build time in UTC, commit). That is what lets `apt
upgrade` see each build as an update. The repository keeps one version of the package: a new build replaces the
older one. A version that is already published cannot be published again with different contents; reprepro
refuses it.

The version shown in the web interface (*Settings → About FreeX*) stays the one from `Cargo.toml`.

## On the servers that install FreeX

```sh
sudo install -d /etc/apt/keyrings
sudo curl -fsSL https://apt.mirranet.de/mirranet.gpg -o /etc/apt/keyrings/mirranet.gpg
```

`/etc/apt/sources.list.d/mirranet.sources`:

```
Types: deb
URIs: https://apt.mirranet.de
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/mirranet.gpg
```

On older systems the one-line form in `/etc/apt/sources.list.d/mirranet.list` does the same:

```
deb [signed-by=/etc/apt/keyrings/mirranet.gpg] https://apt.mirranet.de stable main
```

Then:

```sh
sudo apt update
sudo apt install freex-media-server
```

Updates come with `sudo apt update && sudo apt upgrade`; the package restarts the service. A server that has the
package installed from a file already moves to the repository's builds with the next upgrade.

## Troubleshooting

| Message | Cause |
|---|---|
| `NO_PUBKEY` / `The following signatures couldn't be verified` | The key under `Signed-By` is missing or is not the repository's key |
| `File not found … InRelease` | Wrong address or suite, or the web server does not serve `/var/www/apt` |
| *Publish* fails with `sha256 expected … got …` | This version was published before with different contents; build again (new version) |
| *Publish* fails with `gpg: signing failed` | The key named in `SignWith` is not in the keyring of the Jenkins user |

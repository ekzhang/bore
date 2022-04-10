#!/bin/sh

# Heavily modified from https://github.com/japaric/trust/blob/gh-pages/install.sh.

help() {
    cat <<'EOF'
Install a binary release of a Rust crate hosted on GitHub.

Usage:
    install.sh [options]

Options:
    -h, --help      Display this message
    --git SLUG      Get the crate from "https://github/$SLUG"
    -f, --force     Force overwriting an existing binary
    --crate NAME    Name of the crate to install (default <repository name>)
    --tag TAG       Tag (version) of the crate to install (default <latest release>)
    --to LOCATION   Where to install the binary (default /usr/local/bin)
EOF
}

say() {
    echo "install.sh: $1"
}

say_err() {
    say "$1" >&2
}

err() {
    if [ -n "$td" ]; then
        rm -rf "$td"
    fi

    say_err "ERROR $1"
    exit 1
}

need() {
    if ! command -v "$1" > /dev/null 2>&1; then
        err "need $1 (command not found)"
    fi
}

force=false
while test $# -gt 0; do
    case $1 in
        --crate)
            crate=$2
            shift
            ;;
        --force | -f)
            force=true
            ;;
        --git)
            git=$2
            shift
            ;;
        --help | -h)
            help
            exit 0
            ;;
        --tag)
            tag=$2
            shift
            ;;
        --to)
            dest=$2
            shift
            ;;
        *)
            ;;
    esac
    shift
done

# Dependencies
need basename
need curl
need install
need mkdir
need mktemp
need tar

# Optional dependencies
if [ -z "$crate" ] || [ -z "$tag" ] || [ -z "$target" ]; then
    need cut
fi

if [ -z "$tag" ]; then
    need rev
fi

if [ -z "$git" ]; then
    # shellcheck disable=SC2016
    err 'must specify a git repository using `--git`. Example: `install.sh --git japaric/cross`'
fi

url="https://github.com/$git"

if [ "$(curl --head --write-out "%{http_code}\n" --silent --output /dev/null "$url")" -eq "404" ]; then
  err "GitHub repository $git does not exist"
fi

say_err "GitHub repository: $url"

if [ -z "$crate" ]; then
    crate=$(echo "$git" | cut -d'/' -f2)
fi

say_err "Crate: $crate"

if [ -z "$dest" ]; then
    dest="/usr/local/bin"
fi

if [ -e "$dest/$crate" ] && [ $force = false ]; then
    err "$crate already exists in $dest, use --force to overwrite the existing binary"
fi

url="$url/releases"

if [ -z "$tag" ]; then
    tag=$(curl -s "$url/latest" | cut -d'"' -f2 | rev | cut -d'/' -f1 | rev)
    say_err "Tag: latest ($tag)"
else
    say_err "Tag: $tag"
fi


case "$(uname -s)" in
"Darwin")
  case "$(uname -m)" in
    "x86_64")
      target="x86_64-apple-darwin"
      ;;
    "arm64")
      ## replace when M1 builds are working
      # target="aarch64-apple-darwin"
      target="x86_64-apple-darwin"
      ;;
  esac
  ;;
"Linux")
  platform="unknown-linux-musl"
  target="$(uname -m)-$platform"
  ;;
esac

say_err "Target: $target"

url="$url/download/$tag/$crate-$tag-$target.tar.gz"

say_err "Downloading: $url"

if [ "$(curl --head --write-out "%{http_code}\n" --silent --output /dev/null "$url")" -eq "404" ]; then
  err "$url does not exist, you will need to build $crate from source"
fi

td=$(mktemp -d || mktemp -d -t tmp)
curl -sL "$url" | tar -C "$td" -xz

say_err "Installing to: $dest"

for f in "$td"/*; do
    [ -e "$f" ] || break # handle the case of no *.wav files

    test -x "$f" || continue

    if [ -e "$dest/$crate" ] && [ $force = false ]; then
        err "$crate already exists in $dest"
    else
        mkdir -p "$dest"
        cp "$f" "$dest"
        chmod 0755 "$dest/$crate"
    fi
done

rm -rf "$td"

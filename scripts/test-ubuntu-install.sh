#!/usr/bin/env bash
# Cross-release installation check for one Ubuntu 22.04-built Debian package.
set -euo pipefail

fail() { printf 'Ubuntu install test: %s\n' "$*" >&2; exit 1; }
[[ $# == 1 ]] || fail 'usage: bash scripts/test-ubuntu-install.sh path/to/pc-remote-desktop.deb'

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
installer="$root/scripts/install-ubuntu.sh"
probe="$root/scripts/linux-validation-a11y.py"
package=$(realpath -- "$1")
[[ -r $installer ]] || fail "installer not found: $installer"
[[ -r $package && $package == *.deb ]] || fail "readable .deb not found: $package"
command -v docker >/dev/null || fail 'Docker is required.'
docker_installer=$installer
docker_package=$package
docker_probe=$probe
if command -v cygpath >/dev/null; then
  docker_installer=$(cygpath -w "$installer")
  docker_package=$(cygpath -w "$package")
  docker_probe=$(cygpath -w "$probe")
  export MSYS_NO_PATHCONV=1
fi

evidence="$root/artifacts/ubuntu-install"
mkdir -p "$evidence"

for release in ${PC_REMOTE_TEST_RELEASES:-22.04 24.04 26.04}; do
  case $release in 22.04|24.04|26.04) ;; *) fail "unsupported test release: $release" ;; esac
  log="$evidence/install-$release.log"
  if [[ -e $log ]]; then cp "$log" "$evidence/install-$release-previous-$(date +%s).log"; fi
  printf 'Testing Ubuntu %s with %s\n' "$release" "$(basename -- "$package")"
  if docker run --rm -i \
    --mount "type=bind,src=$docker_installer,dst=/harness/install-ubuntu.sh,readonly" \
    --mount "type=bind,src=$docker_package,dst=/harness/package.deb,readonly" \
    --mount "type=bind,src=$docker_probe,dst=/harness/a11y.py,readonly" \
    -e DEBIAN_FRONTEND=noninteractive \
    "ubuntu:$release" bash -s >"$log" 2>&1 <<'IN_CONTAINER'
set -euo pipefail

work=$(mktemp -d /tmp/pc-remote-install-test.XXXXXX)
chmod 777 "$work"
trap 'rm -rf -- "$work"' EXIT

# Run invalid package checks as an ordinary user with sentinel sudo/apt-get
# commands. Rejection must happen during package validation, before either tool.
stubs="$work/stubs"
mkdir -p "$stubs"
cat > "$stubs/sudo" <<'STUB'
#!/bin/sh
printf 'sudo %s\n' "$*" >> "$PC_REMOTE_INSTALL_TEST_CALLS"
exit 99
STUB
cat > "$stubs/apt-get" <<'STUB'
#!/bin/sh
printf 'apt-get %s\n' "$*" >> "$PC_REMOTE_INSTALL_TEST_CALLS"
exit 99
STUB
chmod 755 "$stubs/sudo" "$stubs/apt-get"

make_fixture() {
  local name=$1 package_name=$2 architecture=$3
  local tree="$work/$name"
  mkdir -p "$tree/DEBIAN"
  cat > "$tree/DEBIAN/control" <<CONTROL
Package: $package_name
Version: 0.0.1
Architecture: $architecture
Maintainer: PC Remote Test <nobody@example.invalid>
Description: Temporary invalid package used only by the installer guard test
CONTROL
  dpkg-deb --build "$tree" "$work/$name.deb" >/dev/null
}

architecture=$(dpkg --print-architecture)
make_fixture wrong-name other-desktop "$architecture"
make_fixture wrong-architecture pc-remote all

assert_rejected_before_privileged_tools() {
  local package_path=$1 expected=$2 output="$work/output"
  local calls="$work/privileged-calls"
  rm -f -- "$calls" "$output"
  if runuser -u nobody -- env \
    PATH="$stubs:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    PC_REMOTE_INSTALL_TEST_CALLS="$calls" \
    /bin/bash /harness/install-ubuntu.sh "$package_path" >"$output" 2>&1; then
    echo "Installer accepted invalid package: $package_path" >&2
    exit 1
  fi
  if ! grep -Fq -- "$expected" "$output"; then
    cat "$output" >&2
    echo "Installer did not reject invalid package as expected: $expected" >&2
    exit 1
  fi
  if [[ -e $calls ]]; then
    cat "$calls" >&2
    echo 'Installer invoked sudo or apt-get before rejecting the package.' >&2
    exit 1
  fi
}

command -v runuser >/dev/null || { echo 'runuser is required for preflight tests' >&2; exit 1; }
assert_rejected_before_privileged_tools "$work/wrong-name.deb" 'O pacote informado não é o PC Remote.'
assert_rejected_before_privileged_tools "$work/wrong-architecture.deb" 'Baixe o pacote para a arquitetura'
echo 'Invalid package name and architecture rejected before sudo/apt-get.'

# Install the exact same artifact in each disposable Ubuntu image, then inspect
# the installed ELF dependencies and desktop launch entry.
bash /harness/install-ubuntu.sh /harness/package.deb <<< 'Y'
status=$(dpkg-query -W -f='${db:Status-Status}' pc-remote)
[[ $status == installed ]] || { echo "unexpected dpkg status: $status" >&2; exit 1; }

binary=$(dpkg -L pc-remote | grep -E '/bin/pc-remote-desktop$' | head -n 1)
[[ -n $binary && -x $binary ]] || { echo "installed executable missing: ${binary:-none}" >&2; exit 1; }
ldd "$binary" > "$work/ldd.txt"
cat "$work/ldd.txt"
if grep -Fq 'not found' "$work/ldd.txt"; then
  echo 'Installed executable has unresolved shared libraries.' >&2
  exit 1
fi

desktop=$(dpkg -L pc-remote | grep -E '/applications/[^/]+\.desktop$' | head -n 1)
[[ -r $desktop ]] || { echo 'installed desktop entry missing' >&2; exit 1; }
exec_value=$(sed -n 's/^Exec=//p' "$desktop" | head -n 1)
exec_name=${exec_value%% *}
resolved_exec=$(command -v "$exec_name" || true)
[[ -n $resolved_exec && -x $resolved_exec ]] || {
  echo "desktop Exec does not resolve to an executable: $exec_value" >&2
  exit 1
}
printf 'Installed %s; desktop Exec=%s (%s).\n' "$status" "$exec_value" "$resolved_exec"

# Exercise the installed binary as a normal user, retaining the WebKit sandbox.
# All display, D-Bus and home state belongs to this disposable container.
apt-get install -y --no-install-recommends xvfb xauth x11-utils dbus-daemon \
  at-spi2-core python3-pyatspi > /tmp/smoke-dependencies.log 2>&1 || {
  cat /tmp/smoke-dependencies.log; exit 1;
}
useradd --create-home --shell /bin/bash smoke
mkdir -p /home/smoke/runtime
chown smoke:smoke /home/smoke/runtime
chmod 700 /home/smoke/runtime
timeout 50s runuser -u smoke -- env \
  XDG_SESSION_TYPE=x11 XDG_RUNTIME_DIR=/home/smoke/runtime GTK_MODULES=atk-bridge \
  DBUS_SYSTEM_BUS_ADDRESS=unix:path=/tmp/no-system-bus \
  dbus-run-session -- xvfb-run -a -s '-screen 0 1280x900x24' \
  bash -c '
    set -euo pipefail
    "$1" > "$2/app.log" 2>&1 &
    app_pid=$!
    trap '\''kill "$app_pid" 2>/dev/null || true'\'' EXIT
    for _ in $(seq 1 30); do
      if ! kill -0 "$app_pid" 2>/dev/null; then cat "$2/app.log"; exit 1; fi
      if xwininfo -root -tree 2>/dev/null | grep -Fq "PC Remote" \
        && timeout --signal=KILL 3s python3 /harness/a11y.py --contains "Parear celular" > "$2/a11y.log" 2>&1; then
        echo "Package startup passed: live process, PC Remote window, accessible Parear celular."
        exit 0
      fi
      sleep 0.25
    done
    cat "$2/app.log" "$2/a11y.log"
    exit 1
  ' _ "$binary" "$work"
IN_CONTAINER
  then
    printf 'Ubuntu %s passed.\n' "$release"
    tail -n 12 "$log"
  else
    printf 'Ubuntu %s failed; complete container log follows.\n' "$release" >&2
    cat "$log" >&2
    exit 1
  fi
done

printf 'Requested Ubuntu install checks passed. Logs: %s\n' "$evidence"

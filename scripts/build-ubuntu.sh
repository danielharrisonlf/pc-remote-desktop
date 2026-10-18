#!/usr/bin/env bash
# Build on Ubuntu 22.04 to retain the oldest supported glibc baseline.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
for tool in node npm cargo dpkg-deb sha256sum tar; do
  command -v "$tool" >/dev/null || { echo "Instale $tool antes de compilar. Consulte docs/UBUNTU.md." >&2; exit 1; }
done
npm ci --no-audit --no-fund
npm run tauri -- build --bundles deb
version=$(node -p 'JSON.parse(require("fs").readFileSync("package.json", "utf8")).version')
arch=$(dpkg --print-architecture)
bundle_dir="${CARGO_TARGET_DIR:-$root/src-tauri/target}/release/bundle/deb"
output="$root/release-assets/$version/ubuntu-$arch"
mkdir -p "$output"
shopt -s nullglob
matches=("$bundle_dir"/*_"$version"_"$arch".deb)
[[ ${#matches[@]} == 1 ]] || { echo 'Esperado exatamente um .deb da versão atual.' >&2; exit 1; }
[[ $(dpkg-deb -f "${matches[0]}" Package) == pc-remote ]] || { echo 'Identidade inesperada no pacote gerado.' >&2; exit 1; }
package="pc-remote-desktop-${version}-${arch}.deb"
cp -- "${matches[0]}" "$output/$package"
cp -- scripts/install-ubuntu.sh "$output/install-ubuntu.sh"
chmod +x "$output/install-ubuntu.sh"
printf 'Extraia esta pasta e execute: bash install-ubuntu.sh\nAbra PC Remote no menu. No Wayland, autorize o controle em Configurações.\nAtualize instalando o novo .deb; pareamentos são mantidos.\n' > "$output/LEIA-ME.txt"
(cd "$output" && sha256sum "$package" install-ubuntu.sh LEIA-ME.txt > SHA256SUMS)
archive="pc-remote-ubuntu-${version}-${arch}.tar.gz"
tar -C "$output" -czf "$root/release-assets/$version/$archive" "$package" install-ubuntu.sh LEIA-ME.txt SHA256SUMS
echo "Pacote: $output/$package"
echo "Instalador com instruções: $root/release-assets/$version/$archive"

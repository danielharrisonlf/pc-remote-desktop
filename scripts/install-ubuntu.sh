#!/usr/bin/env bash
# Run as your normal user: bash install-ubuntu.sh [path/to/pc-remote.deb]
set -euo pipefail

fail() { printf 'PC Remote: %s\n' "$*" >&2; exit 1; }
if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  echo 'Uso: bash install-ubuntu.sh [arquivo.deb]'
  echo 'Instala ou atualiza o PC Remote com as dependências, sem compilar.'
  exit 0
fi
[[ $# -le 1 ]] || fail 'Informe apenas um arquivo .deb.'
[[ -r /etc/os-release ]] || fail 'Sistema não identificado.'
. /etc/os-release
[[ $ID == ubuntu ]] || fail 'Este instalador é destinado ao Ubuntu.'
case ${VERSION_ID:-} in
  22.04|24.04|26.04) ;;
  *) fail 'Use Ubuntu 22.04, 24.04 ou 26.04 LTS.' ;;
esac
for tool in dpkg dpkg-deb apt-get realpath; do
  command -v "$tool" >/dev/null || fail "Comando necessário não encontrado: $tool"
done
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [[ $# == 1 ]]; then
  package=$1
else
  shopt -s nullglob
  packages=("$script_dir"/*.deb)
  [[ ${#packages[@]} == 1 ]] || fail 'Informe o caminho do .deb ou deixe apenas um .deb ao lado deste instalador.'
  package=${packages[0]}
fi
[[ -f $package && $package == *.deb ]] || fail 'Arquivo .deb não encontrado.'
package=$(realpath -- "$package")
[[ $(dpkg-deb -f "$package" Package) == pc-remote ]] || fail 'O pacote informado não é o PC Remote.'
arch=$(dpkg --print-architecture)
[[ $(dpkg-deb -f "$package" Architecture) == "$arch" ]] || fail "Baixe o pacote para a arquitetura $arch."
printf 'Instalando PC Remote %s no Ubuntu %s (%s).\n' "$(dpkg-deb -f "$package" Version)" "$VERSION_ID" "$arch"
elevate=()
if [[ $EUID != 0 ]]; then
  command -v sudo >/dev/null || fail 'É necessário sudo para instalar o pacote.'
  elevate=(sudo)
fi
"${elevate[@]}" apt-get update
# apt installs dependencies and performs upgrades while retaining user data.
"${elevate[@]}" apt-get install --no-install-recommends -- "$package"
echo 'Pronto. Abra PC Remote no menu de aplicativos.'
echo 'No Wayland, abra Configurações → Controle no Ubuntu → Autorizar controle.'

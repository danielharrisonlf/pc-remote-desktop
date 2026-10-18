#!/usr/bin/env bash
set -uo pipefail

source_root=/source
workspace=/validation/workspace
evidence=/validation/evidence
cache=/validation/cache
target_dir=${CARGO_TARGET_DIR:-$cache/target}
binary=$target_dir/release/pc-remote-desktop

mkdir -p "$evidence" "$cache" "$workspace" "$target_dir" \
  "${CARGO_HOME:-$cache/cargo}" "${npm_config_cache:-$cache/npm}" \
  /validation/runtime /validation/config /validation/data /validation/cache-home
chmod 700 /validation/runtime

if [[ ${1:-} == --runtime ]]; then
  : > "$evidence/runtime-summary.txt"
  export XDG_RUNTIME_DIR=/validation/runtime
  export XDG_CONFIG_HOME=/validation/config
  export XDG_DATA_HOME=/validation/data
  export XDG_CACHE_HOME=/validation/cache-home
  export GTK_MODULES=atk-bridge
  export PC_REMOTE_LINUX_VALIDATION=1
  export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket

  pulseaudio --daemonize=yes --exit-idle-time=-1 \
    --log-target="file:$evidence/pulseaudio.log"
  cleanup() {
    pulseaudio --kill >/dev/null 2>&1 || true
  }
  trap cleanup EXIT

  pulse_ready=0
  for _ in $(seq 1 20); do
    if pactl info >/dev/null 2>&1; then
      pulse_ready=1
      break
    fi
    sleep 0.25
  done
  if [[ $pulse_ready -ne 1 ]]; then
    echo 'PulseAudio did not become ready' | tee "$evidence/runtime-summary.txt"
    exit 1
  fi
  pactl load-module module-null-sink sink_name=pc_remote_validation \
    sink_properties=device.description=PC-Remote-Validation \
    >"$evidence/pulse-null-sink-module.txt"
  pactl set-default-sink pc_remote_validation

  cargo test --manifest-path "$workspace/src-tauri/Cargo.toml" --test linux_runtime --locked -- --ignored --test-threads=1 --nocapture \
    2>&1 | tee "$evidence/linux-runtime-tests.log"
  test_rc=${PIPESTATUS[0]}

  PC_REMOTE_PORTAL_TEST=1 XDG_SESSION_TYPE=wayland \
    cargo test --manifest-path "$workspace/src-tauri/Cargo.toml" --test wayland_portal --locked -- --ignored --nocapture \
    2>&1 | tee "$evidence/wayland-portal-tests.log"
  portal_rc=${PIPESTATUS[0]}

  app_rc=0
  if [[ ! -x $binary ]]; then
    echo "Expected Linux application binary missing: $binary" | tee -a "$evidence/runtime-summary.txt"
    app_rc=1
  else
    setsid "$binary" >"$evidence/tauri-app.log" 2>&1 &
    app_pid=$!
    window_ready=0
    content_ready=0
    readiness_deadline=$((SECONDS + 45))
    for _ in $(seq 1 45); do
      if (( SECONDS >= readiness_deadline )); then
        break
      fi
      if ! kill -0 "$app_pid" 2>/dev/null; then
        echo 'Tauri process exited before readiness' | tee -a "$evidence/runtime-summary.txt"
        app_rc=1
        break
      fi
      if timeout 3s xwininfo -root -tree 2>/dev/null | grep -Fq '"PC Remote"'; then
        window_ready=1
      fi
      if [[ $window_ready -eq 1 ]] && timeout --signal=KILL 3s python3 "$workspace/scripts/linux-validation-a11y.py" \
          --contains 'Parear celular' >"$evidence/accessibility-readiness.log" 2>&1; then
        content_ready=1
        break
      fi
      sleep 1
    done
    if [[ $window_ready -ne 1 || $content_ready -ne 1 ]]; then
      echo "Tauri readiness failed: process=$app_pid window=$window_ready web_content=$content_ready" \
        | tee -a "$evidence/runtime-summary.txt"
      tail -n 60 "$evidence/tauri-app.log" >>"$evidence/runtime-summary.txt" 2>/dev/null || true
      app_rc=1
    else
      echo "Tauri readiness passed: process=$app_pid mapped_window=PC Remote accessible_content=Parear celular" \
        | tee -a "$evidence/runtime-summary.txt"
    fi
    kill -TERM -- "-$app_pid" 2>/dev/null || kill -TERM "$app_pid" 2>/dev/null || true
    for _ in $(seq 1 12); do
      kill -0 "$app_pid" 2>/dev/null || break
      sleep 0.25
    done
    if kill -0 "$app_pid" 2>/dev/null; then
      kill -KILL -- "-$app_pid" 2>/dev/null || kill -KILL "$app_pid" 2>/dev/null || true
    fi
    wait "$app_pid" 2>/dev/null || true
  fi

  if [[ $test_rc -ne 0 || $portal_rc -ne 0 || $app_rc -ne 0 ]]; then
    printf 'runtime_tests_exit=%s\nwayland_portal_exit=%s\ntauri_smoke_exit=%s\n' "$test_rc" "$portal_rc" "$app_rc" \
      | tee -a "$evidence/runtime-summary.txt"
    exit 1
  fi
  printf 'runtime_tests_exit=0\nwayland_portal_exit=0\ntauri_smoke_exit=0\n' | tee -a "$evidence/runtime-summary.txt"
  exit 0
fi

tar -C "$source_root" \
  --exclude='./.git' \
  --exclude='./.icon-venv' \
  --exclude='./node_modules' \
  --exclude='./dist' \
  --exclude='./artifacts' \
  --exclude='./src-tauri/target' \
  --exclude='./src-tauri/gen' \
  --exclude='./src-tauri/.tauri' \
  --exclude='./release-assets' \
  -cf - . | tar -C "$workspace" -xf -

rustc --version | tee "$evidence/rust-version.txt"
node --version | tee "$evidence/node-version.txt"

cargo test --manifest-path "$workspace/src-tauri/Cargo.toml" --lib --locked -- --include-ignored \
  2>&1 | tee "$evidence/cargo-test-lib.log"
unit_rc=${PIPESTATUS[0]}
if [[ $unit_rc -ne 0 ]]; then
  echo "cargo test --lib failed: $unit_rc" | tee "$evidence/build-summary.txt"
  exit "$unit_rc"
fi

npm ci --prefix "$workspace" --no-audit --no-fund \
  2>&1 | tee "$evidence/npm-ci.log"
npm_rc=${PIPESTATUS[0]}
if [[ $npm_rc -ne 0 ]]; then
  echo "npm ci failed: $npm_rc" | tee "$evidence/build-summary.txt"
  exit "$npm_rc"
fi

npm --prefix "$workspace" test \
  2>&1 | tee "$evidence/npm-test.log"
npm_test_rc=${PIPESTATUS[0]}
if [[ $npm_test_rc -ne 0 ]]; then
  echo "npm test failed: $npm_test_rc" | tee "$evidence/build-summary.txt"
  exit "$npm_test_rc"
fi

npm --prefix "$workspace" run build:ubuntu \
  2>&1 | tee "$evidence/tauri-build.log"
build_rc=${PIPESTATUS[0]}
if [[ $build_rc -ne 0 ]]; then
  echo "Tauri Linux build failed: $build_rc" | tee "$evidence/build-summary.txt"
  exit "$build_rc"
fi

cp "$workspace"/release-assets/*/ubuntu-*/*.deb "$evidence/"
cp "$workspace"/release-assets/*/pc-remote-ubuntu-*.tar.gz "$evidence/"

if [[ ! -x $binary ]]; then
  echo "Tauri build succeeded but binary is missing: $binary" | tee "$evidence/build-summary.txt"
  exit 1
fi
cp "$binary" "$evidence/pc-remote-desktop-linux"
file "$evidence/pc-remote-desktop-linux" | tee "$evidence/linux-binary-file.txt"

dbus-run-session -- xvfb-run -a -s '-screen 0 1280x900x24' \
  bash "${BASH_SOURCE[0]}" --runtime \
  2>&1 | tee "$evidence/runtime-supervisor.log"
runtime_rc=${PIPESTATUS[0]}

printf 'cargo_test_lib_exit=0\nnpm_test_exit=0\ntauri_build_exit=0\nruntime_supervisor_exit=%s\n' "$runtime_rc" \
  | tee "$evidence/build-summary.txt"
exit "$runtime_rc"

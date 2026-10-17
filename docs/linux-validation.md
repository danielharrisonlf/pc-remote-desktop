# Linux build and runtime validation

Run the native Linux build and runtime harness from Windows PowerShell with Docker Desktop:

```powershell
pwsh -File scripts/linux-validation.ps1
```

The script builds the named `linux-system-dependencies`, `linux-rust-toolchain`, and `linux-validation` stages in `Dockerfile.linux`, snapshots its driver and the checkout into a writable container workspace, runs the locked Rust library suite and frontend tests, builds the real Tauri Linux executable with the frontend embedded, and runs Linux backend checks under a private session D-Bus and Xvfb. The final smoke check requires a live app process, a mapped `PC Remote` window, and the accessible `Parear celular` control from the embedded page. Accessibility probes and process cleanup have bounded timeouts.

The seven runtime tests are ignored during ordinary `cargo test` runs and require the harness marker, private Xvfb display, private session bus, and an explicit absent system-bus address before any side effects. The harness exercises XTEST input against an X window, text and image clipboard access and image sequence stability, PulseAudio volume and mute against a temporary null sink, an MPRIS service on the private session bus, XDG autostart writes under a temporary config directory, system identity and network reads, and safe failures when the system D-Bus/logind socket is absent. It never calls a power action against a real logind service.

The container drops Linux capabilities, enables `no-new-privileges`, mounts the source read-only, and does not mount the host X11 display, PulseAudio socket, session bus, or system bus. Network access is used inside Docker for the Ubuntu packages, Cargo crates, and npm packages. Build caches stay in the named Docker volume `pc-remote-linux-validation-cache`.

Evidence logs and the built binary (`pc-remote-desktop-linux`) are written under `artifacts/linux-validation/` (ignored by Git). The logs record the actual unit-test count, runtime test output, compiler and Node versions, binary format, app readiness checks, and any failure details. The library suite command can also be run by itself inside the container:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked
```

The runtime harness uses Node.js 24, Xvfb, X11 utilities, PulseAudio, AT-SPI, and D-Bus session tools installed by the Dockerfile. It is designed for the repository's Ubuntu 22.04/WebKitGTK 4.1 build environment and does not validate Wayland compositor behavior or distro packaging/install integration.

The final 2026-10-17 run passed the complete wrapper with Rust 1.99.1 and Node
24.22.0, using Docker image
`sha256:2a773c394ef8830445fe2d83948f580cf3ecdabd67d92a61022ac3af9a44597f`.

| Acceptance | Result |
| --- | --- |
| Native Rust library suite | 105 passed, 0 failed, 0 ignored (`--include-ignored`) |
| Frontend tests inside Linux | 38 passed |
| Real Linux Tauri release build | Passed; exported ELF x86-64 binary |
| Backend runtime tests | All 7 passed |
| Embedded UI readiness | Live process, mapped window and accessible `Parear celular` |
| WebKit sandbox | Default sandbox retained |
| XTEST regression stability | Additional 32 consecutive repetitions passed |

This checkout has 105 Linux library tests, rather than the brief's estimated 106.
The ordinary `cargo test --lib --locked` run passed 104 with one ignored test;
the wrapper explicitly includes the default-route QR smoke test on Docker's
private bridge so it verifies all 105.

The runtime checks found and now cover two production defects:

- Image writes hashed PNG bytes while polling hashed raw pixels, so unchanged
  image readback advanced sequence 3→4 and emitted an event. Both paths now hash
  decoded RGBA plus dimensions. The regression verifies stable own-write sequence
  and a real external dimension change with otherwise identical pixel bytes.
- Input opened a fresh X11 connection for each operation and only flushed before
  closing it. Some events never reached the server. The backend now synchronizes
  before disconnecting. The receiver verifies key, left-button and wheel press/
  release pairs and the exact relative movement (100,100)→(124,127). The fixture
  initializes the XTEST pointer before input and drains its setup events.

The input fix adds one X-server round trip per operation. The final changes had
an independent review with no remaining material findings. No wire protocol,
app identifiers, signing settings or lockfiles changed.

Final machine-readable summaries are `artifacts/linux-validation/build-summary.txt`
and `runtime-summary.txt`, with every exit code zero. Raw pre-fix evidence is
retained in `attempt1/`, `attempt2/`, `post-clipboard/`, `xtest-diagnostic/` and
`xtest-coordinates/`; the successful targeted pass is in `xtest-sync/`.
The workspace sibling `artifacts/pc-remote-identity-xtest-repeat.log` records the
32-repeat input check, and `pc-remote-identity-linux-final.log` records the final
complete wrapper. The emitted Linux binary SHA-256 is
`6434dcb5b7825e87f32780ee34136c4e91e79e3115cb13366ea6ec79ed9519ad`.

Real system logind power actions, Wayland, physical audio devices and Linux
package installation remain outside this isolated validation. The MPRIS service
is a deterministic D-Bus fixture, audio uses a real PulseAudio null sink, and
logind checks cover safe unavailability. Headless portal services reported
display warnings; the embedded UI readiness passed, while portal/file-picker
integration was not exercised.

Platform prerequisite reference: [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/).

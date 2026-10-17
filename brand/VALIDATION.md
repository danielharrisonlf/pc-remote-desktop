# Identity validation — 2026-10-17

Candidate based on desktop `07c3c4e` and Android `d79d03b`; changes remain in
the working trees. Android target: `pc-remote-android`, not the separate
`pc-remote-android-kotlin` repository.

| Check | Result |
| --- | --- |
| Canonical SVG → generated assets | All 28 outputs reproduce byte-for-byte |
| Windows ICO | Seven valid PNG-compressed RGBA entries: 16, 24, 32, 48, 64, 128, 256px |
| Tauri PNGs | 32, 128, 256, 512px plus 1024px master |
| Android adaptive foreground | Bounds x=28–80dp, y=31.9–78dp; maximum radius 32.67dp inside the 33dp safe radius |
| Android adaptive/themed XML | API 26 foreground/background, API 33 monochrome, both launcher resource names |
| Android legacy fallbacks | 48/72/96/144/192px, regular and round |
| Independent visual/platform review | Passed: tiny icon recognition, masks, XML, ICO decoding and preserved configuration |
| Windows frontend tests | 38 passed across 5 files |
| TypeScript | `npx tsc --noEmit` passed |
| Frontend production build | `npm run build` passed |
| Windows native release | Explicit installed MSVC toolchain build passed |
| Linux native library tests | 105 passed, including the normally ignored route smoke |
| Linux backend runtime | 7 passed; XTEST also passed 32 consecutive repetitions |
| Linux native release/UI smoke | Passed with embedded frontend and default WebKit sandbox |
| Android unit tests | 144 passed, 0 failures/errors/skips |
| Android release and size baseline | Both assembled successfully |
| Release APK verification | `apksigner verify --verbose` passed, APK Signature Scheme v2 |

Commands run from their respective project roots:

```powershell
# Desktop
python scripts/generate-icons.py --android ../pc-remote-android --check
npm test
npx tsc --noEmit
npm run build
cargo +stable-x86_64-pc-windows-msvc build --manifest-path src-tauri/Cargo.toml --release --locked

# pc-remote-android/android
.\gradlew.bat testDebugUnitTest assembleRelease assembleSizeBaseline --console=plain
```

The default Windows Rust toolchain was GNU and lacked `dlltool.exe`. The successful
build selected the already installed MSVC toolchain and local MSVC/Windows SDK
environment; no toolchain default or host installation was changed. Its linker
printed informational import-library messages as warnings. Vite reported callback
timing diagnostics; Gradle reported existing deprecation notices. All final commands
above exited successfully.

Outputs:

- Desktop executable: `src-tauri/target/release/pc-remote-desktop.exe`.
- Android release: `../pc-remote-android/android/app/build/outputs/apk/release/app-release.apk`.
- Android size baseline: `../pc-remote-android/android/app/build/outputs/apk/sizeBaseline/app-sizeBaseline.apk`.
- Visual reference: [preview.png](preview.png).
- Linux build/runtime procedure and results: [Linux validation](../docs/linux-validation.md).

App identifiers, Android permissions and manifest, versions, build/signing settings,
desktop updater configuration, lockfiles and network/protocol files were preserved.
The only Android Kotlin change selects the dedicated white notification icon in place
of the launcher resource. Existing signing selection remains in effect.

Linux validation exposed two backend defects that were corrected: image clipboard
hashing now consistently uses decoded RGBA and dimensions; input waits for X11
requests to be processed before closing its connection. See the Linux report for
regression evidence, the added X-server round trip, and environment limits.

Session evidence is available in the workspace's sibling `artifacts/` directory as
`pc-remote-identity-{npm-test,tsc,vite,gradle,windows-msvc}.log`; the unsuccessful GNU
attempt is retained as `pc-remote-identity-windows-cargo.log`. Linux evidence lives
inside the desktop repository's ignored `artifacts/linux-validation/` directory.

Semantica tools were unavailable in this session; no durable memory entry was recorded.

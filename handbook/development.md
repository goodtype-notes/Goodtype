[Goodtype](../README.md) / Development

# Build on Windows

Install [Rustup](https://rustup.rs/), [Node.js 25.2.1](https://nodejs.org/) and [pnpm 11.9.0](https://pnpm.io/installation). In [Visual Studio Build Tools 2022](https://visualstudio.microsoft.com/visual-cpp-build-tools/), select **Desktop development with C++** and a Windows 10 or 11 SDK. The desktop app also needs [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/). Rustup uses the toolchain pinned in the repository.

```powershell
git clone https://github.com/goodtype-notes/Goodtype.git
cd Goodtype
git switch dev
pnpm install --frozen-lockfile
cargo xtask doctor
pnpm tauri dev
```

Run `cargo xtask verify` before submitting a change. It checks formatting, Clippy, Rust and frontend tests, release tests, types and the production frontend build.

## Package the app

Run `pnpm tauri build` for the executable. For the portable folder and ZIP, run the packaging script from the repository root.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-windows-demo.ps1
```

The script packages the desktop app and its helpers, verifies a fresh extraction and writes a manifest and checksum beside the ZIP in `target`. Keep the extracted folder together when moving it. Retain the two newest verified local packages and their ZIPs. Preserve notebooks, backups and worktrees under `target`.

## Source map

The [desktop app](../apps/desktop) contains the Tauri shell and Svelte frontend. The [core crate](../crates/goodtype-core) owns notebook persistence, history and ink geometry. The [Typst crate](../crates/goodtype-typst) compiles previews and exports PDFs. The canvas and exporter check their ink geometry against [shared fixtures](../fixtures).

## Branches and releases

1. Branch from `dev` using `feature/*` or `fix/*`. Open a pull request to `dev`. Both long lived branches require passing checks and resolved conversations.
2. Each successful merge into `dev` runs full verification, smoke builds on macOS and Linux, and portable Windows packaging. CI publishes a Windows prerelease with a plain tag such as `v0.3.1`, the ZIP, a JSON manifest and a SHA256 checksum. The workflow artifact is available for 14 days.
3. Download and test that exact candidate on Windows. Record the tag, tests performed, results and accepted limitations. Record any physical pen or touch testing separately from automated package checks.
4. Open a promotion PR from `dev` to `main`. Include the following two lines with the actual tag and evidence.

   ```text
   Tested dev build: v0.3.1
   Test evidence: <actual tests, results and accepted limitations>
   ```

5. The tag must point to the exact head of `dev`. A later commit requires another candidate and test record. Promote with a merge commit. CI checks the tree and marks the same tested release stable with its existing assets and checksums. Synchronize `main` back into `dev` through a PR.

CI chooses the shared base version or the next patch after the highest plain tag on that version line, whichever is higher. A rerun for a tagged commit reuses the version. Published assets are immutable. To begin a new minor line, update `Cargo.toml`, `Cargo.lock`, `apps/desktop/package.json` and `apps/desktop/src-tauri/tauri.conf.json` together through `dev`. The project remains in early development until an explicit readiness decision.

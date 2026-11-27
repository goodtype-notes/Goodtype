# Signed addon registry format 2

The first-party registry release publishes `index-v2.json` and `index-v2.json.sig` alongside
unchanged format-1 assets. Both indexes use the same key, sequence, issued and expires values.
See `addon-index-v2.schema.json` for the wire shape. Index and addon objects are closed.
Release objects may add optional `minGoodtype` (plain x.y.z) and manifest `requires` features.
Unknown release fields and unsupported requirements make that release incompatible, not the index.

Hosts fetch format 2 first and use format 1 only after the format-2 index request returns 404.
Signature download failures, bad signatures, expiry and rollback never trigger fallback. Each
signature comes from the tagged registry release that served its index. An index must carry the
format of the file it was served as, so a signed format-1 index under the format-2 name is rejected. The cache records its
format; both formats share one highest accepted sequence, with separate digests at that sequence.
The bundled offline snapshot remains format 1.

Rust chooses the numerically newest release compatible with API, target, features and runtime
app version. The running version is Tauri package_info().version, the same source used by portable
updates; scripts/release.mjs writes it through target/release-config.json. Local versions with
-local metadata skip only minGoodtype. Missing features still block local builds.

Blocked newer versions appear in the store with an Update Goodtype action opening Settings >
App updates and checking through the existing updater. A beta caveat appears when betas are off.
Installed versions remain available; automatic discovery reports skipped incompatible updates.
Rollback refuses a previous version that the index lists as needing a newer Goodtype; versions the
index does not list are left to the manifest checks.


Portable Wasm releases use target `wasm32-goodtype-v0`, `requires` containing `wasm-v0`, and
`wasm: {module: "module.wasm", sha256: "<64 lowercase hex>", abi: "goodtype-wasm-v0"}`.
The same module descriptor appears in the package manifest and signed `addon-update.json`;
the host binds index to signed descriptor and descriptor to staged manifest before activation.
The archive name is `<package>-<version>-wasm32-goodtype-v0.zip` (hybrids keep windows-x64).
The ZIP retains the existing `<package>-<version>-windows-x64/` internal root so extraction,
staging and rollback paths stay unchanged even for portable archives. Archive sha256 remains the ZIP digest. Module sha256 covers the raw module bytes, not the ZIP.
Normal builds regard `wasm-v0` as unsupported until the gate described in addon-api-v0.md is lifted.
Format 1 must not publish Wasm releases. The registry repository must add `wasm-v0` to its
feature-to-first-shipping-version table when an accepted host release actually ships the tier;
do not assign a production version while W1 remains open.

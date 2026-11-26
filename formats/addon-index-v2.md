# Signed addon registry format 2

The first-party registry release publishes `index-v2.json` and `index-v2.json.sig` alongside
unchanged format-1 assets. Both indexes use the same key, sequence, issued and expires values.
See `addon-index-v2.schema.json` for the wire shape. Index and addon objects are closed.
Release objects may add optional `minGoodtype` (plain x.y.z) and manifest `requires` features.
Unknown release fields and unsupported requirements make that release incompatible, not the index.

Hosts fetch format 2 first and use format 1 only after the format-2 index request returns 404.
Signature download failures, bad signatures, expiry and rollback never trigger fallback. Each
signature comes from the tagged registry release that served its index. The cache records its
format; both formats share one highest accepted sequence, with separate digests at that sequence.
The bundled offline snapshot remains format 1.

Rust chooses the numerically newest release compatible with API, target, features and runtime
app version. The running version is Tauri package_info().version, the same source used by portable
updates; scripts/release.mjs writes it through target/release-config.json. Local versions with
-local metadata skip only minGoodtype. Missing features still block local builds.

Blocked newer versions appear in the store with an Update Goodtype action opening Settings >
App updates and checking through the existing updater. A beta caveat appears when betas are off.
Installed versions remain available; automatic discovery reports skipped incompatible updates.

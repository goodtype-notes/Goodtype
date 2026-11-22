# Goodtype addon API 0.1

## Status and stability

`goodtype-api 0.1` is the first-party Windows x64 beta contract. Step 1 implements
the built-in catalog and retains the legacy importer CLI adapter. Step 2
implements manifest v1 validation and the device-local Addons manager. Step 2b
implements index format 1. Step 3a implements the host side of session v0,
ink-selection commands, addon-owned proposals and direct apply. Step 3b runs
`stdio-jsonrpc` sessions in the restricted AppContainer launcher. Legacy
importers stay on the Job Object path for this beta. Settings descriptors
remain pending.

During beta, the host MUST require an exact match to its current
`goodtype-api 0.N` for declarations and handshakes. An incompatible installed
addon MUST be unavailable with a visible Needs update reason. Beta revisions MAY
break compatibility and MUST list each breaking change in the changelog.

At the 1.0 freeze, Goodtype MUST publish a conformance kit and a fixed
deprecation window. From 1.0 onward, changes within a major MUST be additive.
Peers MUST negotiate capabilities, ignore unknown fields only where the
relevant contract explicitly allows them, and handle unknown enum values
safely. A field MUST retain its meaning. The host MUST support the current
and previous major, with pinned fixture addons in CI for every supported
API version.

## Terminology and canonical content

- **Host** means the Rust authority for package trust, execution, scope and persistence.
- **Catalog** means the built-in allowlist of addon identities, repositories,
  executable identities, release keys and pins when available. Today it lists
  Goodnotes, OneNote and math recognition. Math has neither a key nor a pin.
- **Manifest** means the package's declaration of contributions and maximum scope.
- **Grant** means the scope the user authorizes for a particular invocation.

Addons MUST read, produce or transform only ordinary Goodtype content, such as
ink, Typst, images, PDFs, pages and notebooks. The core owns every canonical
object kind and format. Saved content MUST remain readable, editable and
exportable with the addon disabled, incompatible or removed. Rust MUST validate
addon output and retain sole authority for canonical writes. Page commands
default to reviewed proposals. Direct apply requires explicit invocation, an
eligible command and both global and per-addon opt-ins at invocation and commit.
Rust MUST recheck scope and revision under the commit lock, commit as one undo
step and offer Undo in a notice naming the addon.

## Package layout and manifest v1

**Implemented (Step 2).** A ZIP MUST contain one catalog-named top-level directory
(`<package>-<version>-windows-x64/`) with `goodtype-addon.json` at its root,
the approved executable and any verified runtime files. The host MUST verify
the trusted release and archive digest before extraction. Limits are 512 MiB
each for compressed and expanded bytes, 20,000 entries and path depth 24.
The host MUST reject unsafe paths, Windows aliases and collisions, device names,
links and reparse points, and verify the inventory and executable before
activation. The manifest MUST agree with the catalog and verified release.
Only the exact verified legacy releases MAY use a host-owned compatibility
declaration in place of the manifest. A failed verification or health check
MUST preserve the active installation.

The manifest MUST be UTF-8 JSON, at most 64 KiB. All top-level fields below are
required. Objects are closed and readers MUST reject unknown fields, duplicate
JSON keys and duplicate declaration IDs within each declaration list.

| Field | Required value |
| --- | --- |
| `manifestVersion` | Integer `1`. |
| `id` | Catalog addon identifier. |
| `version` | Verified release version, three dot-separated decimal components, at most 32 characters. |
| `apiVersion` | Exact host API string, `goodtype-api 0.1` for this contract. |
| `target` | `x86_64-pc-windows-msvc`. |
| `entrypoint` | Forward-slash relative path to the catalog-approved `.exe` inside the verified package. |
| `transport` | Catalog-approved `legacy-import-cli` or `stdio-jsonrpc`. |
| `permissions` | Unique array of catalog-approved literals from the vocabulary below. |
| `commands` | 1–16 catalog-approved commands, each with `id`, `label` and `surface` (`library-import` or `ink-selection`). |
| `settings` | 0–16 descriptors, each with `id`, `label`, `type`, `default` and optional `hint` and `search` keys. |
| `strings` | Exactly `en` and `de` string tables with identical key sets and at most 128 keys per table. |

Identifiers and localization keys MUST use ASCII lowercase kebab case
(letters and digits with single hyphen separators, 1–48 characters). Each
locale MUST contain `name` and all referenced keys. Text MUST contain 1–256
Unicode characters. Markup delimiters (`<`, `>`), control characters and
interpolation tokens (`{{`, `}}`) are forbidden. The host MUST render plain
text, use English for unsupported locales and refresh text on language change.

A `toggle` has a Boolean default. A `choice` has a `choices` array of 2–8
objects with a nonempty string `value` and localized `label`. Values MUST be
unique and the default MUST equal one of them. Settings values MUST NOT
denote paths, commands or secrets. The manifest supplies no launch arguments
or environment.

In `goodtype-api 0.1` settings descriptors are reserved: a host MUST reject a
manifest whose `settings` array is not empty. A later beta API version enables
them together with host rendering (rollout Step 4).

Illustrative manifest for a future repackaged OneNote release.

```json
{
  "manifestVersion": 1,
  "id": "onenote",
  "version": "0.1.1",
  "apiVersion": "goodtype-api 0.1",
  "target": "x86_64-pc-windows-msvc",
  "entrypoint": "goodtype-onenote-import.exe",
  "transport": "legacy-import-cli",
  "permissions": ["picked-import-file", "publish-import-batch"],
  "commands": [{"id": "import-notebooks", "label": "run", "surface": "library-import"}],
  "settings": [],
  "strings": {
    "en": {"name": "OneNote Importer", "run": "Import OneNote notebooks"},
    "de": {"name": "OneNote-Import", "run": "OneNote-Notizbücher importieren"}
  }
}
```

[Manifest schema](addon-manifest-v1.schema.json) defines the structural checks.
Rust MUST additionally enforce byte limits, duplicate keys, key parity and
references, ID and choice-value uniqueness, default membership, Windows path
safety and catalog binding. Step 2 MUST ship Rust round-trip fixture tests
against the tracked schema and tests for the additional semantic checks.

For the pinned Goodnotes 0.1.1 and OneNote 0.1.0 archives, the host supplies the
declaration and validates the separate legacy protocol, schema 3 and decoder
contract. Rust validates those notebooks through the normal core reader.
Math has no verified legacy release. A new release needs its own declaration.
For importer packages, the legacy inventory also lists and hashes
`goodtype-addon.json`.

The Addons manager stores policy in the device's `addons.json` beside
`settings.json`. Verified legacy installations are migrated as enabled.
A verified update that changes the declared scope is disabled pending review
and clears the addon's direct-apply consent. Step 2 stores the two direct-apply
preferences. Step 3 will consume them for eligible commands.

## Permissions

**Specified (Step 2).** A permission describes a ceiling. A declaration or
settings control MUST NOT grant access. The host MUST show scope before
dispatch, obtain the relevant grant and supply only granted data.

| Permission | Maximum scope |
| --- | --- |
| `picked-import-file` | Read the staged copy of a file explicitly selected by the user. |
| `derived-import-artwork` | Produce derived artwork from that file with separate user consent. |
| `publish-import-batch` | Return staged ordinary notebooks for Rust to validate and publish in the Library. |
| `selected-ink` | Read a scoped snapshot of the committed selection and page dimensions. |
| `add-page-objects` | Propose supported ordinary objects on the granted page, subject to host validation and commit policy. |

A Windows Job Object bounds process resources and lifetime. Filesystem and
network isolation requires a verified restricted launcher. The host MUST
accurately disclose which boundary applies. The Addons manager shows it per
addon from the installed manifest's transport. Session addons are described as
isolated: no network, read-only access to their own install and a private
scratch directory. Resources Windows makes readable to every AppContainer
(system files, and the registry under `registryRead`) remain readable. Legacy
importers are described as running with the user's file and network access and
limited only in memory and running time.

## Registry index format 1

**Implemented (Step 2b).** The first-party GitHub registry publishes `index.json`
and its detached `index.json.sig`. The index MUST be at most 1 MiB. The host
MUST verify the raw 64-byte Ed25519 signature over the exact index bytes with
the embedded registry key before parsing UTF-8 JSON. Unknown fields, duplicate
JSON keys, duplicate addon IDs and duplicate versions within an addon MUST
be rejected.

| Field | Required value |
| --- | --- |
| `format` | Integer `1`. |
| `sequence` | Nonnegative integer up to 9,007,199,254,740,991. |
| `issued`, `expires` | UTC RFC 3339 timestamps ending in `Z`, with expiry later than issuance. |
| `addons` | Array of entries, each with `id`, `repository`, `publicKey` and `versions`. |
| `repository`, `publicKey` | GitHub path `/owner/repository` and 32-byte Ed25519 key as 64 hexadecimal digits. Both MUST match the built-in catalog. |
| `versions` | Array of releases, each with `version`, `api`, `target`, `url`, `size` and `sha256`. |
| `version`, `api`, `target` | Release version as in the manifest, `goodtype-api 0.N`, and `x86_64-pc-windows-msvc`. |
| `url` | HTTPS GitHub release asset URL under that entry's repository, with path `/owner/repository/releases/download/<tag>/<asset>` and no query or fragment. |
| `size`, `sha256` | Positive ZIP byte length up to 512 MiB and SHA-256 as 64 hexadecimal digits. |

Illustrative index. Release metadata for the future package is fictional.

```json
{
  "format": 1,
  "sequence": 42,
  "issued": "2026-11-21T12:00:00Z",
  "expires": "2027-03-01T00:00:00Z",
  "addons": [{
    "id": "onenote",
    "repository": "/Just8it/Goodtype-OneNote-Importer",
    "publicKey": "73bce6988059fe2d4210d9c6dbb4b99e72b458649be3b9f278e9a45ac24c33",
    "versions": [{
      "version": "0.1.1",
      "api": "goodtype-api 0.1",
      "target": "x86_64-pc-windows-msvc",
      "url": "https://github.com/Just8it/Goodtype-OneNote-Importer/releases/download/v0.1.1/goodtype-onenote-importer-0.1.1-windows-x64.zip",
      "size": 1927930,
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
    }]
  }]
}
```

The [index schema](addon-index-v1.schema.json) defines structural checks. The
host MUST reject unknown addon IDs for use, with a diagnostic, and MUST reject
catalog key mismatches and foreign-repository URLs. Key rotation requires
a Goodtype release. Registry CI MUST check asset existence, size and digest,
the addon's signed descriptor and manifest conformance.

The bundled signed snapshot seeds the rollback floor and provides first-launch
and offline metadata. The host MUST persist the highest accepted sequence and
the SHA-256 of its exact signed bytes. Reject lower sequences, a different
digest at the same sequence, and expired indexes. An identical index refreshes
check timestamps and may restore a missing or damaged cache. Store the rollback
floor and check timestamps separately from the cached signed bytes in app-local
data, with an atomic backup of the state. A missing or damaged cache falls back
to the bundled snapshot while retaining the highest recoverable floor.
Failed checks and expiration MUST leave installed
addons usable offline. Fetch from the fixed registry GitHub Releases endpoint.
Redirects MUST use HTTPS. GitHub's latest endpoint may redirect to the same
asset under a tag in the registry repository. Other redirects MUST use the
existing release asset host allowlist
(`objects.githubusercontent.com`, `release-assets.githubusercontent.com`,
`github-releases.githubusercontent.com`), with at most five redirects.

Automatic checks default on and MUST be disableable. Explain that checks fetch
the public list from GitHub without sending notebook or usage data and that
GitHub sees the request as it would any download. The background check runs at
most once per 24 hours, with a separate manager-open check after one hour.
Persist the last attempt independently of the last success so failed attempts
also throttle after restart. Treat an attempt more than 24 hours ahead of the
current clock as invalid. An explicit check remains available. Discovery MUST choose the newest compatible API and target by
numeric release version. Installation requires explicit user action and MUST
verify the index's size and digest, the independently signed addon descriptor
bound to the same release, and the package checks above before switching the
active pointer. Hash and extract from the same retained archive handle, which
denies concurrent writes and deletion on Windows. Keep the previous verified version for local rollback.

### Addon details and install approval

An addon entry MAY include a closed `docs` object with `commit`, `summary` and
`readme`, plus an optional `preview`. The commit MUST be 40 lowercase hex
digits. Summary MUST contain exactly `en` and `de`, each 1 to 160 Unicode
characters of plain text without control characters. Each asset descriptor
MUST contain `path`, `size` and `sha256`. Paths are relative, with characters
from `[A-Za-z0-9_./-]`, and contain no empty segments, `.` segments or
`..` substrings. Sizes are exact byte counts, capped at 262,144 for README
and 2,097,152 for preview. SHA-256 is 64 hexadecimal digits.

When the user opens the addon details, the host fetches the pinned paths at
the given commit on `raw.githubusercontent.com` over HTTPS without following
redirects. It verifies both assets before caching them together in app-local
data. An addon without `docs` uses the newest compatible release tag to fetch
`README.md` from the same repository. That fallback has the same README size
cap and no digest or preview. Cached details are available offline. The host
renders only restricted Markdown blocks, drops HTML and embedded images, and
preserves only HTTPS links. Optional previews require PNG, JPEG or WebP magic.

The one-step install and update commands record the permissions the user
approved in device-local addon policy before downloading. After verifying the
manifest, the host requests further consent only if the manifest exceeds that
approval. After a successful probe and swap, it enables the addon. Replacing
an unverified installation requires a fresh approval and does not carry over
its direct-apply preference.

## Session v0

**Implemented on the host (Step 3a, launcher Step 3b).** Each session process
starts through the shared AppContainer launcher (`sandbox::launch`). The
executable is the exact verified file in the active install, with host argv
`--session`. Each verified addon version has one stable profile,
`Goodtype.Addon.<id>.<version>`, granted read and execute on its install tree
once, before its first session. Disable, remove, update and rollback revoke the
grant and delete the profile. A marker under `addon-sessions/grants` records
each grant, and startup revokes any grant that no longer belongs to an enabled,
active session install. Sessions hold only the `registryRead` capability
(read-only registry access, approved for math on 2026-11-21); they have no
network capability and no loopback exemption. A fresh per-process directory
under app-local `addon-sessions` is the low-integrity scratch, the working
directory, and TEMP/TMP. It is capped at 256 MiB while the process runs, and
exceeding the cap stops the process with `addon-storage-exceeded`. Startup
removes scratch and per-launch profiles left by a crashed run. The environment
is `minimal_environment()`. The job allows one active process with the 3 GiB
math memory budget per process, kills it on an unhandled exception, and only
the three stdio pipe ends are inherited. Every process the host itself creates
takes one spawn lock while those ends are inheritable. Spawns by third-party
crates or Windows components inside the host process are outside that lock.
Every recursive ACL change is serialized and first rejects reparse points in
the tree and its ancestors. Scratch must not overlap a read-only root or hold
the executable, and batch files are refused. A math install with a session
manifest is never started through the legacy `--serve` process, whatever its
authorization state.
A launch or ACL failure returns `addon-sandbox-failed` with a localized
reason. The host never falls back to an unconfined launch. Deadlines,
retirement and epochs are unchanged. The `progress` phase vocabulary is catalog-defined and is currently
only `recognizing`. A warm worker is keyed by verified version, inventory
digest and authorization context (notebook root and grant kind: the writer's
own command or an assistant grant). Each disable, remove, update or rollback
increments an authorization epoch, which revokes pending proposals and ends the
worker. The host retires a worker before its 65,536th request ID.

For `stdio-jsonrpc`, the host sends the first frame. Each frame MUST be one
UTF-8 JSON-RPC 2.0 object followed by LF, optionally preceded by CR, with at
most 4 MiB before the delimiter. Stdout is reserved for protocol. No batches,
callbacks, child-initiated requests or unsolicited startup frames are allowed.
The host MUST use positive decimal string IDs and allow one in-flight request
per process. Replies MUST match the outstanding ID and contain exactly one of
`result` or `error`. Reject malformed or truncated frames, duplicate JSON
keys, an incorrect JSON-RPC version and illegal sequencing. Drain stderr
independently and retain at most 16 KiB locally for explicit diagnostics.

| Method | Direction and payload |
| --- | --- |
| `initialize` | Host request. Params and result contain `apiVersion`, `capabilities` and `addon` (`id` and `version`). Both sides MUST match the API and verified identity. The child returns a unique subset of the host's capability offer. |
| `command/invoke` | Host request after initialization. Params contain `command`, `scope` and validated addon `settings` keyed by descriptor ID. The first result has `batch.operations`. Selection commands require `page-batch-v0`. |
| `command/progress` | Child notification only with negotiated `progress`. Params contain the outstanding `id`, a catalog-defined `phase` and integer `completed` and `total` with `0 ≤ completed ≤ total` and `total > 0`. Host UI updates MUST be coalesced to at most four per second. |
| `command/cancel` | Host notification with `params.id`, only with negotiated `cancel`. Cancellation precludes publication even if a result races it. The host waits at most 1 s for cooperative shutdown, then terminates the job and ends the session. |

The initial `scope` is `{kind, page, strokes}` with kind `ink-selection`.
The page contains finite positive `widthPt` and `heightPt`. Each of 1–2,000
strokes contains an invocation-local `id`, finite `widthPt` in (0, 100],
and `points` as finite `[x, y]` pairs. Each axis has absolute value at most
100,000 pt. A request MUST contain at most 200,000 point pairs and fit the
frame limit. The host keeps notebook root, page ID, revision, credentials
and unrelated content local.

The batch MUST contain 1–32 operations. Math initially supports only
`{op, source, xPt, yPt, layoutWidthPt}` with `op` equal to
`addTypstBlock`, finite geometry in page points and at most 64 KiB of UTF-8
Typst source per operation. The host assigns canonical IDs, paths, scale and
ordering. Unknown fields or invalid operations MUST cause rejection of the
entire batch. The host MUST validate and compile against the granted inputs
and approved bundled resources before preview or commit. Original ink stays.

| Error code | Meaning |
| --- | --- |
| `-32601` | Unknown method |
| `-32602` | Invalid params |
| `-32603` | Internal error |
| `-32001` | Command failure |
| `-32002` | Cancelled |
| `-32003` | Unsupported recognition or output |

The diagnostic `message` is at most 512 characters. The host MUST map codes
to localized statuses. A well-formed command error may leave the session
usable. A framing, sequencing, timeout, pipe, write or exit failure MUST
invalidate it. The host MUST NOT automatically replay the command.

Initial limits are a 60 s handshake, a 30 s math invocation including writes,
a 600 s math idle limit, a 3 GiB math job budget and bounded queues. Progress
MUST NOT extend deadlines. A warm process MUST be restricted to one verified
version and authorization context. End it on disable, remove, update, shutdown
or a change of notebook or grant.

## Installation security notes

The host checks the archive against the built-in pin or verified release
descriptor and verifies the staged inventory and manifest before running a
probe. A first install or a changed capability scope pauses for approval of the
candidate's declared permissions. Cancel or restart discards the candidate.

A host-written provenance record binds the installed inventory digest to the
verified archive digest. Registry installs record the signed index entry digest.
Legacy packages without an addon manifest also have to match the catalog pin.
Rollback verifies the retained release and its provenance before switching the
active pointer. Earlier installations without this record require reinstalling. A
same-user process able to rewrite app-local data is outside the threat model.

Addon policy is stored with an atomic last-good backup. If neither existing copy
is readable, addon execution and automatic discovery pause until the user
confirms resetting the preferences. Reset restores automatic discovery to its
default of on. Manual discovery remains available during recovery.

Sandbox evidence (Windows 11 26200, local math build
`0.1.0-local.20261121121521`): the shared launcher probe denies reads outside
granted roots and writes outside scratch, and finds no inherited handles. With
zero capabilities Winsock fails to start (`WSAStartup` 10107). With
`registryRead` Winsock starts and `socket()` is denied (10013), for loopback
and external targets alike. The math build reports `--version --json`
inside the container. With zero capabilities, its ONNX Runtime start blocks
indefinitely before the model DLL is mapped (about 5 MB working set, no
output), so the handshake times out. With only the `registryRead` capability,
it loads and recognizes: handshake 2.44 s including ACL setup, cold 0.34 to
0.41 s, warm 0.25 to 0.28 s. ONNX Runtime also logs a denied ETW registration.
The Job Object run took 2.76 s, 0.58 s and 0.46 s. The user approved
`registryRead` for math on 2026-11-21. With it, the signed stable release
`v0.1.0` (SHA-256 `415bc6a8…3d02e38`) recognizes inside the container:
handshake 1.24 s, cold 0.37 s, warm 0.24 s. With stable per-version profiles,
the one-time grant takes about 0.1 s. Later session handshakes took 1.18 to
1.64 s, against 1.83 to 2.29 s with a fresh profile and grant per session.
Most of the remaining handshake time is the addon's own startup.

## Legacy importer CLI adapter

**Implemented (existing adapter, retained in Step 1).** The Goodnotes and
OneNote packages ship `importer-manifest.json`, with release version, protocol
and notebook schema versions, target, decoder commit and file inventory.
The host verifies the package and probes the executable with
`--version --json` (30 s) and `--check` (180 s) during installation.
The first MUST exit successfully and report `name`, `version`,
`protocolVersion`, `notebookSchemaVersion` and `decoderCommit` matching the
expected executable and package values. The importer protocol version is
integer `1`, independent of the beta API string. The self-check MUST exit
successfully.

For import, Rust re-verifies the installed files and invokes
`<executable> <source> <staging>/batch` with staging as working directory and
an optional trailing `--allow-derived` on a supported, consented import.
The current adapter passes the picked source path. The staged input copy
described above belongs to the planned restricted launcher. For this beta the
importers deliberately stay on Job Object supervision, as ADR 0045 allows with
disclosure. That means a 4 GiB job memory limit and a four-hour import deadline,
with no filesystem or network confinement.

The host polls `batch/import-report.json` and reads it again when the
process ends. The file is a JSON array capped at 4 MiB and 500 entries.
Each entry has string `status` and `source`, optional `target` and `error`
strings, and optional nonnegative integer `pages` defaulting to zero.
`pending` is unfinished, `failed` carries a per-notebook failure and only
`completed` is eligible for publication. A completed entry MUST identify a
safe relative target within the batch. Rust validates each notebook before
publishing it to the Library, keeps existing destination notebooks and
can retain valid completed results after cancellation.

## Changelog

| API | Changes | Breaking changes |
| --- | --- | --- |
| `goodtype-api 0.1` | Initial beta specification for manifest v1, index format 1 and session v0. Documents the retained CLI adapter. | Initial version. |

Step 2 implemented strict manifest validation, exact legacy declarations, and the local Addons manager without changing the 0.1 wire contract.

Step 2b implemented signed GitHub registry discovery, a bundled offline snapshot,
local rollback protection and optional automatic checks. Installs from the index
verify the archive against the index and the addon's independently signed
descriptor before package validation.

Step 3a implemented the session v0 host client, `ink-selection` commands from
manifest data, validated `page-batch-v0` (`addTypstBlock` only) compiled
without notebook disk fallback or package downloads, and Rust-owned addon
proposals. Insertion works with AI disabled and is one undo step. Direct apply
requires both opt-ins at invocation and commit. A math install declaring
`stdio-jsonrpc` uses `--session`, and other installs keep the legacy path.
The wire contract is unchanged.

Step 3b moved session v0 processes onto the shared AppContainer launcher and
added the `addon-sandbox-failed` command problem. The Addons manager now
discloses the actual boundary for each addon. The wire contract is unchanged.

Step 3b hardening moved session installs to stable per-version profiles with a
one-time tree grant and added the host-side `addon-storage-exceeded` problem
for the 256 MiB scratch cap. The wire contract is unchanged.

Math catalog: Ed25519 key from `Just8it/Goodtype-ink-recognition` (0.1.0), no bootstrap pin; math installs from the signed index (index-3 onward, now also the bundled snapshot). Math sessions hold `registryRead` only (user-approved 2026-11-21). No wire change.

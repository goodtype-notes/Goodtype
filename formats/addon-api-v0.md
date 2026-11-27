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
| `entrypoint` | Forward-slash relative path to the catalog-approved `.exe` inside the verified package. Absent for content packs. |
| `transport` | Catalog-approved `legacy-import-cli`, `stdio-jsonrpc`, or `content`. |
| `contents` | Required for content packs and forbidden for process transports. Lists the files or Typst package directories the pack contributes. |
| `permissions` | Unique array of catalog-approved literals from the vocabulary below. |
| `commands` | 1–16 catalog-approved commands, each with `id`, `label` and `surface` (`library-import`, `ink-selection`, `insert-form`, `page-generator`, or `export`). |
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

Session manifests may declare up to 16 settings using `type: "toggle"` or
`type: "choice"`. Each setting has an `id`, a `label` key, optional `hint` and
`search` keys, and a required `default`. A choice supplies 2–8 entries with
unique kebab-case `value` tokens and localized `label` keys. The default must
be one of the values. A toggle defaults to a Boolean. These are device-local
preferences and do not grant capabilities. CLI importers retain empty settings.

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
| `read-pages` | Read the selected pages for an explicit export. |
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
    "repository": "/goodtype-notes/Goodtype-OneNote-Importer",
    "publicKey": "73bce6988059fe2d4210d9c6dbb4b99e72b458649be3b9f278e9a45ac24c33",
    "versions": [{
      "version": "0.1.1",
      "api": "goodtype-api 0.1",
      "target": "x86_64-pc-windows-msvc",
      "url": "https://github.com/goodtype-notes/Goodtype-OneNote-Importer/releases/download/v0.1.1/goodtype-onenote-importer-0.1.1-windows-x64.zip",
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
| `initialize` | Host request. Params and result contain `apiVersion`, `capabilities` and `addon` (`id` and `version`). Both sides MUST match the API and verified identity. The child returns a unique subset of the host's capability offer (`progress`, `cancel`, `page-batch-v0`, `path-edit-v0`, `scoped-reads-v0`). |
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

### Declarative session contributions

`insert-form` and `page-generator` require `add-page-objects` and a `form` array
of up to 12 fields. A form supports the settings field types plus single-line
`text` (`maxLength` 1–512 Unicode characters, optional `default`, optional
`pattern` of `any`, `number` or `identifier`) and `number` (finite `min`,
`max`, positive `step`, and an aligned `default`). The host sends
`{command, form, settings, page: {widthPt, heightPt}, anchor: {x, y}}` with
exactly the declared field values and no notebook content.

Insert and generator results use `{batch: {operations: […]}}`. The host
limits the total to 32 object operations, including group children, and accepts:

- `{op: "addTypstBlock", source, xPt, yPt, layoutWidthPt}`.
- `{op: "addShape", x, y, geometry, style}` using canonical shape geometry and style.
  The drawn outline, measured by its Bézier extrema rather than its handles,
  must stay half the stroke width inside the page.
- `{op: "addGroup", children}` with one or more blocks or shapes and no nested group.
- `{op: "addPage", template, ops}` exclusively for a generator. Each result
  has 1–16 such operations. A null template means plain paper. Its `ops`,
  which may be empty, contain objects and groups on the requested page size.

Rust assigns IDs and validates the entire proposal before it can be published.
A generated batch is reviewed and committed together at the selected position.
Insert proposals may skip review only when both direct-apply settings permit it.

An `export` command requires `read-pages` and a
`format: {extension, label, assets?}` descriptor. The lowercase alphanumeric
extension is 1–8 characters and cannot be `goodtype`. After the user selects
current page or all pages and chooses a destination, the host sends
`{command, settings, pages}`. Each page is canonical JSON with inline ink
layers and Typst `source` replacing `sourcePath`. Image and PDF content is
omitted unless `assets` is true, in which case the referenced assets are
included in a path-keyed base64 map. The result is `{bytes: "<base64>"}`.
Both directions remain within the session frame budget. A successful reply
is decoded and atomically written to the single chosen destination.

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
successfully. `notebookSchemaVersion` may be any schema the core reader accepts
(1 up to the current one); every imported notebook is validated by that reader
before it is published. Signed release descriptors follow the same rule; only
importers carry `protocolVersion`, `notebookSchemaVersion` and a 40-hex
`decoderCommit`. Session addons (`stdio-jsonrpc`) are bound by their
`goodtype-api` manifest and handshake, and their descriptors carry an empty
`decoderCommit` (amended 2026-11-24 after the goodtype-notes releases).

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

## Content packs

Content packs use the same signed ZIP, staged install, activation and rollback as
other addons. They omit `entrypoint` and supply empty `permissions`, `commands`
and `settings`. No executable probe or addon process runs. Debug builds can
load a local folder for a catalog-known content addon. Registry releases use
`content-update.json` and `content-update.json.sig` beside the ZIP. The
descriptor uses the release archive and source fields without importer version
fields and with an empty `decoderCommit`.

`contents` is an object with optional `shapes`, `templates`, `palettes`,
`themes` and `typstPackages` arrays. At least one must be nonempty. JSON
contributions live at `content/<kind>/<file>.json` with at most 32 files per
kind and 1 MiB per file. The entire ZIP expands to at most 16 MiB. JSON rejects
unknown fields and duplicate keys. Labels and search keywords refer to the
manifest's English and German string tables.

- Shape libraries provide `id`, `label` and `items`. An item includes `id`,
  `label`, `search`, `widthPt`, `heightPt` and up to 64 `objects`. A shape
  object supplies `type: "shape"`, local `x` and `y`, core `geometry` and
  partial `style`. A label supplies `type: "typst"`, local `x` and `y`,
  `source` (up to 2 KiB), `layoutWidthPt` and `scale`. The host bounds shapes
  by their geometry, including the actual Bézier extrema. Strokes may
  extend beyond the item. Typst labels may overhang each edge by half
  the larger item dimension. Use `fillColor: "pen"` to inherit the active pen
  colour, `null` for no fill, or a hex colour for a fixed fill. Shape files
  sit directly in `content/shapes/`. A pack may contain up to 512 items. Inserted
  geometry, source and pen defaults become ordinary notebook objects.
- Template files contain a `templates` array of `{label, template}` entries.
  Definitions are ordinary page templates whose IDs are the addon ID, a period
  and a kebab-case identifier. A pack may contain up to 32 templates.
- Palette files contain a `palettes` array of `{id, label, colors}` entries,
  each with 2–24 unique six-digit hex colours. A pack may contain up to 16 palettes.
- Theme files contain a `themes` array of `{id, label, tokens}` entries. Only
  the app's configurable UI colour tokens accept six-digit hex or
  `rgb(r g b / a%)` values. The host checks text, muted and quiet contrast
  on the panel and surround, and on-accent contrast on blueprint, at 4.5:1.
  A pack may contain up to 8 themes. Missing themes return to Default.
- A shape item with two explicit ports can declare `stretch` (ADR 0049).
  `axis` lists the port indices in left-to-right order on a horizontal axis.
  `roles` lists one role per object in order. A `fixedBody` rule uses one or
  more `body` parts, a `startLead` and an `endLead`, and optionally a
  `throughLine`. A body with a through-line and no leads requires
  `virtualSockets`, two distances from the start port. Optional `leadFloors`
  default to 4 pack points per end. A `span` rule uses only `span` parts
  made from lines, open splines or square-corner rectangles and stops at
  half the authored span or 4 pack points, whichever is larger. At least
  one span part must meet both ports. The host stores the resolved rule on
  a Group even for a single shape. Publish these packs after the host
  supports the declaration.
- Typst package directories must be `content/typst/preview/<name>/<version>`
  with matching `typst.toml`. A pack may contain up to 16. Goodtype resolves
  regular local Typst packages first, then app-owned offline copies, then
  downloads when allowed. Offline copies support saved labels after addon removal.

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

Math catalog: Ed25519 key from `goodtype-notes/Goodtype-Math-Recognition` (formerly `Just8it/Goodtype-ink-recognition`) (0.1.0), no bootstrap pin; math installs from the signed index (index-3 onward, now also the bundled snapshot). Math sessions hold `registryRead` only (user-approved 2026-11-21). No wire change.

Step 5 adds the `content` transport and `contents` to manifest v1. Existing
process manifests and the API version are unchanged.
Step 4 activates session settings and adds insert forms, page generators and
export formats, the `read-pages` permission and the three page-batch operations
above. These changes are additive within `goodtype-api 0.1`.

The Step 4/5 integration validates `addShape` objects and content-pack shapes
through one host path. Content-pack descriptors omit `protocolVersion` and
`notebookSchemaVersion` and carry an empty `decoderCommit`. No wire change.

## Object data, tools and reactions (Step 3c)

`own-objects` permits tools and own-object commands. `react-to-changes`
permits reactions to changes in referenced objects. Rust validates both against
the installed manifest, catalog ceiling and current policy.

Tools declare an ID, label key, stroke-only path icon, kind (`place`, `connect`
or `frame`), snap mode (`none` or `ports`) and optional setting IDs. The host
sends one `tool/apply` per gesture. Parameters include the tool ID, settings,
page ID and size, points with optional object/port attachments, and snapshots
of attached objects. Ports in snapshots use page points. A tool declaring
`"stroke": true` receives a validated core `ShapeStyle` in `style`. The host
initializes a device-local slot from the pen in hand and retains it by tool ID.
Tools without a stroke slot still receive `pen` with the active pen's colour
and width.

Choice options may declare an `icon` with the same stroke-path rules as a tool
icon. The host presents up to five fully icon-labelled choices or up to four
short text choices as segments when room permits. In a vertical or crowded
rail, those segments move into the tool card. Other choices appear as lists
and toggles as switches in that card. Settings ? Palette lists enabled addon
tools and retains visibility by `addon:<id>/<tool>`.

An `own-object`
command receives the selected objects owned by the addon and snapshots of
their refs. A `page/changed` reaction receives the page ID and size, changed
referenced snapshots, deleted IDs, and the affected objects owned by the addon.

These methods return a `page-batch-v0` result with an `ops` array. Create ops
may attach `addonData` under the calling addon's key. Shape and group create
ops may also store object-local ports. `updateObject` and `deleteObject` require
an existing entry for the caller. Own-object commands and reactions can update
or delete objects in the request scope. An unauthorized op rejects the batch
with `-32002`. An empty `ops` array leaves the page unchanged.

Shape-pack items may define up to 32 finite points as `ports`, relative to
the item origin. Insertion copies them to the created shape or group and
accounts for its placement scale and local origin.

Reactions within 750 ms amend the triggering undo step while its revision
is current. A late result is discarded and can be requested once after at
least one second without another commit. A chain visits each addon at most
once and has at most two rounds. Disabled or unavailable addons leave page
content in place. Detach removes one addon's data from selected objects in
one undo step.

For local development, the debug Addons page accepts a catalog-known session
addon folder containing its manifest and Windows executable. The same
permission approval and AppContainer session apply to local installs.

### Path edits (ADR 0046 amendment)

A catalog-approved `own-object` command with `own-objects` may declare
`"pathEdit":{"choice":"routing","snap":"ports"}`. The choice names a declared
choice setting. The session MUST negotiate `path-edit-v0` before use. The
minimum Connectors version is 0.2.0 and its session always requires the capability.

On `command/invoke` the host sends `command`, device `settings`, the page ID
and dimensions (`page.id`, `widthPt`, `heightPt`), and the selected source
`object`. The source contains `id`, `type`, `x`, `y`, `rotation`,
`scale`, `geometry`, `style`, and only the calling addon's existing
`addonData` entry, keyed by addon ID when present. There is no notebook path or
unrelated page data. `refs` contains scoped part snapshots (ID, type, transform,
bounds, page-space ports and shape geometry when applicable), limited to the
source's live references and the newly dropped target. `points` contains two
ordered page-space points, each with `attach` (null or `{objectId,port}`).
Only the edited end can have a supplied attachment. The addon resolves the
unchanged end from the caller's entry and the snapshots. The `edit` is either
`{kind:"choice",value}` or `{kind:"endpoint",end:"start"|"end"}`. A choice
does not change device settings.

Return exactly one `updateObject` in `batch.operations` with the captured ID,
line or open-spline `geometry`, and non-null single-entry `addonData`.
Refs must be a subset of the grant and include any new dropped target.
Preserve the source transform, style, order and group membership. A recognized
legacy conversion may supply the captured full style with missing caps filled
in on an already-owned spline. Existing explicit caps are preserved.

A displayed port drop commits directly in one undo step. Routing choices and
free endpoint releases follow proposal review or the two direct-apply opt-ins.

## Minimum host features

Manifests may declare `requires: ["end-caps", "group-stretch", "path-edit-v0", "scoped-reads-v0"]`.
The field is optional and omitted when empty. The host accepts at most 16 unique
identifier-safe feature names (lowercase ASCII letters/digits separated by hyphens,
using the same bounded identifier validation as contribution IDs). Unknown requirements
refuse installation and enablement with "Needs a newer Goodtype" and the feature names.
An installed incompatible package stays on disk but contributes nothing and cannot launch.
Content packs use the same gate. `wasm-v0` is accepted only by builds with the
`wasm-tier` Cargo feature (see the gated Wasm tier); other builds refuse it the same way.

This host implements caps through core ShapeStyle and shared frontend/export cap geometry,
stretch through core stretch rules and frontend stretch gestures, and path edits through
Rust revision-bound connector grants and batch validation. Connectors must explicitly
require `path-edit-v0`; command access remains restricted to `change-routing`. Version
numbers no longer grant path-edit access. Existing pre-0.2 packages without this declaration
remain refused; an explicitly declaring package is judged by its contract rather than version.

Released v0.3 hosts deserialize Manifest with `deny_unknown_fields`, so even `requires: []`
is refused there with their existing generic manifest error. Keep the field absent for packages
that need no new feature and should work on those hosts.

Released format 1 indexes stay strict. [Index format 2](addon-index-v2.md) carries
per-release requirements and minimum host versions for pre-download selection.
The package manifest remains the installation gate. Scoped reads additionally
negotiate `scoped-reads-v0` at handshake. Shared manifest conformance inputs and
expected outcomes live in `fixtures/addon-requires`; sibling repositories can copy
or fetch them at a pinned commit.

### Reaction fan-out

After a committed change, the host partitions affected owned objects in page order into
calls of at most 32 objects, capped at 16 chunks (512 objects) across all addons per committed change.
An addon may receive several calls for the same captured page revision and referenced
object snapshot. Each call sees only changed snapshots and deleted IDs referenced by its own
objects (a referenced snapshot carries its ports); unknown dependencies disclose nothing. Each reply still has the 32-operation ceiling and may only modify its
chunk's owned objects. All replies are validated before one history amendment is published;
a failed chunk leaves the user's change intact. A late reaction is retried once on idle.


### Scoped geometry reads (`scoped-reads-v0`)

Native sessions MUST declare `requires: ["scoped-reads-v0"]` (alongside any other
requirements) and negotiate the capability of the same name. Only object commands,
tools, path edits and reaction chunks receive reads. The host adds a `reads` object
to their invocation params:

```json
{"reads":{"revision":7,"groups":["symbol"],"region":{"x":0,"y":0,"width":214,"height":204},"maxObjects":256,"maxBytes":65536,"maxCalls":16,"maxTotalBytes":262144,"maxRegionSidePt":1024.0}}
```

`groups` lists only group IDs already present in this invocation's admitted object
snapshots (selected owned objects and admitted references/changed objects). A group
read grants no writes or independent grant on a newly discovered nested group.
`region` is the axis-aligned envelope of admitted objects and completed gesture
points, expanded by 144 page points (two inches) and clipped to the captured page.
With no such geometry it is empty. Queries MUST fit wholly inside this region and
have positive width/height of at most 1024 points each. Bounds and ports are in page
points; boxes include object transforms and groups enclose their explicit members.

While the invocation is pending, the addon MAY send these JSON-RPC requests over
its existing stdout and read the response from stdin. IDs MUST be nonempty strings
of at most 80 bytes; use an addon prefix to distinguish host request IDs. Calls are
sequential; finish a read before sending the invocation result.

```json
{"jsonrpc":"2.0","id":"addon-read-1","method":"group/descendants","params":{"revision":7,"groupId":"symbol"}}
{"jsonrpc":"2.0","id":"addon-read-1","result":{"revision":7,"objects":[{"id":"child","type":"shape","parentId":"symbol","x":30.0,"y":40.0,"rotation":0.0,"scale":2.0,"bounds":{"x":30.0,"y":40.0,"width":40.0,"height":20.0},"ports":[{"x":30.0,"y":50.0}],"geometry":{"kind":"rectangle","widthPt":20.0,"heightPt":10.0,"cornerRadiusPt":0.0}}]}}
```

Descendants follow only explicit `childIds`, recursively. Each result identifies its
immediate `parentId`. Siblings are emitted in child-list order, then nested groups
are visited in reverse sibling order. The requested root itself is omitted. Only
IDs, kinds, transforms, bounds, ports and shape geometry are returned. No source,
text, styles, timestamps, ink samples, assets or `addonData` are exposed.

```json
{"jsonrpc":"2.0","id":"addon-read-2","method":"region/obstacles","params":{"revision":7,"bounds":{"x":20.0,"y":20.0,"width":100.0,"height":100.0}}}
{"jsonrpc":"2.0","id":"addon-read-2","result":{"revision":7,"objects":[{"id":"unrelated","type":"shape","bounds":{"x":30.0,"y":40.0,"width":40.0,"height":20.0},"ports":[{"x":30.0,"y":50.0}]}]}}
```

Obstacles include all page objects whose axis-aligned bounds intersect the query
(inclusive edges), in page object order, including explicit group boxes and members.
The example shows one entry; all matching entries are returned. Each entry has
**exactly** `id`, `type`, `bounds`, `ports`; no shape geometry, source, text, ink
points, asset paths/bytes or addon data. This bounded geometry read deliberately
includes unrelated parts within the granted region, without granting object access.
Loose ink strokes are not page objects and are not included.

Both methods read an immutable snapshot captured for that invocation, never a newer
page. `revision` MUST equal `reads.revision`; existing batch commit preconditions
still reject edits if the page changed meanwhile. Grants and budgets expire when
the invocation finishes, even for a warm session. Request objects are closed.

Limits apply to both methods: at most 256 objects and 64 KiB of UTF-8 serialized
`result` per response; at most 16 requests (including rejected requests) and 256 KiB
of successful serialized results per invocation. These limits bound routing work,
large symbol reads and repeated queries without approaching the 4 MiB frame limit.
A limit failure returns no partial list: routing MUST NOT mistake truncation for a
clear route. A semantic refusal is:

```json
{"jsonrpc":"2.0","id":"addon-read-1","error":{"code":-32004,"message":"out-of-scope"}}
```

Other fixed protocol reason tokens are `revision-mismatch`, `limit-exceeded`, and
`invalid-params`. Malformed RPC frames, unsupported methods or reads without a live
grant fail the session. Reads share the invocation deadline and cancellation rules.
These are protocol diagnostics, not host UI strings.

Connectors should query the completed route's bounding region with useful clearance,
clamped to `reads.region` and `maxRegionSidePt`, and use unrelated boxes as obstacles
and the indexed page-space ports as endpoints. Exclude its own path and handle
endpoint objects/groups and members consistently to avoid routing into their boxes.
On a cap failure, retry a smaller useful region within the call budget or use a
conservative routing fallback; never assume an error means no obstacles. The host
still validates the returned batch and its captured revision at release/commit.
A Wasm entry receives no `reads` in this step (see the gated Wasm tier below); this feature
changes no notebook format.
The shared requirement fixture includes the feature; wire conformance cases are in
`fixtures/addon-scoped-reads/requests.json`.


## Gated Wasm tier (`goodtype-wasm-v0`)

The first-party runtime (Extism, Wasmtime, wasmparser) is compiled only with
`--features wasm-tier`. Without it, manifests and registry entries still parse, but a Wasm entry,
`requires: ["wasm-v0"]` or a `wasm32-goodtype-v0` package is refused as needing a newer
Goodtype, never as corrupt. Native entries default to `native`
and retain their existing transport, permissions and validation. This is still API 0.1:
execution selection does not change canonical notebook data.

A module descriptor is `"wasm": {"module":"module.wasm", "sha256":"<64 lowercase hex>",
"abi":"goodtype-wasm-v0"}`. Commands and tools optionally select `"entry":"wasm"`;
manifest-level `"reaction":"wasm"` and `"preview":"wasm"` select the corresponding exports.
Omitted entries use native execution. Preview `native` means the ordinary host preview.
Wasm entries require the descriptor and `wasm-v0`; a descriptor with no Wasm entries is refused.
Only capability-free object permissions (`own-objects`, `react-to-changes`, `add-page-objects`)
are admitted. Catalog ceilings still apply. Paths are fixed to `module.wasm` at the package root.

A portable package uses target `wasm32-goodtype-v0`, omits the native `entrypoint`, and selects
Wasm for every declared command/tool and for reactions when permitted. A hybrid Windows package
keeps its native target/entrypoint and selects each entry independently. Inventory sizes/hashes,
signed ZIP verification, catalog binding, staging, install approval and rollback remain in force.
The module's bytes are checked against the manifest digest before compilation or Worker delivery.
Declared exports must exist. No paths or URLs are accepted by the frontend module command.

`commit({revision, gesture, scope, settings})` receives Rust-captured native invocation parameters
in `scope`; `gesture` is `tool` or `command`. `react({revision, changed, owned, refs, page})`
receives the captured chunk's changed and owned object snapshots; `refs` contains deleted dependency
IDs, and `page` contains only page geometry. Both return the existing page-batch envelope. Rust
owns revision/authorization checks and admits the result through the same batch, ownership,
path-edit, proposal and undo rules as native replies. No executable is launched for a Wasm entry.
Wasm entries receive no scoped reads in this step: neither payload carries `reads`, and a module
has no way to issue `group/descendants` or `region/obstacles`. Obstacle-aware Connectors routing
under Wasm is therefore still open; until it is designed, obstacle routing needs the native session.

Admission: module <= 1 MiB; input/output <= 32 KiB each; one non-shared 32-bit memory with an
explicit maximum <= 128 standard pages; at most one table, explicitly bounded <= 1024 entries;
no start section. Only the spike's vetted `extism:host/env` kernel imports are allowed (no config,
vars, clocks, random, logging, filesystem, HTTP, process or secrets). SIMD, threads and relaxed
SIMD are disabled. Exported functions are `() -> i32`: preview plus commit or react; only
preview/commit/react, memory, __data_end and __heap_base may be exported. Rust uses 1 M call fuel,
2 M initialization fuel and a 25 ms timeout, WASI off, empty host/path lists and no custom hosts.
Compiled modules are cached by digest with one cache entry. Malformed, trapped or over-budget
output never reaches persistence.

Live drag previews use a dedicated bundled Worker with rehashed bytes, no Tauri bridge, a 5 s
startup watchdog and a 16 ms call watchdog. Failure terminates the Worker and removes marks,
leaving the ordinary drag preview. Each Worker pins one digest for its gesture. Addon availability
changes (disable, remove, rollback, install/update) terminate the preview, including pending
module loads, so it cannot continue showing old-module marks after an addon replacement.
The host admits at most 128 marks and 512 total path vertices
(including spline handles), bounded finite coordinates/transforms (absolute value <= 100000,
positive scale), and <= 32 KiB output. Marks have x/y/rotation/scale and geometry:
`polyline {points}`, `bezier {points: [start, control1, control2, end]}`, `label {text}`,
`ring {radius}`, or the W1 open `spline {nodes, closed:false}`. Each geometry includes `kind`.
Labels are plain text, <= 256 UTF-16 units, without control characters; the host never inserts HTML.
Ring radii are positive and <= 4096. Optional style `{color,width}` defaults to #2563eb/1.5;
color is a six-digit hex or black/white/red/blue/green/orange; finite widths clamp to [0.25,8].
The SVG accessibility name identifies the owning addon. Marks are ephemeral, never notebook data.

### W1 production gate: single lifting procedure

Until packaged WebView2 parity and real pen/mixed-gesture measurements in the W1 report pass,
normal builds must not enable `wasm-tier` and must keep the normal Tauri CSP unchanged.
Developer testing explicitly opts into **both** `--features wasm-tier` and
`--config src-tauri/tauri.wasm-spike.conf.json` (dev URL port 1435); the old `wasm-spike`
feature remains an alias. Production frontend bundles omit the Worker unless explicitly built
with `VITE_WASM_TIER=1`; use that flag only with the opt-in Rust build and experimental CSP.
The flag never enables Rust execution. There is no environment-variable sideload loader.
After the outstanding W1 evidence is accepted, enable `wasm-tier` in the shipping build and apply
only the experimental config's `wasm-unsafe-eval`/`worker-src 'self'` CSP additions to the normal
config in one reviewed change. Do not claim production acceptance from Rust or Node tests.

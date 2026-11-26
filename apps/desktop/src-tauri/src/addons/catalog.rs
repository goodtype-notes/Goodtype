use super::{Addon, Kind, PinnedRelease};

/// v0.1.1, published 2026-10-21 from commit bb93582f (clean tree). The hash was computed from an
/// independent download and agrees with the release's `SHA256SUMS`, its descriptor JSON and
/// GitHub's asset digest. The package then installed through `install_archive`, passed its own
/// `--check`, and imported the private sample notebooks (see the opt-in test below). Re-pin only
/// after repeating that.
pub static GOODNOTES: Addon = Addon {
    id: "goodnotes",
    legacy: Some("0.1.1"),
    en: ("Goodnotes Importer", "Import Goodnotes notebooks"),
    de: ("Goodnotes-Import", "Goodnotes-Notizbücher importieren"),
    i18n_prefix: "addons.goodnotes",
    name: "goodtype-goodnotes-import",
    package: "goodtype-goodnotes-importer",
    repository: "/goodtype-notes/Goodtype-Goodnotes-Importer",
    // Matches that repository's release-public-key.pem.
    public_key: Some([
        0x42, 0x8c, 0xc6, 0x8f, 0x6d, 0x0b, 0x38, 0xea, 0xe7, 0x4d, 0xcb, 0x21, 0x1e, 0x48, 0x77,
        0xc6, 0xeb, 0x51, 0x54, 0x8d, 0xaa, 0xbf, 0x24, 0xde, 0x64, 0xa1, 0x85, 0x88, 0x5c, 0x82,
        0x5b, 0x3c,
    ]),
    pinned: Some(PinnedRelease {
        version: "0.1.1",
        url: "https://github.com/goodtype-notes/Goodtype-Goodnotes-Importer/releases/download/v0.1.1/goodtype-goodnotes-importer-0.1.1-windows-x64.zip",
        size: 14_881_982,
        sha256: "ef8bfd150626eab885cf1479131e5945fcfd2a9da4152aab3a03311507db5742",
        decoder_commit: "66028455fd7fc61b7931679915d811a25485e3ad",
    }),
    contribution_surfaces: &[],
    kind: Kind::Import {
        filter: "Goodnotes",
        extensions: &["goodnotes", "zip"],
        batch: "Goodnotes Import",
        artwork: true,
    },
};

/// v0.1.0, published 2026-10-22 from commit 9806bbd2 (clean tree). The hash was computed from an
/// independent download and agrees with the release's `SHA256SUMS`, its signed descriptor and
/// GitHub's asset digest; the package installed through `install_archive`, passed `--check` and
/// imported OneNote sample sections (see the opt-in test below). Re-pin only after repeating that.
pub static ONENOTE: Addon = Addon {
    id: "onenote",
    legacy: Some("0.1.0"),
    en: ("OneNote Importer", "Import OneNote notebooks"),
    de: ("OneNote-Import", "OneNote-Notizbücher importieren"),
    i18n_prefix: "addons.onenote",
    name: "goodtype-onenote-import",
    package: "goodtype-onenote-importer",
    repository: "/goodtype-notes/Goodtype-OneNote-Importer",
    // Matches that repository's release-public-key.pem; distinct from the Goodnotes key.
    public_key: Some([
        0x73, 0xbc, 0xe6, 0x98, 0x80, 0x1a, 0x59, 0xfe, 0x2d, 0x42, 0x3f, 0xd9, 0xc6, 0xdb, 0xa8,
        0x68, 0x7f, 0xe3, 0xb9, 0x2f, 0x60, 0xbb, 0x9f, 0x27, 0x49, 0x9e, 0x72, 0x42, 0xc2, 0x33,
        0x6c, 0x3b,
    ]),
    pinned: Some(PinnedRelease {
        version: "0.1.0",
        url: "https://github.com/goodtype-notes/Goodtype-OneNote-Importer/releases/download/v0.1.0/goodtype-onenote-importer-0.1.0-windows-x64.zip",
        size: 1_927_930,
        sha256: "a09f7899eef1280819a7da0b12e1355a76e2a71c774e738b0945e07bac9a63dc",
        decoder_commit: "727389a674665dc07e0c31818cd4b54b64b9c684",
    }),
    contribution_surfaces: &[],
    kind: Kind::Import {
        filter: "OneNote",
        extensions: &["one", "onetoc2", "onepkg", "zip"],
        batch: "OneNote Import",
        artwork: false,
    },
};

pub static MATH: Addon = Addon {
    id: "math",
    legacy: None,
    en: ("Math Recognition", "Recognize math"),
    de: ("Formelerkennung", "Formel erkennen"),
    i18n_prefix: "addons.math",
    name: "goodtype-math-recognize",
    package: "goodtype-math-recognizer",
    repository: "/goodtype-notes/Goodtype-Math-Recognition",
    // Raw Ed25519 key from that repository's release-public-key.pem (v0.1.0, 2026-11-21; kept
    // when the repository moved to goodtype-notes). No bootstrap pin: math is installed from the
    // signed registry index (index-3 onward).
    public_key: Some([
        0xc9, 0xbe, 0xee, 0xed, 0x66, 0x4e, 0x24, 0x5b, 0x76, 0x77, 0x13, 0x80, 0xed, 0x37, 0xb6,
        0x6c, 0x94, 0x05, 0xd4, 0x45, 0xe2, 0x88, 0x0a, 0xf9, 0x35, 0x6d, 0x13, 0xa3, 0x84, 0x95,
        0x55, 0x81,
    ]),
    pinned: None,
    contribution_surfaces: &[
        super::manifest::Surface::InsertForm,
        super::manifest::Surface::PageGenerator,
    ],
    kind: Kind::Recognize {
        recognizes: "math",
        check_failure: "math recognizer self-check failed",
    },
};

pub(super) static CATALOG: &[&Addon] = &[
    &GOODNOTES,
    &ONENOTE,
    &MATH,
    &ENGINEERING,
    &STUDY,
    &CONNECTORS,
];

/// Connectors between parts (ADR 0046): one `connect` tool, re-routing as a reaction and
/// own-object commands. Runs in the same AppContainer session as math. Installed from the signed
/// registry index (index-4 onward), like math.
pub(super) static CONNECTORS: Addon = Addon {
    id: "connectors",
    legacy: None,
    en: ("Connectors", "Connect parts"),
    de: ("Verbinder", "Teile verbinden"),
    i18n_prefix: "addons.connectors",
    name: "goodtype-connectors",
    package: "goodtype-connectors",
    repository: "/goodtype-notes/Goodtype-Connectors",
    // Raw Ed25519 key from that repository's release-public-key.pem (v0.1.0, 2026-11-24).
    public_key: Some([
        0x21, 0x05, 0x97, 0xa7, 0xb6, 0xad, 0x9c, 0x6d, 0x9c, 0xeb, 0x51, 0x44, 0xd8, 0xef, 0xdf,
        0xf2, 0xa1, 0xb3, 0x87, 0x35, 0x2a, 0xe2, 0xf7, 0x07, 0x91, 0x92, 0x4a, 0x43, 0x7f, 0xdf,
        0xb4, 0xb7,
    ]),
    pinned: None,
    contribution_surfaces: &[super::manifest::Surface::OwnObject],
    kind: Kind::Objects {
        permissions: &[
            super::manifest::Permission::OwnObjects,
            super::manifest::Permission::ReactToChanges,
        ],
        tools: &[("connect", super::manifest::ToolKind::Connect)],
    },
};

pub(super) static ENGINEERING: Addon = Addon {
    id: "engineering-shapes",
    legacy: None,
    en: ("Engineering Shapes", ""),
    de: ("Technische Formen", ""),
    i18n_prefix: "addons.engineeringShapes",
    name: "",
    package: "goodtype-engineering-shapes",
    repository: "/goodtype-notes/Goodtype-Engineering-Shapes",
    // Raw Ed25519 key from that repository's release-public-key.pem (v0.1.0, 2026-11-24).
    public_key: Some([
        0x3c, 0xfb, 0xfe, 0x38, 0xce, 0xe6, 0xbb, 0xc1, 0xda, 0x2c, 0x63, 0xca, 0xff, 0x5b, 0x8e,
        0xf1, 0xf0, 0xf7, 0x6d, 0xb6, 0xdf, 0x0c, 0x1c, 0x0f, 0x17, 0x63, 0x72, 0x87, 0x81, 0xb7,
        0x82, 0xd9,
    ]),
    pinned: None,
    contribution_surfaces: &[],
    kind: Kind::Content,
};

pub(super) static STUDY: Addon = Addon {
    id: "study-templates",
    legacy: None,
    en: ("Study Templates", ""),
    de: ("Lernvorlagen", ""),
    i18n_prefix: "addons.studyTemplates",
    name: "",
    package: "goodtype-study-templates",
    repository: "/goodtype-notes/Goodtype-Study-Templates",
    // Raw Ed25519 key from that repository's release-public-key.pem (v0.1.0, 2026-11-24).
    public_key: Some([
        0x3c, 0x7b, 0x9d, 0x34, 0x94, 0x06, 0x05, 0x53, 0x25, 0xee, 0x05, 0x15, 0x65, 0x05, 0x32,
        0x8d, 0x76, 0x3e, 0x3c, 0x68, 0x4c, 0xdb, 0xc3, 0x6d, 0x1c, 0x36, 0x1a, 0x59, 0xc9, 0x2a,
        0x7a, 0xcd,
    ]),
    pinned: None,
    contribution_surfaces: &[],
    kind: Kind::Content,
};

impl Addon {
    pub(super) fn allows_path_edit(&self, command: &str) -> bool {
        self.id == CONNECTORS.id && command == "change-routing"
    }
}

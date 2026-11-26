//! ADR 0050 W1: experimental only, without registry admission.
#![cfg_attr(not(test), allow(dead_code))] // W1 executes commit only in the gate harness.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use wasmparser::{ExternalKind, Parser, Payload, TypeRef};

pub const MODULE_LIMIT: usize = 1024 * 1024;
pub const IO_LIMIT: usize = 32 * 1024;
const KERNEL: &[&str] = &[
    "alloc",
    "free",
    "length",
    "length_unsafe",
    "load_u8",
    "load_u64",
    "store_u8",
    "store_u64",
    "input_length",
    "input_load_u8",
    "input_load_u64",
    "output_set",
    "error_set",
];

pub fn validate(bytes: &[u8], digest: &str) -> Result<(), String> {
    if bytes.len() > MODULE_LIMIT || format!("{:x}", Sha256::digest(bytes)) != digest {
        return Err("module size or digest".into());
    }
    let mut features = wasmparser::WasmFeatures::MVP
        | wasmparser::WasmFeatures::MUTABLE_GLOBAL
        | wasmparser::WasmFeatures::SIGN_EXTENSION
        | wasmparser::WasmFeatures::BULK_MEMORY
        | wasmparser::WasmFeatures::REFERENCE_TYPES
        | wasmparser::WasmFeatures::MULTI_VALUE
        | wasmparser::WasmFeatures::SATURATING_FLOAT_TO_INT;
    features.set(wasmparser::WasmFeatures::FLOATS, true);
    wasmparser::Validator::new_with_features(features)
        .validate_all(bytes)
        .map_err(|e| e.to_string())?;
    let (mut memories, mut tables, mut preview, mut commit) = (0, 0, false, false);
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let mut entrypoints = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(|e| e.to_string())? {
            Payload::TypeSection(section) => {
                for ty in section.into_iter_err_on_gc_types() {
                    types.push(ty.map_err(|e| e.to_string())?);
                }
            }
            Payload::FunctionSection(section) => {
                for ty in section {
                    functions.push(ty.map_err(|e| e.to_string())?);
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    let import = import.map_err(|e| e.to_string())?;
                    if import.module != "extism:host/env"
                        || !KERNEL.contains(&import.name)
                        || !matches!(import.ty, TypeRef::Func(_))
                    {
                        return Err("denied import".into());
                    }
                    if let TypeRef::Func(index) = import.ty {
                        functions.push(index);
                    }
                }
            }
            Payload::MemorySection(section) => {
                for memory in section {
                    let memory = memory.map_err(|e| e.to_string())?;
                    memories += 1;
                    if memories > 1
                        || memory.shared
                        || memory.memory64
                        || memory.initial > 128
                        || memory.maximum.is_none_or(|max| max > 128)
                        || memory.page_size_log2.is_some()
                    {
                        return Err("memory ceiling".into());
                    }
                }
            }
            Payload::TableSection(section) => {
                for table in section {
                    let table = table.map_err(|e| e.to_string())?.ty;
                    tables += 1;
                    if tables > 1
                        || table.initial > 1024
                        || table.maximum.is_none_or(|max| max > 1024)
                    {
                        return Err("table ceiling".into());
                    }
                }
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.map_err(|e| e.to_string())?;
                    match (export.name, export.kind) {
                        ("preview", ExternalKind::Func) => {
                            preview = true;
                            entrypoints.push(export.index);
                        }
                        ("commit", ExternalKind::Func) => {
                            commit = true;
                            entrypoints.push(export.index);
                        }
                        ("memory", ExternalKind::Memory)
                        | ("__data_end" | "__heap_base", ExternalKind::Global) => {}
                        _ => return Err("denied export".into()),
                    }
                }
            }
            Payload::StartSection { .. } => return Err("start section".into()),
            _ => {}
        }
    }
    if memories != 1 || !preview || !commit {
        return Err("missing ABI exports or memory".into());
    }
    for index in entrypoints {
        let ty = functions
            .get(index as usize)
            .and_then(|index| types.get(*index as usize))
            .ok_or("export type")?;
        if !ty.params().is_empty() || ty.results() != [wasmparser::ValType::I32] {
            return Err("export signature".into());
        }
    }
    Ok(())
}

type Cache = Mutex<HashMap<String, Arc<extism::CompiledPlugin>>>;
static CACHE: OnceLock<Cache> = OnceLock::new();

// Release builds have no sideload path at all.
#[cfg(debug_assertions)]
pub fn sideload() -> Result<(Vec<u8>, String), String> {
    let path = std::env::var_os("GOODTYPE_WASM_MODULE").ok_or("no sideload")?;
    let digest = std::env::var("GOODTYPE_WASM_SHA256").map_err(|_| "no pinned digest")?;
    let bytes = crate::store::read_bounded(std::path::Path::new(&path), MODULE_LIMIT)?;
    validate(&bytes, &digest)?;
    Ok((bytes, digest))
}

#[cfg(debug_assertions)]
#[tauri::command]
pub fn addon_wasm_bytes(
    app: tauri::AppHandle,
    addon_id: String,
    digest: String,
) -> Result<Vec<u8>, String> {
    use tauri::Manager;
    if addon_id != "connectors" {
        return Err("addon-unavailable".into());
    }
    let importers = app.state::<super::Importers>();
    let (job, addon) = importers.get(&addon_id)?;
    super::selection::authorization(&app, job, addon)?;
    let (bytes, pinned) = sideload()?;
    if digest != pinned {
        return Err("module digest".into());
    }
    Ok(bytes)
}

pub fn compile(bytes: &[u8], digest: &str) -> Result<Arc<extism::CompiledPlugin>, String> {
    validate(bytes, digest)?;
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(module) = cache.get(digest) {
        return Ok(module.clone());
    }
    let manifest = extism::Manifest::new([extism::Wasm::data(bytes.to_vec())])
        .with_timeout(Duration::from_millis(25))
        .with_memory_max(128)
        .with_allowed_hosts(std::iter::empty())
        .with_allowed_paths(std::iter::empty());
    let compiled = extism::PluginBuilder::new(manifest)
        .with_wasi(false)
        .with_cache_disabled()
        .with_fuel_limit(1_000_000)
        .with_initialization_fuel_limit(2_000_000)
        .compile()
        .map_err(|e| e.to_string())?;
    let compiled = Arc::new(compiled);
    cache.clear();
    cache.insert(digest.into(), compiled.clone());
    Ok(compiled)
}

pub fn call(
    module: &extism::CompiledPlugin,
    export: &str,
    input: &[u8],
) -> Result<Vec<u8>, String> {
    if !matches!(export, "preview" | "commit") || input.len() > IO_LIMIT {
        return Err("input ceiling".into());
    }
    let mut plugin = extism::Plugin::new_from_compiled(module).map_err(|e| e.to_string())?;
    let output: &[u8] = plugin.call(export, input).map_err(|e| e.to_string())?;
    if output.len() > IO_LIMIT {
        return Err("output ceiling".into());
    }
    serde_json::from_slice::<serde_json::Value>(output).map_err(|e| e.to_string())?;
    Ok(output.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[cfg(windows)]
    fn resident_bytes() -> u64 {
        let command = format!("(Get-Process -Id {}).WorkingSet64", std::process::id());
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &command])
            .output()
            .unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn guest(body: &str) -> Vec<u8> {
        wat::parse_str(format!("(module (memory (export \"memory\") 1 128) (func (export \"preview\") (result i32) {body}) (func (export \"commit\") (result i32) i32.const 0))")).unwrap()
    }
    #[test]
    fn rejects_bad_digest_and_size() {
        let bytes = guest("i32.const 0");
        assert_eq!(
            validate(&bytes, "bad").unwrap_err(),
            "module size or digest"
        );
        // A valid ABI plus an oversized custom section: size is the only violation.
        let mut large = bytes.clone();
        large.push(0); // custom section
        let mut length = MODULE_LIMIT + 1; // empty name plus padding
        loop {
            let byte = (length & 0x7f) as u8;
            length >>= 7;
            large.push(byte | if length > 0 { 0x80 } else { 0 });
            if length == 0 {
                break;
            }
        }
        large.push(0); // empty custom-section name
        large.resize(large.len() + MODULE_LIMIT, 0);
        assert_eq!(
            validate(&large, &hash(&large)).unwrap_err(),
            "module size or digest"
        );
    }

    #[test]
    fn rejects_effects_memory_tables_and_start() {
        let abi = "(func (export \"preview\") (result i32) i32.const 0) (func (export \"commit\") (result i32) i32.const 0)";
        for (property, memory, reason) in [
            (
                "(import \"wasi_snapshot_preview1\" \"fd_write\" (func))",
                "(memory 1 128)",
                "denied import",
            ),
            (
                "(import \"extism:host/env\" \"config_get\" (func))",
                "(memory 1 128)",
                "denied import",
            ),
            ("", "(memory 1 129)", "memory ceiling"),
            ("", "(memory 1)", "memory ceiling"),
            ("(table 1 funcref)", "(memory 1 128)", "table ceiling"),
            ("(table 1 1025 funcref)", "(memory 1 128)", "table ceiling"),
            (
                "(table 1 1 funcref) (table 1 1 funcref)",
                "(memory 1 128)",
                "table ceiling",
            ),
            (
                "(func $start) (start $start)",
                "(memory 1 128)",
                "start section",
            ),
        ] {
            let bytes = wat::parse_str(format!("(module {property} {memory} {abi})")).unwrap();
            assert_eq!(
                validate(&bytes, &hash(&bytes)).unwrap_err(),
                reason,
                "{property} {memory}"
            );
        }
        let bytes = guest("i32.const 0");
        validate(&bytes, &hash(&bytes)).unwrap();
    }

    #[test]
    fn traps_stuck_calls_and_malformed_output_are_errors() {
        for body in [
            "unreachable",
            "(loop $forever br $forever) i32.const 0",
            "i32.const 0",
        ] {
            let bytes = guest(body);
            let compiled = compile(&bytes, &hash(&bytes)).unwrap();
            let start = std::time::Instant::now();
            assert!(call(&compiled, "preview", b"{}").is_err());
            assert!(start.elapsed() < Duration::from_secs(2));
        }
    }

    #[test]
    #[ignore = "requires GOODTYPE_WASM_MODULE; sideload-only W1 measurement"]
    fn measure_router() {
        let bytes = std::fs::read(std::env::var("GOODTYPE_WASM_MODULE").unwrap()).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        println!("module_bytes={} digest={digest}", bytes.len());
        #[cfg(windows)]
        let before_memory = resident_bytes();
        let start = std::time::Instant::now();
        let compiled = compile(&bytes, &digest).unwrap();
        println!("cold_compile_ms={}", start.elapsed().as_secs_f64() * 1000.0);
        let fixtures: Value = serde_json::from_slice(
            &std::fs::read(std::env::var("GOODTYPE_WASM_FIXTURES").unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(fixtures["digest"], digest);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/notebooks/minimal");
        for fixture in fixtures["fixtures"].as_array().unwrap() {
            let mut snapshot = goodtype_core::storage::open_notebook(&root).unwrap();
            // Capture an ordinary-object fixture; Rust supplies the revision to the envelope.
            snapshot.page.revision = fixture["commit"]["revision"].as_u64().unwrap();
            let scope = &fixture["commit"]["scope"];
            let mut sources = scope["objects"]
                .as_array()
                .or_else(|| scope["changed"].as_array())
                .unwrap()
                .clone();
            if let Some(own) = scope["own"].as_array() {
                sources.extend(own.clone());
            }
            for original in fixtures["fixtures"][0]["commit"]["scope"]["objects"]
                .as_array()
                .unwrap()
            {
                if !sources.iter().any(|object| object["id"] == original["id"]) {
                    sources.push(original.clone());
                }
            }
            for source in sources {
                let mut object = json!({"id": source["id"], "type":"shape", "x":source["bounds"]["x"].as_f64().unwrap_or(0.0),
                    "y":source["bounds"]["y"].as_f64().unwrap_or(0.0), "rotation":0,"scale":1,"zIndex":0,"readingOrder":0,
                    "createdAt":"2026-11-26T00:00:00Z","modifiedAt":"2026-11-26T00:00:00Z",
                    "geometry":{"kind":"rectangle","widthPt":48,"heightPt":16,"cornerRadiusPt":0}, "style":{"strokeColor":"#182c39","strokeWidthPt":1.5,"fillColor":null,"opacity":1}});
                if source.get("geometry").is_some() {
                    for key in [
                        "x",
                        "y",
                        "rotation",
                        "scale",
                        "geometry",
                        "style",
                        "addonData",
                    ] {
                        if let Some(v) = source.get(key) {
                            object[key] = v.clone();
                        }
                    }
                }
                object["readingOrder"] = json!(snapshot.page.objects.len());
                object["zIndex"] = json!(snapshot.page.objects.len());
                if let Some(width) = source["bounds"]["width"].as_f64() {
                    object["geometry"]["widthPt"] = json!(width);
                }
                let object: goodtype_core::PageObject = serde_json::from_value(object).unwrap();
                snapshot.page.reading_order.push(object.fields().id.clone());
                snapshot.page.objects.push(object);
            }
            let mut request = fixture["commit"].clone();
            request["revision"] = json!(snapshot.page.revision);
            let result = call(&compiled, "commit", &serde_json::to_vec(&request).unwrap()).unwrap();
            assert_eq!(
                std::str::from_utf8(&result).unwrap(),
                fixture["commitRaw"].as_str().unwrap(),
                "byte-exact Wasmtime/V8 parity"
            );
            let result: Value = serde_json::from_slice(&result).unwrap();
            let native: Value =
                serde_json::from_str(fixture["nativeReply"].as_str().unwrap()).unwrap();
            assert_eq!(
                result, native["result"],
                "native/V8/Wasmtime commit parity: {}",
                fixture["name"]
            );
            super::super::batch::apply_owned(&root, snapshot, "connectors", None, result).unwrap();
            let preview = call(
                &compiled,
                "preview",
                &serde_json::to_vec(&fixture["preview"]).unwrap(),
            )
            .unwrap();
            assert_eq!(
                std::str::from_utf8(&preview).unwrap(),
                fixture["previewRaw"].as_str().unwrap()
            );
        }
        let mut plugin = extism::Plugin::new_from_compiled(&compiled).unwrap();
        let mut times = Vec::new();
        let fixtures = fixtures["fixtures"].as_array().unwrap();
        for index in 0..620 {
            let input = serde_json::to_vec(&fixtures[index % fixtures.len()]["preview"]).unwrap();
            let start = std::time::Instant::now();
            let output: &[u8] = plugin.call("preview", &input).unwrap();
            assert!(output.len() <= IO_LIMIT);
            if index >= 20 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            std::thread::sleep(Duration::from_micros(16667).saturating_sub(start.elapsed()));
        }
        times.sort_by(f64::total_cmp);
        println!(
            "rust_60hz_samples={} p50_ms={} p95_ms={} p99_ms={}",
            times.len(),
            times[299],
            times[569],
            times[593]
        );
        #[cfg(windows)]
        println!(
            "native_process_working_set_delta_bytes={}",
            resident_bytes().saturating_sub(before_memory)
        );
    }
}

//! Plugin discovery (mirrors `VstPluginScanner`).
//!
//! - `*.vst3` bundles resolve to the binary inside (prefer `x86_64-win`),
//!   listed once; inner binaries are skipped.
//! - `*.dll` files are accepted when they are 64-bit PE files exporting
//!   `VSTPluginMain` or legacy `main` (parsed by hand, no deps).
//! - Results sorted VST3-then-VST2, name-ascending.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateFormat {
    Vst2,
    Vst3,
}

#[derive(Debug, Clone)]
pub struct PluginCandidate {
    pub name: String,
    pub path: PathBuf,
    pub format: CandidateFormat,
}

/// Scan `folder` recursively for loadable plugins.
pub fn scan(folder: &Path) -> Vec<PluginCandidate> {
    let mut out = Vec::new();
    scan_dir(folder, folder, &mut out);
    out.sort_by(|a, b| {
        let rank = |c: &PluginCandidate| match c.format {
            CandidateFormat::Vst3 => 0,
            CandidateFormat::Vst2 => 1,
        };
        (rank(a), a.name.to_lowercase()).cmp(&(rank(b), b.name.to_lowercase()))
    });
    out
}

fn scan_dir(root: &Path, dir: &Path, out: &mut Vec<PluginCandidate>) {
    let entries = fs::read_dir(dir)
        .map(|r| r.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    for entry in entries {
        let path = entry.path();
        // Skip anything inside a `.vst3` bundle: bundles are handled whole.
        if path != root
            && path
                .ancestors()
                .skip(1)
                .any(|a| a.extension().and_then(|x| x.to_str()) == Some("vst3"))
        {
            continue;
        }
        if path.is_dir() {
            if path.extension().and_then(|x| x.to_str()) == Some("vst3") {
                if let Some(binary) = resolve_vst3_bundle(&path) {
                    out.push(PluginCandidate {
                        name: bundle_display_name(&path),
                        path: binary,
                        format: CandidateFormat::Vst3,
                    });
                }
                continue; // Never descend into bundles.
            }
            scan_dir(root, &path, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("dll") {
            if is_vst2_dll(&path) {
                out.push(PluginCandidate {
                    name: path
                        .file_stem()
                        .and_then(|x| x.to_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    path: path.clone(),
                    format: CandidateFormat::Vst2,
                });
            }
        } else if path.extension().and_then(|x| x.to_str()) == Some("vst3") && path.is_file() {
            // Loose single-file `.vst3` module (not a bundle directory).
            out.push(PluginCandidate {
                name: path
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                path: path.clone(),
                format: CandidateFormat::Vst3,
            });
        }
    }
}

fn bundle_display_name(bundle: &Path) -> String {
    bundle
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// `Bundle.vst3/Contents/<arch>-win/Binary.vst3`, preferring `x86_64-win`.
fn resolve_vst3_bundle(bundle: &Path) -> Option<PathBuf> {
    let contents = bundle.join("Contents");
    for arch in ["x86_64-win", "x86-win", "arm64-win"] {
        let dir = contents.join(arch);
        if let Ok(entries) = fs::read_dir(&dir) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            files.sort();
            if let Some(first) = files.into_iter().next() {
                return Some(first);
            }
        }
    }
    None
}

/// True when `path` is an AMD64 PE with a `VSTPluginMain` or `main` export.
pub fn is_vst2_dll(path: &Path) -> bool {
    let data = fs::read(path).unwrap_or_default();
    pe_exports_vst_entry(&data)
}

fn read_u16_le(data: &[u8], off: usize) -> Option<u16> {
    data.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32_le(data: &[u8], off: usize) -> Option<u32> {
    data.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn pe_exports_vst_entry(data: &[u8]) -> bool {
    // MZ header + PE offset.
    if data.len() < 0x40 || &data[0..2] != b"MZ" {
        return false;
    }
    let pe_off = read_u32_le(data, 0x3C).unwrap_or(0) as usize;
    if data.len() < pe_off + 6 || &data[pe_off..pe_off + 4] != b"PE\0\0" {
        return false;
    }
    // COFF: machine (AMD64 = 0x8664), number of sections, optional size.
    let machine = read_u16_le(data, pe_off + 4).unwrap_or(0);
    if machine != 0x8664 {
        return false;
    }
    let num_sections = read_u16_le(data, pe_off + 6).unwrap_or(0) as usize;
    let opt_size = read_u16_le(data, pe_off + 20).unwrap_or(0) as usize;
    let opt_off = pe_off + 24;
    // Optional header magic: 0x20b = PE32+.
    let magic = read_u16_le(data, opt_off).unwrap_or(0);
    let export_dir_entry = if magic == 0x20b { opt_off + 112 } else { opt_off + 96 };
    let export_rva = read_u32_le(data, export_dir_entry).unwrap_or(0) as usize;
    if export_rva == 0 {
        return false;
    }
    // Section table follows the optional header; map RVA -> file offset.
    let sections_off = opt_off + opt_size;
    let map_rva = |rva: usize| -> Option<usize> {
        for i in 0..num_sections {
            let s = sections_off + i * 40;
            let vaddr = read_u32_le(data, s + 12).unwrap_or(0) as usize;
            let vsize = read_u32_le(data, s + 8).unwrap_or(0) as usize;
            let raw = read_u32_le(data, s + 20).unwrap_or(0) as usize;
            if rva >= vaddr && rva < vaddr + vsize.max(1) {
                return Some(raw + (rva - vaddr));
            }
        }
        None
    };
    let e = match map_rva(export_rva) {
        Some(v) => v,
        None => return false,
    };
    // Export directory: name count at +24, names array RVA at +32.
    let num_names = read_u32_le(data, e + 24).unwrap_or(0) as usize;
    let names_rva = read_u32_le(data, e + 32).unwrap_or(0) as usize;
    if num_names == 0 || num_names > 100_000 || names_rva == 0 {
        return false;
    }
    let arr = match map_rva(names_rva) {
        Some(v) => v,
        None => return false,
    };
    for i in 0..num_names.min(10_000) {
        let name_rva = read_u32_le(data, arr + i * 4).unwrap_or(0) as usize;
        let Some(fo) = map_rva(name_rva) else {
            continue;
        };
        let Some(tail) = data.get(fo..) else {
            continue;
        };
        let end = tail.iter().position(|&b| b == 0).unwrap_or(0);
        let name = &data[fo..fo + end];
        if name == b"VSTPluginMain" || name == b"main" {
            return true;
        }
    }
    false
}

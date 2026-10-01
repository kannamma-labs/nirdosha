use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

fn sha256_file(path: &Path) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut file = std::fs::File::open(path).expect("open template file");
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).expect("read template file");
    hasher.update(&buf);
    format!("{:x}", hasher.finalize())
}

#[derive(serde::Serialize)]
struct FileIntegrity {
    path: String,
    sha256: String,
}

#[derive(serde::Serialize)]
struct TemplateIntegrity {
    id: String,
    files: Vec<FileIntegrity>,
}

#[derive(serde::Serialize)]
struct IntegrityManifest {
    templates: Vec<TemplateIntegrity>,
}

fn main() {
    println!("cargo:rerun-if-changed=templates");

    let templates_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
    let manifest_path = templates_dir.join("manifest.json");

    let mut templates: BTreeMap<String, Vec<FileIntegrity>> = BTreeMap::new();
    for entry in std::fs::read_dir(&templates_dir).expect("read templates dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let id = path.file_name().unwrap().to_string_lossy().to_string();
        let mut files = Vec::new();
        for walk in walkdir::WalkDir::new(&path).sort_by_file_name() {
            let walk = walk.expect("walkdir entry");
            if !walk.file_type().is_file() {
                continue;
            }
            let full = walk.path();
            let rel = full.strip_prefix(&templates_dir).unwrap();
            files.push(FileIntegrity {
                path: rel.to_string_lossy().replace('\\', "/"),
                sha256: sha256_file(full),
            });
        }
        templates.insert(id, files);
    }

    let manifest = IntegrityManifest {
        templates: templates
            .into_iter()
            .map(|(id, files)| TemplateIntegrity { id, files })
            .collect(),
    };

    let json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    std::fs::write(&manifest_path, json).expect("write manifest.json");
}

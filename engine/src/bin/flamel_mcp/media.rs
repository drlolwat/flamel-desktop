use super::*;

pub(super) fn handle(
    name: &str,
    args: &Value,
    _base_url: &str,
    _token: &str,
    _session_id: &str,
    project_path: &str,
) -> Result<String, String> {
    match name {
        // ── Consolidated media tool ──────────────────────────────────────
        "assets" | "media" | "fetch_image" | "fetch_model3d" | "create_model3d" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or(name);
            match action {
                "fetch_model" | "fetch_model3d" => {
                    let url = args.get("url").and_then(|v| v.as_str());
                    let path = args.get("path").and_then(|v| v.as_str());
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();

                    let (model_path, cleanup) = if let Some(u) = url {
                        if !u.starts_with("http://") && !u.starts_with("https://") {
                            return Err(
                                "fetch_model3d: url must start with http:// or https://".into()
                            );
                        }
                        let ext = u
                            .rsplit('.')
                            .next()
                            .unwrap_or("stl")
                            .split('?')
                            .next()
                            .unwrap_or("stl");
                        let ext = if ext.len() > 6 { "stl" } else { ext };
                        let tmp = format!("/tmp/flamel_model_{ts}.{ext}");
                        let dl = flamel::mcp_parent_guard::command("curl")
                            .args([
                                "-s",
                                "-L",
                                "--max-time",
                                "60",
                                "--max-filesize",
                                "52428800",
                                "-o",
                                &tmp,
                                u,
                            ])
                            .output()
                            .map_err(|e| format!("fetch_model3d: curl failed: {e}"))?;
                        if !dl.status.success() {
                            return Err(format!(
                                "fetch_model3d: download failed: {}",
                                String::from_utf8_lossy(&dl.stderr)
                            ));
                        }
                        (tmp, true)
                    } else if let Some(p) = path {
                        (resolve_path(p, project_path), false)
                    } else {
                        return Err("fetch_model3d: provide url or path".into());
                    };

                    let meta = parse_model3d_metadata(&model_path);
                    let render_png = render_model3d_to_png(&model_path, ts);

                    if cleanup {
                        let _ = std::fs::remove_file(&model_path);
                    }

                    match render_png {
                        Some(b64) => {
                            let content = serde_json::json!([
                                {"type": "text",  "text": meta},
                                {"type": "image", "data": b64, "mimeType": "image/png"}
                            ]);
                            Ok(format!("data:multi;{}", content))
                        }
                        None => Ok(meta),
                    }
                }
                "create_model" | "create_model3d" => {
                    let scad = args
                        .get("scad")
                        .and_then(|v| v.as_str())
                        .ok_or("scad required")?;
                    let fmt = args
                        .get("output_format")
                        .and_then(|v| v.as_str())
                        .unwrap_or("stl");
                    let allowed_fmts = ["stl", "off", "amf", "3mf", "csg", "dxf", "svg", "png"];
                    if !allowed_fmts.contains(&fmt) {
                        return Err(format!(
                            "create_model3d: unsupported format '{fmt}'. Use: stl, off, amf, 3mf"
                        ));
                    }
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    let scad_file = format!("/tmp/flamel_scad_{ts}.scad");
                    let out_file = args
                        .get("output_path")
                        .and_then(|v| v.as_str())
                        .map(|p| resolve_path(p, project_path))
                        .unwrap_or_else(|| format!("/tmp/flamel_model_{ts}.{fmt}"));

                    std::fs::write(&scad_file, scad)
                        .map_err(|e| format!("create_model3d: failed to write scad: {e}"))?;

                    // Find openscad — check PATH then common locations
                    let openscad_bin = ["openscad", "/usr/local/bin/openscad", "/usr/bin/openscad"]
                        .iter()
                        .find(|b| {
                            flamel::mcp_parent_guard::command(b)
                                .arg("--version")
                                .output()
                                .map(|o| o.status.success())
                                .unwrap_or(false)
                        })
                        .copied()
                        .ok_or("create_model3d: openscad not found")?;

                    let result = flamel::mcp_parent_guard::command(openscad_bin)
                        .args(["-o", &out_file, &scad_file])
                        .output()
                        .map_err(|e| format!("create_model3d: openscad failed: {e}"))?;

                    let _ = std::fs::remove_file(&scad_file);

                    if !result.status.success() {
                        let stderr = String::from_utf8_lossy(&result.stderr);
                        let stdout = String::from_utf8_lossy(&result.stdout);
                        return Err(format!("OpenSCAD error:\n{stderr}{stdout}"));
                    }

                    let size = std::fs::metadata(&out_file).map(|m| m.len()).unwrap_or(0);
                    let summary = format!(
                        "Created {out_file} ({:.1} KB)\n\n{}",
                        size as f64 / 1024.0,
                        parse_model3d_metadata(&out_file)
                    );

                    // Try to render a preview
                    let render_png = if fmt == "stl" || fmt == "off" || fmt == "amf" || fmt == "3mf"
                    {
                        render_model3d_to_png(&out_file, ts)
                    } else {
                        None
                    };

                    match render_png {
                        Some(b64) => {
                            let content = serde_json::json!([
                                {"type": "text",  "text": summary},
                                {"type": "image", "data": b64, "mimeType": "image/png"}
                            ]);
                            Ok(format!("data:multi;{}", content))
                        }
                        None => Ok(summary),
                    }
                }
                "fetch_image" | "image" => {
                    let url = args
                        .get("url")
                        .and_then(|v| v.as_str())
                        .ok_or("url required")?;

                    // Validate it looks like a URL
                    if !url.starts_with("http://") && !url.starts_with("https://") {
                        return Err(
                            "fetch_image: url must start with http:// or https://".to_string()
                        );
                    }

                    // Download to a temp file, capture content-type via -w
                    let tmp = format!(
                        "/tmp/flamel_img_{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    );

                    let out = flamel::mcp_parent_guard::command("curl")
                        .args([
                            "-s",
                            "-L",
                            "--max-time",
                            "30",
                            "--max-filesize",
                            "10485760",
                            "-w",
                            "%{content_type}",
                            "-o",
                            &tmp,
                            url,
                        ])
                        .output()
                        .map_err(|e| format!("fetch_image: curl failed: {e}"))?;

                    let content_type = String::from_utf8_lossy(&out.stdout).to_string();
                    let content_type = content_type.trim();

                    // Determine MIME type — fall back to sniffing from URL extension
                    let mime = if content_type.starts_with("image/") {
                        // Strip any params like "; charset=..."
                        content_type
                            .split(';')
                            .next()
                            .unwrap_or(content_type)
                            .trim()
                            .to_string()
                    } else {
                        let lower = url.to_lowercase();
                        if lower.ends_with(".png") {
                            "image/png".to_string()
                        } else if lower.ends_with(".gif") {
                            "image/gif".to_string()
                        } else if lower.ends_with(".webp") {
                            "image/webp".to_string()
                        } else if lower.ends_with(".svg") {
                            "image/svg+xml".to_string()
                        } else {
                            "image/jpeg".to_string()
                        }
                    };

                    // Base64-encode the file
                    let b64_out = flamel::mcp_parent_guard::command("base64")
                        .args(["-w", "0", &tmp])
                        .output()
                        .map_err(|e| format!("fetch_image: base64 failed: {e}"))?;

                    let _ = std::fs::remove_file(&tmp);

                    if !b64_out.status.success() {
                        return Err("fetch_image: failed to encode image".to_string());
                    }

                    let b64 = String::from_utf8_lossy(&b64_out.stdout).trim().to_string();
                    if b64.is_empty() {
                        return Err(
                            "fetch_image: downloaded file is empty (bad URL or non-image response)"
                                .to_string(),
                        );
                    }

                    Ok(format!("data:{mime};base64,{b64}"))
                }
                _ => Err(format!(
                    "Unknown media action '{action}'. Use: fetch_image, fetch_model, create_model"
                )),
            }
        }
        _ => Err(format!("Unknown media tool: {name}")),
    }
}

/// Parse metadata from a 3D model file — STL, GLB, GLTF, OBJ. Zero deps, pure Rust.
fn parse_model3d_metadata(path: &str) -> String {
    use std::io::Read;
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let size_str = if file_size > 1_048_576 {
        format!("{:.1} MB", file_size as f64 / 1_048_576.0)
    } else {
        format!("{:.1} KB", file_size as f64 / 1024.0)
    };

    match ext.as_str() {
        "stl" => {
            let mut f = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(e) => return format!("STL — could not open: {e}"),
            };
            let mut header = [0u8; 80];
            if f.read_exact(&mut header).is_err() {
                return "STL — file too small".into();
            }
            // ASCII STL starts with "solid"
            if header.starts_with(b"solid") {
                let mut content = String::new();
                let _ = std::fs::File::open(path)
                    .and_then(|mut f| { f.read_to_string(&mut content) }.map(|_| ()));
                let triangles = content.matches("facet normal").count();
                return format!(
                    "Format: STL (ASCII)\nFile size: {size_str}\nTriangles: {triangles}"
                );
            }
            let mut count_bytes = [0u8; 4];
            if f.read_exact(&mut count_bytes).is_err() {
                return "STL binary — could not read triangle count".into();
            }
            let tri_count = u32::from_le_bytes(count_bytes) as usize;
            // Parse vertices for bounding box
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];
            let mut buf = [0u8; 50];
            for _ in 0..tri_count.min(500_000) {
                if f.read_exact(&mut buf).is_err() {
                    break;
                }
                // normal(12) + v1(12) + v2(12) + v3(12) + attr(2) = 50
                for vi in 0..3usize {
                    let off = 12 + vi * 12;
                    let x = f32::from_le_bytes(buf[off..off + 4].try_into().unwrap_or([0; 4]));
                    let y = f32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap_or([0; 4]));
                    let z = f32::from_le_bytes(buf[off + 8..off + 12].try_into().unwrap_or([0; 4]));
                    if x.is_finite() {
                        min[0] = min[0].min(x);
                        max[0] = max[0].max(x);
                    }
                    if y.is_finite() {
                        min[1] = min[1].min(y);
                        max[1] = max[1].max(y);
                    }
                    if z.is_finite() {
                        min[2] = min[2].min(z);
                        max[2] = max[2].max(z);
                    }
                }
            }
            let dims = if min[0] < max[0] {
                format!(
                    "\nBounding box: {:.2} x {:.2} x {:.2} mm",
                    max[0] - min[0],
                    max[1] - min[1],
                    max[2] - min[2]
                )
            } else {
                String::new()
            };
            format!("Format: STL (binary)\nFile size: {size_str}\nTriangles: {tri_count}{dims}")
        }
        "glb" => {
            let mut f = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(e) => return format!("GLB — could not open: {e}"),
            };
            let mut hdr = [0u8; 12];
            if f.read_exact(&mut hdr).is_err() {
                return "GLB — file too small".into();
            }
            let magic = u32::from_le_bytes(hdr[0..4].try_into().unwrap_or([0; 4]));
            if magic != 0x46546C67 {
                return format!("Not a valid GLB (magic mismatch)");
            }
            // Read JSON chunk header
            let mut chunk_hdr = [0u8; 8];
            if f.read_exact(&mut chunk_hdr).is_err() {
                return "GLB — missing JSON chunk".into();
            }
            let json_len =
                u32::from_le_bytes(chunk_hdr[0..4].try_into().unwrap_or([0; 4])) as usize;
            let mut json_bytes = vec![0u8; json_len.min(1_048_576)];
            let _ = f.read_exact(&mut json_bytes);
            if let Ok(s) = std::str::from_utf8(&json_bytes) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                    let meshes = v["meshes"].as_array().map(|a| a.len()).unwrap_or(0);
                    let mats = v["materials"].as_array().map(|a| a.len()).unwrap_or(0);
                    let nodes = v["nodes"].as_array().map(|a| a.len()).unwrap_or(0);
                    let anims = v["animations"].as_array().map(|a| a.len()).unwrap_or(0);
                    let mut info = format!(
                        "Format: GLB (binary GLTF)\nFile size: {size_str}\nMeshes: {meshes}  Materials: {mats}  Nodes: {nodes}"
                    );
                    if anims > 0 {
                        info.push_str(&format!("  Animations: {anims}"));
                    }
                    if let Some(asset) = v["asset"].as_object() {
                        if let Some(generator) = asset.get("generator").and_then(|v| v.as_str()) {
                            info.push_str(&format!("\nGenerator: {generator}"));
                        }
                    }
                    return info;
                }
            }
            format!("Format: GLB\nFile size: {size_str}")
        }
        "gltf" => {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => return format!("GLTF — could not read: {e}"),
            };
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                let meshes = v["meshes"].as_array().map(|a| a.len()).unwrap_or(0);
                let mats = v["materials"].as_array().map(|a| a.len()).unwrap_or(0);
                let nodes = v["nodes"].as_array().map(|a| a.len()).unwrap_or(0);
                let anims = v["animations"].as_array().map(|a| a.len()).unwrap_or(0);
                let mut info = format!(
                    "Format: GLTF (JSON)\nFile size: {size_str}\nMeshes: {meshes}  Materials: {mats}  Nodes: {nodes}"
                );
                if anims > 0 {
                    info.push_str(&format!("  Animations: {anims}"));
                }
                return info;
            }
            format!("Format: GLTF\nFile size: {size_str}")
        }
        "obj" => {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => return format!("OBJ — could not read: {e}"),
            };
            let verts = content.lines().filter(|l| l.starts_with("v ")).count();
            let faces = content.lines().filter(|l| l.starts_with("f ")).count();
            let groups = content
                .lines()
                .filter(|l| l.starts_with("g ") || l.starts_with("o "))
                .count();
            let mats = content.lines().filter(|l| l.starts_with("usemtl ")).count();
            format!(
                "Format: OBJ\nFile size: {size_str}\nVertices: {verts}  Faces: {faces}  Groups: {groups}  Materials: {mats}"
            )
        }
        _ => format!("Format: {}\nFile size: {size_str}", ext.to_uppercase()),
    }
}

/// Try to render a 3D model to a PNG using f3d + xvfb-run.
/// Returns base64-encoded PNG on success, None if rendering not available.
fn render_model3d_to_png(model_path: &str, ts: u128) -> Option<String> {
    let render_out = format!("/tmp/flamel_render_{ts}.png");
    // Find f3d
    let f3d_bin = ["f3d", "/usr/local/bin/f3d", "/usr/bin/f3d"]
        .iter()
        .find(|b| {
            flamel::mcp_parent_guard::command(b)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
        .copied()?;

    // Try xvfb-run first, then bare f3d
    let result = if flamel::mcp_parent_guard::command("xvfb-run")
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        flamel::mcp_parent_guard::command("xvfb-run")
            .args([
                "-a",
                f3d_bin,
                model_path,
                "--output",
                &render_out,
                "--resolution",
                "800,600",
                "--no-background",
            ])
            .output()
            .ok()?
    } else {
        flamel::mcp_parent_guard::command(f3d_bin)
            .args([
                model_path,
                "--output",
                &render_out,
                "--resolution",
                "800,600",
                "--no-background",
            ])
            .output()
            .ok()?
    };

    if !result.status.success() || !std::path::Path::new(&render_out).exists() {
        return None;
    }

    // base64 encode
    let b64 = flamel::mcp_parent_guard::command("base64")
        .args(["-w", "0", &render_out])
        .output()
        .ok()?;
    let _ = std::fs::remove_file(&render_out);
    if b64.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&b64.stdout).trim().to_string())
}

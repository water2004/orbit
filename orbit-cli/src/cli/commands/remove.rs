use anyhow::{Context, Result};

pub async fn handle(input: String) -> Result<()> {
    // 1. 加载 orbit.toml
    let manifest_path = std::path::Path::new("orbit.toml");
    if !manifest_path.exists() {
        anyhow::bail!("orbit.toml not found in this directory.");
    }
    let mut manifest = orbit_core::OrbitManifest::from_path(manifest_path)
        .context("failed to parse orbit.toml")?;

    // 2. 按 key 或 slug 精确匹配
    let key = match find_by_slug(&input, &manifest) {
        Some(k) => k,
        None => {
            // 没匹配到 → 列出所有候选让用户选
            if manifest.dependencies.is_empty() {
                anyhow::bail!("No dependencies in orbit.toml.");
            }
            eprintln!("'{input}' not found in orbit.toml. Installed dependencies:");
            let deps: Vec<(&String, &orbit_core::manifest::DependencySpec)> =
                manifest.dependencies.iter().collect();
            for (i, (k, spec)) in deps.iter().enumerate() {
                let slug = spec.slug().unwrap_or("—");
                eprintln!("  [{i}] {k}  (slug: {slug})");
            }
            eprint!("\nChoose a number (or press Enter to cancel): ");
            let mut choice = String::new();
            std::io::stdin().read_line(&mut choice).ok();
            let trimmed = choice.trim();
            if trimmed.is_empty() {
                anyhow::bail!("Remove cancelled.");
            }
            match trimmed.parse::<usize>() {
                Ok(i) if i < deps.len() => deps[i].0.clone(),
                _ => anyhow::bail!("Invalid choice."),
            }
        }
    };

    let spec = manifest.dependencies.swap_remove(&key)
        .expect("dependency entry should exist");

    // 3. 加载 orbit.lock
    let lock_path = std::path::Path::new("orbit.lock");
    let mut lockfile = if lock_path.exists() {
        orbit_core::OrbitLockfile::from_path(lock_path)
            .context("failed to parse orbit.lock")?
    } else {
        anyhow::bail!("orbit.lock not found.");
    };

    // 4. 用 resolver 反查依赖图
    let slug = spec.slug().unwrap_or(&key);
    let dependents = orbit_core::resolver::dependents(slug, &key, &lockfile.entries);
    if !dependents.is_empty() {
        anyhow::bail!(
            "'{key}' is required by: {}\nRemove those mods first.",
            dependents.join(", ")
        );
    }

    // 5. 从 lockfile 找到对应条目获取文件名
    let filename = lockfile.entries.iter()
        .find(|e| e.name == key || e.mod_id.as_deref() == Some(slug))
        .map(|e| e.filename.clone());

    // 6. 删除 JAR
    if let Some(ref fname) = filename {
        let jar_path = std::path::Path::new("mods").join(fname);
        if jar_path.exists() {
            std::fs::remove_file(&jar_path)
                .context(format!("failed to remove {}", jar_path.display()))?;
        }
    }

    // 6. 从 lockfile 移除
    lockfile.entries.retain(|e| e.name != key);

    // 7. 写入文件
    let toml_content = manifest.to_toml_string()
        .context("failed to serialize orbit.toml")?;
    std::fs::write(manifest_path, toml_content)
        .context("failed to write orbit.toml")?;

    let lock_content = lockfile.to_toml_string()
        .context("failed to serialize orbit.lock")?;
    std::fs::write(lock_path, lock_content)
        .context("failed to write orbit.lock")?;

    println!("Removed '{key}'{}.",
        if filename.is_some() { " and its JAR file" } else { "" });

    Ok(())
}

/// 按 key 名或 slug 字段精确查找依赖，返回 key。
fn find_by_slug(name: &str, manifest: &orbit_core::OrbitManifest) -> Option<String> {
    manifest.dependencies.iter().find_map(|(key, spec)| {
        if key == name || spec.slug() == Some(name) {
            Some(key.clone())
        } else {
            None
        }
    })
}

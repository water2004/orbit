use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ApiResponse<T> {
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub struct PagedResponse<T> {
    pub data: Vec<T>,
    pub pagination: Pagination,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    pub index: u32,
    pub page_size: u32,
    pub result_count: u32,
    pub total_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Game {
    pub id: u32,
    pub slug: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: u32,
    pub name: String,
    pub slug: String,
    pub is_class: Option<bool>,
    pub class_id: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mod {
    pub id: u32,
    pub name: String,
    pub slug: String,
    pub summary: String,
    pub download_count: u64,
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub authors: Vec<ModAuthor>,
    #[serde(default)]
    pub links: Option<ModLinks>,
    #[serde(default)]
    pub logo: Option<ModAsset>,
    #[serde(default)]
    pub screenshots: Vec<ModAsset>,
    #[serde(default)]
    pub latest_files: Vec<File>,
    #[serde(default)]
    pub latest_files_indexes: Vec<FileIndex>,
    pub is_available: bool,
    #[serde(default)]
    pub date_modified: String,
}

/// Relevant project links exposed by the CurseForge Core API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModLinks {
    #[serde(default)]
    pub website_url: Option<String>,
    #[serde(default)]
    pub wiki_url: Option<String>,
    #[serde(default)]
    pub issues_url: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
}

/// A CurseForge project image. The same official shape is used for the logo
/// and screenshot list.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModAsset {
    pub id: u32,
    pub mod_id: u32,
    pub title: String,
    pub description: String,
    pub thumbnail_url: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModAuthor {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub id: u32,
    pub mod_id: u32,
    pub is_available: bool,
    pub display_name: String,
    pub file_name: String,
    #[serde(default)]
    pub hashes: Vec<FileHash>,
    pub file_date: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub sortable_game_versions: Vec<SortableGameVersion>,
    #[serde(default)]
    pub dependencies: Vec<FileDependency>,
    pub file_fingerprint: u64,
}

impl File {
    pub fn sha1(&self) -> String {
        self.hashes
            .iter()
            .find(|hash| hash.algo == 1)
            .map(|hash| hash.value.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileHash {
    pub value: String,
    pub algo: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortableGameVersion {
    pub game_version_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDependency {
    pub mod_id: u32,
    pub relation_type: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileIndex {
    pub game_version: String,
    pub mod_loader: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetModsRequest<'a> {
    pub mod_ids: &'a [u32],
    pub filter_pc_only: bool,
}

#[derive(Debug, Serialize)]
pub struct FingerprintsRequest<'a> {
    pub fingerprints: &'a [u32],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintMatches {
    #[serde(default)]
    pub exact_matches: Vec<FingerprintMatch>,
}

#[derive(Debug, Deserialize)]
pub struct FingerprintMatch {
    pub file: File,
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum ModLoaderType {
    Forge = 1,
    Fabric = 4,
    Quilt = 5,
    NeoForge = 6,
}

impl ModLoaderType {
    pub fn parse(loader: &str) -> Option<Self> {
        match loader.to_ascii_lowercase().as_str() {
            "forge" => Some(Self::Forge),
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            "neoforge" => Some(Self::NeoForge),
            _ => None,
        }
    }

    pub fn name(value: u8) -> &'static str {
        match value {
            1 => "forge",
            4 => "fabric",
            5 => "quilt",
            6 => "neoforge",
            _ => "",
        }
    }
}

#[derive(Debug, Default)]
pub struct SearchModsParams<'a> {
    pub game_id: u32,
    pub class_id: u32,
    pub search_filter: Option<&'a str>,
    pub slug: Option<&'a str>,
    pub game_version: Option<&'a str>,
    pub mod_loader_type: Option<ModLoaderType>,
    pub index: u32,
    pub page_size: u32,
}

#[derive(Debug, Default)]
pub struct GetFilesParams<'a> {
    pub game_version: Option<&'a str>,
    pub mod_loader_type: Option<ModLoaderType>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_names_roundtrip_to_official_enum_values() {
        for (name, value) in [("forge", 1), ("fabric", 4), ("quilt", 5), ("neoforge", 6)] {
            let loader = ModLoaderType::parse(name).unwrap();
            assert_eq!(loader as u8, value);
            assert_eq!(ModLoaderType::name(value), name);
        }
        assert!(ModLoaderType::parse("unknown").is_none());
    }

    #[test]
    fn file_sha1_uses_the_hash_algorithm_tag() {
        let file: File = serde_json::from_str(
            r#"{
                "id": 1,
                "modId": 2,
                "isAvailable": true,
                "displayName": "Example",
                "fileName": "example.jar",
                "hashes": [
                    {"value": "md5", "algo": 2},
                    {"value": "sha1", "algo": 1}
                ],
                "fileDate": "2026-01-01T00:00:00Z",
                "downloadUrl": null,
                "gameVersions": [],
                "sortableGameVersions": [],
                "dependencies": [],
                "fileFingerprint": 3
            }"#,
        )
        .unwrap();

        assert_eq!(file.sha1(), "sha1");
    }

    #[test]
    fn project_presentation_assets_follow_the_official_core_api_shape() {
        let project: Mod = serde_json::from_str(
            r#"{
                "id": 42,
                "name": "Example",
                "slug": "example",
                "summary": "Example mod",
                "downloadCount": 7,
                "categories": [],
                "authors": [],
                "links": {
                    "websiteUrl": "https://example.invalid/project",
                    "wikiUrl": null,
                    "issuesUrl": "https://example.invalid/issues",
                    "sourceUrl": "https://example.invalid/source"
                },
                "logo": {
                    "id": 1,
                    "modId": 42,
                    "title": "Logo",
                    "description": "",
                    "thumbnailUrl": "https://example.invalid/logo-small.png",
                    "url": "https://example.invalid/logo.png"
                },
                "screenshots": [{
                    "id": 2,
                    "modId": 42,
                    "title": "World",
                    "description": "In game",
                    "thumbnailUrl": "https://example.invalid/world-small.png",
                    "url": "https://example.invalid/world.png"
                }],
                "latestFiles": [],
                "latestFilesIndexes": [],
                "isAvailable": true
            }"#,
        )
        .unwrap();

        assert_eq!(
            project
                .logo
                .as_ref()
                .map(|asset| asset.thumbnail_url.as_str()),
            Some("https://example.invalid/logo-small.png")
        );
        assert_eq!(project.screenshots.len(), 1);
        assert_eq!(
            project.links.and_then(|links| links.source_url),
            Some("https://example.invalid/source".to_string())
        );
    }
}

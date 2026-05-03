use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────
// Auxiliary / sub-structures
// ─────────────────────────────────────────────

/// The license of a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
}

/// A message from a moderator regarding the project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeratorMessage {
    pub message: String,
    pub body: Option<String>,
}

/// A donation link for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DonationUrl {
    pub id: String,
    pub platform: String,
    pub url: String,
}

/// A gallery image attached to a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryItem {
    pub url: String,
    pub featured: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    pub created: String,
    pub ordering: Option<i64>,
}

// ─────────────────────────────────────────────
// ProjectInfo trait
// ─────────────────────────────────────────────

/// 一致的 Project 接口封装，使得无论是直接查询 `Project` 还是从 `SearchHit` 返回的结果
/// 都可以用相似的逻辑进行处理。
pub trait ProjectInfo {
    fn get_id(&self) -> &str;
    fn get_slug(&self) -> &str;
    fn get_title(&self) -> &str;
    fn get_description(&self) -> &str;
    fn get_categories(&self) -> &[String];
    fn get_client_side(&self) -> &str;
    fn get_server_side(&self) -> &str;
    fn get_project_type(&self) -> &str;
    fn get_downloads(&self) -> i64;
    fn get_versions(&self) -> &[String];
    fn get_followers(&self) -> i64;
    fn get_author(&self) -> &str;
    fn get_date_created(&self) -> &str;
    fn get_date_modified(&self) -> &str;
}

// ─────────────────────────────────────────────
// Project
// ─────────────────────────────────────────────

/// 通过 ID/Slug 获取项目时返回的完整信息。
/// 对应 `GET /project/{id|slug}` 和 `GET /projects` 的响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    // ── required fields ──
    /// The ID of the project, encoded as a base62 string.
    pub id: String,
    /// The slug of a project, used for vanity URLs.
    pub slug: String,
    /// The type of the project (mod, modpack, resourcepack, shader).
    pub project_type: String,
    /// The ID of the team that has ownership of this project.
    pub team: String,
    /// The title of the project.
    pub title: String,
    /// A short description of the project.
    pub description: String,
    /// A long form description of the project (markdown).
    pub body: String,
    /// The date the project was published (ISO-8601).
    pub published: String,
    /// The date the project was last updated (ISO-8601).
    pub updated: String,
    /// The status of the project.
    pub status: String,
    /// Client-side support status (required, optional, unsupported, unknown).
    pub client_side: String,
    /// Server-side support status (required, optional, unsupported, unknown).
    pub server_side: String,
    /// The number of times the project has been downloaded.
    pub downloads: i64,
    /// The number of followers the project has.
    pub followers: i64,
    /// The categories of the project.
    pub categories: Vec<String>,

    // ── optional fields ──
    /// The link to the long description. Always null, legacy compatibility.
    pub body_url: Option<String>,
    /// The date the project was approved (ISO-8601).
    pub approved: Option<String>,
    /// The date the project was queued for review (ISO-8601).
    pub queued: Option<String>,
    /// The requested status when submitting for review or scheduling.
    pub requested_status: Option<String>,
    /// A message that a moderator sent regarding the project.
    pub moderator_message: Option<ModeratorMessage>,
    /// The license of the project.
    pub license: Option<License>,
    /// A list of categories which are searchable but non-primary.
    pub additional_categories: Option<Vec<String>>,
    /// A list of all of the loaders supported by the project.
    pub loaders: Option<Vec<String>>,
    /// A list of the version IDs of the project.
    pub versions: Option<Vec<String>>,
    /// A list of all of the game versions supported by the project.
    pub game_versions: Option<Vec<String>>,
    /// A list of donation links for the project.
    pub donation_urls: Option<Vec<DonationUrl>>,
    /// A list of images that have been uploaded to the project's gallery.
    pub gallery: Option<Vec<GalleryItem>>,
    /// An optional link to where to submit bugs or issues with the project.
    pub issues_url: Option<String>,
    /// An optional link to the source code of the project.
    pub source_url: Option<String>,
    /// An optional link to the project's wiki page or other relevant information.
    pub wiki_url: Option<String>,
    /// An optional invite link to the project's discord.
    pub discord_url: Option<String>,
    /// The URL of the project's icon.
    pub icon_url: Option<String>,
    /// The RGB color of the project, automatically generated from the project icon.
    pub color: Option<i64>,
    /// The ID of the moderation thread associated with this project.
    pub thread_id: Option<String>,
    /// The monetization status of the project.
    pub monetization_status: Option<String>,
}

impl ProjectInfo for Project {
    fn get_id(&self) -> &str { &self.id }
    fn get_slug(&self) -> &str { &self.slug }
    fn get_title(&self) -> &str { &self.title }
    fn get_description(&self) -> &str { &self.description }
    fn get_categories(&self) -> &[String] { &self.categories }
    fn get_client_side(&self) -> &str { &self.client_side }
    fn get_server_side(&self) -> &str { &self.server_side }
    fn get_project_type(&self) -> &str { &self.project_type }
    fn get_downloads(&self) -> i64 { self.downloads }
    fn get_versions(&self) -> &[String] { self.versions.as_deref().unwrap_or(&[]) }
    fn get_followers(&self) -> i64 { self.followers }
    fn get_author(&self) -> &str { &self.team }
    fn get_date_created(&self) -> &str { &self.published }
    fn get_date_modified(&self) -> &str { &self.updated }
}

// ─────────────────────────────────────────────
// SearchHit / SearchResult
// ─────────────────────────────────────────────

/// 搜索接口中返回的单条结果。
/// 对应 `GET /search` 响应中 `hits` 数组中的对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    // ── required fields ──
    /// The slug of the project.
    pub slug: String,
    /// The title of the project.
    pub title: String,
    /// The description of the project.
    pub description: String,
    /// Client-side support status.
    pub client_side: String,
    /// Server-side support status.
    pub server_side: String,
    /// The type of the project (mod, modpack, resourcepack, shader).
    pub project_type: String,
    /// The number of times the project has been downloaded.
    pub downloads: i64,
    /// The project's ID, encoded as base62.
    pub project_id: String,
    /// The username of the project's author.
    pub author: String,
    /// The ID of the project's author.
    pub author_id: String,
    /// A list of the minecraft versions supported by the project.
    pub versions: Vec<String>,
    /// The total number of users following the project.
    pub follows: i64,
    /// The date the project was added to search (ISO-8601).
    pub date_created: String,
    /// The date the project was last modified (ISO-8601).
    pub date_modified: String,
    /// The SPDX license ID of the project.
    pub license: String,

    // ── optional fields ──
    /// The name of the organization that owns this project.
    pub organization: Option<String>,
    /// The ID of the organization that owns this project.
    pub organization_id: Option<String>,
    /// The categories of the project.
    pub categories: Option<Vec<String>>,
    /// The URL of the project's icon.
    pub icon_url: Option<String>,
    /// The RGB color of the project, automatically generated from the project icon.
    pub color: Option<i64>,
    /// The ID of the moderation thread associated with this project.
    pub thread_id: Option<String>,
    /// The monetization status of the project.
    pub monetization_status: Option<String>,
    /// A list of display categories (non-secondary).
    pub display_categories: Option<Vec<String>>,
    /// The latest version ID of the project (base62 string).
    pub latest_version: Option<String>,
    /// All gallery images attached to the project.
    pub gallery: Option<Vec<String>>,
    /// The featured gallery image of the project.
    pub featured_gallery: Option<String>,
}

impl ProjectInfo for SearchHit {
    fn get_id(&self) -> &str { &self.project_id }
    fn get_slug(&self) -> &str { &self.slug }
    fn get_title(&self) -> &str { &self.title }
    fn get_description(&self) -> &str { &self.description }
    fn get_categories(&self) -> &[String] { self.categories.as_deref().unwrap_or(&[]) }
    fn get_client_side(&self) -> &str { &self.client_side }
    fn get_server_side(&self) -> &str { &self.server_side }
    fn get_project_type(&self) -> &str { &self.project_type }
    fn get_downloads(&self) -> i64 { self.downloads }
    fn get_versions(&self) -> &[String] { &self.versions }
    fn get_followers(&self) -> i64 { self.follows }
    fn get_author(&self) -> &str { &self.author }
    fn get_date_created(&self) -> &str { &self.date_created }
    fn get_date_modified(&self) -> &str { &self.date_modified }
}

/// 搜索接口的顶层响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub offset: i64,
    pub limit: i64,
    pub total_hits: i64,
}

// ─────────────────────────────────────────────
// Version / VersionFile / VersionDependency / Hashes
// ─────────────────────────────────────────────

/// 版本信息。
/// 对应 `GET /version/{id}`、`GET /project/{id|slug}/version` 等端点的响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    // ── required fields ──
    /// The ID of the version, encoded as base62.
    pub id: String,
    /// The ID of the project this version belongs to.
    pub project_id: String,
    /// The ID of the author who published this version.
    pub author_id: String,
    /// The date the version was published (ISO-8601).
    pub date_published: String,
    /// The number of times the version has been downloaded.
    pub downloads: i64,
    /// An array of file objects associated with this version.
    pub files: Vec<VersionFile>,
    /// The name of the version.
    pub name: String,
    /// The version number.
    pub version_number: String,
    /// The game versions the version supports.
    pub game_versions: Vec<String>,
    /// The version type (alpha, beta, release).
    pub version_type: String,
    /// The loaders the version supports.
    pub loaders: Vec<String>,
    /// Whether the version is featured or not.
    pub featured: bool,

    // ── optional fields ──
    /// A link to the changelog for this version. Always null, legacy compatibility.
    pub changelog_url: Option<String>,
    /// The changelog of the version.
    pub changelog: Option<String>,
    /// An array of dependency objects associated with this version.
    pub dependencies: Option<Vec<VersionDependency>>,
    /// The status of the version.
    pub status: Option<String>,
    /// The requested status of the version.
    pub requested_status: Option<String>,
}

/// A file associated with a version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFile {
    /// The ID of the file, encoded as base62.
    pub id: String,
    /// An object containing the hashes of the file.
    pub hashes: Hashes,
    /// The URL to download the file.
    pub url: String,
    /// The name of the file.
    pub filename: String,
    /// Whether the file is the primary file of the version.
    pub primary: bool,
    /// The size of the file in bytes.
    pub size: i64,
    /// The type of the additional file (e.g. resource packs for datapacks).
    pub file_type: Option<String>,
}

/// A map of hashes of the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hashes {
    /// The SHA-512 hash of the file.
    pub sha512: String,
    /// The SHA-1 hash of the file.
    pub sha1: String,
}

/// A dependency of a version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDependency {
    /// The type of the dependency (required, optional, incompatible, embedded).
    pub dependency_type: String,
    /// The ID of the version that this version depends on.
    pub version_id: Option<String>,
    /// The ID of the project that this version depends on.
    pub project_id: Option<String>,
    /// The file name of the dependency.
    pub file_name: Option<String>,
}

// ─────────────────────────────────────────────
// ProjectDependencyList
// ─────────────────────────────────────────────

/// 项目依赖列表。
/// 对应 `GET /project/{id|slug}/dependencies` 的响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDependencyList {
    /// An array of project objects that are dependencies of the given project.
    pub projects: Vec<Project>,
    /// An array of version objects that are dependencies of the given project.
    pub versions: Vec<Version>,
}

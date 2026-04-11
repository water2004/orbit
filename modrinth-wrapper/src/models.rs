use serde::{Deserialize, Serialize};

/// 一致的 Project 接口封装，使得无论是直接查询 `Project` 还是从 `SearchHit` 返回的结果
/// 都可以用相似的逻辑进行处理。
pub trait ProjectInfo {
    fn get_id(&self) -> &str;
    fn get_slug(&self) -> Option<&str>;
    fn get_title(&self) -> Option<&str>;
    fn get_description(&self) -> Option<&str>;
    fn get_categories(&self) -> Option<&[String]>;
    fn get_client_side(&self) -> Option<&str>;
    fn get_server_side(&self) -> Option<&str>;
    fn get_project_type(&self) -> &str;
    fn get_downloads(&self) -> i32;
    fn get_versions(&self) -> &[String];
    fn get_followers(&self) -> i32;
    fn get_author(&self) -> &str;
    fn get_date_created(&self) -> &str;
    fn get_date_modified(&self) -> &str;
}

/// 发送请求或者通过 ID/Slug 单个获取项目时的完整信息。
#[derive(Debug, Serialize, Deserialize)]
pub struct Project {
    /// The ID of the project. encoded as a base62 string.
    pub id: String,
    /// The ID of the team that has ownership of this project.
    pub team: String,
    /// The date the project was published (ISO-8601).
    pub published: String,
    /// The date the project was last updated (ISO-8601).
    pub updated: String,
    /// The number of followers the project has.
    pub followers: i32,
    /// A list of version IDs associated with this project.
    pub versions: Vec<String>,
    /// The number of times the project has been downloaded.
    pub downloads: i32,
    /// The type of project (mod, modpack, resourcepack, shader).
    pub project_type: String,
    /// The slug of a project, used for vanity URLs.
    pub slug: Option<String>,
    /// The title of the project.
    pub title: Option<String>,
    /// A short description of the project.
    pub description: Option<String>,
    /// Available game versions.
    pub game_versions: Option<Vec<String>>,
    /// Supported loaders (e.g. fabric, forge).
    pub loaders: Option<Vec<String>>,
    /// Categories applied to the project.
    pub categories: Option<Vec<String>>,
    /// Indicates support status on the client (required, optional, unsupported, unknown).
    pub client_side: Option<String>,
    /// Indicates support status on the server (required, optional, unsupported, unknown).
    pub server_side: Option<String>,
    /// A long-form description of the project, typically markdown.
    pub body: Option<String>,
    /// The license of the project.
    pub license: Option<License>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct License {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
}

impl ProjectInfo for Project {
    fn get_id(&self) -> &str { &self.id }
    fn get_slug(&self) -> Option<&str> { self.slug.as_deref() }
    fn get_title(&self) -> Option<&str> { self.title.as_deref() }
    fn get_description(&self) -> Option<&str> { self.description.as_deref() }
    fn get_categories(&self) -> Option<&[String]> { self.categories.as_deref() }
    fn get_client_side(&self) -> Option<&str> { self.client_side.as_deref() }
    fn get_server_side(&self) -> Option<&str> { self.server_side.as_deref() }
    fn get_project_type(&self) -> &str { &self.project_type }
    fn get_downloads(&self) -> i32 { self.downloads }
    fn get_versions(&self) -> &[String] { &self.versions }
    fn get_followers(&self) -> i32 { self.followers }
    fn get_author(&self) -> &str { &self.team }
    fn get_date_created(&self) -> &str { &self.published }
    fn get_date_modified(&self) -> &str { &self.updated }
}

/// 搜索接口中返回的单条目，它的结构与 `Project` 高度类似，
/// 但 Modrinth API 在部分属性名的设计上会有不同。
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchHit {
    /// The type of project (mod, modpack, resourcepack, shader).
    pub project_type: String,
    /// The total number of downloads.
    pub downloads: i32,
    /// The project's ID.
    pub project_id: String,
    /// The username or team that created the project.
    pub author: String,
    /// The latest versions available.
    pub versions: Vec<String>,
    /// The number of followers.
    pub follows: i32,
    /// Date created (ISO-8601).
    pub date_created: String,
    /// Date last modified (ISO-8601).
    pub date_modified: String,
    /// The project's license short-ID.
    pub license: String,
    /// The project slug.
    pub slug: Option<String>,
    /// The project title.
    pub title: Option<String>,
    /// The project description.
    pub description: Option<String>,
    /// A list of associated categories.
    pub categories: Option<Vec<String>>,
    /// Client-side support.
    pub client_side: Option<String>,
    /// Server-side support.
    pub server_side: Option<String>,
}

impl ProjectInfo for SearchHit {
    fn get_id(&self) -> &str { &self.project_id }
    fn get_slug(&self) -> Option<&str> { self.slug.as_deref() }
    fn get_title(&self) -> Option<&str> { self.title.as_deref() }
    fn get_description(&self) -> Option<&str> { self.description.as_deref() }
    fn get_categories(&self) -> Option<&[String]> { self.categories.as_deref() }
    fn get_client_side(&self) -> Option<&str> { self.client_side.as_deref() }
    fn get_server_side(&self) -> Option<&str> { self.server_side.as_deref() }
    fn get_project_type(&self) -> &str { &self.project_type }
    fn get_downloads(&self) -> i32 { self.downloads }
    fn get_versions(&self) -> &[String] { &self.versions }
    fn get_followers(&self) -> i32 { self.follows }
    fn get_author(&self) -> &str { &self.author }
    fn get_date_created(&self) -> &str { &self.date_created }
    fn get_date_modified(&self) -> &str { &self.date_modified }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub offset: i32,
    pub limit: i32,
    pub total_hits: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub author_id: String,
    pub date_published: String,
    pub downloads: i32,
    pub files: Vec<VersionFile>,
    pub name: Option<String>,
    pub version_number: Option<String>,
    pub changelog: Option<String>,
    pub dependencies: Option<Vec<VersionDependency>>,
    pub game_versions: Option<Vec<String>>,
    pub version_type: Option<String>,
    pub loaders: Option<Vec<String>>,
    pub featured: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VersionFile {
    pub hashes: Hashes,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Hashes {
    pub sha512: String,
    pub sha1: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dependencies {
    pub projects: Vec<Project>,
    pub versions: Vec<Version>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VersionDependency {
    pub dependency_type: String,
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub file_name: Option<String>,
}


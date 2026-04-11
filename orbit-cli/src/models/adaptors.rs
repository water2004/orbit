use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct UnifiedMod {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub source: ModSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModSource {
    Modrinth,
    CurseForge,
}

#[derive(Debug, Clone)]
pub struct ModVersion {
    pub id: String,
    pub project_id: String,
    pub version_number: String,
    pub date_published: String,
    pub files: Vec<ModFile>,
}

#[derive(Debug, Clone)]
pub struct ModFile {
    pub filename: String,
    pub url: String,
    // (Algorithm -> Hash) like {"sha1": "...", "sha512": "..."}
    pub hashes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Category {
    pub id: String,
    pub name: String,
    // CurseForge sub-categories or Modrinth nested categories
    pub sub_categories: Vec<Category>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SortType {
    Relevance,
    Downloads,
    UpdateDate,
    CreationDate,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub results: Vec<UnifiedMod>,
    pub total_count: usize,
    pub offset: usize,
    pub limit: usize,
}

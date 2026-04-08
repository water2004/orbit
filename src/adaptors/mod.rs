pub use crate::models::adaptors::{
    UnifiedMod,
    ModSource,
    ModVersion,
    ModFile,
    Category,
    SortType,
    SortOrder,
    SearchResult,
};
pub trait ModProvider {
    fn search(
        &self,
        query: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
        category: Option<&str>,
        offset: usize,
        limit: usize,
        sort_type: SortType,
        sort_order: SortOrder,
    ) -> Result<SearchResult, Box<dyn std::error::Error>>;
    
    fn get_mod(&self, id: &str) -> Result<Option<UnifiedMod>, Box<dyn std::error::Error>>;
    
    // ==== 本地同步与检测必需的关键接口 ====
    
    /// 获取当前平台的模组分类树
    fn get_categories(&self) -> Result<Vec<Category>, Box<dyn std::error::Error>>;
    
    /// 按哈希查询特定文件版本 (对应 update_mods.sh 中按 sha1 查询和 Java 的 getRemoteVersionByLocalFile)
    fn get_version_by_hash(
        &self, 
        hash: &str, 
        algorithm: &str
    ) -> Result<Option<ModVersion>, Box<dyn std::error::Error>>;
    
    /// 按模组 ID 获取版本列表，并可通过游戏版本与加载器过滤 (对应 update_mods.sh 查询更新及依赖树处理)
    fn get_versions(
        &self, 
        mod_id: &str, 
        mc_version: Option<&str>, 
        loader: Option<&str>
    ) -> Result<Vec<ModVersion>, Box<dyn std::error::Error>>;
    
    /// 解析模组的依赖项
    fn resolve_dependency(&self, id: &str) -> Result<Option<UnifiedMod>, Box<dyn std::error::Error>>;
}

pub mod modrinth;
pub mod curseforge;

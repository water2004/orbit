# modrinth-wrapper

一个轻量的 Rust 封装，用于访问 Modrinth v2 API（项目 / 版本 / 文件 / 依赖等）。

**主要特性**
- 简单的 `Client` 初始化（强制指定 `User-Agent`）
- 封装常用 API：搜索项目、获取项目、批量获取项目、获取依赖、版本查询、按哈希查询版本与批量查询
- 一致的 `ProjectInfo` trait，用于统一处理 `Project` 与 `SearchHit`
- 完整的集成测试（依赖真实 Modrinth API，位于 `tests/api_tests.rs`）

## 安装
在你的 `Cargo.toml` 添加：

```toml
modrinth-wrapper = { path = "../modrinth-wrapper" }
```

或当发布到 crates.io 后使用：

```toml
modrinth-wrapper = "0.1.0"
```

## 快速开始

示例：创建客户端并获取项目信息

```rust
use modrinth_wrapper::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// 必须提供明确且可识别的 User-Agent
	let client = Client::new("your_github_username/your_project/0.1.0 (your_email@example.com)")?;

	let project = client.get_project("fabric-api").await?;
	println!("project title: {:?}", project.title);

	Ok(())
}
```

常用方法（示例）：
- `Client::new(user_agent: &str)` — 构造请求客户端
- `get_project(&self, id_or_slug: &str) -> Result<Project>`
- `get_projects(&self, ids: &[&str]) -> Result<Vec<Project>>`
- `search_projects(&self, query: &str) -> Result<SearchResult>`
- `list_versions(&self, project_id: &str) -> Result<Vec<Version>>`
- `get_version(&self, project_id: &str, version_id_or_number: &str) -> Result<Version>`
- `get_version_from_hash(&self, hash: &str) -> Result<Version>`
- `get_versions_from_hashes(&self, hashes: Vec<String>) -> Result<HashMap<String, Version>>`
- `get_project_dependencies(&self, id_or_slug: &str) -> Result<Dependencies>`
- `get_latest_version_from_hash` / `get_latest_versions_from_hashes` — 根据 loader 与 game_versions 筛选最新版本

(实现文件见 `src/client.rs`、`src/api.rs`、`src/models.rs`)

## 测试
项目包含集成测试，直接调用真实 Modrinth API，运行：

```bash
cargo test
```

注意：这些测试会发起网络请求并依赖 `api.modrinth.com`，请确保网络可用并遵守 Modrinth 的速率限制（300 requests/min）。

## 文档
仓库中包含了部分从 Modrinth 官方文档同步并补全的说明，位于 `modrinth-docs/`，包含：
- `project/`（如 `get.md`, `search.md`, `dependencies.md` 等）
- `version/` 与 `version_file/`（hash / updates 等）

如果你发现官方文档有更新，建议同步对应的 `modrinth-docs/` 文件并运行测试验证行为。

## 贡献
欢迎提交 issue 或 PR：
- 新增或修正 API 封装
- 增加更细粒度的错误类型、重试或速率限制处理
- 增加更多测试用例或文档补全

请在 PR 中保持改动最小且附上测试或复现步骤。
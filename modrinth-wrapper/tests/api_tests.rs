use modrinth_wrapper::Client;
use modrinth_wrapper::api::{SearchParams, ListVersionsParams};

fn create_client() -> Client {
    Client::new("test-user-agent-fabric-api (https://github.com/test/test)").unwrap()
}

#[tokio::test]
async fn test_get_project() {
    let client = create_client();
    let project = client.get_project("fabric-api").await;
    assert!(project.is_ok(), "Failed to fetch project: {:?}", project.err());
    let p = project.unwrap();
    assert_eq!(p.slug, "fabric-api");
    assert!(!p.title.is_empty());
    assert!(!p.description.is_empty());
    assert!(!p.project_type.is_empty());
}

#[tokio::test]
async fn test_get_projects() {
    let client = create_client();
    let projects = client.get_projects(&["fabric-api", "iris"]).await;
    assert!(projects.is_ok(), "Failed to fetch projects: {:?}", projects.err());
    assert!(projects.unwrap().len() >= 1);
}

#[tokio::test]
async fn test_get_project_dependencies() {
    let client = create_client();
    let deps = client.get_project_dependencies("fabric-api").await;
    assert!(deps.is_ok(), "Failed to fetch project dependencies: {:?}", deps.err());
}

#[tokio::test]
async fn test_search_projects() {
    let client = create_client();
    let result = client.search_projects("fabric-api").await;
    assert!(result.is_ok(), "Failed to search projects: {:?}", result.err());
    let res = result.unwrap();
    assert!(!res.hits.is_empty());
    // Verify required fields are present
    let hit = &res.hits[0];
    assert!(!hit.slug.is_empty());
    assert!(!hit.title.is_empty());
    assert!(!hit.project_id.is_empty());
    assert!(!hit.author.is_empty());
}

#[tokio::test]
async fn test_search_with_params() {
    let client = create_client();
    let result = client.search(
        SearchParams::new("fabric")
            .index("downloads")
            .limit(5)
    ).await;
    assert!(result.is_ok(), "Failed to search with params: {:?}", result.err());
    let res = result.unwrap();
    assert!(res.hits.len() <= 5);
    assert!(res.limit <= 5);
}

#[tokio::test]
async fn test_list_versions() {
    let client = create_client();
    let result = client.list_versions("fabric-api").await;
    assert!(result.is_ok(), "Failed to list versions: {:?}", result.err());
    let versions = result.unwrap();
    assert!(!versions.is_empty());
    // Verify required fields
    let v = &versions[0];
    assert!(!v.id.is_empty());
    assert!(!v.name.is_empty());
    assert!(!v.version_number.is_empty());
    assert!(!v.game_versions.is_empty());
    assert!(!v.loaders.is_empty());
}

#[tokio::test]
async fn test_list_versions_with_params() {
    let client = create_client();
    let result = client.list_versions_with_params(
        "fabric-api",
        ListVersionsParams::new()
            .loaders(&["fabric"])
            .include_changelog(false)
    ).await;
    assert!(result.is_ok(), "Failed to list versions with params: {:?}", result.err());
    let versions = result.unwrap();
    assert!(!versions.is_empty());
    // All versions should have fabric in their loaders
    for v in &versions {
        assert!(v.loaders.iter().any(|l| l == "fabric"), "Version {} does not have fabric loader", v.id);
    }
}

#[tokio::test]
async fn test_get_version_by_id() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let first_version_id = &versions[0].id;

    let result = client.get_version_by_id(first_version_id).await;
    assert!(result.is_ok(), "Failed to get version by id: {:?}", result.err());
    assert_eq!(result.unwrap().id, *first_version_id);
}

#[tokio::test]
async fn test_get_version() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let first_version_id = &versions[0].id;

    let result = client.get_version("fabric-api", first_version_id).await;
    assert!(result.is_ok(), "Failed to get version: {:?}", result.err());
    assert_eq!(result.unwrap().id, *first_version_id);
}

#[tokio::test]
async fn test_get_versions_by_ids() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let id1 = versions[0].id.as_str();
    let id2 = versions[1].id.as_str();

    let result = client.get_versions_by_ids(&[id1, id2]).await;
    assert!(result.is_ok(), "Failed to get versions by ids: {:?}", result.err());
    let fetched = result.unwrap();
    assert_eq!(fetched.len(), 2);
}

#[tokio::test]
async fn test_get_version_from_hash() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = &versions[0].files[0].hashes.sha1;

    let result = client.get_version_from_hash(hash, None, None).await;
    assert!(result.is_ok(), "Failed to get version from hash: {:?}", result.err());
}

#[tokio::test]
async fn test_get_versions_from_hashes() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = versions[0].files[0].hashes.sha1.as_str();

    let result = client.get_versions_from_hashes(&[hash], None).await;
    assert!(result.is_ok(), "Failed to get versions from hashes: {:?}", result.err());
    let map = result.unwrap();
    assert!(map.contains_key(hash));
}

#[tokio::test]
async fn test_get_latest_version_from_hash() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = &versions[0].files[0].hashes.sha1;

    let loaders: Vec<&str> = versions[0].loaders.iter().map(|s| s.as_str()).collect();
    let game_versions: Vec<&str> = versions[0].game_versions.iter().map(|s| s.as_str()).collect();

    let result = client.get_latest_version_from_hash(hash, &loaders, &game_versions, None).await;
    assert!(result.is_ok(), "Failed to get latest version from hash: {:?}", result.err());
}

#[tokio::test]
async fn test_get_latest_versions_from_hashes() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = versions[0].files[0].hashes.sha1.as_str();

    let loaders: Vec<&str> = versions[0].loaders.iter().map(|s| s.as_str()).collect();
    let game_versions: Vec<&str> = versions[0].game_versions.iter().map(|s| s.as_str()).collect();

    let result = client.get_latest_versions_from_hashes(&[hash], &loaders, &game_versions, None).await;
    assert!(result.is_ok(), "Failed to get latest versions from hashes: {:?}", result.err());
    let map = result.unwrap();
    assert!(map.contains_key(hash));
}

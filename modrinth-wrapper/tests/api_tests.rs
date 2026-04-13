use modrinth_wrapper::Client;

fn create_client() -> Client {
    Client::new("test-user-agent-fabric-api (https://github.com/test/test)").unwrap()
}

#[tokio::test]
async fn test_get_project() {
    let client = create_client();
    let project = client.get_project("fabric-api").await;
    assert!(project.is_ok(), "Failed to fetch project: {:?}", project.err());
    assert_eq!(project.unwrap().slug.as_deref(), Some("fabric-api"));
}

#[tokio::test]
async fn test_get_projects() {
    let client = create_client();
    // Test getting multiple specific IDs. (You can also pass slugs if you know they are guaranteed unique across your target sets, but using slug for testing is fine here.)
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
    assert!(!result.unwrap().hits.is_empty());
}

#[tokio::test]
async fn test_list_versions() {
    let client = create_client();
    let result = client.list_versions("fabric-api").await;
    assert!(result.is_ok(), "Failed to list versions: {:?}", result.err());
    assert!(!result.unwrap().is_empty());
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
async fn test_get_version_from_hash() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = &versions[0].files[0].hashes.sha1;
    
    let result = client.get_version_from_hash(hash).await;
    assert!(result.is_ok(), "Failed to get version from hash: {:?}", result.err());
}

#[tokio::test]
async fn test_get_versions_from_hashes() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = versions[0].files[0].hashes.sha1.clone();
    
    let result = client.get_versions_from_hashes(vec![hash.clone()]).await;
    assert!(result.is_ok(), "Failed to get versions from hashes: {:?}", result.err());
    let map = result.unwrap();
    assert!(map.contains_key(&hash));
}

#[tokio::test]
async fn test_get_latest_version_from_hash() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = &versions[0].files[0].hashes.sha1;

    // use the same loaders and game_versions as the version to ensure a match
    let loaders_vec = versions[0].loaders.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<&str>>()).unwrap_or_else(|| vec!["fabric"]);
    let game_versions_vec = versions[0].game_versions.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<&str>>()).unwrap_or_else(|| vec!["1.18"]);

    let result = client.get_latest_version_from_hash(hash, &loaders_vec, &game_versions_vec, None).await;
    assert!(result.is_ok(), "Failed to get latest version from hash: {:?}", result.err());
}

#[tokio::test]
async fn test_get_latest_versions_from_hashes() {
    let client = create_client();
    let versions = client.list_versions("fabric-api").await.unwrap();
    let hash = versions[0].files[0].hashes.sha1.clone();

    let loaders_vec = versions[0].loaders.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<&str>>()).unwrap_or_else(|| vec!["fabric"]);
    let game_versions_vec = versions[0].game_versions.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<&str>>()).unwrap_or_else(|| vec!["1.18"]);

    let result = client.get_latest_versions_from_hashes(&[&hash], &loaders_vec, &game_versions_vec, None).await;
    assert!(result.is_ok(), "Failed to get latest versions from hashes: {:?}", result.err());
    let map = result.unwrap();
    assert!(map.contains_key(&hash));
}

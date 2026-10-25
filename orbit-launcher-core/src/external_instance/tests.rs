use super::*;
use serde_json::json;
use std::path::Path;

fn json_file(root: &Path, relative: &str, value: serde_json::Value) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
fn standard(root: &Path, id: &str) {
    json_file(
        root,
        &format!("versions/{id}/{id}.json"),
        json!({"id":id,"inheritsFrom":"1.21.1","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.14"}]}),
    );
}
#[test]
fn hmcl_shared_isolated_custom_and_global_are_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for (id, kind) in [("shared", 0), ("isolated", 1), ("custom", 2), ("global", 1)] {
        standard(root, id);
        json_file(
            root,
            &format!("versions/{id}/hmclversion.cfg"),
            json!({"usesGlobal":id=="global","gameDirType":kind,"gameDir":root.to_str().unwrap()}),
        );
    }
    let catalog = inspect_external_instances(root).unwrap();
    let source = |id: &str| catalog.instances.iter().find(|c| c.id == id).unwrap();
    assert_eq!(
        source("shared").game_directory.as_ref().unwrap(),
        &dunce::canonicalize(root).unwrap()
    );
    assert_eq!(
        source("isolated").game_directory.as_ref().unwrap(),
        &dunce::canonicalize(root.join("versions/isolated")).unwrap()
    );
    assert_eq!(
        source("custom").game_directory,
        source("shared").game_directory
    );
    assert!(source("global").game_directory.is_none());
    assert_eq!(
        source("global").runtime.as_ref().unwrap().minecraft,
        "1.21.1"
    );
    // Exact version selection must not enumerate siblings.
    assert_eq!(
        inspect_external_instances(&root.join("versions/isolated"))
            .unwrap()
            .instances
            .len(),
        1
    );
}
#[test]
fn hmcl_merged_profile_uses_game_patch_not_display_id() {
    let temp = tempfile::tempdir().unwrap();
    standard(temp.path(), "pretty-name");
    json_file(
        temp.path(),
        "versions/pretty-name/pretty-name.json",
        json!({"id":"pretty-name","downloads":{"client":{}},"patches":[{"id":"game","version":"26.2"},{"id":"fabric","libraries":[{"name":"net.fabricmc:fabric-loader:0.20.1"}]}]}),
    );
    let c = inspect_external_instances(temp.path()).unwrap();
    assert_eq!(c.instances[0].runtime.as_ref().unwrap().minecraft, "26.2");
}
#[test]
fn pcl_reads_explicit_v2_and_legacy_without_assuming_global_defaults() {
    let temp = tempfile::tempdir().unwrap();
    for (id, text) in [
        ("yes", "VersionArgumentIndieV2:True"),
        ("no", "VersionArgumentIndieV2:False"),
        ("old", "VersionArgumentIndie:1"),
        ("unknown", "VersionArgumentIndie:0"),
    ] {
        standard(temp.path(), id);
        let ini = temp.path().join(format!("versions/{id}/PCL/Setup.ini"));
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(ini, text).unwrap();
    }
    let c = inspect_external_instances(temp.path()).unwrap();
    let find = |id: &str| c.instances.iter().find(|c| c.id == id).unwrap();
    assert!(
        find("yes")
            .game_directory
            .as_ref()
            .unwrap()
            .ends_with("versions/yes")
    );
    assert_eq!(
        find("no").game_directory.as_ref().unwrap(),
        &dunce::canonicalize(temp.path()).unwrap()
    );
    assert!(
        find("old")
            .game_directory
            .as_ref()
            .unwrap()
            .ends_with("versions/old")
    );
    assert!(find("unknown").game_directory.is_none());
}
#[test]
fn inheritance_is_traversed_and_cycles_are_errors() {
    let temp = tempfile::tempdir().unwrap();
    standard(temp.path(), "fabric");
    json_file(
        temp.path(),
        "versions/1.21.1/1.21.1.json",
        json!({"id":"1.21.1","downloads":{"client":{}},"mainClass":"net.minecraft.client.main.Main"}),
    );
    let c = inspect_external_instances(&temp.path().join("versions/fabric")).unwrap();
    assert_eq!(c.instances[0].runtime.as_ref().unwrap().minecraft, "1.21.1");
    json_file(
        temp.path(),
        "versions/1.21.1/1.21.1.json",
        json!({"id":"1.21.1","inheritsFrom":"fabric"}),
    );
    assert!(
        inspect_external_instances(&temp.path().join("versions/fabric"))
            .unwrap()
            .instances[0]
            .error
            .as_ref()
            .unwrap()
            .contains("cyclic")
    );
}
#[test]
fn official_profiles_preserve_ambiguity_between_distinct_game_directories() {
    let temp = tempfile::tempdir().unwrap();
    standard(temp.path(), "fabric");
    std::fs::create_dir(temp.path().join("another-game")).unwrap();
    json_file(
        temp.path(),
        "launcher_profiles.json",
        json!({"profiles":{"a":{"lastVersionId":"fabric"},"b":{"lastVersionId":"fabric","gameDir":temp.path().join("another-game")}}}),
    );
    assert!(
        inspect_external_instances(temp.path()).unwrap().instances[0]
            .game_directory
            .is_none()
    );
}
#[test]
fn prism_curseforge_gdlauncher_and_atlauncher_normalize_to_one_model() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    json_file(
        root,
        "prism/mmc-pack.json",
        json!({"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.21.1"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.14"}]}),
    );
    std::fs::create_dir(root.join("prism/.minecraft")).unwrap();
    json_file(
        root,
        "curse/minecraftinstance.json",
        json!({"name":"Curse pack","gameVersion":"1.21.1","baseModLoader":{"name":"fabric-0.16.14-1.21.1"}}),
    );
    json_file(
        root,
        "gd/instance.json",
        json!({"_version":"1","name":"GD pack","game_configuration":{"version":{"release":"1.21.1","modloaders":[{"type":"Fabric","version":"0.16.14"}]}}}),
    );
    std::fs::create_dir(root.join("gd/instance")).unwrap();
    json_file(
        root,
        "atl/instance.json",
        json!({"id":"1.21.1","downloads":{"client":{}},"launcher":{"name":"AT pack"},"libraries":[{"name":"net.fabricmc:fabric-loader:0.16.14"}]}),
    );
    let c = inspect_external_instances(root).unwrap();
    assert_eq!(c.instances.len(), 4);
    for c in c.instances {
        let runtime = c.runtime.unwrap();
        assert_eq!(runtime.minecraft, "1.21.1");
        assert_eq!(runtime.loader, LoaderKind::Fabric);
        assert_eq!(runtime.loader_version.as_deref(), Some("0.16.14"));
        assert!(c.game_directory.is_some());
    }
}
#[test]
fn modrinth_reads_only_the_applied_content_set_from_a_readonly_database() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("profiles/my-pack")).unwrap();
    let path = root.join("app.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE settings(id INTEGER, custom_dir TEXT); INSERT INTO settings VALUES(0,NULL); CREATE TABLE instances(id TEXT,path TEXT,name TEXT,applied_content_set_id TEXT); CREATE TABLE instance_content_sets(id TEXT,instance_id TEXT,game_version TEXT,loader TEXT,loader_version TEXT); INSERT INTO instances VALUES('one','my-pack','My pack','applied'); INSERT INTO instance_content_sets VALUES('applied','one','1.21.1','fabric','0.16.14'); INSERT INTO instance_content_sets VALUES('draft','one','26.2','fabric','0.20.1');").unwrap();
    drop(db);
    let before = std::fs::read(&path).unwrap();
    let c = inspect_external_instances(root).unwrap();
    assert_eq!(c.instances.len(), 1);
    assert_eq!(c.instances[0].runtime.as_ref().unwrap().minecraft, "1.21.1");
    assert_eq!(before, std::fs::read(path).unwrap());
}
#[test]
fn empty_directories_are_not_instances() {
    let temp = tempfile::tempdir().unwrap();
    assert!(inspect_external_instances(temp.path()).is_err());
}

#[test]
fn custom_and_duplicate_prism_components_are_not_silently_dropped() {
    let temp = tempfile::tempdir().unwrap();
    for components in [
        json!([{"uid":"net.minecraft","version":"1.21.1"},{"uid":"custom.runtime","version":"1"}]),
        json!([{"uid":"net.minecraft","version":"1.21.1"},{"uid":"net.minecraft","version":"26.2"}]),
    ] {
        json_file(
            temp.path(),
            "mmc-pack.json",
            json!({"formatVersion":1,"components":components}),
        );
        assert!(inspect_external_instances(temp.path()).is_err());
    }
}

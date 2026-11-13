use super::*;

#[test]
fn plugin_manifest_is_read_once_per_entry_and_runtime_flags_stay_live() {
    let root = std::env::temp_dir().join(format!(
        "vue-plugin-memo-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("nested")).unwrap();
    let package = root.join("package.json");
    let entry = root.join("entry.ts");
    std::fs::write(&package, "{}").unwrap();
    std::fs::write(
        root.join("nested/package.json"),
        r#"{"devDependencies":{"@vitejs/plugin-vue":"1"}}"#,
    )
    .unwrap();
    let _automatic = runtime_flag(None);
    MANIFEST_READS.with(|reads| reads.set(0));
    prepare_project(&entry);
    for _ in 0..10 {
        assert!(!enabled(&entry));
        assert!(enabled(&root.join("nested/component.vue")));
    }
    MANIFEST_READS.with(|reads| assert_eq!(reads.get(), 2));
    {
        let _off = runtime_flag(Some("0"));
        assert!(!enabled(&root.join("nested/component.vue")));
    }
    {
        let _on = runtime_flag(Some("1"));
        assert!(enabled(&entry));
    }
    assert!(!enabled(&entry));
    std::fs::write(&package, r#"{"dependencies":{"@vitejs/plugin-vue":"1"}}"#).unwrap();
    assert!(!enabled(&entry));
    prepare_project(&entry);
    assert!(enabled(&entry));
    std::fs::write(&package, "{}").unwrap();
    prepare_project(&entry);
    assert!(!enabled(&entry));
    std::fs::remove_dir_all(root).unwrap();
}

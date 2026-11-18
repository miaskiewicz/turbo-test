use super::*;

#[test]
fn vue_defaults_off_without_manifest_reads_and_explicit_flags_remain_live() {
    let root = std::env::temp_dir().join(format!(
        "vue-explicit-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"devDependencies":{"@vitejs/plugin-vue":"1"}}"#,
    )
    .unwrap();
    let component = root.join("Component.vue");
    std::fs::write(&component, "<template><p>component</p></template>").unwrap();
    let _unset = runtime_flag(None);
    MANIFEST_READS.with(|reads| reads.set(0));
    for _ in 0..100 {
        assert!(!enabled(&component));
        assert!(!enabled(&root.join("react.tsx")));
    }
    MANIFEST_READS.with(|reads| assert_eq!(reads.get(), 0));
    {
        let _on = runtime_flag(Some("1"));
        assert!(enabled(&component));
        {
            let _off = runtime_flag(Some("0"));
            assert!(!enabled(&component));
        }
        assert!(enabled(&component));
    }
    assert!(!enabled(&component));
    std::fs::remove_dir_all(root).unwrap();
}

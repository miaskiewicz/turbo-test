use super::*;
use std::path::PathBuf;

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("tt-vite-asset-{}", std::process::id()));
    std::fs::create_dir_all(d.join("a")).unwrap();
    let d = d.canonicalize().unwrap();
    std::fs::write(d.join("a/t.txt"), "hi\n").unwrap();
    std::fs::write(d.join("a/m.ts"), "export {};").unwrap();
    std::fs::write(
        d.join("a/s.svg"),
        "<svg>\n  <g fill=\"#f\"> 5% </g>\n</svg>\n",
    )
    .unwrap();
    std::fs::write(d.join("a/big.svg"), "<svg>".repeat(1000)).unwrap();
    std::fs::write(
        d.join("a/lfs.svg"),
        "version https://git-lfs.github.com/spec/v1\n",
    )
    .unwrap();
    d
}

#[test]
fn values_follow_vite() {
    let d = dir();
    let v = |id: &str| value(&d.join(id), &d);
    assert_eq!(v("a/t.txt?raw").as_deref(), Some("hi\n"));
    assert_eq!(v("a/m.ts?raw&x").as_deref(), Some("export {};"));
    assert_eq!(v("a/t.txt").as_deref(), Some("/a/t.txt"));
    assert_eq!(v("a/t.txt?url").as_deref(), Some("/a/t.txt"));
    assert_eq!(v("a/t.txt?v=1&url").as_deref(), Some("/a/t.txt?v=1"));
    assert_eq!(v("a/t.txt?raw=true").as_deref(), Some("/a/t.txt?raw=true"));
    assert_eq!(v("a/m.ts?url").as_deref(), Some("/a/m.ts"));
    assert_eq!(v("a/m.ts"), None);
    assert_eq!(v("a/m.ts?x=1"), None);
    assert_eq!(v("a/nope.txt?raw"), None);
    assert_eq!(
        v("a/s.svg").as_deref(),
        Some("data:image/svg+xml,%3csvg%3e%3cg%20fill='%23f'%3e%205%25%20%3c/g%3e%3c/svg%3e")
    );
    assert_eq!(
        v("a/s.svg?no-inline").as_deref(),
        Some("/a/s.svg?no-inline")
    );
    assert_eq!(v("a/big.svg").as_deref(), Some("/a/big.svg"));
    assert_eq!(v("a/lfs.svg").as_deref(), Some("/a/lfs.svg"));
    assert_eq!(
        value(&d.join("a/t.txt"), Path::new("/elsewhere")).unwrap(),
        format!(
            "/@fs/{}/a/t.txt",
            crate::coverage::slash_path(&d).trim_start_matches('/')
        )
    );
    assert!(is_asset_id(Path::new("/x/p.PNG")));
    assert!(is_asset_id(Path::new("/x/m.ts?raw")));
    assert!(!is_asset_id(Path::new("/x/m.ts?rawx")));
    assert!(!is_asset_id(Path::new("/x/s.css")));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn svg_data_urls() {
    assert_eq!(
        svg_data_url(b"<svg a=\"it's\"/>"),
        "data:image/svg+xml;base64,PHN2ZyBhPSJpdCdzIi8+"
    );
    assert_eq!(
        svg_data_url(b"<svg><text>x</text></svg>"),
        "data:image/svg+xml;base64,PHN2Zz48dGV4dD54PC90ZXh0Pjwvc3ZnPg=="
    );
    assert_eq!(
        svg_data_url(b"<svg> x  <g/>\n</svg>"),
        "data:image/svg+xml,%3csvg%3e%20x%20%3cg/%3e%3c/svg%3e"
    );
    assert!(nested_quotes("'a\"b'") && !nested_quotes("\"ab\" 'c'") && !nested_quotes("\"a'"));
}

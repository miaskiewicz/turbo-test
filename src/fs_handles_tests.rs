use super::*;
use libc::{O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_SYNC, O_TRUNC, O_WRONLY};

#[test]
fn open_flag_strings_map_like_node_string_to_flags() {
    let write = O_TRUNC | O_CREAT;
    let append = O_APPEND | O_CREAT;
    let cases = [
        ("r", O_RDONLY),
        ("rs", O_RDONLY | O_SYNC),
        ("sr", O_RDONLY | O_SYNC),
        ("r+", O_RDWR),
        ("rs+", O_RDWR | O_SYNC),
        ("sr+", O_RDWR | O_SYNC),
        ("w", write | O_WRONLY),
        ("wx", write | O_WRONLY | O_EXCL),
        ("xw", write | O_WRONLY | O_EXCL),
        ("w+", write | O_RDWR),
        ("wx+", write | O_RDWR | O_EXCL),
        ("xw+", write | O_RDWR | O_EXCL),
        ("a", append | O_WRONLY),
        ("ax", append | O_WRONLY | O_EXCL),
        ("xa", append | O_WRONLY | O_EXCL),
        ("as", append | O_WRONLY | O_SYNC),
        ("sa", append | O_WRONLY | O_SYNC),
        ("a+", append | O_RDWR),
        ("ax+", append | O_RDWR | O_EXCL),
        ("xa+", append | O_RDWR | O_EXCL),
        ("as+", append | O_RDWR | O_SYNC),
        ("sa+", append | O_RDWR | O_SYNC),
    ];
    for (flags, expected) in cases {
        assert_eq!(open_flags(flags), Some(expected), "{flags}");
    }
    for invalid in ["", "zz", "R", "rw", "w++", "x"] {
        assert_eq!(open_flags(invalid), None, "{invalid}");
    }
}

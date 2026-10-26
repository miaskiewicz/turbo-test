use super::*;

#[test]
fn format_time_is_vitests() {
    assert_eq!(format_time(0.0), "0ms");
    assert_eq!(format_time(2.5), "3ms");
    assert_eq!(format_time(1000.0), "1000ms");
    assert_eq!(format_time(1000.4), "1.00s");
    assert_eq!(format_time(12_345.0), "12.35s");
}

#[test]
fn dividers_fill_thirty_columns() {
    assert_eq!(
        divider(" Failed Tests 3 ", None),
        "⎯⎯⎯⎯⎯⎯⎯ Failed Tests 3 ⎯⎯⎯⎯⎯⎯⎯"
    );
    // an odd remainder goes to the right
    assert_eq!(
        divider("abc", None),
        format!("{}abc{}", "⎯".repeat(13), "⎯".repeat(14))
    );
    assert_eq!(
        divider("[1/3]", Some(1)),
        format!("{}[1/3]⎯", "⎯".repeat(24))
    );
    // wider than the columns: no fill on the left
    assert_eq!(
        divider(&"x".repeat(40), Some(1)),
        format!("{}⎯", "x".repeat(40))
    );
}

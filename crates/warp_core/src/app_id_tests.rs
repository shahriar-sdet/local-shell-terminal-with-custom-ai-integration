use super::*;

#[test]
fn test_parse_valid_app_id() {
    let app_id_string = "com.example.App";
    let app_id = AppId::parse(app_id_string).expect("should not fail to parse");
    assert_eq!(app_id.qualifier(), "com");
    assert_eq!(app_id.organization(), "example");
    assert_eq!(app_id.application_name(), "App");
    assert_eq!(app_id_string, &app_id.to_string());
}

#[test]
fn test_parse_invalid_app_id() {
    assert!(
        AppId::parse("com.example").is_err(),
        "should fail to parse two-part app ID string"
    );
}

#[test]
fn test_parse_app_id_with_reverse_dns_qualifier() {
    let app_id_string = "io.github.someone.App";
    let app_id = AppId::parse(app_id_string).expect("should not fail to parse");
    assert_eq!(app_id.qualifier(), "io.github");
    assert_eq!(app_id.organization(), "someone");
    assert_eq!(app_id.application_name(), "App");
    assert_eq!(app_id_string, &app_id.to_string());
}

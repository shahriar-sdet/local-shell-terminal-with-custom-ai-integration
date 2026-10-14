use super::*;

// The guard is process-global, so the whole lifecycle is exercised in one test to keep it
// independent of test ordering when run under `cargo test`'s shared process.
#[test]
fn restricted_policy_only_allows_configured_origin() {
    let lm_studio = Url::parse("http://localhost:1234/v1").unwrap();
    let warp_server = Url::parse("https://app.warp.dev/ai/multi-agent").unwrap();

    assert!(is_url_allowed(&warp_server), "unrestricted by default");

    restrict();
    assert!(
        check_url(&lm_studio).is_err(),
        "nothing allowed until configured"
    );
    assert!(check_host("localhost").is_err());

    set_allowed_urls([&lm_studio]);
    assert!(check_url(&lm_studio).is_ok());
    assert!(check_url(&Url::parse("http://LOCALHOST:1234/v1/models").unwrap()).is_ok());
    assert!(check_host("localhost").is_ok());

    assert!(check_url(&Url::parse("http://localhost:8080/").unwrap()).is_err());
    assert!(check_url(&Url::parse("https://localhost:1234/").unwrap()).is_err());
    let blocked = check_url(&warp_server).unwrap_err();
    assert_eq!(blocked.target, "https://app.warp.dev");
    assert!(check_host("app.warp.dev").is_err());

    let recorded = recent_blocked_attempts();
    assert!(
        recorded
            .iter()
            .any(|attempt| attempt.target == "https://app.warp.dev")
    );
    assert!(
        recorded
            .iter()
            .all(|attempt| !attempt.target.contains("multi-agent")),
        "paths are never recorded"
    );

    let ip_endpoint = Url::parse("http://127.0.0.1:1234/v1").unwrap();
    set_allowed_urls([&ip_endpoint]);
    assert!(check_host("127.0.0.1").is_ok());
    assert!(check_host("localhost").is_err());
    assert_eq!(allowed_origins(), vec!["http://127.0.0.1:1234".to_owned()]);
}

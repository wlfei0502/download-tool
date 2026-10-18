use download_core::protocol::parse_dltool_url;

#[test]
fn parses_add() {
    let req = parse_dltool_url("dltool://add?url=http%3A%2F%2Fhost%2Fd%2Ft&fileName=a.zip&size=9")
        .unwrap();
    assert_eq!(req.url, "http://host/d/t");
    assert_eq!(req.file_name, "a.zip");
    assert_eq!(req.size, Some(9));
    assert_eq!(req.referrer, None);
    assert_eq!(req.user_agent, None);
}

#[test]
fn rejects_other_schemes() {
    let err = parse_dltool_url("https://add?url=http%3A%2F%2Fhost%2Fa&fileName=a.zip").unwrap_err();
    assert!(err.contains("dltool"), "{err}");
}

#[test]
fn rejects_non_add_actions() {
    let err =
        parse_dltool_url("dltool://remove?url=http%3A%2F%2Fhost%2Fa&fileName=a.zip").unwrap_err();
    assert!(err.contains("add"), "{err}");
}

#[test]
fn rejects_non_http_download_urls() {
    let err =
        parse_dltool_url("dltool://add?url=https%3A%2F%2Fhost%2Fa&fileName=a.zip").unwrap_err();
    assert!(err.contains("http"), "{err}");
}

#[test]
fn derives_a_file_name_when_file_name_is_omitted() {
    let req = parse_dltool_url("DLTOOL://add?url=http://example.com/from-scheme.bin").unwrap();
    assert_eq!(req.url, "http://example.com/from-scheme.bin");
    assert_eq!(req.file_name, "from-scheme.bin");
    assert_eq!(req.size, None);

    let directory = parse_dltool_url("dltool://add?url=http%3A%2F%2Fexample.com%2F").unwrap();
    assert_eq!(directory.file_name, "download");
}

#[test]
fn keeps_referrer_and_user_agent() {
    let req = parse_dltool_url(
        "dltool://add?url=http%3A%2F%2Fhost%2Fa&fileName=a.bin&referrer=http%3A%2F%2Fhost%2Ffiles&userAgent=Mozilla%2F5.0",
    )
    .unwrap();
    assert_eq!(req.referrer.as_deref(), Some("http://host/files"));
    assert_eq!(req.user_agent.as_deref(), Some("Mozilla/5.0"));
}

#[test]
fn decodes_a_file_name_taken_from_the_download_path() {
    let req = parse_dltool_url("dltool://add?url=http%3A%2F%2Fexample.com%2Fa%2520b.zip").unwrap();
    assert_eq!(req.file_name, "a b.zip");
}

#[test]
fn rejects_download_urls_with_userinfo() {
    let err =
        parse_dltool_url("dltool://add?url=http%3A%2F%2Fuser%3Apass%40host%2Fd%2Ft&fileName=a.zip")
            .unwrap_err();
    assert!(err.contains("user info"), "{err}");
}

#[test]
fn replaces_an_unsafe_file_name() {
    let req = parse_dltool_url("dltool://add?url=http%3A%2F%2Fhost%2Fd%2Ft&fileName=..%2Fsecret")
        .unwrap();
    assert_eq!(req.file_name, "t");
}

use download_core::aria2::rpc::Aria2Client;
use download_core::engine::Aria2Api;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn add_uri_returns_gid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_partial_json(json!({
            "method": "aria2.addUri",
            "params": [
                ["http://example.com/file.bin"],
                {
                    "dir": "/tmp",
                    "out": "file.bin",
                    "max-connection-per-server": "8",
                    "allow-overwrite": "false",
                    "auto-file-renaming": "true"
                }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": "2089b05ecca3d829"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Aria2Client::new(format!("{}/jsonrpc", server.uri()), None);
    let gid = Aria2Api::add_uri(
        &client,
        "http://example.com/file.bin",
        "/tmp",
        "file.bin",
        8,
    )
    .await
    .unwrap();
    assert_eq!(gid, "2089b05ecca3d829");
    server.verify().await;
}

#[tokio::test]
async fn change_global_option_sets_overall_download_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_partial_json(json!({
            "method": "aria2.changeGlobalOption",
            "params": [{
                "max-overall-download-limit": "1048576"
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": "OK"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Aria2Client::new(format!("{}/jsonrpc", server.uri()), None);
    Aria2Api::set_max_overall_download_limit(&client, Some(1_048_576))
        .await
        .expect("changeGlobalOption");
    server.verify().await;
}

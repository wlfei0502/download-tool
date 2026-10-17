use download_core::aria2::rpc::Aria2Client;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn add_uri_returns_gid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": "2089b05ecca3d829"
        })))
        .mount(&server)
        .await;

    let client = Aria2Client::new(format!("{}/jsonrpc", server.uri()), None);
    let gid = client
        .add_uri("http://example.com/file.bin", "/tmp", "file.bin")
        .await
        .unwrap();
    assert_eq!(gid, "2089b05ecca3d829");
}

use download_core::manager::TaskManager;
use download_core::task::NewTaskRequest;

#[test]
fn dedupes_same_url() {
    let mut mgr = TaskManager::new();
    let req = NewTaskRequest {
        url: "http://example.com/d/1".into(),
        file_name: "a.zip".into(),
        size: None,
        referrer: None,
        user_agent: None,
    };
    let a = mgr.add(req.clone()).expect("first");
    let b = mgr.add(req).expect("second");
    assert_eq!(a, b);
    assert_eq!(mgr.list().len(), 1);
}

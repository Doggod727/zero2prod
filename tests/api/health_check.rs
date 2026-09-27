//! tests/api/health_check.rs

use crate::helpers::spawn_app;

// 'tokio::test' 是 'tokio::main' 的测试等价物
// 它使得我们无需添加 '#[test]'
#[tokio::test]
async fn health_check_works() {
    // 准备
    let test_app = spawn_app().await;

    let client = reqwest::Client::new();

    // 执行
    let response = client
        .get(&format!("{}/health_check", test_app.address))
        .send()
        .await
        .expect("Failed to execute request.");

    // 断言
    assert!(response.status().is_success());
    assert_eq!(Some(0), response.content_length());
}

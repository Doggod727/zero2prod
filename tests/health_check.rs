use std::net::TcpListener;

// ! tests/health_check.rs
// 'tokio::test' 是 'tokio::main' 的测试等价物
// 它使得我们无需添加 '#[test]'
#[tokio::test]
async fn health_check_works() {
    // 准备
    let address = spawn_app();

    let client = reqwest::Client::new();

    // 执行
    let response = client
        .get(&format!("{}/health_check", address))
        .send()
        .await
        .expect("Failed to execute request.");

    // 断言
    assert!(response.status().is_success());
    assert_eq!(Some(0), response.content_length());
}

// 在后台某处启动应用程序
// spawn_app 是唯一合理依赖应用程序代码的部分。其他的一切测试都与底层实现细节无关。
fn spawn_app() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind random port");
    let port = listener.local_addr().unwrap().port(); // local_addr()获取端口
    // 如果直接调用run，由于HttpServer::run()返回一个Server，其不会主动关闭，我们的测试就不会结束
    // tokio::spawn方法就十分方便，其接受一个future，并交给运行时轮询，而无需等待其完成。
    let server = zero2prod::run(listener).expect("Failed to bind address");
    // 启动服务器作为后台任务
    // tokio::spawn返回一个指向spawned future的handle
    let _ = tokio::spawn(server);
    format!("http://127.0.0.1:{}", port)
}

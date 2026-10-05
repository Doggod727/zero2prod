use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
// ! tests/helpers.rs
use once_cell::sync::Lazy;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;
use wiremock::MockServer;
use zero2prod::configurations::{get_configurations, DatabaseSettings};
use zero2prod::startup::{Application};
use zero2prod::telemetry::{get_subscriber, init_subscriber};

// 使用once_cell确保tracing只能被初始化一次
static TRACING: Lazy<()> = Lazy::new(|| {
    let default_filter_level = "info".to_string();
    let subscriber_name = "test".to_string();
    // 由于'sink'是'get_subscriber'返回类型的一部分
    // 导致两个条件分支中'subscriber'的返回类型不一样
    // 因此没办法将其提取出来
    if std::env::var("TEST_LOG").is_ok() {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::stdout);
        init_subscriber(subscriber);
    } else {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::sink);
        init_subscriber(subscriber);
    }
});

/// 在发送给邮件API的请求中所包含的确认链接
pub struct ConfirmationLinks {
    pub html: reqwest::Url,
    pub plain_text: reqwest::Url,
}

pub struct TestUser {
    pub user_id: Uuid,
    pub username: String,
    pub password: String,
}

impl TestUser {
    pub fn generate() -> Self {
        Self {
            user_id: Uuid::new_v4(),
            username: Uuid::new_v4().to_string(),
            password: Uuid::new_v4().to_string()
        }
    }
    async fn store(&self, pool: &PgPool) {
        let salt = SaltString::generate(&mut rand::thread_rng());
        let password_hash = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(15000, 2, 1, None).unwrap(),
        )
        .hash_password(self.password.as_bytes(), &salt)
        .unwrap()
        .to_string();
        sqlx::query!(
            "INSERT INTO users (user_id, username, password_hash)
                VALUES($1, $2, $3)",
            self.user_id,
            self.username,
            password_hash
        )
        .execute(pool)
        .await
        .expect("Failed to store test user");
        dbg!(&self.user_id);
        dbg!(&password_hash);
    }

    pub async fn login(&self, app: &TestApp) {
        app.post_login(&serde_json::json!({
            "username": &self.username,
            "password": &self.password
        }))
        .await;
    }
}
pub struct TestApp {
    pub address: String,
    pub db_pool: PgPool,
    pub email_server: MockServer,
    pub port: u16,
    pub(crate) test_user: TestUser,
    pub api_client: reqwest::Client,
    /// 测试库名。Drop 时用它删除这个库。
    db_name: String,
    /// 不带库名的连接参数，用于连到维护库 postgres 执行 DROP DATABASE。
    db_options: sqlx::postgres::PgConnectOptions,
}

/// 测试结束后自动删掉本次用例的数据库。
///
/// 为什么需要它：`spawn_app` 每个用例都 `CREATE DATABASE`，不清理的话跑一次测试
/// 就泄漏几十个库。攒到上千个之后 Postgres 开始抖动，测试会以 `ConnectionReset` /
/// `PoolTimedOut` 这种和被测逻辑无关的方式随机失败——那是最难排查的一类失败。
///
/// 这里踩过四个坑，都写在注释里了：
///   1. `Drop` 是同步的、删库是异步的 → 需要桥接；
///   2. `Handle::current()` 必须在**当前线程**取。`std::thread::spawn` 出来的线程
///      不继承 tokio 运行时上下文，在新线程里调用它会 panic（no reactor running）；
///   3. **不能在 Drop 里用测试自己的运行时 block_on**：`#[tokio::test]` 默认单线程，
///      Drop 阻塞等待、而清理又要靠同一个运行时推进 → 互相等，死锁。
///      所以删库用**独立的运行时**放在**独立线程**里跑；
///   4. **必须 join 这个线程**。分离线程会在测试进程退出时被直接杀掉，
///      表现就是"写了清理，库却一个没少"。
///
/// 另外**不要依赖 `pool.close()`**：application 的后台任务也握着这个池的连接，
/// close 会一直等它们归还。这里改成从 Postgres 侧 `pg_terminate_backend` 踢掉残留会话。
impl Drop for TestApp {
    fn drop(&mut self) {
        let db_name = self.db_name.clone();
        let options = self.db_options.clone();
        let pool = self.db_pool.clone();

        // A) 在测试自己的运行时上异步关掉池（不阻塞），尽量让连接先还回去
        tokio::runtime::Handle::current().spawn(async move {
            pool.close().await;
        });

        // B) 删库放在独立线程 + 独立运行时，并 join 等它完成
        let db_name_for_thread = db_name.clone();
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to build a runtime for test database cleanup");

            runtime.block_on(async move {
                let mut connection = PgConnection::connect_with(&options)
                    .await
                    .expect("Failed to connect to Postgres while dropping test database");

                // 踢掉还挂在这个测试库上的会话（app 后台任务 + 上面的池）
                connection
                    .execute(
                        format!(
                            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
                            db_name_for_thread
                        )
                        .as_str(),
                    )
                    .await
                    .expect("Failed to terminate connections to test database");

                // DROP DATABASE 必须自成一条语句（不能和上面的 SELECT 同批）
                if let Err(e) = connection
                    .execute(format!("DROP DATABASE IF EXISTS \"{}\"", db_name_for_thread).as_str())
                    .await
                {
                    eprintln!("Failed to drop test database {db_name_for_thread}: {e}");
                }
            });
        });

        // 等清理线程收工（最多等 5 秒，避免个别情况挂住整个测试进程）
        let _ = handle.join();
    }
}

impl TestApp {
    pub async fn post_subscriptions(&self, body: String) -> reqwest::Response {
        self.api_client
            .post(&format!("{}/subscriptions", &self.address))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .expect("Failed to execute request.")
    }

    /// GET /subscriptions —— 渲染订阅表单（也是 POST 之后 303 的落点）
    pub async fn get_subscriptions(&self) -> reqwest::Response {
        self.api_client
            .get(&format!("{}/subscriptions", &self.address))
            .send()
            .await
            .expect("Failed to execute request.")
    }

    pub async fn get_subscriptions_html(&self) -> String {
        self.get_subscriptions().await.text().await.unwrap()
    }

    /// 从发送给邮件API的邮件中提取出确认连接
    pub fn get_confirmation_links(&self, email_request: &wiremock::Request) -> ConfirmationLinks {
        let body: serde_json::Value = serde_json::from_slice(&email_request.body).unwrap(); // 服务器发送的http请求的请求体

        // 从指定连接中提取连接
        let get_links = |s: &str| {
            let links: Vec<_> = linkify::LinkFinder::new()
                .links(s)
                .filter(|l| *l.kind() == linkify::LinkKind::Url)
                .collect();
            assert_eq!(links.len(), 1);
            let raw_link = links[0].as_str().to_owned();
            let mut confirmation_link = reqwest::Url::parse(&raw_link).unwrap();
            assert_eq!(confirmation_link.host_str().unwrap(), "127.0.0.1");
            confirmation_link.set_port(Some(self.port)).unwrap();
            confirmation_link
        };

        let html = get_links(&body["HtmlBody"].as_str().unwrap());
        let plain_text = get_links(&body["TextBody"].as_str().unwrap());
        ConfirmationLinks { html, plain_text }
    }

    pub async fn post_newsletters(&self, body: serde_json::Value) -> reqwest::Response {
        self.api_client
            .post(&format!("{}/admin/newsletters", &self.address))
            .form(&body)
            .send()
            .await
            .expect("Failed to execute request.")
    }

    pub async fn get_newsletters(&self) -> reqwest::Response {
        self.api_client
            .get(&format!("{}/admin/newsletters", &self.address))
            .send()
            .await
            .expect("Failed to execute request.")
    }

    pub async fn get_newsletters_html(&self) -> String {
        self.get_newsletters().await.text().await.unwrap()
    }

    pub async fn post_login<Body>(&self, body: &Body) -> reqwest::Response
    where
        Body: serde::Serialize,
    {
        // reqwest自动处理重定向
        // 提交无效表单 -> 登录错误303
        // 重定向到GET /login, 返回200
        self.api_client
            .post(&format!("{}/login", &self.address))
            .form(body)
            .send()
            .await
            .expect("Failed to execute request.")
    }

    pub async fn get_login_html(&self) -> String {
        self.api_client
            .get(&format!("{}/login", &self.address))
            .send()
            .await
            .expect("Failed to execute request.")
            .text()
            .await
            .unwrap()
    }

    pub async fn get_admin_dashboard(&self) -> reqwest::Response {
        self.api_client
            .get(&format!("{}/admin/dashboard", &self.address))
            .send()
            .await
            .expect("Failed to execute request.")
    }
    pub async fn get_admin_dashboard_html(&self) -> String {
        self.get_admin_dashboard().await.text().await.unwrap()
    }

    pub async fn get_change_password(&self) -> reqwest::Response {
        self.api_client
            .get(&format!("{}/admin/password", &self.address))
            .send()
            .await
            .expect("Failed to execute request.")
    }

    pub async fn post_change_password<Body>(&self, body: &Body) -> reqwest::Response
        where
            Body: serde::Serialize {
        self.api_client
            .post(&format!("{}/admin/password", &self.address))
            .form(&body)
            .send()
            .await
            .expect("Failed to execute request")
    }
    pub async fn get_change_password_html(&self) -> String {
        self.get_change_password().await.text().await.unwrap()
    }
    
    pub async fn post_logout(&self) -> reqwest::Response {
        self.api_client
            .post(&format!("{}/admin/logout", &self.address))
            .send()
            .await
            .expect("Failed to execute request")
    }
}
// 在后台某处启动应用程序
// spawn_app 是唯一合理依赖应用程序代码的部分。其他的一切测试都与底层实现细节无关。
pub async fn spawn_app() -> TestApp {
    Lazy::force(&TRACING);

    let email_server = MockServer::start().await;

    // 为了测试的隔离性，随机化配置
    let configuration = {
        let mut c = get_configurations().expect("Failed to read configuration"); // 获取配置
                                                                                 // 为每一个测试获取不同的数据库
        c.database.database_name = Uuid::new_v4().to_string();
        // 使用系统提供的随机端口
        c.application.port = 0;
        c.email_client.base_url = email_server.uri();
        c
    };

    // 创建并迁移数据库
    let (db_pool, db_name) = configure_database(&configuration.database).await;

    let application = Application::build(configuration.clone())
        .await
        .expect("Failed to build application");
    let application_port = application.port();
    // 在应用启动之前获取端
    // 启动服务器作为后台任务
    // tokio::spawn返回一个指向spawned future的handle
    let _ = tokio::spawn(application.run_until_stopped());
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .cookie_store(true)
        .build()
        .unwrap();
    let test_app = TestApp {
        address: format!("http://localhost:{}", application_port),
        db_pool,
        email_server,
        port: application_port,
        test_user: TestUser::generate(),
        api_client: client,
        db_name,
        // 不带库名，Drop 时连到维护库 postgres 上删库
        db_options: configuration.database.without_db(),
    };
    test_app.test_user.store(&test_app.db_pool).await;
    test_app
}
async fn configure_database(config: &DatabaseSettings) -> (PgPool, String) {
    // 创建数据库
    let mut connection = PgConnection::connect_with(&config.without_db())
        .await
        .expect("Failed to connect to Postgres");
    connection
        .execute(format!(r#"CREATE DATABASE "{}";"#, config.database_name).as_str())
        .await
        .expect("Failed to create database");
    // 迁移数据库
    let connection_pool = PgPool::connect_with(config.with_db())
        .await
        .expect("Failed to connect to Postgres");
    sqlx::migrate!("./migrations")
        .run(&connection_pool)
        .await
        .expect("Failed to migrate the database");
    (connection_pool, config.database_name.clone())
}

pub fn assert_is_redirect_to(response: &reqwest::Response, location: &str) {
    assert_eq!(response.status().as_u16(), 303);
    assert_eq!(response.headers().get("LOCATION").unwrap(), location);
}

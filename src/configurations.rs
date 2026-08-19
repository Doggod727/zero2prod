//! src/configurations.rs
use secrecy::{ExposeSecret, Secret};
#[derive(serde::Deserialize)]
pub struct Settings {
    pub database: DatabaseSettings, // 数据库链接配置
    pub application: ApplicationSettings, // 应用端口
}

#[derive(serde::Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    pub password: Secret<String>, // Secret通过反序列化逻辑委托给包装类型实现了Deserialize
    pub port: u16,
    pub host: String,
    pub database_name: String,
}
#[derive(serde::Deserialize)]
pub struct ApplicationSettings {
    pub port: u16,
    pub host: String,
}

// 读取配置信息
pub fn get_configurations() -> Result<Settings, config::ConfigError> {
    let base_path = std::env::current_dir().expect("Failed to determine the current directory");
    let configuration_directory = base_path.join("configuration");

    // 检查运行时环境
    // 如果没有指定，默认时local
    let environment: Environment = std::env::var("APP_ENVIRONMENT")
        .unwrap_or_else(|_| "local".into())
        .try_into()
        .expect("Failed to parse APP_ENVIRONMENT");
    let environment_file = format!("{}.yaml", environment.as_str());
    // 初始化配置读取器
    let settings = config::Config::builder()
        // 从一个叫做'configurations.yaml'的文件中读取配置值
        .add_source(config::File::from(configuration_directory.join("base.yaml")))
        .add_source(config::File::from(configuration_directory.join(&environment_file)))
        .build()?;
    // 尝试将读取的类型转化为Settings类型
    settings.try_deserialize::<Settings>()
}

// 使用PgConnection::connect进行数据库链接， 提供一个URL
impl DatabaseSettings {
    pub fn connection_string(&self) -> Secret<String> {
        Secret::new(format!(
            "postgres://{}:{}@{}:{}/{}",
            self.username, self.password.expose_secret(), self.host, self.port, self.database_name
        ))
    }

    pub fn connection_string_without_db(&self) -> Secret<String> {
        Secret::new(format!("postgres://{}:{}@{}:{}", self.username, self.password.expose_secret(), self.host, self.port))
    }
}

/// 应用程序可能的运行时环境
pub enum Environment {
    Local,
    Production,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Local => "local",
            Environment::Production => "production"
        }
    }
}

impl TryFrom<String> for Environment {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "production" => Ok(Self::Production),
            other => Err(
                format!("{} is not a supported environment. Use either 'local' or 'production'.", other)
            ),
        }
    }
}
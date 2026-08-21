//! src/configurations.rs
use secrecy::{ExposeSecret, Secret};
use serde::Deserialize;
use serde_aux::field_attributes::deserialize_number_from_string;
use sqlx::ConnectOptions;
use sqlx::postgres::PgConnectOptions;
use sqlx::postgres::PgSslMode;
use crate::domain::SubscriberEmail;

#[derive(serde::Deserialize)]
pub struct Settings {
    pub database: DatabaseSettings, // 数据库链接配置
    pub application: ApplicationSettings, // 应用端口
    pub email_client: EmailClientSettings,
}

#[derive(serde::Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    pub password: Secret<String>, // Secret通过反序列化逻辑委托给包装类型实现了Deserialize
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
    pub host: String,
    pub database_name: String,
    // 确定是否要加密链接
    pub require_ssl: bool,
}
#[derive(serde::Deserialize)]
pub struct ApplicationSettings {
    #[serde(deserialize_with = "deserialize_number_from_string")]
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
        // 从环境变量中添加设置(前缀为APP, _为分割符)
        .add_source(config::Environment::with_prefix("APP").prefix_separator("_").separator("__"))
        .build()?;
    // 尝试将读取的类型转化为Settings类型
    settings.try_deserialize::<Settings>()
}

// 使用PgConnection::connect进行数据库链接， 提供一个URL
impl DatabaseSettings {
    pub fn with_db(&self) -> PgConnectOptions {
        let mut options = self.without_db().database(&self.database_name);
        options.log_statements(tracing::log::LevelFilter::Trace);
        options
    }

    pub fn without_db(&self) -> PgConnectOptions {
        let ssl_mode = if self.require_ssl {
            PgSslMode::Require
        } else {
            // 尝试加密链接，如果失败，则回退到未加密的链接
            PgSslMode::Prefer
        };
        PgConnectOptions::new()
            .host(&self.host)
            .username(&self.username)
            .password(self.password.expose_secret())
            .port(self.port)
            .ssl_mode(ssl_mode)
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

#[derive(Deserialize)]
pub struct EmailClientSettings {
    pub base_url: String,
    pub sender: String,
    pub authorization_token: Secret<String>,
    pub timeout_milliseconds: u64,
}

impl EmailClientSettings {
    pub fn sender(&self) -> Result<SubscriberEmail, String> {
        Ok(SubscriberEmail::parse(self.sender.clone())?)
    }
    
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.timeout_milliseconds)
    }
}